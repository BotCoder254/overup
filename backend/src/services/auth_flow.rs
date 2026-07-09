use anyhow::anyhow;
use axum_extra::extract::cookie::Cookie;
use chrono::{Duration, Utc};
use oauth2::{
    AuthorizationCode, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, Scope, TokenResponse,
};

use crate::db;
use crate::error::{AppError, AppResult};
use crate::services::{github, session};
use crate::state::AppState;

/// Pending login transactions are short-lived by design.
const STATE_TTL_MINUTES: i64 = 10;

/// Start the Authorization Code + PKCE flow: persist the transaction
/// (hashed state -> verifier) and return the GitHub authorize URL.
pub async fn begin_login(state: &AppState) -> AppResult<String> {
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let (authorize_url, csrf_state) = state
        .oauth
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("read:user".to_string()))
        .add_scope(Scope::new("user:email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    db::oauth_states::insert(
        &state.pool,
        &session::hash_token(csrf_state.secret()),
        pkce_verifier.secret(),
        Utc::now() + Duration::minutes(STATE_TTL_MINUTES),
    )
    .await?;

    Ok(authorize_url.to_string())
}

/// Complete the flow after GitHub redirects back. Validates the state
/// against the stored transaction, exchanges the code server-to-server,
/// mirrors the GitHub profile, rotates any pre-existing session, and
/// returns the new session cookie. Whether onboarding is still due is the
/// frontend's call via /api/me — workspace membership is the source of truth.
pub async fn complete_login(
    state: &AppState,
    code: String,
    oauth_state: String,
    previous_session_token: Option<String>,
) -> AppResult<Cookie<'static>> {
    // Single-use, expiring lookup: a replayed or forged `state` finds nothing.
    let verifier = db::oauth_states::take(&state.pool, &session::hash_token(&oauth_state))
        .await?
        .ok_or_else(|| AppError::OAuth(anyhow!("unknown, expired, or replayed state")))?;

    let token = state
        .oauth
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(PkceCodeVerifier::new(verifier))
        .request_async(&state.http)
        .await
        .map_err(|e| AppError::OAuth(anyhow!("code exchange failed: {e}")))?;

    let profile = github::fetch_profile(&state.http, token.access_token().secret())
        .await
        .map_err(AppError::OAuth)?;

    let user = db::users::upsert_by_github(&state.pool, &profile).await?;

    // Session rotation: authentication state changed, so any session the
    // browser was already carrying is invalidated before issuing a new one.
    if let Some(previous) = previous_session_token {
        db::sessions::delete_by_token_hash(&state.pool, &session::hash_token(&previous)).await?;
    }

    let cookie = session::create_session(&state.pool, &state.config, user.id).await?;

    tracing::info!(user_id = %user.id, username = %user.username, "user signed in via GitHub");

    Ok(cookie)
}
