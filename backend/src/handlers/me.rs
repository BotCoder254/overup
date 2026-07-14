use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::user::MeResponse;
use crate::services::session;
use crate::state::AppState;

pub async fn get_me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> AppResult<Json<MeResponse>> {
    let workspace = db::workspaces::find_summary_for_user(&state.pool, user.id).await?;
    Ok(Json(MeResponse::from_user(user, workspace)))
}

/// Explicit allow-list DTO (OWASP API3 — no mass assignment): only the
/// display name and email are user-controllable. The avatar mirrors GitHub,
/// the username IS the GitHub login; unknown fields are rejected outright.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateMeRequest {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

const DISPLAY_NAME_MAX_CHARS: usize = 80;
const EMAIL_MAX_LEN: usize = 254;

/// NFC-normalize + whitespace-collapse a display name; 1-80 chars, no
/// control characters (same shape as the workspace name validator).
fn validate_display_name(raw: &str) -> AppResult<String> {
    let normalized: String = raw.nfc().collect();
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = collapsed.chars().count();
    if len == 0 {
        return Err(AppError::Validation("display name cannot be empty".into()));
    }
    if len > DISPLAY_NAME_MAX_CHARS {
        return Err(AppError::Validation(format!(
            "display name must be at most {DISPLAY_NAME_MAX_CHARS} characters"
        )));
    }
    if collapsed.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "display name contains unsupported characters".into(),
        ));
    }
    Ok(collapsed)
}

/// Pragmatic, deliberately generic email check: single `@`, non-empty local
/// part, dotted domain, no whitespace/control characters, RFC length cap.
/// The error never reveals whether an address exists anywhere.
fn validate_email(raw: &str) -> AppResult<String> {
    let email = raw.trim();
    let valid = email.len() <= EMAIL_MAX_LEN
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
        && matches!(email.split_once('@'), Some((local, domain))
            if !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'));
    if !valid {
        return Err(AppError::Validation("invalid email address".into()));
    }
    Ok(email.to_string())
}

/// PATCH /api/me — explicit profile edit. Edited fields flip their
/// `*_customized` flags so future GitHub logins stop mirroring them.
pub async fn update_me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Json(body): Json<UpdateMeRequest>,
) -> AppResult<Json<MeResponse>> {
    let display_name = body
        .display_name
        .as_deref()
        .map(validate_display_name)
        .transpose()?;
    let email = body.email.as_deref().map(validate_email).transpose()?;

    if display_name.is_none() && email.is_none() {
        return Err(AppError::Validation("nothing to update".into()));
    }

    let updated =
        db::users::update_profile(&state.pool, user.id, display_name.as_deref(), email.as_deref())
            .await?
            .ok_or(AppError::Unauthorized)?;

    let workspace = db::workspaces::find_summary_for_user(&state.pool, updated.id).await?;

    // Audit into the user's workspace ledger (metadata lists the fields
    // touched, never the values). Users without a workspace have no ledger
    // to write to — the tracing record still exists.
    if let Some(ws) = &workspace {
        let fields: Vec<&str> = [
            display_name.is_some().then_some("displayName"),
            email.is_some().then_some("email"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
        sqlx::query(
            r#"
            INSERT INTO audit_logs
                (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
            VALUES ($1, $2, 'user.profile_updated', 'user', $2, $3, $4)
            "#,
        )
        .bind(ws.id)
        .bind(updated.id)
        .bind(serde_json::json!({ "fields": fields }))
        .bind(request_id)
        .execute(&state.pool)
        .await?;
    }

    tracing::info!(user_id = %updated.id, "profile updated");
    Ok(Json(MeResponse::from_user(updated, workspace)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAccountRequest {
    confirm_username: String,
}

/// DELETE /api/me — destroy the account, its workspace, and everything the
/// workspace owns (cascades). Requires retyping the exact username. The
/// session cookie is cleared in the response; every other session row
/// cascades away with the user.
pub async fn delete_me(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    jar: CookieJar,
    Json(body): Json<DeleteAccountRequest>,
) -> AppResult<Response> {
    if body.confirm_username != user.username {
        return Err(AppError::Validation(
            "confirmation does not match your username".into(),
        ));
    }

    let logo_keys = db::users::delete_account(&state.pool, user.id).await?;

    // Best-effort R2 cleanup of workspace logos; row state is already final.
    if let Some(r2) = state.r2.as_ref() {
        for key in logo_keys {
            if let Err(error) = r2.delete_object(&key).await {
                tracing::warn!(%key, error = ?error, "failed to delete logo object for deleted account");
            }
        }
    }

    // The workspace's audit ledger was explicitly deleted with the account
    // (retention policy: full removal), so the durable record of the
    // deletion itself lives in tracing.
    tracing::info!(user_id = %user.id, username = %user.username, "account deleted");

    let cleared = session::build_clear_cookie(&state.config);
    Ok((jar.add(cleared), StatusCode::NO_CONTENT).into_response())
}
