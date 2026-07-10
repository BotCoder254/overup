//! One-time WebSocket ticket minting.
//!
//! Deployments whose SPA proxy cannot forward WebSocket upgrades (Netlify)
//! authenticate their sockets with a ticket instead of the session cookie:
//! the browser POSTs here first (this endpoint rides the proxy, so the
//! cookie, CSRF header, and RBAC all apply as usual), then presents the
//! returned ticket as `?ticket=...` on the direct-to-API upgrade GET, where
//! `dashboard_ws`/`browser_ws` redeem it. See `services/ws_ticket.rs` for
//! the credential properties (single-use, 60 s, hash-only, user+workspace
//! bound).

use axum::Json;
use axum::extract::{Path, State};
use serde_json::json;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::services::authz;
use crate::state::AppState;

/// POST /api/workspaces/{workspace_id}/ws-ticket
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    // content.read — a ticket only ever grants what the WS handlers grant
    // (read-only streams), and they re-check RBAC at redemption anyway.
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let ticket = state
        .ws_tickets
        .mint(user.id, workspace_id)
        .ok_or(AppError::Conflict("too many pending tickets"))?;

    Ok(Json(json!({ "ticket": ticket, "expiresInSeconds": 60 })))
}
