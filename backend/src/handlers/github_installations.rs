use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::github_installation::InstallationResponse;
use crate::services::{authz, session};
use crate::state::AppState;

/// GitHub installation ids are u64-ish; anything longer is hostile.
const MAX_SETUP_PARAM_LEN: usize = 20;

#[derive(Debug, Deserialize)]
pub struct SetupParams {
    installation_id: Option<String>,
    setup_action: Option<String>,
}

/// GET /auth/github/app/setup — the GitHub App's Setup URL. GitHub sends the
/// browser here after an install/update with `installation_id` in the query.
/// The parameter is untrusted: it is verified against the GitHub API with an
/// app JWT before anything is persisted. All failures collapse into one
/// opaque redirect marker.
pub async fn setup(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(params): Query<SetupParams>,
) -> Response {
    let frontend = state.config.frontend_url.clone();
    let fail = |reason: &'static str| {
        tracing::warn!(reason, "github app setup rejected");
        Redirect::to(&format!("{frontend}/?error=install_failed")).into_response()
    };

    // Top-level GET navigation: the SameSite=Lax session cookie flows.
    let Some(token) = jar
        .get(&state.config.cookie_name)
        .map(|c| c.value().to_string())
    else {
        return fail("no session cookie");
    };
    let user = match db::sessions::find_valid_user(&state.pool, &session::hash_token(&token)).await
    {
        Ok(Some(user)) => user,
        Ok(None) => return fail("session invalid"),
        Err(error) => {
            tracing::error!(error = ?error, "setup session lookup failed");
            return fail("session lookup failed");
        }
    };

    // An org member *requested* an install: there is no installation yet and
    // nothing to verify or link — the org owner must approve first. Send the
    // user back with an informational marker rather than a failure.
    if params.setup_action.as_deref() == Some("request") {
        tracing::info!("github app install requested; awaiting org approval");
        return Redirect::to(&format!("{frontend}/?info=install_pending")).into_response();
    }

    let Some(raw_id) = params.installation_id else {
        return fail("missing installation_id");
    };
    if raw_id.len() > MAX_SETUP_PARAM_LEN
        || params
            .setup_action
            .as_deref()
            .is_some_and(|a| a.len() > MAX_SETUP_PARAM_LEN)
    {
        return fail("oversized parameters");
    }
    let Ok(installation_id) = raw_id.parse::<i64>() else {
        return fail("installation_id not numeric");
    };
    if installation_id <= 0 {
        return fail("installation_id out of range");
    }

    let Ok(Some(workspace)) = db::workspaces::find_summary_for_user(&state.pool, user.id).await
    else {
        return fail("user has no workspace");
    };
    // Linking grants the workspace access to repositories — a write.
    if authz::require_permission(&state.pool, user.id, workspace.id, authz::CONTENT_WRITE)
        .await
        .is_err()
    {
        return fail("permission denied");
    }

    // Server-side verification of the untrusted id: the installation must
    // belong to *this* app, and the current user must plausibly own it —
    // either the installation account is their GitHub account, or the
    // `installation.created` webhook recorded them as the installer.
    let installation = match state
        .github_app
        .get_installation(&state.http, installation_id)
        .await
    {
        Ok(installation) => installation,
        Err(error) => {
            tracing::warn!(error = ?error, "setup verification against github failed");
            return fail("installation verification failed");
        }
    };
    let account_matches = installation.account.login.eq_ignore_ascii_case(&user.username);
    let installer_matches = matches!(
        db::github_installations::find_event_sender(&state.pool, installation_id).await,
        Ok(Some(sender)) if sender.eq_ignore_ascii_case(&user.username)
    );
    if !account_matches && !installer_matches {
        return fail("installation not owned by user");
    }
    if installation.suspended_at.is_some() {
        return fail("installation suspended");
    }

    let linked = db::github_installations::link(
        &state.pool,
        workspace.id,
        installation_id,
        &installation.account.login,
        &installation.account.account_type,
        installation.account.avatar_url.as_deref(),
        user.id,
    )
    .await;

    match linked {
        Ok(db::github_installations::LinkOutcome::Linked(row)) => {
            if let Err(error) = sqlx::query(
                r#"
                INSERT INTO audit_logs
                    (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
                VALUES ($1, $2, 'installation.linked', 'github_installation', $3, $4)
                "#,
            )
            .bind(workspace.id)
            .bind(user.id)
            .bind(row.id)
            .bind(json!({ "accountLogin": row.account_login }))
            .execute(&state.pool)
            .await
            {
                tracing::error!(error = ?error, "failed to write installation audit log");
            }
            tracing::info!(
                workspace_id = %workspace.id,
                account = %row.account_login,
                "github app installation linked"
            );
            Redirect::to(&format!(
                "{frontend}/w/{}/repositories?installed=1",
                workspace.slug
            ))
            .into_response()
        }
        Ok(db::github_installations::LinkOutcome::ClaimedElsewhere) => {
            fail("installation claimed by another workspace")
        }
        Err(error) => {
            tracing::error!(error = ?error, "failed to persist installation link");
            fail("persistence failed")
        }
    }
}

/// GET /api/workspaces/{workspace_id}/installations
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let installations = db::github_installations::list_for_workspace(&state.pool, workspace_id)
        .await?
        .into_iter()
        .map(InstallationResponse::from)
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "installations": installations,
        "installUrl": format!(
            "https://github.com/apps/{}/installations/new",
            state.config.github_app_slug
        ),
    })))
}

/// DELETE /api/workspaces/{workspace_id}/installations/{installation_id}
pub async fn unlink(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, installation_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let Some(numeric_id) =
        db::github_installations::delete_for_workspace(&state.pool, workspace_id, installation_id)
            .await?
    else {
        return Err(AppError::NotFound);
    };
    state.github_app.evict_token(numeric_id).await;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'installation.unlinked', 'github_installation', $3, '{}'::jsonb, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(installation_id)
    .bind(request_id)
    .execute(&state.pool)
    .await?;

    tracing::info!(%workspace_id, "github app installation unlinked");
    Ok(StatusCode::NO_CONTENT.into_response())
}
