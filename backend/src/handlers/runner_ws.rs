//! Runner WebSocket endpoint (`/runner/ws`).
//!
//! Runners hold one persistent outbound connection to the control plane —
//! the server never dials execution machines. Authentication happens before
//! the upgrade via `Authorization: Bearer <token>` (hash lookup against
//! non-revoked runners; same non-guessable-token design as sessions).
//! Every job-scoped message is validated against the job's actual
//! assignment — a runner can only ever affect jobs assigned to it.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use protocol::{RunnerMsg, ServerMsg};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

use crate::db;
use crate::models::runner::{Runner, RunnerResponse};
use crate::services::log_hub::BrowserEvent;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::services::{pipeline_run, session};
use crate::state::AppState;

/// Refetch a runner and publish its current state to the workspace-wide live
/// feed. Used at connect/disconnect (low-frequency, once-per-connection
/// events) — the heartbeat health path publishes the already-sanitized
/// health directly instead, to avoid a query on every 30s tick.
async fn publish_runner(state: &AppState, workspace_id: Uuid, runner_id: Uuid) {
    if let Ok(Some(runner)) = db::runners::find_by_id(&state.pool, workspace_id, runner_id).await {
        state.workspace_hub.publish(
            workspace_id,
            WorkspaceEvent::RunnerUpdate { runner: RunnerResponse::from(runner) },
        );
    }
}

/// Largest frame accepted from a runner (log chunks dominate).
const MAX_RUNNER_FRAME: usize = 128 * 1024;
const HEARTBEAT_INTERVAL_SECS: u64 = 30;
/// The first frame must be a hello within this window.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Static error categories a runner may report; anything else is coerced so
/// runner-supplied text never lands in the database.
const KNOWN_ERROR_CATEGORIES: &[&str] = &[
    "step_failed",
    "image_pull_failed",
    "container_error",
    "checkout_failed",
    "artifact_upload_failed",
    "timeout",
    "cancelled",
    "internal",
];

/// Ceilings for runner-reported metrics. Values above the cap are dropped
/// (not clamped, not rejected): a lying runner loses the field, never the
/// job result.
const MAX_METRIC_DURATION_MS: u64 = 7 * 24 * 3600 * 1000; // one week
const MAX_METRIC_PERMILLE: u64 = 1_024_000; // 1024 cores
const MAX_METRIC_BYTES: u64 = 1 << 40; // 1 TiB
const MAX_METRIC_SAMPLES: u32 = 100_000;

/// Validate runner telemetry into the camelCase object stored on the job
/// row and echoed in the `job.metrics` event. Returns None when nothing
/// survives validation.
fn sanitize_metrics(metrics: &protocol::JobMetrics) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    let mut put = |key: &str, value: Option<u64>, cap: u64| {
        if let Some(v) = value
            && v <= cap
        {
            map.insert(key.to_string(), serde_json::json!(v));
        }
    };
    put("imagePullMs", metrics.image_pull_ms, MAX_METRIC_DURATION_MS);
    put("execMs", metrics.exec_ms, MAX_METRIC_DURATION_MS);
    put("cpuPeakPermille", metrics.cpu_peak_permille, MAX_METRIC_PERMILLE);
    put("cpuAvgPermille", metrics.cpu_avg_permille, MAX_METRIC_PERMILLE);
    put("memPeakBytes", metrics.mem_peak_bytes, MAX_METRIC_BYTES);
    put("netRxBytes", metrics.net_rx_bytes, MAX_METRIC_BYTES);
    put("netTxBytes", metrics.net_tx_bytes, MAX_METRIC_BYTES);
    put("blkioReadBytes", metrics.blkio_read_bytes, MAX_METRIC_BYTES);
    put("blkioWriteBytes", metrics.blkio_write_bytes, MAX_METRIC_BYTES);
    put(
        "sampleCount",
        metrics.sample_count.map(u64::from),
        u64::from(MAX_METRIC_SAMPLES),
    );
    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map))
    }
}

/// Ceilings for runner-reported host health. Same "clamp or drop, never
/// trust free text" posture as [`sanitize_metrics`].
const MAX_HEALTH_PERMILLE: u64 = 1_024_000; // 1024 cores
const MAX_HEALTH_BYTES: u64 = 1 << 40; // 1 TiB
const MAX_HEALTH_UPTIME_SECS: u64 = 10 * 365 * 24 * 3600; // 10 years
const MAX_HEALTH_STRING_LEN: usize = 64;

/// Length-cap and strip control characters from a runner-supplied string
/// field before it can ever reach storage or the browser.
fn sanitize_health_string(value: &str) -> Option<String> {
    let cleaned: String = value.chars().filter(|c| !c.is_control()).collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_HEALTH_STRING_LEN).collect())
}

fn sanitize_health(health: &protocol::RunnerHealth) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();
    let mut put_num = |key: &str, value: Option<u64>, cap: u64| {
        if let Some(v) = value
            && v <= cap
        {
            map.insert(key.to_string(), serde_json::json!(v));
        }
    };
    put_num("cpuPermille", health.cpu_permille, MAX_HEALTH_PERMILLE);
    put_num("memUsedBytes", health.mem_used_bytes, MAX_HEALTH_BYTES);
    put_num("memTotalBytes", health.mem_total_bytes, MAX_HEALTH_BYTES);
    put_num("diskUsedBytes", health.disk_used_bytes, MAX_HEALTH_BYTES);
    put_num("diskTotalBytes", health.disk_total_bytes, MAX_HEALTH_BYTES);
    put_num("uptimeSecs", health.uptime_secs, MAX_HEALTH_UPTIME_SECS);
    if let Some(v) = health.docker_version.as_deref().and_then(sanitize_health_string) {
        map.insert("dockerVersion".to_string(), serde_json::json!(v));
    }
    if let Some(v) = health.os.as_deref().and_then(sanitize_health_string) {
        map.insert("os".to_string(), serde_json::json!(v));
    }
    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map))
    }
}

/// GET /runner/ws
pub async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let (runner, is_bootstrap) = match authenticate(&state, &headers).await {
        Ok(pair) => pair,
        // Static category strings only (never dynamic detail) — the runner
        // logs the body so operators can tell an expired registration token
        // from a revoked/rotated one.
        Err(reason) => return (StatusCode::UNAUTHORIZED, reason).into_response(),
    };

    ws.max_message_size(MAX_RUNNER_FRAME)
        .on_upgrade(move |socket| handle(state, runner, is_bootstrap, socket))
}

/// Bearer token -> SHA-256 -> non-revoked runner row. Tries the permanent
/// credential first, then falls back to a still-valid bootstrap credential
/// (the guided-wizard registration flow) — the returned bool marks which.
/// Failures carry a static 401 category for the runner's logs.
async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<(Runner, bool), &'static str> {
    let Some(token) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return Err("missing_token");
    };
    if token.is_empty() || token.len() > 128 {
        return Err("invalid_token");
    }
    let hash = session::hash_token(token);
    if let Some(runner) = db::runners::find_by_token_hash(&state.pool, &hash).await.ok().flatten() {
        return Ok((runner, false));
    }
    if let Some(runner) = db::runners::find_by_bootstrap_token_hash(&state.pool, &hash)
        .await
        .ok()
        .flatten()
    {
        return Ok((runner, true));
    }
    if db::runners::bootstrap_token_hash_expired(&state.pool, &hash)
        .await
        .unwrap_or(false)
    {
        return Err("bootstrap_expired");
    }
    Err("invalid_token")
}

async fn handle(state: AppState, runner: Runner, is_bootstrap: bool, socket: WebSocket) {
    let runner_id = runner.id;
    let workspace_id = runner.workspace_id;
    let (mut sink, mut stream) = socket.split();

    // First frame must be hello.
    let hello = tokio::time::timeout(HELLO_TIMEOUT, stream.next()).await;
    let version = match hello {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<RunnerMsg>(&text) {
                Ok(RunnerMsg::Hello { version, labels, name, .. }) => {
                    tracing::info!(%runner_id, runner = %name, ?labels, "runner connected");
                    version
                }
                _ => {
                    let _ = sink.close().await;
                    return;
                }
            }
        }
        _ => {
            let _ = sink.close().await;
            return;
        }
    };

    if db::runners::mark_connected(&state.pool, runner_id, &version)
        .await
        .is_err()
    {
        let _ = sink.close().await;
        return;
    }
    publish_runner(&state, workspace_id, runner_id).await;

    // A bootstrap-authenticated connection must leave with a permanent
    // credential — the bootstrap token is one-time and already close to
    // being cleared server-side.
    let permanent_token = if is_bootstrap {
        let (token, token_hash) = session::generate_token();
        match db::runners::exchange_bootstrap_token(&state.pool, runner_id, &token_hash).await {
            Ok(true) => Some(token),
            Ok(false) => {
                // Lost a race with another connection using the same
                // bootstrap token, or it was already exchanged/expired.
                tracing::warn!(%runner_id, "bootstrap token exchange failed; closing");
                let _ = sink.close().await;
                return;
            }
            Err(error) => {
                tracing::warn!(%runner_id, error = ?error, "bootstrap token exchange error");
                let _ = sink.close().await;
                return;
            }
        }
    } else {
        None
    };

    let (mut outbound, conn) = state.runner_hub.register(runner_id);

    let ack = ServerMsg::HelloAck {
        runner_id,
        heartbeat_interval_secs: HEARTBEAT_INTERVAL_SECS,
        protocol_version: protocol::PROTOCOL_VERSION,
        permanent_token,
    };
    if send_msg(&mut sink, &ack).await.is_err() {
        state.runner_hub.unregister_conn(runner_id, &conn);
        return;
    }

    // Fresh capacity may unblock queued work.
    state.scheduler.poke();

    // job_id -> pipeline_id for jobs this connection is executing; avoids a
    // lookup per log chunk. Authoritative checks still hit the database for
    // state transitions.
    let mut active_jobs: HashMap<Uuid, Uuid> = HashMap::new();
    let mut ping = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            server_msg = outbound.recv() => {
                let Some(server_msg) = server_msg else { break };
                let disconnect = matches!(server_msg, ServerMsg::Error { .. });
                if send_msg(&mut sink, &server_msg).await.is_err() {
                    break;
                }
                if disconnect {
                    break;
                }
            }
            _ = ping.tick() => {
                if send_msg(&mut sink, &ServerMsg::Ping).await.is_err() {
                    break;
                }
            }
            inbound = stream.next() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(msg) = serde_json::from_str::<RunnerMsg>(&text) else {
                            tracing::debug!(%runner_id, "unparseable runner frame; closing");
                            break;
                        };
                        if let Err(error) =
                            on_runner_msg(&state, workspace_id, runner_id, msg, &mut active_jobs).await
                        {
                            tracing::warn!(%runner_id, error = ?error, "runner message handling failed");
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        let _ = db::runners::touch_last_seen(&state.pool, runner_id).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // binary frames are not part of the protocol
                    Some(Err(_)) => break,
                }
            }
        }
    }

    // Cleanup: this connection only. A newer connection for the same runner
    // keeps its registration.
    state.runner_hub.unregister_conn(runner_id, &conn);
    if !state.runner_hub.is_connected(runner_id) {
        tracing::info!(%runner_id, "runner disconnected");
        let _ = db::runners::mark_offline(&state.pool, runner_id).await;
        if let Err(error) = pipeline_run::orphan_runner_jobs(&state, runner_id).await {
            tracing::warn!(%runner_id, error = ?error, "orphan recovery failed");
        }
        state.scheduler.poke();
        publish_runner(&state, workspace_id, runner_id).await;
    }
}

async fn send_msg(
    sink: &mut (impl SinkExt<Message> + Unpin),
    msg: &ServerMsg,
) -> Result<(), ()> {
    let Ok(json) = serde_json::to_string(msg) else {
        return Err(());
    };
    sink.send(Message::Text(json.into())).await.map_err(|_| ())
}

async fn on_runner_msg(
    state: &AppState,
    workspace_id: Uuid,
    runner_id: Uuid,
    msg: RunnerMsg,
    active_jobs: &mut HashMap<Uuid, Uuid>,
) -> anyhow::Result<()> {
    match &msg {
        RunnerMsg::Heartbeat { health: Some(health), .. } => {
            match sanitize_health(health) {
                Some(sanitized) => {
                    db::runners::touch_last_seen_with_health(&state.pool, runner_id, &sanitized)
                        .await?;
                    // No refetch here — the sanitized health JSON is already
                    // in hand, and this fires on a hot 30s-per-runner cadence.
                    state.workspace_hub.publish(
                        workspace_id,
                        WorkspaceEvent::RunnerHealth {
                            runner_id,
                            health: sanitized,
                            last_seen_at: chrono::Utc::now(),
                        },
                    );
                }
                None => db::runners::touch_last_seen(&state.pool, runner_id).await?,
            }
        }
        _ => db::runners::touch_last_seen(&state.pool, runner_id).await?,
    }

    match msg {
        RunnerMsg::Hello { .. } | RunnerMsg::Heartbeat { .. } => {}
        RunnerMsg::JobAck { job_id } => {
            if let Some(job) = db::pipeline_jobs::ack(&state.pool, job_id, runner_id).await? {
                active_jobs.insert(job.id, job.pipeline_id);
                pipeline_run::on_job_started(state, &job).await?;
            }
        }
        RunnerMsg::JobStage { job_id, stage, .. } => {
            // Stages come from a fixed vocabulary; anything else is dropped.
            if !protocol::STAGES.contains(&stage.as_str())
                || matches!(stage.as_str(), "queued" | "done")
            {
                return Ok(());
            }
            if let Some(job) =
                db::pipeline_jobs::set_stage(&state.pool, job_id, runner_id, &stage).await?
            {
                pipeline_run::on_job_stage(state, &job).await?;
            }
        }
        RunnerMsg::Log { job_id, seq, stream, text } => {
            let pipeline_id = match active_jobs.get(&job_id) {
                Some(pipeline_id) => *pipeline_id,
                None => {
                    // Cold path (e.g. logs before ack round-trip finished):
                    // verify the assignment authoritatively.
                    let Some(job) =
                        db::pipeline_jobs::find_assigned(&state.pool, job_id, runner_id).await?
                    else {
                        return Ok(());
                    };
                    active_jobs.insert(job.id, job.pipeline_id);
                    job.pipeline_id
                }
            };
            state
                .log_hub
                .ingest_log(
                    &state.pool,
                    pipeline_id,
                    job_id,
                    seq.min(i64::MAX as u64) as i64,
                    stream,
                    &text,
                    state.config.max_log_bytes_per_job,
                )
                .await?;
        }
        RunnerMsg::ArtifactRequest { job_id, name, size_bytes, content_type } => {
            handle_artifact_request(state, runner_id, job_id, name, size_bytes, content_type)
                .await?;
        }
        RunnerMsg::ArtifactDone { job_id, name, checksum_sha256, .. } => {
            handle_artifact_done(state, runner_id, job_id, name, checksum_sha256).await?;
        }
        RunnerMsg::JobResult { job_id, conclusion, exit_code, error_category, metrics } => {
            active_jobs.remove(&job_id);
            let category = error_category
                .as_deref()
                .map(|c| {
                    KNOWN_ERROR_CATEGORIES
                        .iter()
                        .find(|known| **known == c)
                        .copied()
                        .unwrap_or("runner_error")
                });
            let metrics_json = sanitize_metrics(&metrics);
            if let Some(job) = db::pipeline_jobs::finish_from_runner(
                &state.pool,
                job_id,
                runner_id,
                conclusion.as_str(),
                exit_code,
                category,
                metrics_json.as_ref(),
            )
            .await?
            {
                pipeline_run::record_event(
                    state,
                    job.pipeline_id,
                    Some(job.id),
                    "job.metrics",
                    None,
                    None,
                    Some(runner_id),
                    None,
                    metrics_json.unwrap_or_else(|| serde_json::json!({})),
                )
                .await?;
                pipeline_run::on_job_finished(state, &job).await?;
            }
        }
    }
    Ok(())
}

async fn handle_artifact_request(
    state: &AppState,
    runner_id: Uuid,
    job_id: Uuid,
    name: String,
    size_bytes: u64,
    content_type: String,
) -> anyhow::Result<()> {
    use crate::services::github_app::is_safe_name_segment;

    let deny = |reason: &str| {
        state.runner_hub.send(
            runner_id,
            ServerMsg::ArtifactDeny {
                job_id,
                name: name.clone(),
                reason: reason.to_string(),
            },
        );
    };

    // The job must genuinely be running on this runner.
    let Some(job) = db::pipeline_jobs::find_assigned(&state.pool, job_id, runner_id).await? else {
        deny("job is not assigned to this runner");
        return Ok(());
    };
    let Some(r2) = &state.r2 else {
        deny("artifact storage is not configured");
        return Ok(());
    };
    if !is_safe_name_segment(&name) {
        deny("invalid artifact name");
        return Ok(());
    }
    if size_bytes == 0 || size_bytes > state.config.max_artifact_bytes as u64 {
        deny("artifact exceeds the size limit");
        return Ok(());
    }
    let count = db::artifacts::count_for_job(&state.pool, job.id).await?;
    if count >= state.config.max_artifacts_per_job {
        deny("artifact count limit reached");
        return Ok(());
    }
    let content_type = if content_type.len() <= 128 && content_type.is_ascii() {
        content_type
    } else {
        "application/octet-stream".to_string()
    };

    let pipeline = db::pipelines::find_by_id(&state.pool, job.pipeline_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("pipeline vanished"))?;
    let key = crate::services::r2::R2::artifact_key(
        pipeline.workspace_id,
        pipeline.id,
        job.id,
        &name,
    );

    let artifact = db::artifacts::insert_pending(
        &state.pool,
        pipeline.workspace_id,
        pipeline.id,
        job.id,
        &name,
        &key,
        size_bytes as i64,
        &content_type,
        // Retention clock starts at upload request; the janitor deletes the
        // R2 object and flips the row to expired once it lapses.
        Some(chrono::Utc::now() + chrono::Duration::days(state.config.artifact_retention_days)),
    )
    .await?;

    let put_url = match r2.presign_put(&artifact.r2_key, &content_type).await {
        Ok(url) => url,
        Err(error) => {
            tracing::warn!(error = ?error, "artifact presign failed");
            deny("artifact storage error");
            return Ok(());
        }
    };

    state.runner_hub.send(
        runner_id,
        ServerMsg::ArtifactGrant {
            job_id,
            name,
            put_url,
            key: artifact.r2_key,
            expires_at: chrono::Utc::now()
                + chrono::Duration::from_std(crate::services::r2::UPLOAD_URL_TTL)
                    .unwrap_or(chrono::Duration::minutes(15)),
        },
    );
    Ok(())
}

async fn handle_artifact_done(
    state: &AppState,
    runner_id: Uuid,
    job_id: Uuid,
    name: String,
    checksum_sha256: String,
) -> anyhow::Result<()> {
    let Some(job) = db::pipeline_jobs::find_assigned(&state.pool, job_id, runner_id).await? else {
        return Ok(());
    };
    let Some(r2) = &state.r2 else {
        return Ok(());
    };
    if !crate::services::github_app::is_safe_name_segment(&name) {
        return Ok(());
    }
    let checksum = if checksum_sha256.len() == 64
        && checksum_sha256.chars().all(|c| c.is_ascii_hexdigit())
    {
        checksum_sha256.to_ascii_lowercase()
    } else {
        return Ok(());
    };

    let pipeline = db::pipelines::find_by_id(&state.pool, job.pipeline_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("pipeline vanished"))?;
    let key = crate::services::r2::R2::artifact_key(
        pipeline.workspace_id,
        pipeline.id,
        job.id,
        &name,
    );

    // Trust the bucket, not the runner: the object must exist and fit.
    let verified_size = match r2.head_size(&key).await {
        Ok(Some(size)) if size > 0 && size <= state.config.max_artifact_bytes => size,
        Ok(_) => {
            db::artifacts::mark_failed(&state.pool, job.id, &name).await?;
            return Ok(());
        }
        Err(error) => {
            tracing::warn!(error = ?error, "artifact verification failed");
            db::artifacts::mark_failed(&state.pool, job.id, &name).await?;
            return Ok(());
        }
    };

    if let Some(artifact) =
        db::artifacts::mark_uploaded(&state.pool, job.id, &name, verified_size, &checksum).await?
    {
        sqlx::query(
            r#"
            INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
            VALUES ($1, NULL, 'artifact.uploaded', 'artifact', $2, $3)
            "#,
        )
        .bind(pipeline.workspace_id)
        .bind(artifact.id)
        .bind(serde_json::json!({ "name": artifact.name, "sizeBytes": verified_size }))
        .execute(&state.pool)
        .await?;

        state.log_hub.publish(
            pipeline.id,
            BrowserEvent::Artifact {
                artifact: artifact.into(),
            },
        );
    }
    Ok(())
}
