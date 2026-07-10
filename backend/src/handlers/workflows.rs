use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::workflow::{WorkflowDetailResponse, WorkflowSummaryResponse};
use crate::services::{authz, github_app, workflow_parse};
use crate::state::AppState;

/// GET /api/workspaces/{workspace_id}/workflows — the workspace catalog.
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let workflows = db::workflows::list_for_workspace(&state.pool, workspace_id)
        .await?
        .into_iter()
        .map(WorkflowSummaryResponse::from)
        .collect::<Vec<_>>();

    Ok(Json(json!({ "workflows": workflows })))
}

/// GET /api/workspaces/{workspace_id}/workflows/{workflow_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, workflow_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<WorkflowDetailResponse>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let row = db::workflows::find_detail_for_workspace(&state.pool, workspace_id, workflow_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let jobs = db::workflows::list_jobs(&state.pool, workflow_id).await?;

    Ok(Json(WorkflowDetailResponse::from_rows(row, jobs)))
}

#[derive(Debug, Deserialize)]
pub struct ValidateRequest {
    content: String,
}

/// POST /api/workspaces/{workspace_id}/workflows/validate
///
/// Offline validation for the editor: same parser as sync, no side effects,
/// nothing persisted, nothing executed. Route carries its own 1 MiB body
/// limit; the content cap below is the real bound.
pub async fn validate(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Json(body): Json<ValidateRequest>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    if body.content.len() > github_app::MAX_WORKFLOW_FILE_BYTES {
        return Err(AppError::Validation(
            "workflow content exceeds the size limit".into(),
        ));
    }

    let parsed = workflow_parse::parse_and_validate(&body.content);
    Ok(Json(json!({
        "status": parsed.status(),
        "diagnostics": parsed.diagnostics,
        "triggers": parsed.triggers,
        "jobs": parsed
            .jobs
            .iter()
            .map(|job| json!({
                "key": job.key,
                "name": job.name,
                "needs": job.needs,
                "runsOn": job.runs_on,
                "uses": job.uses,
                "stepCount": job.step_count,
            }))
            .collect::<Vec<_>>(),
    })))
}
