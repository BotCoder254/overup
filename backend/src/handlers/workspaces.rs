use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::db::workspaces::ProvisionOutcome;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::workspace::{WorkspaceMemberResponse, WorkspaceResponse};
use crate::services::object_store;
use crate::services::workspace as workspace_service;
use crate::services::{authz, image_sniff};
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

/// Query for the create-form pre-flight availability check.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailabilityQuery {
    name: String,
}

/// GET /api/workspaces/availability?name= — advisory pre-flight for the
/// create-workspace form. Authenticated (any signed-in user) but NOT
/// workspace-scoped: during onboarding no workspace exists yet, and slugs
/// are global identifiers already exposed in URLs, so this leaks no
/// cross-workspace state. The unique index inside `provision` remains the
/// authoritative guard; this only powers the live "Available / will be
/// saved as …" hint. Invalid names get the same sanitized 422 as creation.
pub async fn check_availability(
    State(state): State<AppState>,
    CurrentUser(_user): CurrentUser,
    Query(query): Query<AvailabilityQuery>,
) -> AppResult<Json<serde_json::Value>> {
    // Reuse the creation validators so the hint matches what would actually
    // be persisted (NFC-normalize, length + character allow-list).
    let name = workspace_service::normalize_and_validate_name(&query.name)?;
    let base = workspace_service::slugify(&name);

    // Walk the same deterministic collision ladder creation uses and settle
    // on the first free candidate. `available` means the clean base slug is
    // both unreserved and unclaimed (attempt 0 yields the bare base).
    let mut assigned = base.clone();
    for attempt in 0..workspace_service::MAX_SLUG_ATTEMPTS {
        let candidate = workspace_service::slug_candidate(&base, attempt);
        if !db::workspaces::slug_exists(&state.pool, &candidate).await? {
            assigned = candidate;
            break;
        }
    }
    let available = assigned == base;

    Ok(Json(json!({
        "name": name,
        "slug": base,
        "available": available,
        "adjustedSlug": if available { serde_json::Value::Null } else { json!(assigned) },
    })))
}

/// Same allow-list stance as creation: only the name is accepted, and the
/// slug is immutable (it anchors routing and the reserved-slug policy).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateWorkspaceRequest {
    name: String,
}

const STORAGE_NOT_CONFIGURED: &str = "object storage is not configured";
/// Server-side cap on logo bytes (the route's body limit is the transport
/// cap; this is the authoritative one).
pub const MAX_LOGO_BYTES: usize = 2 * 1024 * 1024;

/// PATCH /api/workspaces/{workspace_id} — rename (name only; slug immutable).
pub async fn update_workspace(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateWorkspaceRequest>,
) -> AppResult<Json<WorkspaceResponse>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::WORKSPACE_MANAGE).await?;

    let name = workspace_service::normalize_and_validate_name(&body.name)?;
    let workspace = db::workspaces::update_name(&state.pool, workspace_id, &name)
        .await?
        .ok_or(AppError::NotFound)?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'workspace.updated', 'workspace', $1, $3, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(json!({ "name": name }))
    .bind(request_id)
    .execute(&state.pool)
    .await?;

    Ok(Json(workspace.into()))
}

/// GET /api/workspaces/{workspace_id}/members — read-only members table.
pub async fn list_members(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    let members: Vec<WorkspaceMemberResponse> =
        db::workspaces::list_members(&state.pool, workspace_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(Json(json!({ "members": members })))
}

/// PUT /api/workspaces/{workspace_id}/logo — raw image bytes through the
/// backend (never a presigned PUT: the server must see the content to
/// verify it). Magic-byte detection decides the type; the client's
/// Content-Type and filename are ignored entirely (OWASP File Upload).
pub async fn upload_logo(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::WORKSPACE_MANAGE).await?;
    let storage = state
        .storage
        .as_ref()
        .ok_or(AppError::Conflict(STORAGE_NOT_CONFIGURED))?;

    if body.is_empty() {
        return Err(AppError::Validation("logo image is empty".into()));
    }
    if body.len() > MAX_LOGO_BYTES {
        return Err(AppError::Validation("logo image is too large".into()));
    }
    let Some((content_type, ext)) = image_sniff::detect(&body) else {
        return Err(AppError::Validation(
            "unsupported image format (PNG, JPEG, GIF, or WebP required)".into(),
        ));
    };

    // Server-generated key — the object lands before the row points at it,
    // so a crash in between leaves only an unreferenced object. The write
    // falls back to the secondary store; the accepting backend is recorded.
    let key = object_store::logo_key(workspace_id, ext);
    let backend = storage.put_object(&key, body.to_vec(), content_type).await?;

    let previous = db::workspaces::set_logo_key(
        &state.pool,
        workspace_id,
        Some(&key),
        Some(backend.as_str()),
    )
    .await?
    .ok_or(AppError::NotFound)?;

    // Best-effort removal of the replaced object; the row already moved on.
    if let Some((old_key, old_backend)) = previous.filter(|(old, _)| old != &key)
        && let Err(error) = storage.store_for(&old_backend).delete_object(&old_key).await
    {
        tracing::warn!(key = %old_key, error = ?error, "failed to delete replaced workspace logo");
    }

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'workspace.logo_updated', 'workspace', $1, $3, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(json!({ "contentType": content_type, "sizeBytes": body.len() }))
    .bind(request_id)
    .execute(&state.pool)
    .await?;

    let url = storage
        .store_for(backend.as_str())
        .presign_get_inline(&key)
        .await?;
    Ok(Json(json!({ "logoUrl": url })))
}

/// DELETE /api/workspaces/{workspace_id}/logo
pub async fn remove_logo(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::WORKSPACE_MANAGE).await?;

    let previous = db::workspaces::set_logo_key(&state.pool, workspace_id, None, None)
        .await?
        .ok_or(AppError::NotFound)?;

    if let Some((old_key, old_backend)) = previous {
        if let Some(storage) = state.storage.as_ref()
            && let Err(error) = storage.store_for(&old_backend).delete_object(&old_key).await
        {
            tracing::warn!(key = %old_key, error = ?error, "failed to delete removed workspace logo");
        }
        let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
        sqlx::query(
            r#"
            INSERT INTO audit_logs
                (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
            VALUES ($1, $2, 'workspace.logo_removed', 'workspace', $1, $3, $4)
            "#,
        )
        .bind(workspace_id)
        .bind(user.id)
        .bind(json!({}))
        .bind(request_id)
        .execute(&state.pool)
        .await?;
    }

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// GET /api/workspaces/{workspace_id}/logo-url — short-lived presigned GET
/// for the current logo, or null. Reads never 409 on missing R2 config.
pub async fn logo_url(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let workspace = db::workspaces::find_by_id(&state.pool, workspace_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let url = match (workspace.logo_key.as_deref(), state.storage.as_ref()) {
        (Some(key), Some(storage)) => Some(
            storage
                .store_for(workspace.logo_storage_backend.as_deref().unwrap_or("r2"))
                .presign_get_inline(key)
                .await?,
        ),
        _ => None,
    };
    Ok(Json(json!({ "url": url })))
}

/// Zero-config runners: when this deployment can provision hosted runners
/// and RUNNER_AUTO_PROVISION is on, every new workspace gets one in the
/// background. Every failure path is warn-only — workspace creation never
/// fails or slows because of it; denials/failures surface on the Runners
/// page through the usual provision_error UX. No RBAC check: the actor
/// literally just created (and owns) the workspace. The gate is the live
/// Docker connection, not just config — while the daemon is unreachable the
/// auto-provision is skipped (warn-only) instead of enqueueing rows that can
/// only fail; the user can create a hosted runner from the wizard once the
/// provisioner reconnects.
fn spawn_auto_provision_runner(
    state: &AppState,
    workspace_id: uuid::Uuid,
    user_id: uuid::Uuid,
    request_id: Option<String>,
) {
    let Some(cfg) = state.config.runner_provisioner.as_ref() else {
        return;
    };
    let Some(provisioner) = state.runner_provisioner.clone() else {
        return;
    };
    if !cfg.auto_provision {
        return;
    }
    let profile = cfg.default_profile;

    let task_state = state.clone();
    tokio::spawn(async move {
        if !provisioner.available().await {
            tracing::warn!(%workspace_id, "hosted runner auto-provision skipped: docker unavailable");
            return;
        }
        let labels = crate::handlers::runners::default_labels();
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
