//! Toolchains API: a catalog of the language-toolchain images overup
//! understands (`services::toolchain_images`), tagged with the deployment's
//! default image, prewarm/install state, and whether an image allow-list is
//! active. The catalog read rides `content.read`; install/uninstall (an
//! on-demand `docker pull`/`rmi` on the hosted-runner daemon) ride
//! `content.write`, matching hosted-runner management. Image strings are always
//! the static catalog values — no user text ever reaches Docker.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::toolchain::ToolchainsResponse;
use crate::services::authz;
use crate::services::toolchain_images;
use crate::state::AppState;

/// Cap for an on-demand toolchain pull — matches the hosted-create budget.
const PULL_TIMEOUT: Duration = Duration::from_secs(600);

/// GET /api/workspaces/{workspace_id}/toolchains
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<ToolchainsResponse>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    // Prewarm state comes from the provisioner's prepull list (only present
    // when hosted provisioning is configured); otherwise nothing is prewarmed.
    let prepull: &[String] = state
        .config
        .runner_provisioner
        .as_ref()
        .map(|p| p.prepull_images.as_slice())
        .unwrap_or(&[]);

    let install_supported = match &state.runner_provisioner {
        Some(provisioner) => provisioner.available().await,
        None => false,
    };
    let installed = db::toolchain_images::list(&state.pool).await?;

    Ok(Json(ToolchainsResponse::build(
        state.config.default_job_image.clone(),
        prepull,
        !state.config.image_allowlist.is_empty(),
        install_supported,
        &installed,
    )))
}

/// POST /api/workspaces/{workspace_id}/toolchains/{key}/install
pub async fn install(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, key)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    // Only known catalog keys install, and only their static -latest image.
    let toolchain = toolchain_images::all()
        .iter()
        .find(|t| t.key == key)
        .ok_or(AppError::NotFound)?;
    let image = toolchain.image_latest;

    // Pulls need a live provisioner daemon. Static category the UI maps to copy.
    let Some(provisioner) = state.runner_provisioner.clone() else {
        return Err(AppError::Conflict("hosted_runner_unavailable"));
    };
    if !provisioner.available().await {
        return Err(AppError::Conflict("hosted_runner_unavailable"));
    }

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    db::toolchain_images::upsert_pending(&state.pool, toolchain.key, image).await?;
    record_audit(
        &state,
        workspace_id,
        user.id,
        "toolchain.install_requested",
        toolchain.key,
        image,
        request_id,
    )
    .await?;

    // The pull can take minutes — run it off the request path and let the row
    // status (observed via the catalog GET) drive the UI.
    let pool = state.pool.clone();
    let tc_key = toolchain.key.to_string();
    let img = image.to_string();
    tokio::spawn(async move {
        let outcome = tokio::time::timeout(PULL_TIMEOUT, provisioner.pull(&img)).await;
        match outcome {
            Ok(Ok(())) => {
                let _ = db::toolchain_images::mark_installed(&pool, &tc_key).await;
                let _ = insert_audit(&pool, workspace_id, Some(user.id), "toolchain.installed", &tc_key, &img).await;
            }
            Ok(Err(err)) => {
                tracing::warn!(key = %tc_key, error = ?err, "toolchain install pull failed");
                let _ = db::toolchain_images::mark_failed(&pool, &tc_key, "image_pull_failed").await;
                let _ = insert_audit(&pool, workspace_id, Some(user.id), "toolchain.install_failed", &tc_key, &img).await;
            }
            Err(_) => {
                tracing::warn!(key = %tc_key, "toolchain install pull timed out");
                let _ = db::toolchain_images::mark_failed(&pool, &tc_key, "pull_timeout").await;
                let _ = insert_audit(&pool, workspace_id, Some(user.id), "toolchain.install_failed", &tc_key, &img).await;
            }
        }
    });

    Ok(StatusCode::ACCEPTED)
}

/// DELETE /api/workspaces/{workspace_id}/toolchains/{key}
pub async fn uninstall(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, key)): Path<(Uuid, String)>,
    headers: HeaderMap,
) -> AppResult<StatusCode> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let toolchain = toolchain_images::all()
        .iter()
        .find(|t| t.key == key)
        .ok_or(AppError::NotFound)?;

    // Never remove the image jobs fall back to — that would break every job
    // that declares no container.
    if toolchain.image_latest == state.config.default_job_image {
        return Err(AppError::Conflict("toolchain_in_use"));
    }

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    // Dropping the row stops prewarming immediately; the rmi is best-effort.
    let removed = db::toolchain_images::delete(&state.pool, toolchain.key).await?;
    if removed.is_none() {
        return Err(AppError::NotFound);
    }
    record_audit(
        &state,
        workspace_id,
        user.id,
        "toolchain.uninstalled",
        toolchain.key,
        toolchain.image_latest,
        request_id,
    )
    .await?;

    if let (Some(provisioner), Some(image)) = (state.runner_provisioner.clone(), removed) {
        tokio::spawn(async move {
            let _ = provisioner.remove_image(&image).await;
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Audit helper carrying the request id (for the synchronous request path).
async fn record_audit(
    state: &AppState,
    workspace_id: Uuid,
    actor: Uuid,
    action: &str,
    key: &str,
    image: &str,
    request_id: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, $3, 'toolchain', NULL, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(action)
    .bind(json!({ "key": key, "image": image }))
    .bind(request_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Audit helper for the background pull result (no request id available).
async fn insert_audit(
    pool: &sqlx::PgPool,
    workspace_id: Uuid,
    actor: Option<Uuid>,
    action: &str,
    key: &str,
    image: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
        VALUES ($1, $2, $3, 'toolchain', NULL, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(action)
    .bind(json!({ "key": key, "image": image }))
    .execute(pool)
    .await?;
    Ok(())
}
