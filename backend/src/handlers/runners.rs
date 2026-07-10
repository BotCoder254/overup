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
use crate::services::{authz, pipeline_run, session};
use crate::state::AppState;

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
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "runner": RunnerResponse::from(runner),
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

    tracing::info!(%workspace_id, %runner_id, "runner revoked");
    Ok(StatusCode::NO_CONTENT.into_response())
}
