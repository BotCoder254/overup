//! Active-sessions management (Settings → Authentication).
//!
//! OWASP Session Management: revocation is server-side row deletion — a
//! revoked token fails `find_valid_user` on its next request and the client
//! is redirected to sign-in. Token hashes never leave the database; the
//! "current session" flag is computed in SQL against the hash of the cookie
//! the caller presented. IPs are masked server-side before they leave the API.

use std::net::IpAddr;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::services::session;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    /// Masked before it leaves the API (IPv4: last octet hidden; IPv6:
    /// first three hextets kept).
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub current: bool,
}

/// Mask an IP for display: enough to recognize "my office" vs "somewhere
/// else" without exposing the full address to other tabs/screens.
fn mask_ip(raw: &str) -> Option<String> {
    match raw.parse::<IpAddr>().ok()? {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            Some(format!("{}.{}.{}.x", o[0], o[1], o[2]))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            Some(format!("{:x}:{:x}:{:x}::…", s[0], s[1], s[2]))
        }
    }
}

/// The hash of the session token this request rode in on.
fn current_token_hash(state: &AppState, jar: &CookieJar) -> AppResult<String> {
    jar.get(&state.config.cookie_name)
        .map(|cookie| session::hash_token(cookie.value()))
        .ok_or(AppError::Unauthorized)
}

/// GET /api/me/sessions
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    jar: CookieJar,
) -> AppResult<Json<serde_json::Value>> {
    let current_hash = current_token_hash(&state, &jar)?;
    let rows = db::sessions::list_for_user(
        &state.pool,
        user.id,
        &current_hash,
        state.config.session_idle_timeout_hours,
    )
    .await?;
    let sessions: Vec<SessionResponse> = rows
        .into_iter()
        .map(|row| SessionResponse {
            id: row.id,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            expires_at: row.expires_at,
            ip: row.ip.as_deref().and_then(mask_ip),
            user_agent: row.user_agent,
            current: row.is_current,
        })
        .collect();
    Ok(Json(json!({ "sessions": sessions })))
}

/// Audit a session mutation into the user's workspace ledger (metadata only;
/// skipped when the user has no workspace — audit_logs requires one).
async fn audit_session_action(
    state: &AppState,
    user_id: Uuid,
    action: &str,
    metadata: serde_json::Value,
    headers: &HeaderMap,
) -> AppResult<()> {
    if let Some(ws) = db::workspaces::find_summary_for_user(&state.pool, user_id).await? {
        let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
        sqlx::query(
            r#"
            INSERT INTO audit_logs
                (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
            VALUES ($1, $2, $3, 'user', $2, $4, $5)
            "#,
        )
        .bind(ws.id)
        .bind(user_id)
        .bind(action)
        .bind(metadata)
        .bind(request_id)
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

/// DELETE /api/me/sessions/{session_id} — revoke one session. Revoking the
/// current session is allowed (the client handles the resulting 401 as a
/// sign-out). Not-found and not-yours are the same flat outcome.
pub async fn revoke(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(session_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    let deleted = db::sessions::delete_by_id_for_user(&state.pool, user.id, session_id).await?;
    if deleted == 0 {
        return Err(AppError::NotFound);
    }
    audit_session_action(&state, user.id, "session.revoked", json!({}), &headers).await?;
    tracing::info!(user_id = %user.id, "session revoked");
    Ok(Json(json!({ "revoked": true })))
}

/// POST /api/me/sessions/revoke-all — revoke every session except the one
/// making this request (the GitHub/Google "sign out other sessions" shape).
pub async fn revoke_all(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    jar: CookieJar,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    let current_hash = current_token_hash(&state, &jar)?;
    let revoked =
        db::sessions::delete_all_for_user_except(&state.pool, user.id, &current_hash).await?;
    audit_session_action(
        &state,
        user.id,
        "sessions.revoked_all",
        json!({ "count": revoked }),
        &headers,
    )
    .await?;
    tracing::info!(user_id = %user.id, count = revoked, "other sessions revoked");
    Ok(Json(json!({ "revoked": revoked })))
}
