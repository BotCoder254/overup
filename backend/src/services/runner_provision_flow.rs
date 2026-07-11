//! Shared "create and wait" hosted-runner flow, called by both the wizard
//! endpoint (`handlers::runners::create_hosted`) and workspace-creation
//! auto-provisioning. Creates the quota-guarded pending rows, then
//! provisions the Docker container(s) in a background task.
//!
//! Credential hygiene: rows are created with NO credential of any kind.
//! The one-time bootstrap token is minted just-in-time by the background
//! task — AFTER the potentially minutes-long image pull — armed onto the
//! row via a guarded UPDATE, injected into the container environment, and
//! wiped (zeroize) immediately after. The plaintext therefore lives for
//! seconds, never spans a long await, and never reaches a response, a log
//! line, or a crash dump of a task parked on the pull.

use std::time::Duration;

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::db;
use crate::error::AppError;
use crate::models::runner::RunnerResponse;
use crate::services::runner_profiles::ResourceProfile;
use crate::services::runner_provisioner::ProvisionParams;
use crate::services::session;
use crate::services::workspace_hub::WorkspaceEvent;
use crate::state::AppState;

/// Budget for the shared image pull (first pull of a fresh image can take
/// minutes on a slow link).
const PULL_TIMEOUT: Duration = Duration::from_secs(600);

/// Budget for creating + starting ONE container once the image is hot.
const PER_INSTANCE_TIMEOUT: Duration = Duration::from_secs(120);

/// Why a hosted-runner creation was refused (mapped to static-category
/// conflicts by the handler, logged by the auto-provision hook).
#[derive(Debug)]
pub enum HostedDenied {
    /// Hosted runners are not configured on this deployment.
    Unavailable,
    /// Configured, but the provisioner has no live Docker connection right
    /// now (the reconnect loop will restore it).
    DockerDown,
    NameTaken,
    QuotaExceeded,
}

/// Everything a hosted-runner creation needs, bundled to keep the function
/// signature within clippy's argument-count lint.
pub struct HostedProvisionRequest<'a> {
    pub workspace_id: Uuid,
    pub created_by: Uuid,
    /// Used verbatim for a single instance, suffixed `-1..-N` otherwise.
    pub name: &'a str,
    pub labels: &'a [String],
    pub profile: ResourceProfile,
    pub instances: u32,
    pub request_id: Option<&'a str>,
}

/// Create the pending managed rows (quota-guarded, credential-less) and
/// spawn one background provision task for the batch. Returns as soon as the
/// rows exist; the wizard/Runners page observes progress through the rows
/// (`status` stays `offline` while provisioning runs, `provision_error` is
/// the static failure signal).
///
/// Callers must have already authorized the actor for this workspace.
pub async fn start_hosted_provision(
    state: &AppState,
    req: HostedProvisionRequest<'_>,
) -> Result<Result<Vec<RunnerResponse>, HostedDenied>, AppError> {
    let Some(provisioner) = state.runner_provisioner.clone() else {
        return Ok(Err(HostedDenied::Unavailable));
    };
    let Some(cfg) = state.config.runner_provisioner.as_ref() else {
        return Ok(Err(HostedDenied::Unavailable));
    };
    // Refuse up front while Docker is unreachable: pending rows created now
    // would only sit until the janitor purges them as failed.
    if !provisioner.available().await {
        return Ok(Err(HostedDenied::DockerDown));
    }

    let outcome = db::runners::create_hosted_pending(
        &state.pool,
        db::runners::CreateHostedPendingParams {
            workspace_id: req.workspace_id,
            base_name: req.name,
            labels: req.labels,
            resource_profile: req.profile.as_str(),
            instances: req.instances,
            created_by: req.created_by,
            request_id: req.request_id,
        },
        cfg.max_per_workspace,
        cfg.max_global,
    )
    .await?;

    let runners = match outcome {
        db::runners::HostedCreateOutcome::Created(runners) => runners,
        db::runners::HostedCreateOutcome::NameTaken => return Ok(Err(HostedDenied::NameTaken)),
        db::runners::HostedCreateOutcome::QuotaExceeded => {
            return Ok(Err(HostedDenied::QuotaExceeded));
        }
    };

    // Respond immediately and provision in the background: the first image
    // pull can take minutes, far past any browser/proxy timeout. Failed rows
    // are NOT deleted (the wizard must be able to observe the failure); the
    // janitor purges them once the bootstrap credential expires — or, for
    // rows that never got armed, once they age out.
    tracing::info!(
        workspace_id = %req.workspace_id,
        runner = %req.name,
        instances = runners.len(),
        profile = req.profile.as_str(),
        "hosted runner provisioning started"
    );
    let responses: Vec<RunnerResponse> = runners.into_iter().map(RunnerResponse::from).collect();
    for response in &responses {
        state.workspace_hub.publish(
            req.workspace_id,
            WorkspaceEvent::RunnerUpdate { runner: response.clone() },
        );
    }

    let workspace_id = req.workspace_id;
    let profile = req.profile;
    let batch: Vec<(Uuid, String, Vec<String>)> = responses
        .iter()
        .map(|r| (r.id, r.name.clone(), r.labels.clone()))
        .collect();
    let task_state = state.clone();
    tokio::spawn(async move {
        // Phase 1: the shared image pull. Deliberately BEFORE any token is
        // minted — no plaintext credential exists while this (potentially
        // minutes-long) future is parked.
        match tokio::time::timeout(PULL_TIMEOUT, provisioner.ensure_image()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(
                    %workspace_id,
                    category = error.category(),
                    detail = %error.detail(),
                    "hosted runner image pull failed"
                );
                for (runner_id, name, _) in &batch {
                    fail_instance(&task_state, workspace_id, *runner_id, name, error.category())
                        .await;
                }
                return;
            }
            Err(_) => {
                tracing::error!(%workspace_id, "hosted runner image pull timed out");
                for (runner_id, name, _) in &batch {
                    fail_instance(&task_state, workspace_id, *runner_id, name, "provision_timeout")
                        .await;
                }
                return;
            }
        }

        // Phase 2: per instance — mint, arm, start, wipe. Sequential on
        // purpose: the image is hot so create+start is fast, and each
        // plaintext token exists only within its own iteration.
        for (runner_id, name, labels) in &batch {
            let (token, token_hash) = session::generate_token();
            // Wiped on drop at the end of this iteration — including the
            // early-continue and panic-unwind paths.
            let token = Zeroizing::new(token);
            let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);

            match db::runners::arm_bootstrap(
                &task_state.pool,
                workspace_id,
                *runner_id,
                &token_hash,
                expires_at,
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => {
                    // Revoked or purged while the image pulled: no credential
                    // was ever live, and no container is ever created.
                    tracing::info!(%workspace_id, %runner_id, "runner row gone before arm; skipping provision");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(%workspace_id, %runner_id, error = ?error, "failed to arm bootstrap credential");
                    fail_instance(&task_state, workspace_id, *runner_id, name, "bootstrap_arm_failed")
                        .await;
                    continue;
                }
            }

            let result = tokio::time::timeout(
                PER_INSTANCE_TIMEOUT,
                provisioner.provision(ProvisionParams {
                    runner_id: *runner_id,
                    workspace_id,
                    name,
                    labels,
                    // An unavoidable transient copy serializes into the
                    // Docker create request; the Zeroizing wrapper bounds
                    // the lifetime of OUR copy to this iteration.
                    bootstrap_token: &token,
                    limits: profile.limits(),
                }),
            )
            .await;

            match result {
                Ok(Ok(container_id)) => {
                    match db::runners::set_container_id(
                        &task_state.pool,
                        workspace_id,
                        *runner_id,
                        &container_id,
                    )
                    .await
                    {
                        Ok(true) => {
                            tracing::info!(%workspace_id, %runner_id, "hosted runner provisioned");
                            audit_system(
                                &task_state,
                                workspace_id,
                                *runner_id,
                                "runner.provisioned",
                                serde_json::json!({
                                    "name": name,
                                    "profile": profile.as_str(),
                                }),
                            )
                            .await;
                            publish_runner(&task_state, workspace_id, *runner_id).await;
                        }
                        Ok(false) => {
                            // Revoked between arm and start: the fresh
                            // container has no owning row — tear it down now.
                            tracing::info!(%workspace_id, %runner_id, "runner row gone after provision; removing orphan container");
                            if let Err(error) =
                                provisioner.deprovision(*runner_id, &container_id).await
                            {
                                tracing::warn!(%runner_id, error = ?error, "failed to remove orphaned hosted runner container");
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%workspace_id, %runner_id, error = ?error, "failed to record runner container id");
                        }
                    }
                }
                Ok(Err(error)) => {
                    // Static category into the row; Docker detail stays here.
                    tracing::error!(
                        %workspace_id, %runner_id,
                        category = error.category(),
                        detail = %error.detail(),
                        "hosted runner provisioning failed"
                    );
                    fail_instance(&task_state, workspace_id, *runner_id, name, error.category())
                        .await;
                }
                Err(_) => {
                    tracing::error!(%workspace_id, %runner_id, "hosted runner provisioning timed out");
                    // The cancelled future may have left a partial container
                    // behind; names are deterministic, so sweep by name.
                    provisioner.cleanup_partial(*runner_id).await;
                    fail_instance(&task_state, workspace_id, *runner_id, name, "provision_timeout")
                        .await;
                }
            }
        }
    });

    Ok(Ok(responses))
}

/// Flag one instance as failed: static category onto the row, a system audit
/// entry, and a live update to watchers (the wizard's polling picks it up
/// either way).
async fn fail_instance(
    state: &AppState,
    workspace_id: Uuid,
    runner_id: Uuid,
    name: &str,
    category: &str,
) {
    if let Err(error) =
        db::runners::set_provision_error(&state.pool, workspace_id, runner_id, category).await
    {
        tracing::warn!(%workspace_id, %runner_id, error = ?error, "failed to record provision error");
    }
    audit_system(
        state,
        workspace_id,
        runner_id,
        "runner.provision_failed",
        serde_json::json!({ "name": name, "category": category }),
    )
    .await;
    publish_runner(state, workspace_id, runner_id).await;
}

/// System-initiated audit entry (`actor_user_id` NULL). Best-effort: an
/// audit-write failure is logged, never fails provisioning.
async fn audit_system(
    state: &AppState,
    workspace_id: Uuid,
    runner_id: Uuid,
    action: &str,
    metadata: serde_json::Value,
) {
    if let Err(error) =
        db::runners::insert_system_runner_audit(&state.pool, workspace_id, runner_id, action, metadata)
            .await
    {
        tracing::warn!(%workspace_id, %runner_id, action, error = ?error, "failed to write runner audit entry");
    }
}

/// Push the row's current state to live dashboard/wizard watchers.
async fn publish_runner(state: &AppState, workspace_id: Uuid, runner_id: Uuid) {
    if let Ok(Some(runner)) = db::runners::find_by_id(&state.pool, workspace_id, runner_id).await {
        state.workspace_hub.publish(
            workspace_id,
            WorkspaceEvent::RunnerUpdate { runner: RunnerResponse::from(runner) },
        );
    }
}
