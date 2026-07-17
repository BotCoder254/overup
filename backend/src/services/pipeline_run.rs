//! The pipeline execution state machine.
//!
//! Every transition follows the same shape: a guarded conditional UPDATE
//! (so races and replays collapse into no-ops), an append to the immutable
//! pipeline_events ledger, and a broadcast to live browser subscribers.
//! Pipelines finalize automatically once every job is terminal; failed or
//! skipped dependencies eagerly skip their dependents so nothing waits on a
//! job that can never succeed.

use uuid::Uuid;

use crate::db;
use crate::error::{AppError, AppResult};
use crate::models::pipeline::{Pipeline, PipelineJob};
use crate::models::repository::Repository;
use crate::services::log_hub::BrowserEvent;
use crate::services::pipeline_plan::{self, PlanError};
use crate::services::workspace_hub::WorkspaceEvent;
use crate::state::AppState;

/// Trigger context shared by webhook pushes and manual dispatches.
pub struct TriggerContext<'a> {
    pub trigger: &'a str,
    pub triggered_by: Option<Uuid>,
    pub commit_sha: &'a str,
    pub commit_message: Option<&'a str>,
    pub commit_author: Option<&'a str>,
    /// Actor identity snapshot: webhook sender (push) or the acting user
    /// (dispatch/rerun). Login capped and avatar sanitized by the caller.
    pub actor_login: Option<&'a str>,
    pub actor_avatar_url: Option<&'a str>,
    pub git_ref: &'a str,
    /// Validated workflow_dispatch-style inputs (manual dispatch/rerun only).
    /// Always an object; values are strings/numbers/booleans, already checked
    /// against the workflow's parsed input definitions by the handler.
    pub inputs: Option<&'a serde_json::Value>,
    /// Pull request number (pull_request trigger; reruns copy the original's).
    pub pr_number: Option<i32>,
    pub request_id: Option<&'a str>,
    /// Webhook delivery id for event-triggered pipelines (None for manual
    /// dispatch/rerun). Makes creation idempotent per (delivery, workflow),
    /// so a retried delivery can never create duplicate pipelines.
    pub webhook_delivery_id: Option<&'a str>,
}

/// Result of `create_pipeline`. `newly_created = false` means a retried
/// webhook delivery hit the idempotency guard and `pipeline` is the row the
/// earlier attempt created (already published + scheduled).
pub struct CreatedPipeline {
    pub pipeline: Pipeline,
    pub newly_created: bool,
}

/// `INPUT_<NAME>` env var name for a dispatch input, GitHub-actions style:
/// uppercased, with every non-alphanumeric character mapped to `_`.
fn input_env_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 6);
    out.push_str("INPUT_");
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push('_');
        }
    }
    out
}

/// Stringify an input value for the job environment (booleans/numbers arrive
/// typed from validation; env vars are strings).
fn input_env_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Plan and persist a new pipeline for one workflow, then wake the
/// scheduler. The plan is snapshotted per job, so later workflow edits never
/// change a queued pipeline.
pub async fn create_pipeline(
    state: &AppState,
    repository: &Repository,
    workflow_id: Uuid,
    workflow_name: &str,
    workflow_path: &str,
    raw_content: &str,
    ctx: &TriggerContext<'_>,
) -> AppResult<CreatedPipeline> {
    let mut plans = pipeline_plan::build_plans(raw_content, &state.config.default_job_image)
        .map_err(|err| match err {
            PlanError::Invalid => {
                AppError::Validation("workflow has validation errors and cannot run".into())
            }
            PlanError::NoRunnableJobs => {
                AppError::Validation("workflow defines no runnable jobs".into())
            }
        })?;

    // Environments resolve live by name at dispatch (deliberately not
    // failed or auto-created here): an unknown name simply means no
    // environment secrets, which deserves a visible notice on the plan.
    for plan in &mut plans {
        let Some(name) = plan.plan["environment"].as_str().map(str::to_string) else {
            continue;
        };
        if db::environments::find_by_name(&state.pool, repository.workspace_id, &name)
            .await?
            .is_none()
            && let Some(notices) = plan.plan["notices"].as_array_mut()
        {
            notices.push(serde_json::json!(format!(
                "environment `{name}` is not defined in this workspace; \
                 environment secrets will not be injected"
            )));
        }
    }

    // Manual dispatch inputs ride into every job's snapshotted plan env as
    // INPUT_<NAME>. Inputs are non-secret by definition (values were
    // validated + capped by the handler); confidential-looking values still
    // get masked by the existing plan-response and log-hub paths.
    if let Some(inputs) = ctx.inputs.and_then(serde_json::Value::as_object)
        && !inputs.is_empty()
    {
        for plan in &mut plans {
            if let Some(env) = plan.plan["env"].as_object_mut() {
                for (name, value) in inputs {
                    env.insert(
                        input_env_name(name),
                        serde_json::Value::String(input_env_value(value)),
                    );
                }
            }
        }
    }

    let new = db::pipelines::NewPipeline {
        workspace_id: repository.workspace_id,
        repository_id: repository.id,
        workflow_id,
        workflow_name,
        workflow_path,
        trigger: ctx.trigger,
        triggered_by: ctx.triggered_by,
        commit_sha: ctx.commit_sha,
        commit_message: ctx.commit_message,
        commit_author: ctx.commit_author,
        actor_login: ctx.actor_login,
        actor_avatar_url: ctx.actor_avatar_url,
        git_ref: ctx.git_ref,
        trigger_inputs: ctx.inputs,
        pr_number: ctx.pr_number,
        timeout_seconds: state.config.pipeline_timeout_seconds,
        job_timeout_seconds: state.config.job_timeout_seconds,
        request_id: ctx.request_id,
        webhook_delivery_id: ctx.webhook_delivery_id,
    };
    let (pipeline, _jobs, newly_created) = db::pipelines::create(&state.pool, &new, &plans).await?;

    // A retried delivery returning an existing pipeline was already
    // published and scheduled on the first attempt — don't repeat either.
    if newly_created {
        // publish_pipeline only fires on later transitions (started/finished);
        // the Dashboard's KPI strip and timeline need to see brand-new
        // pipelines immediately too.
        state.workspace_hub.publish(
            pipeline.workspace_id,
            WorkspaceEvent::PipelineUpdate {
                id: pipeline.id,
                status: pipeline.status.clone(),
                conclusion: pipeline.conclusion.clone(),
                started_at: pipeline.started_at,
                finished_at: pipeline.finished_at,
            },
        );
        state.scheduler.poke();
    }
    Ok(CreatedPipeline {
        pipeline,
        newly_created,
    })
}

fn publish_job(state: &AppState, job: &PipelineJob) {
    state.log_hub.publish(
        job.pipeline_id,
        BrowserEvent::JobUpdate {
            job: job.clone().into(),
        },
    );
}

fn publish_pipeline(state: &AppState, pipeline: &Pipeline) {
    state.log_hub.publish(
        pipeline.id,
        BrowserEvent::PipelineUpdate {
            id: pipeline.id,
            status: pipeline.status.clone(),
            conclusion: pipeline.conclusion.clone(),
            started_at: pipeline.started_at,
            finished_at: pipeline.finished_at,
        },
    );
    // The Dashboard/Runners workspace-wide feed only needs the materially
    // meaningful transitions this helper already gates on (started,
    // finished/failed/cancelled/timed-out) — not every per-job update.
    state.workspace_hub.publish(
        pipeline.workspace_id,
        WorkspaceEvent::PipelineUpdate {
            id: pipeline.id,
            status: pipeline.status.clone(),
            conclusion: pipeline.conclusion.clone(),
            started_at: pipeline.started_at,
            finished_at: pipeline.finished_at,
        },
    );
}

/// Ledger append + live broadcast in one call.
#[allow(clippy::too_many_arguments)]
pub async fn record_event(
    state: &AppState,
    pipeline_id: Uuid,
    job_id: Option<Uuid>,
    event_type: &str,
    from_state: Option<&str>,
    to_state: Option<&str>,
    runner_id: Option<Uuid>,
    actor: Option<Uuid>,
    payload: serde_json::Value,
) -> sqlx::Result<()> {
    let event = db::pipeline_events::append(
        &state.pool,
        pipeline_id,
        job_id,
        event_type,
        from_state,
        to_state,
        runner_id,
        actor,
        payload,
    )
    .await?;
    state.log_hub.publish(
        pipeline_id,
        BrowserEvent::Event {
            event: event.into(),
        },
    );
    Ok(())
}

/// A job was assigned to a runner (post-claim).
pub async fn on_job_assigned(state: &AppState, job: &PipelineJob) -> sqlx::Result<()> {
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        "job.assigned",
        Some("queued"),
        Some("assigned"),
        job.runner_id,
        None,
        serde_json::json!({ "attempt": job.attempt }),
    )
    .await?;
    publish_job(state, job);
    Ok(())
}

/// The runner acknowledged and started executing.
pub async fn on_job_started(state: &AppState, job: &PipelineJob) -> sqlx::Result<()> {
    if let Some(pipeline) = db::pipelines::mark_in_progress(&state.pool, job.pipeline_id).await? {
        record_event(
            state,
            pipeline.id,
            None,
            "pipeline.started",
            Some("queued"),
            Some("in_progress"),
            None,
            None,
            serde_json::json!({}),
        )
        .await?;
        publish_pipeline(state, &pipeline);
        // Report in_progress to GitHub (best-effort, spawned).
        crate::services::github_checks::spawn_started(state, pipeline.id);
    }
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        "job.started",
        Some("assigned"),
        Some("preparing"),
        job.runner_id,
        None,
        serde_json::json!({ "attempt": job.attempt }),
    )
    .await?;
    publish_job(state, job);
    Ok(())
}

/// Stage telemetry (pulling_image, running, ...). `payload` is built
/// server-side from validated shapes only (e.g. `{"containerId": …}` after
/// the hex short-id check) — never runner free text.
pub async fn on_job_stage(
    state: &AppState,
    job: &PipelineJob,
    payload: serde_json::Value,
) -> sqlx::Result<()> {
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        "job.stage",
        None,
        Some(&job.stage),
        job.runner_id,
        None,
        payload,
    )
    .await?;
    publish_job(state, job);
    Ok(())
}

/// Structured per-step progress. The caller has already allow-listed
/// `status` and bounds-checked `index` against the signed plan; the step
/// name is resolved from that plan here — runner text never reaches the
/// ledger.
pub async fn on_job_step(
    state: &AppState,
    job: &PipelineJob,
    index: usize,
    status: &str,
    exit_code: Option<i32>,
) -> sqlx::Result<()> {
    let step_name = job
        .plan
        .get("steps")
        .and_then(|steps| steps.as_array())
        .and_then(|steps| steps.get(index))
        .and_then(|step| step.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("");
    let event_type = if status == "started" {
        "job.step_started"
    } else {
        "job.step_finished"
    };
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        event_type,
        None,
        Some(status),
        job.runner_id,
        None,
        serde_json::json!({
            "stepIndex": index,
            "stepName": step_name,
            "status": status,
            "exitCode": exit_code,
            "attempt": job.attempt,
        }),
    )
    .await
}

/// Shared post-processing for every terminal job transition: free the
/// runner, drop mask state, record + broadcast, skip dependents of
/// non-successful jobs, finalize the pipeline when it is done, and wake the
/// scheduler for the newly unblocked work.
pub async fn on_job_finished(state: &AppState, job: &PipelineJob) -> sqlx::Result<()> {
    // Flush any masked carry-over output, then drop masks after a grace
    // period (straggler chunks racing the completion stay masked).
    state
        .log_hub
        .finish_job(
            &state.pool,
            job.pipeline_id,
            job.id,
            state.config.max_log_bytes_per_job,
        )
        .await?;
    if let Some(runner_id) = job.runner_id {
        db::runners::release(&state.pool, runner_id).await?;
    }

    let conclusion = job.conclusion.as_deref().unwrap_or("failure");
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        "job.completed",
        Some("in_progress"),
        Some(conclusion),
        job.runner_id,
        None,
        serde_json::json!({
            "exitCode": job.exit_code,
            "errorCategory": job.error_category,
            "attempt": job.attempt,
        }),
    )
    .await?;
    publish_job(state, job);

    if conclusion != "success" {
        skip_dependents(state, job.pipeline_id).await?;
    }

    // Archive the (masked) log to R2 in the background; Postgres stays the
    // source of truth until the janitor prunes inside the hot window.
    crate::services::log_archive::spawn_archive(state, job);

    maybe_finalize(state, job.pipeline_id).await?;
    state.scheduler.poke();
    Ok(())
}

/// A job went back to the queue (runner lost, unacked assignment).
pub async fn on_job_requeued(state: &AppState, job: &PipelineJob) -> sqlx::Result<()> {
    // Flush carry and schedule a generation-guarded clear; the re-dispatch
    // re-registers the same values (bumping the generation) so masks stay
    // live if the job is picked up again inside the grace window.
    state
        .log_hub
        .finish_job(
            &state.pool,
            job.pipeline_id,
            job.id,
            state.config.max_log_bytes_per_job,
        )
        .await?;
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        "job.requeued",
        Some("in_progress"),
        Some("queued"),
        None,
        None,
        serde_json::json!({ "attempt": job.attempt }),
    )
    .await?;
    publish_job(state, job);
    state.scheduler.poke();
    Ok(())
}

/// Skip every queued job that (transitively) depends on a job that can no
/// longer succeed.
async fn skip_dependents(state: &AppState, pipeline_id: Uuid) -> sqlx::Result<()> {
    let jobs = db::pipeline_jobs::list_for_pipeline(&state.pool, pipeline_id).await?;

    // Fixpoint over job keys that can never succeed.
    let mut doomed: std::collections::HashSet<&str> = jobs
        .iter()
        .filter(|j| {
            j.status == "completed" && j.conclusion.as_deref() != Some("success")
        })
        .map(|j| j.job_key.as_str())
        .collect();

    let mut to_skip: Vec<Uuid> = Vec::new();
    loop {
        let mut changed = false;
        for job in &jobs {
            if job.status != "queued" || doomed.contains(job.job_key.as_str()) {
                continue;
            }
            if job.needs.iter().any(|need| doomed.contains(need.as_str())) {
                doomed.insert(job.job_key.as_str());
                to_skip.push(job.id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    if to_skip.is_empty() {
        return Ok(());
    }

    let skipped = db::pipeline_jobs::mark_skipped(&state.pool, &to_skip).await?;
    for job in &skipped {
        record_event(
            state,
            pipeline_id,
            Some(job.id),
            "job.completed",
            Some("queued"),
            Some("skipped"),
            None,
            None,
            serde_json::json!({ "reason": "dependency_failed" }),
        )
        .await?;
        publish_job(state, job);
    }
    Ok(())
}

/// GitHub-style conclusion aggregation. None while any job is still live.
pub fn compute_conclusion(statuses: &[(String, Option<String>)]) -> Option<&'static str> {
    if statuses.is_empty() || statuses.iter().any(|(status, _)| status != "completed") {
        return None;
    }
    let has = |c: &str| {
        statuses
            .iter()
            .any(|(_, conclusion)| conclusion.as_deref() == Some(c))
    };
    Some(if has("cancelled") {
        "cancelled"
    } else if has("timed_out") {
        "timed_out"
    } else if has("failure") && has("success") {
        "partial"
    } else if has("failure") {
        "failure"
    } else {
        "success"
    })
}

/// Finalize the pipeline once every job is terminal. Idempotent — the
/// guarded UPDATE makes concurrent calls collapse.
pub async fn maybe_finalize(state: &AppState, pipeline_id: Uuid) -> sqlx::Result<()> {
    let statuses = db::pipeline_jobs::statuses_for_pipeline(&state.pool, pipeline_id).await?;
    let Some(conclusion) = compute_conclusion(&statuses) else {
        return Ok(());
    };
    let Some(pipeline) = db::pipelines::finalize(&state.pool, pipeline_id, conclusion).await?
    else {
        return Ok(());
    };

    record_event(
        state,
        pipeline.id,
        None,
        "pipeline.completed",
        Some("in_progress"),
        Some(conclusion),
        None,
        None,
        serde_json::json!({}),
    )
    .await?;
    publish_pipeline(state, &pipeline);
    // Report the terminal conclusion to GitHub (best-effort, spawned). This
    // is the single terminal convergence point, so cancel/timeout/partial all
    // report through here.
    crate::services::github_checks::spawn_completed(state, pipeline.id);

    sqlx::query(
        r#"
        INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
        VALUES ($1, NULL, 'pipeline.completed', 'pipeline', $2, $3)
        "#,
    )
    .bind(pipeline.workspace_id)
    .bind(pipeline.id)
    .bind(serde_json::json!({ "conclusion": conclusion, "number": pipeline.number }))
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// User-requested cancellation: queued jobs cancel immediately, running jobs
/// get a cancel signal (their runners report the terminal result); a runner
/// that is gone is force-finished here. The pipeline finalizes as soon as
/// every job is terminal.
pub async fn cancel_pipeline(
    state: &AppState,
    pipeline: &Pipeline,
    actor: Option<Uuid>,
    request_id: Option<&str>,
) -> AppResult<()> {
    if pipeline.status == "completed" {
        return Err(AppError::Conflict("pipeline already finished"));
    }

    record_event(
        state,
        pipeline.id,
        None,
        "pipeline.cancel_requested",
        None,
        None,
        None,
        actor,
        serde_json::json!({}),
    )
    .await?;

    let cancelled = db::pipeline_jobs::cancel_queued(&state.pool, pipeline.id).await?;
    for job in &cancelled {
        record_event(
            state,
            pipeline.id,
            Some(job.id),
            "job.completed",
            Some("queued"),
            Some("cancelled"),
            None,
            actor,
            serde_json::json!({}),
        )
        .await?;
        publish_job(state, job);
    }

    let running = db::pipeline_jobs::running_for_pipeline(&state.pool, pipeline.id).await?;
    for job in running {
        let signalled = job.runner_id.is_some_and(|runner_id| {
            state.runner_hub.send(
                runner_id,
                protocol::ServerMsg::JobCancel {
                    job_id: job.id,
                    reason: protocol::CancelReason::User,
                },
            )
        });
        if !signalled {
            // Runner unreachable: terminate authoritatively server-side.
            if let Some(finished) =
                db::pipeline_jobs::force_finish(&state.pool, job.id, "cancelled", None).await?
            {
                on_job_finished(state, &finished).await?;
            }
        }
    }

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'pipeline.cancelled', 'pipeline', $3, $4, $5)
        "#,
    )
    .bind(pipeline.workspace_id)
    .bind(actor)
    .bind(pipeline.id)
    .bind(serde_json::json!({ "number": pipeline.number }))
    .bind(request_id)
    .execute(&state.pool)
    .await?;

    maybe_finalize(state, pipeline.id).await?;
    Ok(())
}

/// User-requested cancellation of a single job. A queued job cancels
/// immediately (guarded UPDATE — a concurrent claim falls through to the
/// running path); a running job gets a cancel signal to its runner, or is
/// force-finished when the runner is unreachable. Dependents of a cancelled
/// job are skipped exactly as for any other non-success conclusion.
pub async fn cancel_job(
    state: &AppState,
    pipeline: &Pipeline,
    job: &PipelineJob,
    actor: Option<Uuid>,
    request_id: Option<&str>,
) -> AppResult<()> {
    if job.status == "completed" {
        return Err(AppError::Conflict("job already finished"));
    }

    record_event(
        state,
        pipeline.id,
        Some(job.id),
        "job.cancel_requested",
        None,
        None,
        None,
        actor,
        serde_json::json!({}),
    )
    .await?;

    if let Some(cancelled) = db::pipeline_jobs::cancel_one_queued(&state.pool, job.id).await? {
        record_event(
            state,
            pipeline.id,
            Some(cancelled.id),
            "job.completed",
            Some("queued"),
            Some("cancelled"),
            None,
            actor,
            serde_json::json!({}),
        )
        .await?;
        publish_job(state, &cancelled);
        skip_dependents(state, pipeline.id).await?;
        maybe_finalize(state, pipeline.id).await?;
    } else {
        // The job was claimed (or started) in the meantime: re-read its
        // current assignment and cancel through the runner.
        let Some(current) =
            db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline.id, job.id).await?
        else {
            return Err(AppError::NotFound);
        };
        if current.status == "completed" {
            return Err(AppError::Conflict("job already finished"));
        }
        let signalled = current.runner_id.is_some_and(|runner_id| {
            state.runner_hub.send(
                runner_id,
                protocol::ServerMsg::JobCancel {
                    job_id: current.id,
                    reason: protocol::CancelReason::User,
                },
            )
        });
        if !signalled {
            // Runner unreachable: terminate authoritatively server-side.
            if let Some(finished) =
                db::pipeline_jobs::force_finish(&state.pool, current.id, "cancelled", None).await?
            {
                on_job_finished(state, &finished).await?;
            }
        }
    }

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'job.cancelled', 'pipeline_job', $3, $4, $5)
        "#,
    )
    .bind(pipeline.workspace_id)
    .bind(actor)
    .bind(job.id)
    .bind(serde_json::json!({ "pipeline": pipeline.id, "number": pipeline.number, "job": job.job_key }))
    .bind(request_id)
    .execute(&state.pool)
    .await?;

    Ok(())
}

/// A running job exhausted its budget: signal the runner (best effort) and
/// finish it authoritatively.
pub async fn timeout_job(state: &AppState, job: &PipelineJob) -> sqlx::Result<()> {
    if let Some(runner_id) = job.runner_id {
        state.runner_hub.send(
            runner_id,
            protocol::ServerMsg::JobCancel {
                job_id: job.id,
                reason: protocol::CancelReason::Timeout,
            },
        );
    }
    if let Some(finished) =
        db::pipeline_jobs::force_finish(&state.pool, job.id, "timed_out", Some("timeout")).await?
    {
        on_job_finished(state, &finished).await?;
    }
    Ok(())
}

/// The whole pipeline exhausted its wall-clock budget.
pub async fn timeout_pipeline(state: &AppState, pipeline: &Pipeline) -> sqlx::Result<()> {
    let cancelled = db::pipeline_jobs::cancel_queued(&state.pool, pipeline.id).await?;
    for job in &cancelled {
        record_event(
            state,
            pipeline.id,
            Some(job.id),
            "job.completed",
            Some("queued"),
            Some("cancelled"),
            None,
            None,
            serde_json::json!({ "reason": "pipeline_timeout" }),
        )
        .await?;
        publish_job(state, job);
    }
    let running = db::pipeline_jobs::running_for_pipeline(&state.pool, pipeline.id).await?;
    for job in running {
        timeout_job(state, &job).await?;
    }
    maybe_finalize(state, pipeline.id).await?;
    Ok(())
}

/// Orphan handling for one disconnected/revoked/stale runner.
pub async fn orphan_runner_jobs(state: &AppState, runner_id: Uuid) -> sqlx::Result<()> {
    let (requeued, failed) = db::pipeline_jobs::requeue_orphans(&state.pool, runner_id).await?;
    for job in &requeued {
        on_job_requeued(state, job).await?;
    }
    for job in &failed {
        on_job_finished(state, job).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::compute_conclusion;

    fn s(status: &str, conclusion: Option<&str>) -> (String, Option<String>) {
        (status.to_string(), conclusion.map(str::to_string))
    }

    #[test]
    fn conclusion_matrix() {
        // Still running -> no conclusion yet.
        assert_eq!(
            compute_conclusion(&[s("completed", Some("success")), s("in_progress", None)]),
            None
        );
        assert_eq!(compute_conclusion(&[]), None);

        assert_eq!(
            compute_conclusion(&[s("completed", Some("success")), s("completed", Some("success"))]),
            Some("success")
        );
        assert_eq!(
            compute_conclusion(&[s("completed", Some("failure")), s("completed", Some("skipped"))]),
            Some("failure")
        );
        assert_eq!(
            compute_conclusion(&[s("completed", Some("failure")), s("completed", Some("success"))]),
            Some("partial")
        );
        assert_eq!(
            compute_conclusion(&[s("completed", Some("success")), s("completed", Some("cancelled"))]),
            Some("cancelled")
        );
        assert_eq!(
            compute_conclusion(&[s("completed", Some("timed_out")), s("completed", Some("failure"))]),
            Some("timed_out")
        );
        // Skipped jobs alone never fail a pipeline.
        assert_eq!(
            compute_conclusion(&[s("completed", Some("success")), s("completed", Some("skipped"))]),
            Some("success")
        );
    }
}
