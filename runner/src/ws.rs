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
use crate::health::HealthSampler;
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
    new_token: &mut Option<String>,
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

    let (socket, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            let status = response.status();
            // The control plane attaches a static category ("invalid_token",
            // "bootstrap_expired", "missing_token") to auth rejections.
            let reason = response
                .body()
                .as_deref()
                .map(|body| String::from_utf8_lossy(body).trim().to_string())
                .unwrap_or_default();
            if status.as_u16() == 401 {
                if reason == "bootstrap_expired" {
                    tracing::error!(
                        "control plane rejected the token (401 bootstrap_expired): the \
                         registration token expired before its first use (1 hour limit) — \
                         register the runner again and use the fresh token"
                    );
                } else {
                    tracing::error!(
                        reason = %reason,
                        "control plane rejected the token (401): it was revoked, rotated, \
                         or was a one-time bootstrap token that has already been exchanged. \
                         Regenerate the token in the UI and update RUNNER_TOKEN — and set \
                         RUNNER_TOKEN_FILE so the exchanged permanent token survives restarts"
                    );
                }
            }
            anyhow::bail!("websocket connect rejected: HTTP {status} {reason}");
        }
        Err(error) => {
            return Err(error).context("websocket connect failed (is the control plane up?)");
        }
    };
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
    let (runner_id, heartbeat_secs, permanent_token) = tokio::time::timeout(HELLO_TIMEOUT, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<ServerMsg>(text.as_str()) {
                        Ok(ServerMsg::HelloAck {
                            runner_id,
                            heartbeat_interval_secs,
                            protocol_version,
                            permanent_token,
                        }) => {
                            if protocol_version != protocol::PROTOCOL_VERSION {
                                anyhow::bail!(
                                    "protocol version mismatch: server {protocol_version}, runner {}",
                                    protocol::PROTOCOL_VERSION
                                );
                            }
                            return Ok((runner_id, heartbeat_interval_secs, permanent_token));
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
    if let Some(token) = permanent_token {
        tracing::info!("received a permanent token from a bootstrap exchange");
        *new_token = Some(token);
    }

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
    let mut health_sampler = HealthSampler::new();
    // Refreshed on the heartbeat cadence; Ping-triggered replies reuse the
    // last sample rather than resampling on every server ping.
    let mut last_health: Option<protocol::RunnerHealth> = None;
    let ctx = MsgContext { config, docker: &docker, runner_id, out_tx: &out_tx, grants: &grants, current: &current };

    loop {
        tokio::select! {
            outbound = out_rx.recv() => {
                // Executor side never closes: we hold out_tx.
                let Some(outbound) = outbound else { return Ok(Disconnect::Lost) };
                send(&mut sink, &outbound).await?;
            }
            _ = heartbeat.tick() => {
                let busy_job_id = current.lock().unwrap().as_ref().map(|j| j.job_id);
                let health = health_sampler.sample(&docker).await;
                last_health = Some(health.clone());
                send(&mut sink, &RunnerMsg::Heartbeat { busy_job_id, health: Some(health) }).await?;
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(msg) = serde_json::from_str::<ServerMsg>(text.as_str()) else {
                            tracing::warn!("unparseable server frame; ignoring");
                            continue;
                        };
                        if let Some(disconnect) = on_server_msg(&ctx, msg, &last_health) {
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

/// Everything a connection-scoped inbound message handler needs, bundled to
/// keep the function signature within clippy's argument-count lint.
#[derive(Clone, Copy)]
struct MsgContext<'a> {
    config: &'a RunnerConfig,
    docker: &'a Option<bollard::Docker>,
    runner_id: uuid::Uuid,
    out_tx: &'a mpsc::Sender<RunnerMsg>,
    grants: &'a GrantWaiters,
    current: &'a CurrentJob,
}

fn on_server_msg(
    ctx: &MsgContext<'_>,
    msg: ServerMsg,
    last_health: &Option<protocol::RunnerHealth>,
) -> Option<Disconnect> {
    let MsgContext { config, docker, runner_id, out_tx, grants, current } = *ctx;
    match msg {
        ServerMsg::Ping => {
            let busy_job_id = current.lock().unwrap().as_ref().map(|j| j.job_id);
            let _ = out_tx.try_send(RunnerMsg::Heartbeat { busy_job_id, health: last_health.clone() });
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
        // Job assignment is always server-initiated; the runner never polls
        // for work. So there is nothing to gate here — this is purely
        // operator-visible logging.
        ServerMsg::LifecycleChanged { status } => match status {
            protocol::RunnerLifecycle::Disabled => {
                tracing::info!("disabled by operator — will not receive new jobs");
            }
            protocol::RunnerLifecycle::Draining => {
                tracing::info!("draining — finishing the current job, then going offline");
            }
            protocol::RunnerLifecycle::Resumed => {
                tracing::info!("resumed — eligible for new jobs again");
            }
        },
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
