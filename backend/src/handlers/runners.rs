//! Minimal runner registration API. The full Runner Management module ships
//! later; this is just enough to register, list, and revoke the reference
//! runner. The registration token is returned exactly once — only its
//! SHA-256 hash is stored (the sessions pattern).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::runner::RunnerResponse;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::services::{authz, pipeline_run, session};
use crate::state::AppState;

/// Refetch a runner and publish its current state to the workspace-wide live
/// feed. Used after lifecycle actions (drain/disable/resume/revoke) whose DB
/// functions only return a bool, not the updated row. Silently does nothing
/// if the runner has vanished between the mutation and this call.
async fn publish_runner(state: &AppState, workspace_id: Uuid, runner_id: Uuid) {
    if let Ok(Some(runner)) = db::runners::find_by_id(&state.pool, workspace_id, runner_id).await {
        state.workspace_hub.publish(
            workspace_id,
            WorkspaceEvent::RunnerUpdate { runner: RunnerResponse::from(runner) },
        );
    }
}

const MAX_LABELS: usize = 16;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunnerRequest {
    name: String,
    #[serde(default)]
    labels: Vec<String>,
}

fn is_valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// POST /api/workspaces/{workspace_id}/runners
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateRunnerRequest>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    if !is_valid_name(&body.name) {
        return Err(AppError::Validation(
            "runner name must be 1-64 characters: letters, digits, '-', '_', '.'".into(),
        ));
    }
    if body.labels.len() > MAX_LABELS || body.labels.iter().any(|l| !is_valid_name(l)) {
        return Err(AppError::Validation(
            "labels must each be 1-64 characters: letters, digits, '-', '_', '.'".into(),
        ));
    }

    // Same construction as browser sessions: 32 OS-RNG bytes, hash stored,
    // value shown exactly once in this response.
    let (token, token_hash) = session::generate_token();

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let outcome = db::runners::create(
        &state.pool,
        workspace_id,
        &body.name,
        &body.labels,
        &token_hash,
        user.id,
        request_id,
    )
    .await?;

    let runner = match outcome {
        db::runners::CreateOutcome::Created(runner) => *runner,
        db::runners::CreateOutcome::NameTaken => {
            return Err(AppError::Conflict("a runner with this name already exists"));
        }
    };

    tracing::info!(%workspace_id, runner = %runner.name, "runner registered");
    let response = RunnerResponse::from(runner);
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::RunnerUpdate { runner: response.clone() },
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "runner": response,
            // Shown once; never retrievable again.
            "token": token,
        })),
    )
        .into_response())
}

/// GET /api/workspaces/{workspace_id}/runners
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let runners: Vec<RunnerResponse> = db::runners::list_for_workspace(&state.pool, workspace_id)
        .await?
        .into_iter()
        .map(RunnerResponse::from)
        .collect();
    Ok(Json(json!({ "runners": runners })))
}

/// POST /api/workspaces/{workspace_id}/runners/bootstrap
///
/// Guided-wizard registration: mints a short-lived bootstrap token instead
/// of a permanent one. The runner exchanges it for a permanent credential
/// on its first successful connection (see `runner_ws::authenticate`).
pub async fn bootstrap(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateRunnerRequest>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    if !is_valid_name(&body.name) {
        return Err(AppError::Validation(
            "runner name must be 1-64 characters: letters, digits, '-', '_', '.'".into(),
        ));
    }
    if body.labels.len() > MAX_LABELS || body.labels.iter().any(|l| !is_valid_name(l)) {
        return Err(AppError::Validation(
            "labels must each be 1-64 characters: letters, digits, '-', '_', '.'".into(),
        ));
    }

    let (token, token_hash) = session::generate_token();
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let outcome = db::runners::create_bootstrap(
        &state.pool,
        db::runners::CreateBootstrapParams {
            workspace_id,
            name: &body.name,
            labels: &body.labels,
            bootstrap_token_hash: &token_hash,
            bootstrap_expires_at: expires_at,
            created_by: user.id,
            request_id,
        },
    )
    .await?;

    let runner = match outcome {
        db::runners::CreateOutcome::Created(runner) => *runner,
        db::runners::CreateOutcome::NameTaken => {
            return Err(AppError::Conflict("a runner with this name already exists"));
        }
    };

    tracing::info!(%workspace_id, runner = %runner.name, "runner bootstrap started");
    let response = RunnerResponse::from(runner);
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::RunnerUpdate { runner: response.clone() },
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "runner": response,
            // Shown once; the runner exchanges it for a permanent token on
            // first connect.
            "token": token,
            "expiresAt": expires_at,
        })),
    )
        .into_response())
}

/// GET /api/workspaces/{workspace_id}/runners/{runner_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let runner = db::runners::find_by_id(&state.pool, workspace_id, runner_id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(json!({ "runner": RunnerResponse::from(runner) })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRunnerRequest {
    name: Option<String>,
    labels: Option<Vec<String>>,
}

/// PATCH /api/workspaces/{workspace_id}/runners/{runner_id}
pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<UpdateRunnerRequest>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    if let Some(name) = &body.name
        && !is_valid_name(name)
    {
        return Err(AppError::Validation(
            "runner name must be 1-64 characters: letters, digits, '-', '_', '.'".into(),
        ));
    }
    if let Some(labels) = &body.labels
        && (labels.len() > MAX_LABELS || labels.iter().any(|l| !is_valid_name(l)))
    {
        return Err(AppError::Validation(
            "labels must each be 1-64 characters: letters, digits, '-', '_', '.'".into(),
        ));
    }

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let outcome = db::runners::update(
        &state.pool,
        workspace_id,
        runner_id,
        body.name.as_deref(),
        body.labels.as_deref(),
        user.id,
        request_id,
    )
    .await?;

    let runner = match outcome {
        db::runners::UpdateOutcome::Updated(runner) => *runner,
        db::runners::UpdateOutcome::NameTaken => {
            return Err(AppError::Conflict("a runner with this name already exists"));
        }
        db::runners::UpdateOutcome::NotFound => return Err(AppError::NotFound),
    };

    let response = RunnerResponse::from(runner);
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::RunnerUpdate { runner: response.clone() },
    );
    Ok(Json(json!({ "runner": response })))
}

/// POST /api/workspaces/{workspace_id}/runners/{runner_id}/regenerate-token
///
/// Deliberately does not publish to the workspace hub: rotating the token
/// doesn't change any status field visible in `RunnerResponse`, so there's
/// nothing for the Dashboard/Runners live feed to reflect.
pub async fn regenerate_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let (token, token_hash) = session::generate_token();
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let rotated = db::runners::regenerate_token(
        &state.pool,
        workspace_id,
        runner_id,
        &token_hash,
        user.id,
        request_id,
    )
    .await?;
    if !rotated {
        return Err(AppError::NotFound);
    }

    // The old credential must stop working immediately; the runner is not
    // being removed, just told to reconnect with its new one.
    state
        .runner_hub
        .send(runner_id, protocol::ServerMsg::Error { code: "token_rotated".into() });
    state.runner_hub.unregister(runner_id);

    tracing::info!(%workspace_id, %runner_id, "runner token regenerated");
    Ok((StatusCode::OK, Json(json!({ "token": token }))).into_response())
}

/// POST /api/workspaces/{workspace_id}/runners/{runner_id}/drain
pub async fn drain(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let drained =
        db::runners::drain_start(&state.pool, workspace_id, runner_id, user.id, request_id).await?;
    if !drained {
        return Err(AppError::NotFound);
    }

    state.runner_hub.send(
        runner_id,
        protocol::ServerMsg::LifecycleChanged { status: protocol::RunnerLifecycle::Draining },
    );
    publish_runner(&state, workspace_id, runner_id).await;
    tracing::info!(%workspace_id, %runner_id, "runner draining");
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// POST /api/workspaces/{workspace_id}/runners/{runner_id}/disable
pub async fn disable(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let disabled =
        db::runners::disable(&state.pool, workspace_id, runner_id, user.id, request_id).await?;
    if !disabled {
        return Err(AppError::NotFound);
    }

    state.runner_hub.send(
        runner_id,
        protocol::ServerMsg::LifecycleChanged { status: protocol::RunnerLifecycle::Disabled },
    );
    publish_runner(&state, workspace_id, runner_id).await;
    tracing::info!(%workspace_id, %runner_id, "runner disabled");
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// POST /api/workspaces/{workspace_id}/runners/{runner_id}/resume
pub async fn resume(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let connected = state.runner_hub.is_connected(runner_id);
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let resumed = db::runners::resume(
        &state.pool,
        workspace_id,
        runner_id,
        connected,
        user.id,
        request_id,
    )
    .await?;
    if !resumed {
        return Err(AppError::NotFound);
    }

    if connected {
        state.runner_hub.send(
            runner_id,
            protocol::ServerMsg::LifecycleChanged { status: protocol::RunnerLifecycle::Resumed },
        );
    }
    state.scheduler.poke();
    publish_runner(&state, workspace_id, runner_id).await;
    tracing::info!(%workspace_id, %runner_id, "runner resumed");
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// DELETE /api/workspaces/{workspace_id}/runners/{runner_id}
pub async fn revoke(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, runner_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let revoked =
        db::runners::revoke_for_workspace(&state.pool, workspace_id, runner_id, user.id, request_id)
            .await?;
    if !revoked {
        return Err(AppError::NotFound);
    }

    // Sever the live connection and recover its jobs.
    state.runner_hub.send(runner_id, protocol::ServerMsg::Error { code: "revoked".into() });
    state.runner_hub.unregister(runner_id);
    pipeline_run::orphan_runner_jobs(&state, runner_id).await?;
    state.scheduler.poke();
    publish_runner(&state, workspace_id, runner_id).await;

    tracing::info!(%workspace_id, %runner_id, "runner revoked");
    Ok(StatusCode::NO_CONTENT.into_response())
}
