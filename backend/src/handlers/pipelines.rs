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
use crate::services::{authz, github_app, pipeline_run};
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

/// Opaque-ish keyset cursor: `<rfc3339>~<uuid>`. Shared with the workspace
/// jobs (queue) handler, which uses the identical cursor shape.
pub(crate) fn parse_cursor(raw: &str) -> AppResult<(DateTime<Utc>, Uuid)> {
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

pub(crate) fn format_cursor(at: DateTime<Utc>, id: Uuid) -> String {
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
        Some(t @ ("push" | "manual" | "pull_request" | "tag")) => Some(t.to_string()),
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
pub(crate) fn escape_like(raw: &str) -> String {
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
        // The rerunning user is the actor, not whoever triggered the original.
        actor_login: Some(&user.username),
        actor_avatar_url: github_app::sanitize_avatar_url(user.avatar_url.as_deref()),
        git_ref: &original.git_ref,
        // Reruns reproduce the original run, inputs included.
        inputs: original.trigger_inputs.as_ref(),
        // A rerun of a PR pipeline keeps its PR association visible.
        pr_number: original.pr_number,
        request_id,
        webhook_delivery_id: None,
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
    .await?
    .pipeline;

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
    /// Optional explicit commit (7-40 hex chars). Defaults to the branch head.
    #[serde(default)]
    commit_sha: Option<String>,
    /// workflow_dispatch-style inputs, validated against the workflow's
    /// parsed input definitions before anything is scheduled.
    #[serde(default)]
    inputs: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Caps for submitted dispatch inputs (defense in depth on top of the
/// parser's own definition caps).
const MAX_INPUT_VALUE_BYTES: usize = 1024;
const MAX_INPUTS_TOTAL_BYTES: usize = 16 * 1024;

/// Validate submitted inputs against the workflow's parsed
/// `on.workflow_dispatch.inputs` definitions and produce the effective map
/// (defaults first, then submitted values). Everything the client sent is
/// checked: unknown names, type mismatches, choice membership, missing
/// required values, and size caps all reject with a 422. Returns `None`
/// when the workflow defines no inputs and none were submitted.
fn validate_dispatch_inputs(
    definitions: &[serde_json::Value],
    submitted: Option<&serde_json::Map<String, serde_json::Value>>,
) -> AppResult<Option<serde_json::Value>> {
    let submitted_len = submitted.map(|m| m.len()).unwrap_or(0);
    if definitions.is_empty() {
        if submitted_len > 0 {
            return Err(AppError::Validation(
                "this workflow does not define workflow_dispatch inputs".into(),
            ));
        }
        return Ok(None);
    }

    let mut effective = serde_json::Map::new();
    let mut known: Vec<&str> = Vec::with_capacity(definitions.len());

    for def in definitions {
        let Some(name) = def["name"].as_str() else {
            continue;
        };
        known.push(name);
        let input_type = def["type"].as_str().unwrap_or("string");
        let required = def["required"].as_bool().unwrap_or(false);
        let default = def["default"].as_str();
        let options: Vec<&str> = def["options"]
            .as_array()
            .map(|opts| opts.iter().filter_map(|o| o.as_str()).collect())
            .unwrap_or_default();

        let value = submitted.and_then(|m| m.get(name));
        let resolved = match value {
            Some(value) => {
                // Type check against the declared input type; choice values
                // must be members of the declared options.
                let ok = match input_type {
                    "boolean" => value.is_boolean(),
                    "number" => value.is_number(),
                    // A choice without declared options has no valid values —
                    // reject rather than accept arbitrary strings.
                    "choice" => value
                        .as_str()
                        .is_some_and(|s| !options.is_empty() && options.contains(&s)),
                    // string | environment
                    _ => value.is_string(),
                };
                if !ok {
                    return Err(AppError::Validation(format!(
                        "input `{name}` has an invalid value for type `{input_type}`"
                    )));
                }
                if serde_json::to_string(value)
                    .map(|s| s.len())
                    .unwrap_or(usize::MAX)
                    > MAX_INPUT_VALUE_BYTES
                {
                    return Err(AppError::Validation(format!("input `{name}` is too large")));
                }
                Some(value.clone())
            }
            None => match default {
                // Defaults were stringified at parse time; coerce back to
                // the declared type so env injection stays consistent.
                Some(default) => Some(match input_type {
                    "boolean" => serde_json::Value::Bool(default == "true"),
                    "number" => default
                        .parse::<f64>()
                        .ok()
                        .and_then(|n| serde_json::Number::from_f64(n).map(serde_json::Value::Number))
                        .unwrap_or_else(|| serde_json::Value::String(default.to_string())),
                    _ => serde_json::Value::String(default.to_string()),
                }),
                None if required => {
                    return Err(AppError::Validation(format!(
                        "required input `{name}` is missing"
                    )));
                }
                None => None,
            },
        };
        if let Some(resolved) = resolved {
            effective.insert(name.to_string(), resolved);
        }
    }

    if let Some(submitted) = submitted {
        for name in submitted.keys() {
            if !known.contains(&name.as_str()) {
                return Err(AppError::Validation(format!("unknown input `{name}`")));
            }
        }
    }

    let effective = serde_json::Value::Object(effective);
    if serde_json::to_string(&effective)
        .map(|s| s.len())
        .unwrap_or(usize::MAX)
        > MAX_INPUTS_TOTAL_BYTES
    {
        return Err(AppError::Validation("inputs are too large".into()));
    }
    Ok(Some(effective))
}

/// POST /api/workspaces/{workspace_id}/workflows/{workflow_id}/dispatch
///
/// Manual trigger: runs the workflow's stored revision against the head of
/// the requested branch (default branch when omitted), optionally pinned to
/// an explicit commit, with validated workflow_dispatch inputs.
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

    // Optional explicit commit pin: hex-validated and lowercased; the ref
    // still names the resolved branch.
    let commit_sha = match body.commit_sha.as_deref().map(str::trim) {
        None | Some("") => branch.commit_sha.clone(),
        Some(sha)
            if (7..=40).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit()) =>
        {
            sha.to_ascii_lowercase()
        }
        Some(_) => {
            return Err(AppError::Validation(
                "commit sha must be 7-40 hexadecimal characters".into(),
            ));
        }
    };

    // Inputs validate against a fresh authoritative re-parse of the stored
    // workflow content — never against client-supplied or stale metadata.
    let parsed = crate::services::workflow_parse::parse_and_validate(&workflow.raw_content);
    let empty = Vec::new();
    let definitions = parsed.metadata["dispatchInputs"]
        .as_array()
        .unwrap_or(&empty);
    let inputs = validate_dispatch_inputs(definitions, body.inputs.as_ref())?;

    let git_ref = format!("refs/heads/{}", branch.name);
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    let ctx = pipeline_run::TriggerContext {
        trigger: "manual",
        triggered_by: Some(user.id),
        commit_sha: &commit_sha,
        commit_message: None,
        commit_author: Some(&user.username),
        actor_login: Some(&user.username),
        actor_avatar_url: github_app::sanitize_avatar_url(user.avatar_url.as_deref()),
        git_ref: &git_ref,
        inputs: inputs.as_ref(),
        pr_number: None,
        request_id,
        webhook_delivery_id: None,
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
    .await?
    .pipeline;

    let row = db::pipelines::find_row_for_workspace(&state.pool, workspace_id, pipeline.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((StatusCode::CREATED, Json(json!({ "pipeline": PipelineResponse::from(row) })))
        .into_response())
}

/// GET /api/workspaces/{ws}/pipelines/{pipeline}/jobs/{job}
///
/// The Job Execution page's identity payload: the job (masked plan), its
/// pipeline, its slice of the event ledger, its artifacts, and the assigned
/// runner's sanitized summary (token-free; health pre-clamped at ingest).
pub async fn job_detail(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id, job_id)): Path<(Uuid, Uuid, Uuid)>,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    // Scope chain: pipeline in workspace, job in pipeline.
    let row = db::pipelines::find_row_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let job = db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline_id, job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let events: Vec<PipelineEventResponse> =
        db::pipeline_events::list_for_job(&state.pool, pipeline_id, job.id)
            .await?
            .into_iter()
            .map(PipelineEventResponse::from)
            .collect();
    let artifacts: Vec<ArtifactResponse> = db::artifacts::list_for_job(&state.pool, job.id)
        .await?
        .into_iter()
        .map(ArtifactResponse::from)
        .collect();
    let runner = match job.runner_id {
        Some(runner_id) => db::runners::find_by_id(&state.pool, workspace_id, runner_id)
            .await?
            .map(crate::models::runner::RunnerResponse::from),
        None => None,
    };

    Ok(Json(json!({
        "job": PipelineJobResponse::from(job),
        "pipeline": PipelineResponse::from(row),
        "events": events,
        "artifacts": artifacts,
        "runner": runner,
    })))
}

/// POST /api/workspaces/{ws}/pipelines/{pipeline}/jobs/{job}/cancel
pub async fn job_cancel(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((workspace_id, pipeline_id, job_id)): Path<(Uuid, Uuid, Uuid)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_WRITE).await?;

    let pipeline = db::pipelines::find_for_workspace(&state.pool, workspace_id, pipeline_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let job = db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline_id, job_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    pipeline_run::cancel_job(&state, &pipeline, &job, Some(user.id), request_id).await?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "cancelling" }))).into_response())
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
        && let Some(storage) = &state.storage
    {
        let key = crate::services::log_archive::log_key_for(workspace_id, &job);
        let filename = format!("{}-{}.log.gz", job.job_key, job.attempt);
        // NULL legacy markers read as 'r2' — every pre-marker archive
        // was written when R2 was the only store.
        let url = storage
            .store_for(job.logs_archive_backend.as_deref().unwrap_or("r2"))
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
    headers: HeaderMap,
) -> AppResult<Json<serde_json::Value>> {
    authz::require_permission(&state.pool, user.id, workspace_id, authz::CONTENT_READ).await?;

    let artifact = db::artifacts::find_for_workspace(&state.pool, workspace_id, artifact_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if artifact.status != "uploaded" {
        return Err(AppError::Conflict("artifact is not available for download"));
    }
    let Some(storage) = &state.storage else {
        return Err(AppError::Conflict("artifact storage is not configured"));
    };

    let url = storage
        .store_for(&artifact.storage_backend)
        .presign_get(&artifact.r2_key, &artifact.name)
        .await
        .map_err(AppError::Internal)?;

    // Downloads are audited like uploads and deletes; the presigned URL
    // itself never appears in the log.
    let request_id = headers.get("x-request-id").and_then(|v| v.to_str().ok());
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'artifact.downloaded', 'artifact', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(user.id)
    .bind(artifact.id)
    .bind(json!({
        "name": artifact.name,
        "sizeBytes": artifact.size_bytes,
        "pipelineId": artifact.pipeline_id,
    }))
    .bind(request_id)
    .execute(&state.pool)
    .await?;

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

        for trigger in ["push", "manual", "pull_request", "tag"] {
            let mut query = empty_query();
            query.trigger = Some(trigger.into());
            let filter = build_list_filter(&query, None, 50).unwrap();
            assert_eq!(filter.trigger.as_deref(), Some(trigger));
        }
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

    fn defs() -> Vec<serde_json::Value> {
        vec![
            json!({ "name": "environment", "type": "choice", "required": true,
                    "options": ["staging", "production"] }),
            json!({ "name": "dry_run", "type": "boolean", "required": false,
                    "default": "true" }),
            json!({ "name": "note", "type": "string", "required": false }),
        ]
    }

    fn map(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn dispatch_inputs_defaults_and_required() {
        // Missing required input rejects.
        assert!(validate_dispatch_inputs(&defs(), None).is_err());

        // Required satisfied; default fills the boolean; optional absent.
        let submitted = map(&[("environment", json!("staging"))]);
        let effective = validate_dispatch_inputs(&defs(), Some(&submitted))
            .unwrap()
            .unwrap();
        assert_eq!(effective["environment"], "staging");
        assert_eq!(effective["dry_run"], json!(true));
        assert!(effective.get("note").is_none());
    }

    #[test]
    fn dispatch_inputs_reject_unknown_type_mismatch_and_bad_choice() {
        let submitted = map(&[("environment", json!("staging")), ("ghost", json!("x"))]);
        assert!(validate_dispatch_inputs(&defs(), Some(&submitted)).is_err());

        let submitted = map(&[("environment", json!("staging")), ("dry_run", json!("yes"))]);
        assert!(validate_dispatch_inputs(&defs(), Some(&submitted)).is_err());

        let submitted = map(&[("environment", json!("nonexistent"))]);
        assert!(validate_dispatch_inputs(&defs(), Some(&submitted)).is_err());
    }

    #[test]
    fn choice_without_options_rejects_any_value() {
        let defs = vec![json!({ "name": "target", "type": "choice", "required": false })];
        let submitted = map(&[("target", json!("anything"))]);
        assert!(validate_dispatch_inputs(&defs, Some(&submitted)).is_err());
        // Omitting the optional option-less choice is still fine.
        let effective = validate_dispatch_inputs(&defs, None).unwrap().unwrap();
        assert!(effective.as_object().unwrap().is_empty());
    }

    #[test]
    fn dispatch_inputs_none_defined() {
        // No definitions + no submission → no inputs at all.
        assert_eq!(validate_dispatch_inputs(&[], None).unwrap(), None);
        // No definitions + submission → reject.
        let submitted = map(&[("anything", json!("x"))]);
        assert!(validate_dispatch_inputs(&[], Some(&submitted)).is_err());
    }

    #[test]
    fn dispatch_inputs_enforce_size_caps() {
        let defs = vec![json!({ "name": "note", "type": "string", "required": false })];
        let submitted = map(&[("note", json!("x".repeat(MAX_INPUT_VALUE_BYTES + 1)))]);
        assert!(validate_dispatch_inputs(&defs, Some(&submitted)).is_err());
    }
}
