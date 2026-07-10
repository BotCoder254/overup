//! Browser WebSocket endpoint (`/ws/workspaces/{ws}/pipelines/{p}`).
//!
//! Native WebSockets can't send custom headers, so this endpoint lives
//! outside the CSRF layer and defends itself before upgrading:
//!   1. strict Origin allow-list (CORS does not protect WebSockets),
//!   2. session-cookie authentication (the CurrentUser code path),
//!   3. workspace RBAC (`content.read`),
//!   4. the pipeline must exist in that workspace (404 otherwise).
//!
//! Only then is the connection upgraded, a snapshot sent, and the
//! pipeline's broadcast channel attached.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::db;
use crate::models::pipeline::{PipelineJobResponse, PipelineResponse};
use crate::models::user::User;
use crate::services::log_hub::BrowserEvent;
use crate::services::{authz, session};
use crate::state::AppState;

/// Browsers only send tiny control frames.
const MAX_BROWSER_FRAME: usize = 4 * 1024;
/// Log backfill batch size per subscribe_logs request.
const BACKFILL_BATCH: i64 = 2000;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    #[serde(rename_all = "camelCase")]
    SubscribeLogs {
        job_id: Uuid,
        #[serde(default)]
        from_seq: i64,
    },
    /// Application-level keepalive; the client uses the pong to detect
    /// half-open sockets (browser APIs expose no native ping).
    Ping {},
}

/// Server-side keepalive: detects half-open sockets without waiting for a
/// client message (browsers answer protocol pings automatically).
const SERVER_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// GET /ws/workspaces/{workspace_id}/pipelines/{pipeline_id}
pub async fn connect(
    State(state): State<AppState>,
    Path((workspace_id, pipeline_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // 1. Origin allow-list. A cross-site page can open a WebSocket with the
    // victim's cookies; the Origin header is the reliable defense.
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|origin| origin == state.config.frontend_url);
    if !origin_ok {
        return StatusCode::FORBIDDEN.into_response();
    }

    // 2. Session cookie -> live user (the CurrentUser code path).
    let Some(user) = authenticate(&state, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // 3. Workspace RBAC; flat 403, existence never leaks.
    if authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ)
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    // 4. The pipeline must live in this workspace.
    match db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }

    ws.max_message_size(MAX_BROWSER_FRAME)
        .on_upgrade(move |socket| handle(state, workspace_id, pipeline_id, socket))
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Option<User> {
    let jar = axum_extra::extract::cookie::CookieJar::from_headers(headers);
    let token = jar.get(&state.config.cookie_name)?.value().to_string();
    db::sessions::find_valid_user(&state.pool, &session::hash_token(&token))
        .await
        .ok()
        .flatten()
}

async fn handle(state: AppState, workspace_id: Uuid, pipeline_id: Uuid, socket: WebSocket) {
    // Subscribe BEFORE reading the snapshot so no transition can fall into
    // the gap between them.
    let mut events = state.log_hub.subscribe(pipeline_id);

    let (mut sink, mut stream) = socket.split();

    let snapshot = match build_snapshot(&state, workspace_id, pipeline_id).await {
        Ok(Some(snapshot)) => snapshot,
        _ => {
            let _ = sink.close().await;
            return;
        }
    };
    if sink.send(Message::Text(snapshot.into())).await.is_err() {
        return;
    }

    let mut ping_interval = tokio::time::interval(SERVER_PING_INTERVAL);
    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping_interval.tick().await; // the first tick fires immediately; skip it

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            event = events.recv() => {
                let payload = match event {
                    Ok(event) => serde_json::to_string(&event).ok(),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Fell behind: tell the client to resync over REST.
                        serde_json::to_string(&BrowserEvent::LogGap { job_id: None }).ok()
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(payload) = payload else { continue };
                if sink.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            inbound = stream.next() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(msg) = serde_json::from_str::<ClientMsg>(&text) else {
                            break; // unparseable input closes the socket
                        };
                        if handle_client_msg(&state, pipeline_id, msg, &mut sink).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

async fn build_snapshot(
    state: &AppState,
    workspace_id: Uuid,
    pipeline_id: Uuid,
) -> anyhow::Result<Option<String>> {
    let Some(row) =
        db::pipelines::find_row_for_workspace(&state.pool, workspace_id, pipeline_id).await?
    else {
        return Ok(None);
    };
    let jobs: Vec<PipelineJobResponse> =
        db::pipeline_jobs::list_for_pipeline(&state.pool, pipeline_id)
            .await?
            .into_iter()
            .map(PipelineJobResponse::from)
            .collect();

    Ok(Some(serde_json::to_string(&json!({
        "type": "snapshot",
        "pipeline": PipelineResponse::from(row),
        "jobs": jobs,
    }))?))
}

/// Log backfill: stream stored chunks for one job of THIS pipeline. The
/// pipeline scope was authorized at upgrade; the job must belong to it.
async fn handle_client_msg(
    state: &AppState,
    pipeline_id: Uuid,
    msg: ClientMsg,
    sink: &mut (impl SinkExt<Message> + Unpin),
) -> Result<(), ()> {
    match msg {
        ClientMsg::Ping {} => {
            sink.send(Message::Text(r#"{"type":"pong"}"#.to_string().into()))
                .await
                .map_err(|_| ())?;
        }
        ClientMsg::SubscribeLogs { job_id, from_seq } => {
            let job = db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline_id, job_id)
                .await
                .map_err(|_| ())?;
            let Some(job) = job else {
                return Ok(()); // unknown job: ignore quietly
            };

            let mut cursor = from_seq.max(0);
            loop {
                let chunks =
                    db::pipeline_logs::fetch_range(&state.pool, job.id, cursor, BACKFILL_BATCH)
                        .await
                        .map_err(|_| ())?;
                if chunks.is_empty() {
                    break;
                }
                let done = (chunks.len() as i64) < BACKFILL_BATCH;
                for chunk in chunks {
                    cursor = cursor.max(chunk.seq.saturating_add(1));
                    let event = json!({
                        "type": "log",
                        "jobId": job.id,
                        "seq": chunk.seq,
                        "stream": chunk.stream,
                        "text": chunk.content,
                        "createdAt": chunk.created_at,
                    });
                    let payload = serde_json::to_string(&event).map_err(|_| ())?;
                    sink.send(Message::Text(payload.into()))
                        .await
                        .map_err(|_| ())?;
                }
                if done {
                    break;
                }
            }
        }
    }
    Ok(())
}
