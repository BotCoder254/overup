//! Per-user notification WebSocket (`/ws/workspaces/{ws}/notifications`).
//!
//! Powers the Notification Center bell on every page. Defends itself before
//! upgrading exactly like the dashboard WS (whose `authenticate_browser` it
//! reuses): strict Origin allow-list, ticket-XOR-cookie authentication, then
//! workspace RBAC. The channel it forwards is keyed by the AUTHENTICATED
//! user id — no subscriber can ever receive another user's notifications,
//! because routing (not filtering) is the isolation mechanism.
//!
//! The socket is a latency optimization only: the post-upgrade snapshot
//! carries the authoritative unread count, and clients resync over REST on
//! every (re)connect — a dropped frame is never a correctness problem.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::db;
use crate::handlers::dashboard_ws::{WsQuery, authenticate_browser};
use crate::services::authz;
use crate::state::AppState;

/// Browsers only send tiny control frames.
const MAX_BROWSER_FRAME: usize = 4 * 1024;
const SERVER_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    /// Application-level keepalive (browser APIs expose no native ping).
    Ping {},
}

/// GET /ws/workspaces/{workspace_id}/notifications
pub async fn connect(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // 1. Origin allow-list FIRST (CORS does not protect WebSockets).
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|origin| origin == state.config.frontend_url);
    if !origin_ok {
        return StatusCode::FORBIDDEN.into_response();
    }

    // 2. Ticket XOR session cookie -> live user id.
    let Some(user_id) =
        authenticate_browser(&state, &headers, query.ticket.as_deref(), workspace_id).await
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // 3. Workspace RBAC; flat 403, existence never leaks.
    if authz::require_permission(&state.pool, user_id, workspace_id, authz::CONTENT_READ)
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    ws.max_message_size(MAX_BROWSER_FRAME)
        .on_upgrade(move |socket| handle(state, workspace_id, user_id, socket))
}

async fn handle(state: AppState, workspace_id: Uuid, user_id: Uuid, socket: WebSocket) {
    // Subscribe BEFORE computing the snapshot so no notification can fall
    // into the gap between them (a duplicate frame is harmless — the client
    // patches by id).
    let mut events = state.notification_hub.subscribe(user_id);

    let (mut sink, mut stream) = socket.split();

    // Authoritative badge state up front; the feed itself loads over REST.
    let unread = db::notifications::unread_count(&state.pool, workspace_id, user_id)
        .await
        .unwrap_or(0);
    let snapshot = format!(r#"{{"type":"snapshot","unreadCount":{unread}}}"#);
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
                    // Idempotent-by-id deltas: a lagged subscriber is
                    // corrected by the next REST refetch.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
                        match msg {
                            ClientMsg::Ping {} => {
                                if sink
                                    .send(Message::Text(r#"{"type":"pong"}"#.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
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
