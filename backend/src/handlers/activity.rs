//! Activity Feed API: the workspace's operational audit center, read
//! straight off the immutable `audit_logs` ledger. Strictly read-only —
//! there is no write endpoint, corrections are new ledger entries at the
//! mutating call sites. Reads need `audit.read` (all roles); every filter
//! is allow-listed or length-capped before SQL (the pipelines-ledger
//! pattern), and audit metadata is value-free by construction so nothing
//! confidential can transit this surface.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::activity::{self, ActivityEventResponse};
use crate::services::authz;
use crate::state::AppState;

use super::pipelines::{escape_like, format_cursor, parse_cursor};

const DEFAULT_PAGE: i64 = 30;

/// Feed categories the UI filters on, mapped to the action prefixes they
/// cover (`pipeline` folds in job-level entries; `integration` is the
/// GitHub App installation surface).
fn category_prefixes(category: &str) -> Option<Vec<String>> {
    let prefixes: &[&str] = match category {
        "workspace" => &["workspace"],
        "integration" => &["installation"],
        "repository" => &["repository"],
        "pipeline" => &["pipeline", "job"],
        "runner" => &["runner"],
        "artifact" => &["artifact"],
        "secret" => &["secret"],
        "environment" => &["environment"],
        _ => return None,
    };
    Some(prefixes.iter().map(|p| (*p).to_string()).collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedQuery {
    q: Option<String>,
    category: Option<String>,
    action: Option<String>,
    actor_id: Option<Uuid>,
    created_after: Option<String>,
    created_before: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// Validate every filter in a [`FeedQuery`] into a bound-ready
/// [`db::activity::FeedFilter`] — shared by the JSON feed and the CSV
/// export so both surfaces enforce identical allow-lists and caps.
fn build_feed_filter(query: &FeedQuery) -> AppResult<db::activity::FeedFilter> {
    let action_prefixes = match query.category.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(category) => Some(
            category_prefixes(category)
                .ok_or_else(|| AppError::Validation("invalid category filter".into()))?,
        ),
    };
    let action = match query.action.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(a) if activity::ACTIONS.contains(&a) => Some(a.to_string()),
        Some(_) => return Err(AppError::Validation("invalid action filter".into())),
    };

    let parse_ts = |raw: &Option<String>, name: &'static str| -> AppResult<Option<_>> {
        match raw.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(s) if s.len() <= 64 => chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| Some(dt.with_timezone(&chrono::Utc)))
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
    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 50);

    Ok(db::activity::FeedFilter {
        action_prefixes,
        action,
        actor_id: query.actor_id,
        created_after,
        created_before,
        search_pattern,
        cursor,
        limit,
    })
}

/// GET /api/workspaces/{workspace_id}/activity
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<FeedQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::AUDIT_READ).await?;

    let filter = build_feed_filter(&query)?;
    let rows = db::activity::list_feed(&state.pool, workspace_id, &filter).await?;
    let next_cursor = (rows.len() as i64 == filter.limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.created_at, row.id));
    let events: Vec<ActivityEventResponse> =
        rows.into_iter().map(ActivityEventResponse::from).collect();

    Ok(Json(json!({ "events": events, "nextCursor": next_cursor })))
}

/// GET /api/workspaces/{workspace_id}/activity/summary
pub async fn summary(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::AUDIT_READ).await?;

    let s = db::activity::summary(&state.pool, workspace_id).await?;
    Ok(Json(json!({
        "total": s.total,
        "last24h": s.last_24h,
        "security30d": s.security_30d,
        "failures30d": s.failures_30d,
        "byCategory": {
            "pipeline": s.cat_pipeline,
            "runner": s.cat_runner,
            "repository": s.cat_repository,
            "artifact": s.cat_artifact,
            "secret": s.cat_secret,
            "environment": s.cat_environment,
            "integration": s.cat_integration,
            "workspace": s.cat_workspace,
        },
        "windowDays": 30,
    })))
}

/// Quote one CSV field per RFC 4180 AND neutralize spreadsheet formula
/// injection: audit metadata is workspace-member-supplied text (branch
/// names, repo names, descriptions), and a cell starting with `=`, `+`,
/// `-`, `@`, tab, or CR executes as a formula when the export is opened in
/// Excel/Sheets. Such cells get a leading `'` before quoting.
fn csv_field(raw: &str) -> String {
    let neutralized = match raw.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => format!("'{raw}"),
        _ => raw.to_string(),
    };
    if neutralized.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", neutralized.replace('"', "\"\""))
    } else {
        neutralized
    }
}

/// GET /api/workspaces/{workspace_id}/activity/export
///
/// Compliance export of the (filtered) feed as CSV — same `audit.read`
/// gate and the exact filter validation the JSON feed uses, capped at
/// [`db::activity::EXPORT_MAX_ROWS`] rows. Metadata is value-free by
/// construction, so nothing confidential can leave through this surface.
pub async fn export(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<FeedQuery>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::AUDIT_READ).await?;

    let filter = build_feed_filter(&query)?;
    let rows = db::activity::export_feed(&state.pool, workspace_id, &filter).await?;

    let mut csv = String::from(
        "id,createdAt,action,category,severity,security,actor,subjectType,subjectId,requestId,metadata\r\n",
    );
    for row in rows {
        let (severity, category, security) = activity::classify(&row.action, &row.metadata);
        let line = [
            row.id.to_string(),
            row.created_at.to_rfc3339(),
            row.action.clone(),
            category.to_string(),
            severity.as_str().to_string(),
            security.to_string(),
            row.actor_login.clone().unwrap_or_else(|| "system".into()),
            row.subject_type.clone(),
            row.subject_id.map(|id| id.to_string()).unwrap_or_default(),
            row.request_id.clone().unwrap_or_default(),
            row.metadata.to_string(),
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
                "attachment; filename=\"activity-export.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::csv_field;

    #[test]
    fn csv_field_passes_plain_values_through() {
        assert_eq!(csv_field("pipeline.created"), "pipeline.created");
        assert_eq!(csv_field(""), "");
    }

    #[test]
    fn csv_field_quotes_separators_and_doubles_quotes() {
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn csv_field_neutralizes_formula_injection() {
        // A hostile branch name in audit metadata must not execute when the
        // export is opened in a spreadsheet.
        assert_eq!(
            csv_field("=HYPERLINK(\"http://evil\")"),
            "\"'=HYPERLINK(\"\"http://evil\"\")\""
        );
        assert_eq!(csv_field("+SUM(A1)"), "'+SUM(A1)");
        assert_eq!(csv_field("-2+3"), "'-2+3");
        assert_eq!(csv_field("@cmd"), "'@cmd");
        assert_eq!(csv_field("\tleading-tab"), "'\tleading-tab");
    }
}
