use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::middleware::auth::CurrentUser;
use crate::models::artifact::ArtifactResponse;
use crate::models::pipeline::{
    LogChunkResponse, PipelineEventResponse, PipelineJobResponse, PipelineResponse,
};
use crate::services::{authz, pipeline_run};
use crate::state::AppState;

const DEFAULT_PAGE: i64 = 50;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    repository_id: Option<Uuid>,
    workflow_id: Option<Uuid>,
    status: Option<String>,
    conclusion: Option<String>,
    trigger: Option<String>,
    branch: Option<String>,
    triggered_by: Option<Uuid>,
    runner_id: Option<Uuid>,
    created_after: Option<String>,
    created_before: Option<String>,
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

/// GET /api/workspaces/{workspace_id}/pipelines
pub async fn list(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(workspace_id): Path<Uuid>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let cursor = match &query.cursor {
        None => None,
        Some(raw) => Some(parse_cursor(raw)?),
    };

    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, 100);
    let filter = build_list_filter(&query, cursor, limit)?;

    let rows = db::pipelines::list_for_workspace(&state.pool, workspace_id, &filter).await?;
    let next_cursor = (rows.len() as i64 == limit)
        .then(|| rows.last())
        .flatten()
        .map(|row| format_cursor(row.pipeline.created_at, row.pipeline.id));
    let pipelines: Vec<PipelineResponse> = rows.into_iter().map(PipelineResponse::from).collect();

    Ok(Json(json!({ "pipelines": pipelines, "nextCursor": next_cursor })))
}

/// Opaque-ish keyset cursor: `<rfc3339>~<uuid>`.
fn parse_cursor(raw: &str) -> AppResult<(DateTime<Utc>, Uuid)> {
    let invalid = || AppError::Validation("invalid cursor".into());
    if raw.len() > 128 {
        return Err(invalid());
    }
    let (at, id) = raw.split_once('~').ok_or_else(invalid)?;
    let at = DateTime::parse_from_rfc3339(at)
        .map_err(|_| invalid())?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    Ok((at, id))
}

fn format_cursor(at: DateTime<Utc>, id: Uuid) -> String {
    format!("{}~{id}", at.to_rfc3339())
}

/// Validate and normalize the list filters. Everything is allow-listed or
/// length-capped before it reaches SQL; the free-text search is turned into
/// a pre-escaped ILIKE pattern here so the db layer never sees raw input.
fn build_list_filter(
    query: &ListQuery,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> AppResult<db::pipelines::ListFilter> {
    let status = match query.status.as_deref() {
        None | Some("") => None,
        Some(s @ ("queued" | "in_progress" | "completed")) => Some(s.to_string()),
        Some(_) => return Err(AppError::Validation("invalid status filter".into())),
    };

    let conclusion = match query.conclusion.as_deref() {
        None | Some("") => None,
        Some(c @ ("success" | "failure" | "cancelled" | "timed_out" | "partial")) => {
            // A conclusion only exists on completed pipelines; any other
            // status combination can never match.
            if matches!(status.as_deref(), Some(s) if s != "completed") {
                return Err(AppError::Validation(
                    "conclusion filter requires completed status".into(),
                ));
            }
            Some(c.to_string())
        }
        Some(_) => return Err(AppError::Validation("invalid conclusion filter".into())),
    };

    let trigger = match query.trigger.as_deref() {
        None | Some("") => None,
        Some(t @ ("push" | "manual")) => Some(t.to_string()),
        Some(_) => return Err(AppError::Validation("invalid trigger filter".into())),
    };

    let git_ref = match query.branch.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(branch) if branch.len() <= 255 => Some(format!("refs/heads/{branch}")),
        Some(_) => return Err(AppError::Validation("branch filter too long".into())),
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

    Ok(db::pipelines::ListFilter {
        repository_id: query.repository_id,
        workflow_id: query.workflow_id,
        status,
        conclusion,
        trigger,
        git_ref,
        triggered_by: query.triggered_by,
        runner_id: query.runner_id,
        created_after,
        created_before,
        search_pattern,
        cursor,
        limit,
    })
}

/// Escape LIKE metacharacters so user text only ever matches literally.
fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// GET /api/workspaces/{workspace_id}/pipelines/{pipeline_id}
pub async fn detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let row = db::pipelines::find_row_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let jobs: Vec<PipelineJobResponse> =
        db::pipeline_jobs::list_for_pipeline(&state.pool, pipeline_id)
            .await?
            .into_iter()
            .map(PipelineJobResponse::from)
            .collect();
    let events: Vec<PipelineEventResponse> =
        db::pipeline_events::list_for_pipeline(&state.pool, pipeline_id)
            .await?
            .into_iter()
            .map(PipelineEventResponse::from)
            .collect();

    Ok(Json(json!({
        "pipeline": PipelineResponse::from(row),
        "jobs": jobs,
        "events": events,
    })))
}

/// POST /api/workspaces/{workspace_id}/pipelines/{pipeline_id}/cancel
pub async fn cancel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let pipeline = db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    pipeline_run::cancel_pipeline(&state, &pipeline, Some(user.id), request_id).await?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "cancelling" }))).into_response())
}

/// POST /api/workspaces/{workspace_id}/pipelines/{pipeline_id}/rerun
///
/// A rerun is a brand-new pipeline for the same commit, executing the
/// workflow's current stored revision.
pub async fn rerun(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let original = db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let workflow_id = original
        .workflow_id
        .ok_or(AppError::Conflict("the workflow for this pipeline no longer exists"))?;
    let workflow = db::workflows::find_detail_for_workspace(&state.pool, workspace_id, workflow_id)
        .await?
        .ok_or(AppError::Conflict("the workflow for this pipeline no longer exists"))?;
    let repository =
        db::repositories::find_for_workspace(&state.pool, workspace_id, original.repository_id)
            .await?
            .ok_or(AppError::Conflict("the repository for this pipeline no longer exists"))?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let ctx = pipeline_run::TriggerContext {
        trigger: "manual",
        triggered_by: Some(user.id),
        commit_sha: &original.commit_sha,
        commit_message: original.commit_message.as_deref(),
        commit_author: original.commit_author.as_deref(),
        git_ref: &original.git_ref,
        request_id,
    };
    let pipeline = pipeline_run::create_pipeline(
        &state,
        &repository,
        workflow.id,
        &workflow.name,
        &workflow.path,
        &workflow.raw_content,
        &ctx,
    )
    .await?;

    pipeline_run::record_event(
        &state,
        pipeline.id,
        None,
        "pipeline.rerun",
        None,
        None,
        None,
        Some(user.id),
        json!({ "rerunOf": original.id }),
    )
    .await?;

    let row = db::pipelines::find_row_for_workspace(&state.pool, workspace_id, pipeline.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(json!({ "pipeline": PipelineResponse::from(row) })))
        .into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchRequest {
    #[serde(default)]
    branch: Option<String>,
}

/// POST /api/workspaces/{workspace_id}/workflows/{workflow_id}/dispatch
///
/// Manual trigger: runs the workflow's stored revision against the head of
/// the requested branch (default branch when omitted).
pub async fn dispatch(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, workflow_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(body): Json<DispatchRequest>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let workflow = db::workflows::find_detail_for_workspace(&state.pool, workspace_id, workflow_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let repository =
        db::repositories::find_for_workspace(&state.pool, workspace_id, workflow.repository_id)
            .await?
            .ok_or(AppError::NotFound)?;

    let branch_name = match body.branch.as_deref() {
        None | Some("") => repository.default_branch.clone(),
        Some(name) if name.len() <= 255 => name.to_string(),
        Some(_) => return Err(AppError::Validation("branch name is too long".into())),
    };
    let branches = db::repositories::list_branches(&state.pool, repository.id).await?;
    let branch = branches
        .into_iter()
        .find(|b| b.name == branch_name)
        .ok_or_else(|| AppError::Validation("unknown branch".into()))?;

    let git_ref = format!("refs/heads/{}", branch.name);
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let ctx = pipeline_run::TriggerContext {
        trigger: "manual",
        triggered_by: Some(user.id),
        commit_sha: &branch.commit_sha,
        commit_message: None,
        commit_author: Some(&user.username),
        git_ref: &git_ref,
        request_id,
    };
    let pipeline = pipeline_run::create_pipeline(
        &state,
        &repository,
        workflow.id,
        &workflow.name,
        &workflow.path,
        &workflow.raw_content,
        &ctx,
    )
    .await?;

    let row = db::pipelines::find_row_for_workspace(&state.pool, workspace_id, pipeline.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(json!({ "pipeline": PipelineResponse::from(row) })))
        .into_response())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsQuery {
    from_seq: Option<i64>,
    limit: Option<i64>,
}

/// GET /api/workspaces/{ws}/pipelines/{pipeline}/jobs/{job}/logs
pub async fn job_logs(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id, job_id)): Path<(Uuid, Uuid, Uuid)>,
    Query(query): Query<LogsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    // Scope chain: pipeline in workspace, job in pipeline.
    db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let job = db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline_id, job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let chunks: Vec<LogChunkResponse> = db::pipeline_logs::fetch_range(
        &state.pool,
        job.id,
        query.from_seq.unwrap_or(0).max(0),
        query.limit.unwrap_or(1000),
    )
    .await?
    .into_iter()
    .map(LogChunkResponse::from)
    .collect();

    Ok(Json(json!({ "chunks": chunks, "jobStatus": job.status })))
}

/// GET /api/workspaces/{ws}/pipelines/{pipeline}/jobs/{job}/logs/raw
pub async fn job_logs_raw(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id, job_id)): Path<(Uuid, Uuid, Uuid)>,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let job = db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline_id, job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let chunks = db::pipeline_logs::fetch_all(&state.pool, job.id).await?;

    // Hot chunks pruned but an R2 archive exists: redirect to a short-lived
    // presigned GET (the gzip'd file; browsers download it as-is).
    if chunks.is_empty()
        && job.logs_archived_at.is_some()
        && let Some(r2) = &state.r2
    {
        let key = crate::services::log_archive::log_key_for(workspace_id, &job);
        let filename = format!("{}-{}.log.gz", job.job_key, job.attempt);
        let url = r2
            .presign_get(&key, &filename)
            .await
            .map_err(AppError::Internal)?;
        return Ok((
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, url)],
        )
            .into_response());
    }

    let mut body = String::new();
    for chunk in &chunks {
        body.push_str(&chunk.content);
        if !chunk.content.ends_with('\n') {
            body.push('\n');
        }
    }

    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}-{}.log\"", job.job_key, job.attempt),
            ),
        ],
        body,
    )
        .into_response())
}

/// GET /api/workspaces/{workspace_id}/pipelines/{pipeline_id}/artifacts
pub async fn artifacts(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let artifacts: Vec<ArtifactResponse> =
        db::artifacts::list_for_pipeline(&state.pool, pipeline_id)
            .await?
            .into_iter()
            .map(ArtifactResponse::from)
            .collect();

    Ok(Json(json!({ "artifacts": artifacts })))
}

/// GET /api/workspaces/{workspace_id}/artifacts/{artifact_id}/download
///
/// Returns a short-lived presigned R2 URL as JSON; the blob itself never
/// passes through the control plane.
pub async fn artifact_download(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, artifact_id)): Path<(Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let artifact = db::artifacts::find_for_workspace(&state.pool, workspace_id, artifact_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if artifact.status != "uploaded" {
        return Err(AppError::Conflict("artifact is not available for download"));
    }
    let Some(r2) = &state.r2 else {
        return Err(AppError::Conflict("artifact storage is not configured"));
    };

    let url = r2
        .presign_get(&artifact.r2_key, &artifact.name)
        .await
        .map_err(AppError::Internal)?;
    Ok(Json(json!({ "url": url })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_query() -> ListQuery {
        ListQuery {
            repository_id: None,
            workflow_id: None,
            status: None,
            conclusion: None,
            trigger: None,
            branch: None,
            triggered_by: None,
            runner_id: None,
            created_after: None,
            created_before: None,
            q: None,
            cursor: None,
            limit: None,
        }
    }

    #[test]
    fn escape_like_escapes_metacharacters() {
        assert_eq!(escape_like("50%_done\\x"), "50\\%\\_done\\\\x");
        assert_eq!(escape_like("plain"), "plain");
    }

    #[test]
    fn list_filter_validates_allow_lists() {
        let mut query = empty_query();
        query.status = Some("running".into());
        assert!(build_list_filter(&query, None, 50).is_err());

        let mut query = empty_query();
        query.conclusion = Some("great".into());
        assert!(build_list_filter(&query, None, 50).is_err());

        let mut query = empty_query();
        query.trigger = Some("cron".into());
        assert!(build_list_filter(&query, None, 50).is_err());
    }

    #[test]
    fn conclusion_requires_completed_status() {
        let mut query = empty_query();
        query.status = Some("queued".into());
        query.conclusion = Some("success".into());
        assert!(build_list_filter(&query, None, 50).is_err());

        let mut query = empty_query();
        query.conclusion = Some("success".into());
        let filter = build_list_filter(&query, None, 50).unwrap();
        assert_eq!(filter.conclusion.as_deref(), Some("success"));
    }

    #[test]
    fn branch_becomes_full_ref_and_search_is_escaped() {
        let mut query = empty_query();
        query.branch = Some("main".into());
        query.q = Some("fix 100%".into());
        query.created_after = Some("2026-07-01T00:00:00Z".into());
        let filter = build_list_filter(&query, None, 50).unwrap();
        assert_eq!(filter.git_ref.as_deref(), Some("refs/heads/main"));
        assert_eq!(filter.search_pattern.as_deref(), Some("%fix 100\\%%"));
        assert!(filter.created_after.is_some());

        let mut query = empty_query();
        query.created_before = Some("not-a-date".into());
        assert!(build_list_filter(&query, None, 50).is_err());
    }
}
