//! Toolchains API: a read-only catalog of the language-toolchain images
//! overup understands (`services::toolchain_images`), tagged with the
//! deployment's default image, prewarm state, and whether an image allow-list
//! is active. Rides `content.read` like the other workspace read surfaces.
//! Static strings only — no mutation surface, no user/runner text.

use axum::Json;
use axum::extract::{Path, State};
use uuid::Uuid;

use crate::error::AppResult;
use crate::middleware::auth::CurrentUser;
use crate::models::toolchain::ToolchainsResponse;
use crate::services::authz;
use crate::state::AppState;

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

    Ok(Json(ToolchainsResponse::build(
        state.config.default_job_image.clone(),
        prepull,
        !state.config.image_allowlist.is_empty(),
    )))
}
