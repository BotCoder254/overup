use std::collections::HashSet;

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
use crate::models::repository::{
    AvailableRepoResponse, BranchResponse, RepositoryResponse, SyncRunResponse,
};
use crate::models::workflow::WorkflowSummaryResponse;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::services::{authz, github_app, repo_sync};
use crate::state::AppState;

/// GET /api/workspaces/{workspace_id}/repositories
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let repositories = db::repositories::list_for_workspace(&state.pool, workspace_id)
        .await?
        .into_iter()
        .map(RepositoryResponse::from)
        .collect::<Vec<_>>();

    Ok(Json(json!({ "repositories": repositories })))
}

/// GET /api/workspaces/{workspace_id}/repositories/available
///
/// Live proxy over every linked installation. Nothing here is persisted;
/// GitHub remains the source of truth for what is importable.
pub async fn available(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let connected: HashSet<i64> = db::repositories::list_for_workspace(&state.pool, workspace_id)
        .await?
        .into_iter()
        .map(|r| r.repository.github_repo_id)
        .collect();

    let installations =
        db::github_installations::list_for_workspace(&state.pool, workspace_id).await?;

    let mut repositories = Vec::new();
    for installation in installations
        .iter()
        .filter(|i| i.suspended_at.is_none())
    {
        let token = state
            .github_app
            .installation_token(&state.http, installation.installation_id)
            .await
            .map_err(AppError::Internal)?;
        let repos = github_app::list_installation_repositories(&state.http, &token)
            .await
            .map_err(AppError::Internal)?;
        for repo in repos {
            if repo.archived == Some(true) {
                continue;
            }
            repositories.push(AvailableRepoResponse {
                connected: connected.contains(&repo.id),
                github_repo_id: repo.id,
                installation_id: installation.id,
                owner_avatar_url: github_app::sanitize_avatar_url(repo.owner.avatar_url.as_deref())
                    .map(str::to_string),
                owner: repo.owner.login,
                name: repo.name,
                full_name: repo.full_name,
                private: repo.private,
                default_branch: repo.default_branch,
                language: repo.language,
                description: repo.description,
            });
        }
    }
    repositories.sort_by(|a, b| a.full_name.cmp(&b.full_name));

    Ok(Json(json!({ "repositories": repositories })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    installation_id: Uuid,
    github_repo_id: i64,
}

/// POST /api/workspaces/{workspace_id}/repositories — import one repository.
pub async fn import(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<ImportRequest>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    if body.github_repo_id <= 0 {
        return Err(AppError::Validation("githubRepoId must be positive".into()));
    }

    // The installation UUID must belong to this workspace — client-supplied
    // identifiers are resolved, never trusted.
    let installation = db::github_installations::find_for_workspace(
        &state.pool,
        workspace_id,
        body.installation_id,
    )
    .await?
    .ok_or(AppError::NotFound)?;
    if installation.suspended_at.is_some() {
        return Err(AppError::Conflict("installation is suspended"));
    }

    // Server-side re-verification: the repository must actually be visible
    // to this installation's scoped token.
    let token = state
        .github_app
        .installation_token(&state.http, installation.installation_id)
        .await
        .map_err(AppError::Internal)?;
    let remote = github_app::get_repository(&state.http, &token, body.github_repo_id)
        .await
        .map_err(|error| {
            tracing::warn!(error = ?error, "import target not visible to installation");
            AppError::Validation("repository is not accessible to this installation".into())
        })?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let outcome = db::repositories::import(
        &state.pool,
        workspace_id,
        installation.id,
        remote.id,
        &remote.owner.login,
        github_app::sanitize_avatar_url(remote.owner.avatar_url.as_deref()),
        &remote.name,
        &remote.full_name,
        remote.private,
        remote.default_branch.as_deref().unwrap_or("main"),
        remote.language.as_deref(),
        remote.description.as_deref(),
        user.id,
        request_id,
    )
    .await?;

    let repository = match outcome {
        db::repositories::ImportOutcome::Created(repository) => *repository,
        db::repositories::ImportOutcome::AlreadyImported => {
            return Err(AppError::Conflict("repository is already connected"));
        }
    };

    tracing::info!(
        workspace_id = %workspace_id,
        repository = %repository.full_name,
        "repository imported"
    );

    // Initial sync happens in the background; the card shows progress.
    repo_sync::schedule(&state, repository.id, "import").await?;

    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::ActivityUpdate { category: "repository".into() },
    );

    Ok((
        StatusCode::CREATED,
        Json(RepositoryResponse::from_row(repository, 0)),
    )
        .into_response())
}

/// GET /api/workspaces/{workspace_id}/repositories/{repository_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, repository_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let repository = db::repositories::find_for_workspace(&state.pool, workspace_id, repository_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let branches = db::repositories::list_branches(&state.pool, repository.id)
        .await?
        .into_iter()
        .map(BranchResponse::from)
        .collect::<Vec<_>>();
    let workflows = db::workflows::summaries_for_repo(&state.pool, repository.id)
        .await?
        .into_iter()
        .map(WorkflowSummaryResponse::from)
        .collect::<Vec<_>>();
    let sync_runs = db::repositories::list_sync_runs(&state.pool, repository.id)
        .await?
        .into_iter()
        .map(SyncRunResponse::from)
        .collect::<Vec<_>>();

    let workflow_count = workflows.len() as i64;
    Ok(Json(json!({
        "repository": RepositoryResponse::from_row(repository, workflow_count),
        "branches": branches,
        "workflows": workflows,
        "syncRuns": sync_runs,
    })))
}

/// POST /api/workspaces/{workspace_id}/repositories/{repository_id}/sync
pub async fn sync(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, repository_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    // Resolve within the workspace before touching sync state.
    db::repositories::find_for_workspace(&state.pool, workspace_id, repository_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if !repo_sync::schedule(&state, repository_id, "manual").await? {
        return Err(AppError::Conflict("a sync is already running"));
    }

    Ok((StatusCode::ACCEPTED, Json(json!({ "syncStatus": "syncing" }))).into_response())
}

/// DELETE /api/workspaces/{workspace_id}/repositories/{repository_id}
pub async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, repository_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let deleted = db::repositories::delete_for_workspace(
        &state.pool,
        workspace_id,
        repository_id,
        user.id,
        request_id,
    )
    .await?;
    if !deleted {
        return Err(AppError::NotFound);
    }

    tracing::info!(%workspace_id, %repository_id, "repository removed");
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::ActivityUpdate { category: "repository".into() },
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}
