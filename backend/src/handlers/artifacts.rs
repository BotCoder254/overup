//! Workspace-wide artifact catalog: list with validated filters + keyset
//! pagination, storage summary, provenance detail, and operator delete.
//! Downloads stay on the existing presigned-GET endpoint in
//! `handlers::pipelines`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::artifact::ArtifactCatalogResponse;
use crate::services::authz;
use crate::state::AppState;

use super::pipelines::{escape_like, format_cursor, parse_cursor};

const DEFAULT_PAGE: i64 = 25;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogQuery {
    repository_id: Option<Uuid>,
    workflow_id: Option<Uuid>,
    pipeline_id: Option<Uuid>,
    job_id: Option<Uuid>,
    status: Option<String>,
    q: Option<String>,
    created_after: Option<String>,
    created_before: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// GET /api/workspaces/{workspace_id}/artifacts
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<CatalogQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 50);
    let filter = build_catalog_filter(&query, cursor, limit)?;

    let rows = db::artifacts::list_catalog(&state.pool, workspace_id, &filter).await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.artifact.created_at, row.artifact.id));
    let artifacts: Vec<ArtifactCatalogResponse> =
        rows.into_iter().map(ArtifactCatalogResponse::from).collect();

    Ok(Json(json!({ "artifacts": artifacts, "nextCursor": next_cursor })))
}

/// Validate and normalize the catalog filters. Everything is allow-listed
/// or length-capped before it reaches SQL; the free-text search becomes a
/// pre-escaped ILIKE pattern so the db layer never sees raw input.
fn build_catalog_filter(
    query: &CatalogQuery,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> AppResult<db::artifacts::CatalogFilter> {
    let status = match query.status.as_deref() {
        None | Some("") => None,
        Some(s @ ("pending" | "uploaded" | "failed" | "expired")) => Some(s.to_string()),
        Some(_) => return Err(AppError::Validation("invalid status filter".into())),
    };

    let parse_ts = |raw: &Option<String>, name: &'static str| -> AppResult<Option<DateTime<Utc>>> {
        match raw.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(s) if s.len() <= 64 => DateTime::parse_from_rfc3339(s)
                .map(|dt| Some(dt.with_timezone(&Utc)))
                .map_err(|_| AppError::Validation(format!("invalid {name} timestamp"))),
            Some(_) => Err(AppError::Validation(format!("{name} too long"))),
        }
    };
    let created_after = parse_ts(&query.created_after, "createdAfter")?;
    let created_before = parse_ts(&query.created_before, "createdBefore")?;

    let search_pattern = match query.q.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(q) if q.len() <= 200 => Some(format!("%{}%", escape_like(q))),
        Some(_) => return Err(AppError::Validation("search query too long".into())),
    };

    Ok(db::artifacts::CatalogFilter {
        repository_id: query.repository_id,
        workflow_id: query.workflow_id,
        pipeline_id: query.pipeline_id,
        job_id: query.job_id,
        status,
        search_pattern,
        created_after,
        created_before,
        cursor,
        limit,
    })
}

/// GET /api/workspaces/{workspace_id}/artifacts/summary
pub async fn summary(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let summary = db::artifacts::summary_for_workspace(&state.pool, workspace_id).await?;
    Ok(Json(json!({
        "total": summary.total,
        "uploaded": summary.uploaded,
        "pending": summary.pending,
        "failed": summary.failed,
        "expiringSoon": summary.expiring_soon,
        "totalBytes": summary.total_bytes,
    })))
}

/// GET /api/workspaces/{workspace_id}/artifacts/{artifact_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, artifact_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let row = db::artifacts::find_catalog_row(&state.pool, workspace_id, artifact_id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(json!({ "artifact": ArtifactCatalogResponse::from(row) })))
}

/// DELETE /api/workspaces/{workspace_id}/artifacts/{artifact_id}
///
/// Best-effort R2 object delete first, then hard row delete + audit entry
/// in one transaction. A failed object delete never blocks the row delete —
/// the object becomes unreachable garbage at worst, never a live leak
/// (same policy as the janitor).
pub async fn remove(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, artifact_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let artifact = db::artifacts::find_for_workspace(&state.pool, workspace_id, artifact_id)
        .await?
        .ok_or(AppError::NotFound)?;

    if matches!(artifact.status.as_str(), "uploaded" | "pending")
        && let Some(r2) = &state.r2
        && let Err(error) = r2.delete_object(&artifact.r2_key).await
    {
        tracing::warn!(
            %artifact_id,
            error = ?error,
            "failed to delete artifact object from R2 during operator delete"
        );
    }

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    if db::artifacts::delete_for_workspace(&state.pool, workspace_id, artifact_id, user.id, request_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound);
    }

    tracing::info!(%workspace_id, %artifact_id, "artifact deleted");
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_query() -> CatalogQuery {
        CatalogQuery {
            repository_id: None,
            workflow_id: None,
            pipeline_id: None,
            job_id: None,
            status: None,
            q: None,
            created_after: None,
            created_before: None,
            cursor: None,
            limit: None,
        }
    }

    #[test]
    fn catalog_filter_validates_status_allow_list() {
        let mut query = empty_query();
        query.status = Some("uploaded".into());
        let filter = build_catalog_filter(&query, None, 25).unwrap();
        assert_eq!(filter.status.as_deref(), Some("uploaded"));

        query.status = Some("uploaded'; DROP TABLE artifacts;--".into());
        assert!(build_catalog_filter(&query, None, 25).is_err());
    }

    #[test]
    fn catalog_filter_escapes_search_and_caps_length() {
        let mut query = empty_query();
        query.q = Some("50%_done".into());
        let filter = build_catalog_filter(&query, None, 25).unwrap();
        assert_eq!(filter.search_pattern.as_deref(), Some("%50\\%\\_done%"));

        query.q = Some("x".repeat(201));
        assert!(build_catalog_filter(&query, None, 25).is_err());
    }

    #[test]
    fn catalog_filter_validates_timestamps() {
        let mut query = empty_query();
        query.created_after = Some("2026-07-01T00:00:00Z".into());
        let filter = build_catalog_filter(&query, None, 25).unwrap();
        assert!(filter.created_after.is_some());

        query.created_after = Some("not-a-date".into());
        assert!(build_catalog_filter(&query, None, 25).is_err());
    }
}
