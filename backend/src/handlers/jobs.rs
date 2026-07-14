//! Workspace job queue: the scheduler's console.
//!
//! Read-only visibility into every active (queued or in-progress) job across
//! the workspace, each annotated with a static, server-computed reason for
//! its current wait. Reasons are diagnostic labels derived from the same
//! predicates the scheduler uses (dependency completion, label containment,
//! runner liveness) — they never drive control flow.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::pipeline::{QueueJobResponse, QueueJobRow};
use crate::models::runner::Runner;
use crate::services::authz;
use crate::services::scheduler::labels_satisfy;
use crate::state::AppState;

use super::pipelines::{escape_like, format_cursor, parse_cursor};

const DEFAULT_PAGE: i64 = 50;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueQuery {
    status: Option<String>,
    repository_id: Option<Uuid>,
    workflow_id: Option<Uuid>,
    runner_id: Option<Uuid>,
    label: Option<String>,
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// GET /api/workspaces/{workspace_id}/jobs
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<QueueQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 100);
    let filter = build_queue_filter(&query, cursor, limit)?;

    let rows = db::pipeline_jobs::list_queue_for_workspace(&state.pool, workspace_id, &filter)
        .await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.job.queued_at, row.job.id));

    // One fleet snapshot per page: schedulable = live in the hub AND
    // schedulable in the database (mirrors find_idle_by_ids + liveness).
    let connected: std::collections::HashSet<Uuid> =
        state.runner_hub.connected_ids().into_iter().collect();
    let runners = db::runners::list_for_workspace(&state.pool, workspace_id).await?;
    let online: Vec<OnlineRunner> = runners
        .iter()
        .filter(|runner| connected.contains(&runner.id) && is_schedulable(runner))
        .map(|runner| OnlineRunner {
            labels: runner.labels.clone(),
            idle: runner.status == "idle",
        })
        .collect();

    let jobs: Vec<QueueJobResponse> = rows
        .into_iter()
        .map(|row| {
            let reason = compute_queue_reason(&row, &online);
            QueueJobResponse::from_row(row, reason)
        })
        .collect();

    Ok(Json(json!({ "jobs": jobs, "nextCursor": next_cursor })))
}

/// GET /api/workspaces/{workspace_id}/jobs/summary
pub async fn summary(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    // The fleet numbers mirror scheduler eligibility: only runners with a
    // live hub connection count as idle/busy (see queue_summary).
    let connected = state.runner_hub.connected_ids();
    let summary = db::pipeline_jobs::queue_summary(&state.pool, workspace_id, &connected).await?;
    Ok(Json(json!({ "summary": summary })))
}

/// Validate and normalize the queue filters. Everything is allow-listed or
/// length-capped before it reaches SQL; free-text search becomes a
/// pre-escaped ILIKE pattern here so the db layer never sees raw input.
fn build_queue_filter(
    query: &QueueQuery,
    cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)>,
    limit: i64,
) -> AppResult<db::pipeline_jobs::QueueFilter> {
    let status = match query.status.as_deref() {
        None | Some("") => None,
        Some(s @ ("queued" | "in_progress")) => Some(s.to_string()),
        Some(_) => return Err(AppError::Validation("invalid status filter".into())),
    };

    let label = match query.label.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(label) if label.len() <= 128 => Some(label.to_string()),
        Some(_) => return Err(AppError::Validation("label filter too long".into())),
    };

    let search_pattern = match query.q.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(q) if q.len() <= 200 => Some(format!("%{}%", escape_like(q))),
        Some(_) => return Err(AppError::Validation("search query too long".into())),
    };

    Ok(db::pipeline_jobs::QueueFilter {
        status,
        repository_id: query.repository_id,
        workflow_id: query.workflow_id,
        runner_id: query.runner_id,
        label,
        search_pattern,
        cursor,
        limit,
    })
}

/// A runner the scheduler could hand work to right now (modulo being busy).
struct OnlineRunner {
    labels: Vec<String>,
    idle: bool,
}

fn is_schedulable(runner: &Runner) -> bool {
    runner.revoked_at.is_none() && runner.status != "disabled" && runner.draining_at.is_none()
}

/// Static wait/progress vocabulary for the queue view. Advisory only: the
/// hub snapshot and the database can disagree for up to the stale-runner
/// sweep window, and a job can be claimed between diagnosis and response.
fn compute_queue_reason(row: &QueueJobRow, online: &[OnlineRunner]) -> &'static str {
    if row.job.status == "in_progress" {
        return match row.job.stage.as_str() {
            "assigned" => "dispatching",
            "running" => "running",
            _ => "starting",
        };
    }
    if row.blocked_by_needs {
        return "waiting_dependencies";
    }
    if online.is_empty() {
        return "no_runner_online";
    }
    let matching: Vec<&OnlineRunner> = online
        .iter()
        .filter(|runner| labels_satisfy(&row.job.runs_on, &runner.labels))
        .collect();
    if matching.is_empty() {
        return "no_matching_runner";
    }
    if matching.iter().all(|runner| !runner.idle) {
        return "runners_busy";
    }
    // A matching idle runner exists; the scheduler simply hasn't claimed the
    // job yet (normally a sub-second window).
    "waiting_scheduler"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::pipeline::PipelineJob;
    use chrono::Utc;

    fn job(status: &str, stage: &str, runs_on: &[&str]) -> PipelineJob {
        PipelineJob {
            id: Uuid::new_v4(),
            pipeline_id: Uuid::new_v4(),
            job_key: "build".into(),
            name: None,
            runs_on: runs_on.iter().map(|s| s.to_string()).collect(),
            needs: vec![],
            plan: serde_json::json!({}),
            status: status.into(),
            conclusion: None,
            stage: stage.into(),
            runner_id: None,
            attempt: 1,
            exit_code: None,
            error_category: None,
            timeout_seconds: 3600,
            log_bytes: 0,
            position: 0,
            metrics: None,
            logs_archived_at: None,
            queued_at: Utc::now(),
            assigned_at: None,
            started_at: None,
            finished_at: None,
        }
    }

    fn row(status: &str, stage: &str, runs_on: &[&str], blocked: bool) -> QueueJobRow {
        QueueJobRow {
            job: job(status, stage, runs_on),
            pipeline_number: 1,
            repository_id: Uuid::new_v4(),
            repo_full_name: "octo/repo".into(),
            pipeline_workflow_id: None,
            workflow_name: "ci".into(),
            git_ref: "refs/heads/main".into(),
            trigger: "push".into(),
            blocked_by_needs: blocked,
        }
    }

    fn runner(labels: &[&str], idle: bool) -> OnlineRunner {
        OnlineRunner {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            idle,
        }
    }

    #[test]
    fn queue_reason_matrix() {
        let linux = &["self-hosted", "linux"][..];

        // Dependency wait beats everything else for queued jobs.
        assert_eq!(
            compute_queue_reason(&row("queued", "queued", &["linux"], true), &[]),
            "waiting_dependencies"
        );
        // No online schedulable runner at all.
        assert_eq!(
            compute_queue_reason(&row("queued", "queued", &["linux"], false), &[]),
            "no_runner_online"
        );
        // Online runners exist but none offers the requested labels.
        assert_eq!(
            compute_queue_reason(
                &row("queued", "queued", &["windows"], false),
                &[runner(linux, true)]
            ),
            "no_matching_runner"
        );
        // Matching runners exist but all are busy.
        assert_eq!(
            compute_queue_reason(
                &row("queued", "queued", &["linux"], false),
                &[runner(linux, false)]
            ),
            "runners_busy"
        );
        // Matching idle runner: only the claim itself is outstanding.
        assert_eq!(
            compute_queue_reason(
                &row("queued", "queued", &["linux"], false),
                &[runner(linux, true)]
            ),
            "waiting_scheduler"
        );
        // Empty runs_on accepts any runner.
        assert_eq!(
            compute_queue_reason(&row("queued", "queued", &[], false), &[runner(linux, true)]),
            "waiting_scheduler"
        );
        // In-progress stages map to progress labels regardless of runners.
        assert_eq!(
            compute_queue_reason(&row("in_progress", "assigned", &[], false), &[]),
            "dispatching"
        );
        assert_eq!(
            compute_queue_reason(&row("in_progress", "pulling_image", &[], false), &[]),
            "starting"
        );
        assert_eq!(
            compute_queue_reason(&row("in_progress", "running", &[], false), &[]),
            "running"
        );
    }

    fn empty_query() -> QueueQuery {
        QueueQuery {
            status: None,
            repository_id: None,
            workflow_id: None,
            runner_id: None,
            label: None,
            q: None,
            cursor: None,
            limit: None,
        }
    }

    #[test]
    fn queue_filter_validates_allow_lists() {
        let mut query = empty_query();
        query.status = Some("completed".into());
        assert!(build_queue_filter(&query, None, 50).is_err());

        let mut query = empty_query();
        query.status = Some("running".into());
        assert!(build_queue_filter(&query, None, 50).is_err());

        let mut query = empty_query();
        query.status = Some("in_progress".into());
        let filter = build_queue_filter(&query, None, 50).unwrap();
        assert_eq!(filter.status.as_deref(), Some("in_progress"));
    }

    #[test]
    fn queue_filter_caps_and_escapes() {
        let mut query = empty_query();
        query.label = Some("x".repeat(129));
        assert!(build_queue_filter(&query, None, 50).is_err());

        let mut query = empty_query();
        query.q = Some("50%_done".into());
        let filter = build_queue_filter(&query, None, 50).unwrap();
        assert_eq!(filter.search_pattern.as_deref(), Some("%50\\%\\_done%"));

        let mut query = empty_query();
        query.q = Some("x".repeat(201));
        assert!(build_queue_filter(&query, None, 50).is_err());
    }
}
