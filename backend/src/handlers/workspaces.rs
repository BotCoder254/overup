use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;

use crate::db;
use crate::db::workspaces::ProvisionOutcome;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::workspace::WorkspaceResponse;
use crate::services::workspace as workspace_service;
use crate::state::AppState;

/// Only the name and optional description are accepted. Any other field a
/// client sends — a slug, an owner, an id — is silently dropped by serde;
/// every privileged value is derived server-side.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

const ALREADY_MEMBER: &str = "you already belong to a workspace";

pub async fn create_workspace(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    Json(body): Json<CreateWorkspaceRequest>,
) -> AppResult<(StatusCode, Json<WorkspaceResponse>)> {
    let name = workspace_service::normalize_and_validate_name(&body.name)?;
    let description =
        workspace_service::normalize_and_validate_description(body.description.as_deref())?;

    // Friendly fast-path 409; the unique index inside the provisioning
    // transaction remains the authoritative guard under races.
    if db::workspaces::find_summary_for_user(&state.pool, user.id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(ALREADY_MEMBER));
    }

    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let base = workspace_service::slugify(&name);
    for attempt in 0..workspace_service::MAX_SLUG_ATTEMPTS {
        let slug = workspace_service::slug_candidate(&base, attempt);
        match db::workspaces::provision(
            &state.pool,
            user.id,
            &name,
            description.as_deref(),
            &slug,
            request_id.as_deref(),
        )
        .await?
        {
            ProvisionOutcome::Created(workspace) => {
                tracing::info!(
                    workspace_id = %workspace.id,
                    slug = %workspace.slug,
                    user_id = %user.id,
                    "workspace provisioned"
                );
                return Ok((StatusCode::CREATED, Json(workspace.into())));
            }
            ProvisionOutcome::SlugTaken => {
                tracing::debug!(slug = %slug, user_id = %user.id, "slug collision, retrying");
            }
            ProvisionOutcome::AlreadyMember => return Err(AppError::Conflict(ALREADY_MEMBER)),
        }
    }

    // Client sees a generic 500; the exhausted base stays in server logs.
    Err(AppError::Internal(anyhow::anyhow!(
        "slug candidate space exhausted for base '{base}'"
    )))
}
