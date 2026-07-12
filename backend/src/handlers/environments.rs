//! Environments API: workspace-level deployment environments that act as
//! the highest-precedence secrets scope (environment > repository >
//! workspace at dispatch). Metadata only — no approval gates this phase.
//! Listing/detail ride `content.read`; every mutation needs
//! `environments.manage` and writes its audit entry in-transaction.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::environment::EnvironmentResponse;
use crate::services::authz;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::state::AppState;

use super::pipelines::{escape_like, format_cursor, parse_cursor};

const DEFAULT_PAGE: i64 = 25;
const NAME_MAX: usize = 100;
const DESCRIPTION_MAX: usize = 500;

/// Normalize and validate an environment name. NFC first (the secrets
/// pattern — visually identical Unicode can't dodge the allow-list), then
/// the slug-safe `^[A-Za-z0-9][A-Za-z0-9._-]*$` the migration CHECK also
/// enforces. Case is preserved; uniqueness and dispatch lookup are
/// case-insensitive.
fn validate_name(raw: &str) -> AppResult<String> {
    let name: String = raw.trim().nfc().collect();
    if name.is_empty() {
        return Err(AppError::Validation("environment name is required".into()));
    }
    if name.len() > NAME_MAX {
        return Err(AppError::Validation(format!(
            "environment name must be at most {NAME_MAX} characters"
        )));
    }
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !first_ok || !rest_ok {
        return Err(AppError::Validation(
            "environment names are letters, digits, '.', '_', and '-', \
             starting with a letter or digit"
                .into(),
        ));
    }
    Ok(name)
}

fn validate_description(raw: Option<&str>) -> AppResult<Option<String>> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(d) if d.len() <= DESCRIPTION_MAX => Ok(Some(d.to_string())),
        Some(_) => Err(AppError::Validation(format!(
            "description must be at most {DESCRIPTION_MAX} characters"
        ))),
    }
}

fn audit_events(rows: Vec<db::environments::EnvironmentAuditRow>) -> Vec<serde_json::Value> {
    rows.into_iter()
        .map(|row| {
            json!({
                "action": row.action,
                "actorLogin": row.actor_login,
                "subjectId": row.subject_id,
                "metadata": row.metadata,
                "createdAt": row.created_at,
            })
        })
        .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogQuery {
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// GET /api/workspaces/{workspace_id}/environments
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let search_pattern = match query.q.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(q) if q.len() <= 200 => Some(format!("%{}%", escape_like(q))),
        Some(_) => return Err(AppError::Validation("search query too long".into())),
    };
    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 50);

    let filter = db::environments::CatalogFilter {
        search_pattern,
        cursor,
        limit,
    };
    let rows = db::environments::list_catalog(&state.pool, workspace_id, &filter).await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.created_at, row.id));
    let environments: Vec<EnvironmentResponse> =
        rows.into_iter().map(EnvironmentResponse::from).collect();

    Ok(Json(
        json!({ "environments": environments, "nextCursor": next_cursor }),
    ))
}

/// GET /api/workspaces/{workspace_id}/environments/summary
pub async fn summary(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let summary = db::environments::summary(&state.pool, workspace_id).await?;
    Ok(Json(json!({
        "total": summary.total,
        "withSecrets": summary.with_secrets,
        "createdLast30d": summary.created_last_30d,
        "scopedSecrets": summary.scoped_secrets,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditQuery {
    limit: Option<i64>,
}

/// GET /api/workspaces/{workspace_id}/environments/audit
pub async fn audit(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<AuditQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let limit = query.limit.unwrap_or(20).clamp(1, 50);
    let rows = db::environments::list_audit(&state.pool, workspace_id, None, limit).await?;
    Ok(Json(json!({ "events": audit_events(rows) })))
}

/// GET /api/workspaces/{workspace_id}/environments/{environment_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, environment_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let meta = db::environments::find_meta(&state.pool, workspace_id, environment_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let audit =
        db::environments::list_audit(&state.pool, workspace_id, Some(environment_id), 20).await?;
    Ok(Json(json!({
        "environment": EnvironmentResponse::from(meta),
        "audit": audit_events(audit),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBody {
    name: String,
    description: Option<String>,
}

/// POST /api/workspaces/{workspace_id}/environments
pub async fn create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::ENVIRONMENTS_MANAGE)
        .await?;

    let name = validate_name(&body.name)?;
    let description = validate_description(body.description.as_deref())?;
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());

    match db::environments::insert(
        &state.pool,
        workspace_id,
        &name,
        description.as_deref(),
        user.id,
        request_id,
    )
    .await?
    {
        db::environments::InsertOutcome::Created(meta) => {
            tracing::info!(%workspace_id, environment = %name, "environment created");
            state.workspace_hub.publish(
                workspace_id,
                WorkspaceEvent::ActivityUpdate { category: "environment".into() },
            );
            Ok((
                StatusCode::CREATED,
                Json(json!({ "environment": EnvironmentResponse::from(*meta) })),
            )
                .into_response())
        }
        db::environments::InsertOutcome::DuplicateName => Err(AppError::Conflict(
            "an environment with this name already exists",
        )),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBody {
    name: Option<String>,
    description: Option<String>,
}

/// PATCH /api/workspaces/{workspace_id}/environments/{environment_id}
///
/// Rename and/or re-describe. Renames take effect for FUTURE dispatches —
/// YAML `environment:` names resolve live, so a workflow referencing the
/// old name stops matching (and gets the unknown-environment notice).
pub async fn update(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, environment_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<UpdateBody>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::ENVIRONMENTS_MANAGE)
        .await?;

    let name = match body.name.as_deref() {
        None => None,
        Some(raw) => Some(validate_name(raw)?),
    };
    let description = validate_description(body.description.as_deref())?;
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());

    match db::environments::update(
        &state.pool,
        workspace_id,
        environment_id,
        name.as_deref(),
        description.as_deref(),
        user.id,
        request_id,
    )
    .await?
    {
        db::environments::UpdateOutcome::Updated(meta) => {
            state.workspace_hub.publish(
                workspace_id,
                WorkspaceEvent::ActivityUpdate { category: "environment".into() },
            );
            Ok(Json(
                json!({ "environment": EnvironmentResponse::from(*meta) }),
            ))
        }
        db::environments::UpdateOutcome::DuplicateName => Err(AppError::Conflict(
            "an environment with this name already exists",
        )),
        db::environments::UpdateOutcome::NotFound => Err(AppError::NotFound),
    }
}

/// DELETE /api/workspaces/{workspace_id}/environments/{environment_id}
///
/// Cascades to the environment's secrets — every cascaded secret gets its
/// own audit entry inside the same transaction.
pub async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, environment_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::ENVIRONMENTS_MANAGE)
        .await?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let deleted_secrets =
        db::environments::remove(&state.pool, workspace_id, environment_id, user.id, request_id)
            .await?
            .ok_or(AppError::NotFound)?;

    tracing::info!(%workspace_id, %environment_id, deleted_secrets, "environment deleted");
    state.workspace_hub.publish(
        workspace_id,
        WorkspaceEvent::ActivityUpdate { category: "environment".into() },
    );
    Ok(Json(json!({ "deletedSecrets": deleted_secrets })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_accepts_slug_shapes() {
        assert_eq!(validate_name("production").unwrap(), "production");
        assert_eq!(validate_name("Staging-2").unwrap(), "Staging-2");
        assert_eq!(validate_name("eu.west_1").unwrap(), "eu.west_1");
        assert_eq!(validate_name("  trimmed  ").unwrap(), "trimmed");
    }

    #[test]
    fn name_rejects_bad_shapes() {
        assert!(validate_name("").is_err());
        assert!(validate_name("-starts-with-dash").is_err());
        assert!(validate_name(".starts-with-dot").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("has/slash").is_err());
        assert!(validate_name(&"a".repeat(101)).is_err());
        // Unicode confusables survive NFC but fail the ASCII allow-list.
        assert!(validate_name("pr\u{043e}d").is_err()); // Cyrillic о
    }
}
