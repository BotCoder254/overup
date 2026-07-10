use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::services::authz;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RangeQuery {
    range: Option<String>,
}

/// Validated range window: how far back to look, and the bucket width used
/// by the activity endpoint. Chosen entirely server-side from an allow-list
/// — never interpolated from user text into SQL.
struct Range {
    since: chrono::DateTime<Utc>,
    trunc: &'static str,
}

fn parse_range(raw: Option<&str>) -> AppResult<Range> {
    let now = Utc::now();
    match raw.unwrap_or("24h") {
        "24h" => Ok(Range {
            since: now - Duration::hours(24),
            trunc: "hour",
        }),
        "7d" => Ok(Range {
            since: now - Duration::days(7),
            trunc: "day",
        }),
        "30d" => Ok(Range {
            since: now - Duration::days(30),
            trunc: "day",
        }),
        _ => Err(AppError::Validation("invalid range".into())),
    }
}

/// GET /api/workspaces/{workspace_id}/dashboard/summary?range=24h|7d|30d
pub async fn summary(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<RangeQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    let range = parse_range(query.range.as_deref())?;

    let summary = db::dashboard::summary(&state.pool, workspace_id, range.since).await?;
    Ok(Json(json!({ "summary": summary })))
}

/// GET /api/workspaces/{workspace_id}/dashboard/activity?range=24h|7d|30d
pub async fn activity(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<RangeQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    let range = parse_range(query.range.as_deref())?;

    let buckets =
        db::dashboard::activity_buckets(&state.pool, workspace_id, range.since, range.trunc)
            .await?;
    Ok(Json(json!({ "buckets": buckets })))
}
