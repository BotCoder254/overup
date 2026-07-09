use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;

use crate::error::AppResult;
use crate::services::{auth_flow, session};
use crate::state::AppState;

/// Upper bound for `code` and `state` callback parameters.
const MAX_OAUTH_PARAM_LEN: usize = 512;

/// GET /auth/github/login — full-page navigation entry point.
pub async fn login(State(state): State<AppState>) -> AppResult<Redirect> {
    let authorize_url = auth_flow::begin_login(&state).await?;
    Ok(Redirect::temporary(&authorize_url))
}

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// GET /auth/github/callback — the only URL GitHub is allowed to redirect
/// to. Whatever happens, the browser ends up back on the frontend; failures
/// carry a single opaque marker and nothing else.
pub async fn callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<CallbackParams>,
) -> Response {
    let frontend = &state.config.frontend_url;

    if let Some(error) = &params.error {
        // e.g. the user denied the authorization screen.
        tracing::warn!(oauth_error = %error, "github returned an error on callback");
        return Redirect::to(&format!("{frontend}/auth/callback?error=auth_failed"))
            .into_response();
    }

    let (Some(code), Some(oauth_state)) = (params.code, params.state) else {
        tracing::warn!("callback missing code or state parameter");
        return Redirect::to(&format!("{frontend}/auth/callback?error=auth_failed"))
            .into_response();
    };

    // Input validation before anything touches these values: GitHub codes
    // and our own state tokens are short; anything oversized is hostile.
    if code.len() > MAX_OAUTH_PARAM_LEN || oauth_state.len() > MAX_OAUTH_PARAM_LEN {
        tracing::warn!(
            code_len = code.len(),
            state_len = oauth_state.len(),
            "callback parameters exceed sane length limits"
        );
        return Redirect::to(&format!("{frontend}/auth/callback?error=auth_failed"))
            .into_response();
    }

    let previous_session_token = jar
        .get(&state.config.cookie_name)
        .map(|cookie| cookie.value().to_string());

    match auth_flow::complete_login(&state, code, oauth_state, previous_session_token).await {
        Ok(cookie) => {
            let destination = format!("{frontend}/auth/callback");
            (jar.add(cookie), Redirect::to(&destination)).into_response()
        }
        Err(error) => {
            // Full detail is logged by AppError semantics; the browser only
            // ever learns "it failed".
            tracing::error!(error = ?error, "github oauth callback failed");
            Redirect::to(&format!("{frontend}/auth/callback?error=auth_failed")).into_response()
        }
    }
}

/// POST /auth/logout — CSRF-guarded by the global X-Requested-With check.
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> AppResult<Response> {
    let cleared = match jar.get(&state.config.cookie_name) {
        Some(cookie) => {
            let value = cookie.value().to_string();
            let cleared = session::destroy_session(&state.pool, &state.config, &value).await?;
            tracing::info!("user signed out; session destroyed");
            cleared
        }
        None => session::build_clear_cookie(&state.config),
    };
    Ok((jar.add(cleared), StatusCode::NO_CONTENT).into_response())
}
