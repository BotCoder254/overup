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
                spawn_auto_provision_runner(&state, workspace.id, user.id, request_id.clone());
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

/// Zero-config runners: when this deployment can provision hosted runners
/// and RUNNER_AUTO_PROVISION is on, every new workspace gets one in the
/// background. Every failure path is warn-only — workspace creation never
/// fails or slows because of it; denials/failures surface on the Runners
/// page through the usual provision_error UX. No RBAC check: the actor
/// literally just created (and owns) the workspace. The gate is the live
/// provisioner handle, not just config — init degrades to None when Docker
/// is unreachable at boot.
fn spawn_auto_provision_runner(
    state: &AppState,
    workspace_id: uuid::Uuid,
    user_id: uuid::Uuid,
    request_id: Option<String>,
) {
    let Some(cfg) = state.config.runner_provisioner.as_ref() else {
        return;
    };
    if !cfg.auto_provision || state.runner_provisioner.is_none() {
        return;
    }
    let profile = cfg.default_profile;

    let task_state = state.clone();
    tokio::spawn(async move {
        let labels: Vec<String> = ["self-hosted", "linux", "x64", "ubuntu-latest"]
            .into_iter()
            .map(String::from)
            .collect();
        match crate::services::runner_provision_flow::start_hosted_provision(
            &task_state,
            crate::services::runner_provision_flow::HostedProvisionRequest {
                workspace_id,
                created_by: user_id,
                name: "hosted-1",
                labels: &labels,
                profile,
                instances: 1,
                request_id: request_id.as_deref(),
            },
        )
        .await
        {
            Ok(Ok(runners)) => {
                if let Some(runner) = runners.first() {
                    tracing::info!(%workspace_id, runner_id = %runner.id, "auto-provisioned hosted runner");
                }
            }
            Ok(Err(denied)) => {
                tracing::warn!(%workspace_id, ?denied, "hosted runner auto-provision denied");
            }
            Err(error) => {
                tracing::warn!(%workspace_id, error = ?error, "hosted runner auto-provision failed");
            }
        }
    });
}
