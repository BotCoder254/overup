//! Event-driven job scheduler.
//!
//! Woken by a `Notify` poke whenever anything scheduling-relevant happens
//! (pipeline created, job finished, runner connected) and by a steady tick
//! for the safety sweeps. Every assignment is an atomic two-row claim, so
//! even overlapping passes — or a future multi-instance deployment — can
//! never double-assign a job or a runner. Every non-terminal state is
//! self-healing: unacked assignments revert, stale runners are orphaned,
//! and budgets are enforced by the timeout sweeps.

use std::collections::HashSet;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::db;
use crate::models::pipeline::PipelineJob;
use crate::models::runner::RunnerResponse;
use crate::services::pipeline_run;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::state::AppState;

/// Runner must ack an assignment within this window.
const ACK_TIMEOUT_SECS: i64 = 15;
/// Runner is considered lost after this long without a heartbeat.
const RUNNER_STALE_SECS: i64 = 90;
/// Signed job payloads are valid this long.
const PAYLOAD_TTL_MINUTES: i64 = 5;
/// Max jobs considered per pass.
const ASSIGN_BATCH: i64 = 50;

#[derive(Default)]
pub struct Scheduler {
    notify: Notify,
}

impl Scheduler {
    /// Wake the loop; cheap and callable from anywhere.
    pub fn poke(&self) {
        self.notify.notify_one();
    }
}

/// The scheduler loop, spawned once from main.
pub async fn run(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = state.scheduler.notify.notified() => {}
            _ = interval.tick() => {}
        }
        if let Err(error) = pass(&state).await {
            tracing::warn!(error = ?error, "scheduler pass failed");
        }
    }
}

async fn pass(state: &AppState) -> anyhow::Result<()> {
    revert_unacked(state).await?;
    sweep_stale_runners(state).await?;
    sweep_timeouts(state).await?;
    assign_eligible(state).await?;
    Ok(())
}

/// Assignments the runner never acknowledged go back to the queue and the
/// runner is freed.
async fn revert_unacked(state: &AppState) -> anyhow::Result<()> {
    let reverted = db::pipeline_jobs::revert_unacked(&state.pool, ACK_TIMEOUT_SECS).await?;
    for (job_id, pipeline_id, runner_id) in reverted {
        if let Some(runner_id) = runner_id {
            db::runners::release(&state.pool, runner_id).await?;
        }
        if let Some(job) =
            db::pipeline_jobs::find_for_pipeline(&state.pool, pipeline_id, job_id).await?
        {
            pipeline_run::on_job_requeued(state, &job).await?;
        }
    }
    Ok(())
}

/// Half-open connections: heartbeats stopped but the socket never closed.
async fn sweep_stale_runners(state: &AppState) -> anyhow::Result<()> {
    let stale = db::runners::find_stale(&state.pool, RUNNER_STALE_SECS).await?;
    for runner in stale {
        tracing::warn!(runner_id = %runner.id, "runner stopped heartbeating; orphaning its jobs");
        let (runner_id, workspace_id) = (runner.id, runner.workspace_id);
        state.runner_hub.unregister(runner_id);
        db::runners::mark_offline(&state.pool, runner_id).await?;
        pipeline_run::orphan_runner_jobs(state, runner_id).await?;
        // Sweep-interval scale, not chatty — the one server-initiated
        // offline transition not already covered by a handler call site.
        if let Some(updated) = db::runners::find_by_id(&state.pool, workspace_id, runner_id).await? {
            state.workspace_hub.publish(
                workspace_id,
                WorkspaceEvent::RunnerUpdate { runner: RunnerResponse::from(updated) },
            );
        }
    }
    Ok(())
}

async fn sweep_timeouts(state: &AppState) -> anyhow::Result<()> {
    for job in db::pipeline_jobs::find_timed_out(&state.pool).await? {
        pipeline_run::timeout_job(state, &job).await?;
    }
    for pipeline in db::pipelines::find_timed_out(&state.pool).await? {
        pipeline_run::timeout_pipeline(state, &pipeline).await?;
    }
    Ok(())
}

/// Match eligible jobs to connected idle runners and dispatch signed
/// payloads.
async fn assign_eligible(state: &AppState) -> anyhow::Result<()> {
    let connected = state.runner_hub.connected_ids();
    if connected.is_empty() {
        return Ok(());
    }
    let idle = db::runners::find_idle_by_ids(&state.pool, &connected).await?;
    if idle.is_empty() {
        return Ok(());
    }
    let eligible = db::pipeline_jobs::find_eligible(&state.pool, ASSIGN_BATCH).await?;
    if eligible.is_empty() {
        return Ok(());
    }

    let mut used: HashSet<Uuid> = HashSet::new();
    for job in eligible {
        let Some(runner) = idle.iter().find(|runner| {
            !used.contains(&runner.id) && labels_satisfy(&job.runs_on, &runner.labels)
        }) else {
            continue;
        };

        let Some(claimed) =
            db::pipeline_jobs::claim_for_runner(&state.pool, job.id, runner.id).await?
        else {
            continue;
        };
        used.insert(runner.id);

        match dispatch(state, &claimed, runner.id).await {
            Ok(DispatchOutcome::Sent) => {
                pipeline_run::on_job_assigned(state, &claimed).await?;
            }
            Ok(DispatchOutcome::CheckoutUnavailable) => {
                // The repository needs checkout credentials that could not
                // be minted. Unclaiming would re-dispatch and re-fail in a
                // tight loop, so the job fails with a static category.
                state.log_hub.clear_masks(claimed.id);
                db::runners::release(&state.pool, runner.id).await?;
                used.remove(&runner.id);
                if let Some(finished) = db::pipeline_jobs::force_finish(
                    &state.pool,
                    claimed.id,
                    "failure",
                    Some("checkout_unavailable"),
                )
                .await?
                {
                    pipeline_run::on_job_finished(state, &finished).await?;
                }
            }
            Ok(DispatchOutcome::Unreachable) | Err(_) => {
                // Couldn't build or deliver the payload: undo the claim.
                state.log_hub.clear_masks(claimed.id);
                db::pipeline_jobs::unclaim(&state.pool, claimed.id).await?;
                db::runners::release(&state.pool, runner.id).await?;
                used.remove(&runner.id);
            }
        }
    }
    Ok(())
}

/// GitHub-style label matching: every requested label must be offered by
/// the runner. Jobs with no labels accept any runner.
fn labels_satisfy(runs_on: &[String], runner_labels: &[String]) -> bool {
    runs_on
        .iter()
        .all(|wanted| runner_labels.iter().any(|have| have == wanted))
}

/// Commit refs are interpolated into the tarball URL; only plain hex-ish
/// revision identifiers pass.
fn is_safe_commit_ref(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric())
}

/// How one dispatch attempt ended.
enum DispatchOutcome {
    /// Payload signed and delivered to the runner's connection.
    Sent,
    /// The runner's connection was gone; the claim should be undone.
    Unreachable,
    /// Checkout credentials exist for this repository but could not be
    /// minted; running without source would be worse than failing.
    CheckoutUnavailable,
}

/// Build, sign, and send the job payload.
async fn dispatch(
    state: &AppState,
    job: &PipelineJob,
    runner_id: Uuid,
) -> anyhow::Result<DispatchOutcome> {
    let pipeline = db::pipelines::find_by_id(&state.pool, job.pipeline_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("pipeline row vanished"))?;
    let repository = db::repositories::find_for_workspace(
        &state.pool,
        pipeline.workspace_id,
        pipeline.repository_id,
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("repository row vanished"))?;

    // Short-lived, contents:read-scoped checkout credentials. Ok(None)
    // means the workspace has no (live) installation — an intentional
    // configuration state, so the job runs checkout-less. An actual minting
    // failure fails the job instead: running the workflow against no source
    // would look like success while testing nothing.
    let checkout = match build_checkout(state, &repository, &pipeline.commit_sha).await {
        Ok(checkout) => checkout,
        Err(error) => {
            tracing::warn!(
                pipeline_id = %pipeline.id,
                error = ?error,
                "checkout token minting failed; failing the job"
            );
            return Ok(DispatchOutcome::CheckoutUnavailable);
        }
    };

    let env: std::collections::BTreeMap<String, String> = job
        .plan
        .get("env")
        .and_then(|e| e.as_object())
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let steps: Vec<protocol::JobStep> = job
        .plan
        .get("steps")
        .and_then(|s| s.as_array())
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| {
                    Some(protocol::JobStep {
                        name: step.get("name")?.as_str()?.to_string(),
                        run: step.get("run")?.as_str()?.to_string(),
                        shell: step
                            .get("shell")
                            .and_then(|s| s.as_str())
                            .unwrap_or("sh")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Everything that must never surface in logs is registered before the
    // payload leaves the process.
    let mut masks: Vec<String> = checkout
        .as_ref()
        .map(|c: &protocol::Checkout| vec![c.token.clone()])
        .unwrap_or_default();
    masks.extend(
        env.iter()
            .filter(|(key, _)| crate::models::pipeline::looks_confidential(key))
            .map(|(_, value)| value.clone()),
    );
    state.log_hub.register_masks(job.id, masks);

    let now = Utc::now();
    let payload = protocol::JobPayload {
        job_id: job.id,
        pipeline_id: job.pipeline_id,
        runner_id,
        issued_at: now,
        expires_at: now + chrono::Duration::minutes(PAYLOAD_TTL_MINUTES),
        image: job
            .plan
            .get("image")
            .and_then(|i| i.as_str())
            .unwrap_or(&state.config.default_job_image)
            .to_string(),
        env,
        steps,
        checkout,
        timeout_seconds: job.timeout_seconds.max(0) as u64,
        caps: protocol::JobCaps {
            max_log_bytes: state.config.max_log_bytes_per_job as u64,
            max_artifact_bytes: state.config.max_artifact_bytes as u64,
            max_artifacts: state.config.max_artifacts_per_job as u32,
        },
    };

    let msg = protocol::sign_job_payload(&state.config.runner_job_signing_key, &payload)?;
    Ok(if state.runner_hub.send(runner_id, msg) {
        DispatchOutcome::Sent
    } else {
        DispatchOutcome::Unreachable
    })
}

async fn build_checkout(
    state: &AppState,
    repository: &crate::models::repository::Repository,
    commit_sha: &str,
) -> anyhow::Result<Option<protocol::Checkout>> {
    use crate::services::github_app;

    if !github_app::is_safe_name_segment(&repository.owner)
        || !github_app::is_safe_name_segment(&repository.name)
        || !is_safe_commit_ref(commit_sha)
    {
        anyhow::bail!("repository identity or commit ref failed validation");
    }

    let Some(installation) = db::github_installations::find_for_workspace(
        &state.pool,
        repository.workspace_id,
        repository.installation_id,
    )
    .await?
    else {
        return Ok(None);
    };
    if installation.suspended_at.is_some() {
        return Ok(None);
    }

    let token = state
        .github_app
        .installation_token(&state.http, installation.installation_id)
        .await?;

    Ok(Some(protocol::Checkout {
        tarball_url: format!(
            "https://api.github.com/repos/{}/{}/tarball/{commit_sha}",
            repository.owner, repository.name
        ),
        token,
    }))
}
