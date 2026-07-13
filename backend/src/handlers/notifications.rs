//! Notification Center API. Rows are a per-user projection, so every query
//! and mutation is double-scoped: workspace membership (`content.read` RBAC,
//! flat 403) AND `user_id = caller` in SQL — a forged notification id can
//! never read or flip another user's state. Filters are allow-listed or
//! length-capped before SQL (the pipelines-ledger pattern). Bulk lifecycle
//! ops and preference changes write audit rows in-transaction; individual
//! mark-read is deliberately unaudited per-user UI state.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::notification::{
    CATEGORIES, NotificationResponse, PreferencesResponse, SEVERITIES,
};
use crate::services::authz;
use crate::services::notification_hub::NotificationEvent;
use crate::state::AppState;

use super::activity::csv_field;
use super::pipelines::{escape_like, format_cursor, parse_cursor};

const DEFAULT_PAGE: i64 = 30;
const MAX_BULK_IDS: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    unread: Option<bool>,
    category: Option<String>,
    severity: Option<String>,
    repository_id: Option<Uuid>,
    q: Option<String>,
    created_after: Option<String>,
    created_before: Option<String>,
    archived: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

fn validate_category(raw: Option<&str>) -> AppResult<Option<String>> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(c) if CATEGORIES.contains(&c) => Ok(Some(c.to_string())),
        Some(_) => Err(AppError::Validation("invalid category filter".into())),
    }
}

fn validate_severity(raw: Option<&str>) -> AppResult<Option<String>> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) if SEVERITIES.contains(&s) => Ok(Some(s.to_string())),
        Some(_) => Err(AppError::Validation("invalid severity filter".into())),
    }
}

fn parse_ts(raw: &Option<String>, name: &'static str) -> AppResult<Option<chrono::DateTime<chrono::Utc>>> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) if s.len() <= 64 => chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| Some(dt.with_timezone(&chrono::Utc)))
            .map_err(|_| AppError::Validation(format!("invalid {name} timestamp"))),
        Some(_) => Err(AppError::Validation(format!("{name} too long"))),
    }
}

fn build_filter(query: &ListQuery) -> AppResult<db::notifications::ListFilter> {
    let category = validate_category(query.category.as_deref())?;
    let severity = validate_severity(query.severity.as_deref())?;

    let created_after = parse_ts(&query.created_after, "createdAfter")?;
    let created_before = parse_ts(&query.created_before, "createdBefore")?;
    if let (Some(after), Some(before)) = (created_after, created_before)
        && after > before
    {
        return Err(AppError::Validation(
            "createdAfter must not be later than createdBefore".into(),
        ));
    }

    let search_pattern = match query.q.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(q) if q.len() <= 200 => Some(format!("%{}%", escape_like(q))),
        Some(_) => return Err(AppError::Validation("search query too long".into())),
    };
    let archived = match query.archived.as_deref().map(str::trim) {
        None | Some("") | Some("exclude") => "exclude",
        Some("include") => "include",
        Some("only") => "only",
        Some(_) => return Err(AppError::Validation("invalid archived filter".into())),
    };
    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };

    Ok(db::notifications::ListFilter {
        unread: query.unread,
        category,
        severity,
        repository_id: query.repository_id,
        search_pattern,
        created_after,
        created_before,
        archived,
        cursor,
        limit: query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 100),
    })
}

/// GET /api/workspaces/{workspace_id}/notifications
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let filter = build_filter(&query)?;
    let rows = db::notifications::list(&state.pool, workspace_id, user.id, &filter).await?;
    let next_cursor = (rows.len() as i64 == filter.limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.created_at, row.id));
    let notifications: Vec<NotificationResponse> =
        rows.into_iter().map(NotificationResponse::from).collect();

    Ok(Json(json!({
        "notifications": notifications,
        "nextCursor": next_cursor,
    })))
}

/// GET /api/workspaces/{workspace_id}/notifications/export
///
/// CSV export of the caller's (filtered) notifications — identical filter
/// validation to the JSON list, capped at
/// [`db::notifications::EXPORT_MAX_ROWS`] rows, hardened against
/// spreadsheet formula injection by the shared `csv_field`. Content is
/// server-rendered static-template text by construction, so nothing
/// confidential can leave through this surface.
pub async fn export(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let filter = build_filter(&query)?;
    let rows = db::notifications::export(&state.pool, workspace_id, user.id, &filter).await?;

    let mut csv = String::from(
        "id,createdAt,category,severity,action,title,body,occurrenceCount,readAt,archivedAt\r\n",
    );
    for row in rows {
        let line = [
            row.id.to_string(),
            row.created_at.to_rfc3339(),
            row.category.clone(),
            row.severity.clone(),
            row.action.clone(),
            row.title.clone(),
            row.body.clone(),
            row.occurrence_count.to_string(),
            row.read_at.map(|at| at.to_rfc3339()).unwrap_or_default(),
            row.archived_at.map(|at| at.to_rfc3339()).unwrap_or_default(),
        ]
        .iter()
        .map(|field| csv_field(field))
        .collect::<Vec<_>>()
        .join(",");
        csv.push_str(&line);
        csv.push_str("\r\n");
    }

    Ok((
        [
            (CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                CONTENT_DISPOSITION,
                "attachment; filename=\"notifications-export.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

/// GET /api/workspaces/{workspace_id}/notifications/unread-count
pub async fn unread_count(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    let count = db::notifications::unread_count(&state.pool, workspace_id, user.id).await?;
    Ok(Json(json!({ "count": count })))
}

/// Push the caller's authoritative unread count to their live sockets so
/// sibling tabs converge after any mutation.
async fn publish_unread(state: &AppState, workspace_id: Uuid, user_id: Uuid) {
    if let Ok(count) = db::notifications::unread_count(&state.pool, workspace_id, user_id).await {
        state
            .notification_hub
            .publish(user_id, NotificationEvent::UnreadCount { count });
    }
}

/// POST /api/workspaces/{workspace_id}/notifications/{id}/read
pub async fn mark_read(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    if !db::notifications::mark_read(&state.pool, workspace_id, user.id, id).await? {
        // Flat 404: not the caller's row, already read, or nonexistent —
        // indistinguishable by design.
        return Err(AppError::NotFound);
    }
    publish_unread(&state, workspace_id, user.id).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadAllBody {
    category: Option<String>,
}

/// POST /api/workspaces/{workspace_id}/notifications/read-all
pub async fn read_all(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    body: Option<Json<ReadAllBody>>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    let category = validate_category(
        body.as_ref().and_then(|b| b.category.as_deref()),
    )?;

    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut tx = state.pool.begin().await?;
    let updated =
        db::notifications::mark_all_read(&mut tx, workspace_id, user.id, category.as_deref())
            .await?;
    sqlx::query(
        r#"
        INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'notification.read_all', 'workspace', $1, $3, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(json!({ "count": updated, "category": category }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    publish_unread(&state, workspace_id, user.id).await;
    Ok(Json(json!({ "updated": updated })))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkBody {
    action: String,
    ids: Vec<Uuid>,
}

/// POST /api/workspaces/{workspace_id}/notifications/bulk
pub async fn bulk(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<BulkBody>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    if !matches!(body.action.as_str(), "read" | "unread" | "archive") {
        return Err(AppError::Validation("invalid bulk action".into()));
    }
    if body.ids.is_empty() || body.ids.len() > MAX_BULK_IDS {
        return Err(AppError::Validation(format!(
            "ids must contain between 1 and {MAX_BULK_IDS} entries"
        )));
    }

    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut tx = state.pool.begin().await?;
    let updated =
        db::notifications::bulk_update(&mut tx, workspace_id, user.id, &body.action, &body.ids)
            .await?;
    // Bulk archive is the one destructive-ish lifecycle op — record it.
    if body.action == "archive" {
        sqlx::query(
            r#"
            INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
            VALUES ($1, $2, 'notification.bulk_archived', 'workspace', $1, $3, $4)
            "#,
        )
        .bind(workspace_id)
        .bind(user.id)
        .bind(json!({ "count": updated }))
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    publish_unread(&state, workspace_id, user.id).await;
    Ok(Json(json!({ "updated": updated })))
}

/// GET /api/workspaces/{workspace_id}/notification-preferences
pub async fn get_preferences(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<PreferencesResponse>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;
    let prefs = db::notifications::get_preferences(&state.pool, workspace_id, user.id)
        .await?
        .map(PreferencesResponse::from)
        .unwrap_or_default();
    Ok(Json(prefs))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesBody {
    muted_until: Option<String>,
    disabled_categories: Option<Vec<String>>,
    min_severity: Option<String>,
}

/// PUT /api/workspaces/{workspace_id}/notification-preferences
pub async fn put_preferences(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<PreferencesBody>,
) -> AppResult<Json<PreferencesResponse>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let muted_until = parse_ts(&body.muted_until, "mutedUntil")?;
    if let Some(until) = muted_until {
        let now = chrono::Utc::now();
        if until <= now {
            return Err(AppError::Validation("mutedUntil must be in the future".into()));
        }
        if until > now + chrono::Duration::days(30) {
            return Err(AppError::Validation(
                "mutedUntil must be within 30 days".into(),
            ));
        }
    }

    let disabled_categories = body.disabled_categories.unwrap_or_default();
    if disabled_categories.len() > CATEGORIES.len() {
        return Err(AppError::Validation("too many disabled categories".into()));
    }
    for category in &disabled_categories {
        if !CATEGORIES.contains(&category.as_str()) {
            return Err(AppError::Validation("invalid disabled category".into()));
        }
    }

    let min_severity = match body.min_severity.as_deref().map(str::trim) {
        None | Some("") => "info".to_string(),
        Some(s) if SEVERITIES.contains(&s) => s.to_string(),
        Some(_) => return Err(AppError::Validation("invalid minimum severity".into())),
    };

    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut tx = state.pool.begin().await?;
    let row = db::notifications::put_preferences(
        &mut tx,
        workspace_id,
        user.id,
        muted_until,
        &disabled_categories,
        &min_severity,
    )
    .await?;
    // Metadata records the SHAPE of the change (counts + level), never
    // anything sensitive — consistent with the value-free audit convention.
    sqlx::query(
        r#"
        INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'notification.preferences_updated', 'workspace', $1, $3, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(json!({
        "disabledCategories": disabled_categories.len(),
        "minSeverity": min_severity,
        "muted": muted_until.is_some(),
    }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Json(PreferencesResponse::from(row)))
}
