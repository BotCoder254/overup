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
    pub git_ref: &'a str,
    pub request_id: Option<&'a str>,
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
) -> AppResult<Pipeline> {
    let plans = pipeline_plan::build_plans(raw_content, &state.config.default_job_image).map_err(
        |err| match err {
            PlanError::Invalid => {
                AppError::Validation("workflow has validation errors and cannot run".into())
            }
            PlanError::NoRunnableJobs => {
                AppError::Validation("workflow defines no runnable jobs".into())
            }
        },
    )?;

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
        git_ref: ctx.git_ref,
        timeout_seconds: state.config.pipeline_timeout_seconds,
        job_timeout_seconds: state.config.job_timeout_seconds,
        request_id: ctx.request_id,
    };
    let (pipeline, _jobs) = db::pipelines::create(&state.pool, &new, &plans).await?;

    // publish_pipeline only fires on later transitions (started/finished);
    // the Dashboard's KPI strip and timeline need to see brand-new pipelines
    // immediately too.
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
    Ok(pipeline)
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

/// Stage telemetry (pulling_image, running, ...).
pub async fn on_job_stage(state: &AppState, job: &PipelineJob) -> sqlx::Result<()> {
    record_event(
        state,
        job.pipeline_id,
        Some(job.id),
        "job.stage",
        None,
        Some(&job.stage),
        job.runner_id,
        None,
        serde_json::json!({}),
    )
    .await?;
    publish_job(state, job);
    Ok(())
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
