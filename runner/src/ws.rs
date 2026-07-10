//! Control-plane connection: authenticate, hello handshake, then pump
//! messages both ways. Job payloads are verified (constant-time HMAC +
//! validity window + runner binding) BEFORE they are parsed or executed —
//! a tampered payload never runs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use protocol::{RunnerMsg, ServerMsg};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::artifacts::GrantWaiters;
use crate::{ActiveJob, CurrentJob, RunnerConfig, executor};

const HEARTBEAT: Duration = Duration::from_secs(30);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const OUT_QUEUE: usize = 256;

pub enum Disconnect {
    /// Transient: reconnect with backoff.
    Lost,
    /// Terminal: the control plane revoked this runner.
    Revoked,
}

pub async fn run_connection(
    config: &RunnerConfig,
    docker: Option<bollard::Docker>,
) -> anyhow::Result<Disconnect> {
    // http -> ws, https -> wss.
    let ws_url = format!("{}/runner/ws", config.server_url.replacen("http", "ws", 1));
    let mut request = ws_url
        .into_client_request()
        .context("invalid OVERUP_URL")?;
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", config.token)
            .parse()
            .context("token contains invalid header characters")?,
    );

    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("websocket connect failed (is the control plane up? is the token valid?)")?;
    let (mut sink, mut stream) = socket.split();

    send(
        &mut sink,
        &RunnerMsg::Hello {
            name: config.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            labels: config.labels.clone(),
            docker_available: docker.is_some(),
        },
    )
    .await?;

    // The first server frame must be hello_ack carrying our identity.
    let (runner_id, heartbeat_secs) = tokio::time::timeout(HELLO_TIMEOUT, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<ServerMsg>(text.as_str()) {
                        Ok(ServerMsg::HelloAck {
                            runner_id,
                            heartbeat_interval_secs,
                            protocol_version,
                        }) => {
                            if protocol_version != protocol::PROTOCOL_VERSION {
                                anyhow::bail!(
                                    "protocol version mismatch: server {protocol_version}, runner {}",
                                    protocol::PROTOCOL_VERSION
                                );
                            }
                            return Ok((runner_id, heartbeat_interval_secs));
                        }
                        Ok(_) => continue,
                        Err(_) => anyhow::bail!("unparseable frame during handshake"),
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(error)) => return Err(error.into()),
                None => anyhow::bail!("connection closed during handshake"),
            }
        }
    })
    .await
    .context("hello_ack timed out")??;

    tracing::info!(%runner_id, "connected to control plane");

    let (out_tx, mut out_rx) = mpsc::channel::<RunnerMsg>(OUT_QUEUE);
    let grants = GrantWaiters::default();
    let current: CurrentJob = Arc::new(Mutex::new(None));
    // Honor the server's advertised cadence, clamped to something sane;
    // fall back to the local default when the value is out of range.
    let heartbeat_interval = if (5..=300).contains(&heartbeat_secs) {
        Duration::from_secs(heartbeat_secs)
    } else {
        HEARTBEAT
    };
    let mut heartbeat = tokio::time::interval(heartbeat_interval);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            outbound = out_rx.recv() => {
                // Executor side never closes: we hold out_tx.
                let Some(outbound) = outbound else { return Ok(Disconnect::Lost) };
                send(&mut sink, &outbound).await?;
            }
            _ = heartbeat.tick() => {
                let busy_job_id = current.lock().unwrap().as_ref().map(|j| j.job_id);
                send(&mut sink, &RunnerMsg::Heartbeat { busy_job_id }).await?;
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(msg) = serde_json::from_str::<ServerMsg>(text.as_str()) else {
                            tracing::warn!("unparseable server frame; ignoring");
                            continue;
                        };
                        if let Some(disconnect) = on_server_msg(
                            config, &docker, runner_id, msg, &out_tx, &grants, &current,
                        ) {
                            return Ok(disconnect);
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return Ok(Disconnect::Lost),
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(error = ?error, "websocket error");
                        return Ok(Disconnect::Lost);
                    }
                }
            }
        }
    }
}

fn on_server_msg(
    config: &RunnerConfig,
    docker: &Option<bollard::Docker>,
    runner_id: uuid::Uuid,
    msg: ServerMsg,
    out_tx: &mpsc::Sender<RunnerMsg>,
    grants: &GrantWaiters,
    current: &CurrentJob,
) -> Option<Disconnect> {
    match msg {
        ServerMsg::Ping => {
            let busy_job_id = current.lock().unwrap().as_ref().map(|j| j.job_id);
            let _ = out_tx.try_send(RunnerMsg::Heartbeat { busy_job_id });
        }
        ServerMsg::JobAssign { payload_json, signature_hex } => {
            // Integrity + freshness first; the payload is not even parsed
            // unless the signature over the exact bytes verifies.
            let Some(payload) = protocol::verify_job_payload(
                &config.signing_key,
                &payload_json,
                &signature_hex,
                Utc::now(),
            ) else {
                tracing::warn!("rejected job payload: invalid signature or expired");
                return None;
            };
            // The payload must have been issued to THIS runner.
            if payload.runner_id != runner_id {
                tracing::warn!("rejected job payload issued to a different runner");
                return None;
            }

            let mut slot = current.lock().unwrap();
            if slot.is_some() {
                tracing::warn!("received an assignment while busy; ignoring");
                return None;
            }
            let (cancel_tx, cancel_rx) = watch::channel(false);
            let job_id = payload.job_id;
            *slot = Some(ActiveJob { job_id, cancel: cancel_tx });
            drop(slot);

            let _ = out_tx.try_send(RunnerMsg::JobAck { job_id });
            tokio::spawn(executor::run_job(
                docker.clone(),
                payload,
                out_tx.clone(),
                cancel_rx,
                grants.clone(),
                config.isolation.clone(),
                current.clone(),
            ));
        }
        ServerMsg::JobCancel { job_id, reason } => {
            let slot = current.lock().unwrap();
            if let Some(active) = slot.as_ref()
                && active.job_id == job_id
            {
                tracing::info!(%job_id, ?reason, "cancelling job");
                let _ = active.cancel.send(true);
            }
        }
        ServerMsg::ArtifactGrant { job_id, name, put_url, .. } => {
            grants.resolve(job_id, &name, Ok(put_url));
        }
        ServerMsg::ArtifactDeny { job_id, name, reason } => {
            grants.resolve(job_id, &name, Err(reason));
        }
        ServerMsg::Error { code } => {
            return Some(if code == "revoked" {
                Disconnect::Revoked
            } else {
                Disconnect::Lost
            });
        }
        ServerMsg::HelloAck { .. } => {}
    }
    None
}

async fn send(
    sink: &mut (impl SinkExt<Message> + Unpin),
    msg: &RunnerMsg,
) -> anyhow::Result<()> {
    let json = serde_json::to_string(msg)?;
    sink.send(Message::Text(json.into()))
        .await
        .map_err(|_| anyhow::anyhow!("websocket send failed"))
}
