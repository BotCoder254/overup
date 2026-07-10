//! Workspace-wide browser WebSocket endpoint (`/ws/workspaces/{ws}/dashboard`).
//!
//! Feeds the Dashboard and Runner Management pages live deltas. Like
//! `browser_ws.rs`, this lives outside the CSRF layer and defends itself
//! before upgrading:
//!   1. strict Origin allow-list (CORS does not protect WebSockets),
//!   2. authentication — a one-time `?ticket=` (deployments whose SPA proxy
//!      can't forward upgrades; see services/ws_ticket.rs) OR the session
//!      cookie (same-origin; the CurrentUser code path) — never a fallback
//!      from one to the other,
//!   3. workspace RBAC (`content.read`), re-checked even on the ticket path.
//!
//! There is deliberately no step 4 "resource must exist" check the way the
//! pipeline WS has one: this channel is workspace-wide, so membership
//! (step 3) is the entire authorization boundary — there's no single
//! resource below the workspace to scope to.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::db;
use crate::services::{authz, session};
use crate::state::AppState;

/// Upgrade-time query parameters shared by the browser WS endpoints.
#[derive(Debug, Deserialize)]
pub struct WsQuery {
    /// One-time ticket minted by POST /api/workspaces/{ws}/ws-ticket.
    pub ticket: Option<String>,
}

/// Resolve the connecting user: ticket XOR cookie. When a ticket is present
/// it is consumed (burned) and the cookie is never consulted — a replayed or
/// expired ticket yields a clean 401 the client fixes by minting a fresh one.
pub async fn authenticate_browser(
    state: &AppState,
    headers: &HeaderMap,
    ticket: Option<&str>,
    workspace_id: Uuid,
) -> Option<Uuid> {
    if let Some(ticket) = ticket {
        return state.ws_tickets.consume(ticket, workspace_id);
    }
    let jar = axum_extra::extract::cookie::CookieJar::from_headers(headers);
    let token = jar.get(&state.config.cookie_name)?.value().to_string();
    db::sessions::find_valid_user(&state.pool, &session::hash_token(&token))
        .await
        .ok()
        .flatten()
        .map(|user| user.id)
}

/// Browsers only send tiny control frames.
const MAX_BROWSER_FRAME: usize = 4 * 1024;

/// Server-side keepalive: detects half-open sockets without waiting for a
/// client message (browsers answer protocol pings automatically).
const SERVER_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMsg {
    /// Application-level keepalive; the client uses the pong to detect
    /// half-open sockets (browser APIs expose no native ping).
    Ping {},
}

/// GET /ws/workspaces/{workspace_id}/dashboard
pub async fn connect(
    State(state): State<AppState>,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // 1. Origin allow-list FIRST — before a ticket can even be burned. A
    // cross-site page can open a WebSocket with the victim's cookies; the
    // Origin header is the reliable defense.
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

    // 3. Workspace RBAC; flat 403, existence never leaks. Re-checked on the
    // ticket path too, so membership revoked inside the 60 s window denies.
    if authz::require_permission(&state.pool, user_id, workspace_id, authz::CONTENT_READ)
        .await
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }

    ws.max_message_size(MAX_BROWSER_FRAME)
        .on_upgrade(move |socket| handle(state, workspace_id, socket))
}

async fn handle(state: AppState, workspace_id: Uuid, socket: WebSocket) {
    // Subscribe BEFORE sending the snapshot so no transition can fall into
    // the gap between them.
    let mut events = state.workspace_hub.subscribe(workspace_id);

    let (mut sink, mut stream) = socket.split();

    // The snapshot is deliberately a bare sentinel, not a duplicate of the
    // REST summary/runners/pipelines payloads: the Dashboard and Runners
    // pages already fetch their initial state over REST on mount, so this
    // socket only needs to patch deltas from here on.
    if sink
        .send(Message::Text(r#"{"type":"snapshot"}"#.to_string().into()))
        .await
        .is_err()
    {
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
                    // corrected by the next event or a normal REST refetch,
                    // so there is no gap-resync frame to send here.
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
