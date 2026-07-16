//! Hourly background maintenance.
//!
//! Every step is independent and warn-on-error, so one failing subsystem
//! (e.g. R2 unreachable) never blocks the others. Batches are bounded per
//! pass; anything left over is picked up an hour later.

use std::time::Duration;

use crate::db;
use crate::state::AppState;

const PASS_INTERVAL: Duration = Duration::from_secs(3600);
const BATCH: i64 = 200;

pub async fn run(state: AppState) {
    let mut interval = tokio::time::interval(PASS_INTERVAL);
    loop {
        interval.tick().await;
        pass(&state).await;
    }
}

async fn pass(state: &AppState) {
    // 1. Expired sessions and abandoned login transactions.
    if let Err(error) =
        db::sessions::delete_expired(&state.pool, state.config.session_idle_timeout_hours).await
    {
        tracing::warn!(error = ?error, "failed to purge expired sessions");
    }
    if let Err(error) = db::oauth_states::delete_expired(&state.pool).await {
        tracing::warn!(error = ?error, "failed to purge expired oauth states");
    }

    // 2. Uploaded artifacts whose retention lapsed: delete the stored
    //    object best-effort (routed to the store that holds it), then flip
    //    the row so downloads stop immediately even if the delete failed
    //    (the next pass retries nothing — the object becomes unreachable
    //    garbage at worst, never a live leak).
    match db::artifacts::find_expired(&state.pool, BATCH).await {
        Ok(expired) => {
            for artifact in expired {
                if let Some(storage) = &state.storage
                    && let Err(error) = storage
                        .store_for(&artifact.storage_backend)
                        .delete_object(&artifact.r2_key)
                        .await
                {
                    tracing::warn!(
                        artifact_id = %artifact.id,
                        error = ?error,
                        "failed to delete expired artifact object from storage"
                    );
                }
                if let Err(error) = db::artifacts::mark_expired(&state.pool, artifact.id).await {
                    tracing::warn!(artifact_id = %artifact.id, error = ?error, "failed to mark artifact expired");
                }
            }
        }
        Err(error) => tracing::warn!(error = ?error, "failed to scan expired artifacts"),
    }

    // 3. Pending artifact rows whose upload was never confirmed. The
    //    presigned PUT expired long ago; delete any half-uploaded object
    //    and drop the row (it was never announced to browsers).
    match db::artifacts::find_stale_pending(
        &state.pool,
        state.config.artifact_pending_ttl_hours,
        BATCH,
    )
    .await
    {
        Ok(stale) => {
            for artifact in stale {
                if let Some(storage) = &state.storage
                    && let Err(error) = storage
                        .store_for(&artifact.storage_backend)
                        .delete_object(&artifact.r2_key)
                        .await
                {
                    tracing::warn!(
                        artifact_id = %artifact.id,
                        error = ?error,
                        "failed to delete abandoned artifact object from storage"
                    );
                }
                if let Err(error) =
                    db::artifacts::delete_stale_pending(&state.pool, artifact.id).await
                {
                    tracing::warn!(artifact_id = %artifact.id, error = ?error, "failed to delete stale pending artifact");
                }
            }
        }
        Err(error) => tracing::warn!(error = ?error, "failed to scan stale pending artifacts"),
    }

    // 4. Hot log chunks whose object-storage archive exists and whose hot
    //    window has lapsed. Only runs with storage configured — without it,
    //    Postgres is the only copy and is never pruned.
    if state.storage.is_some() {
        match db::pipeline_jobs::find_prunable_archived(
            &state.pool,
            state.config.log_hot_retention_days,
            BATCH,
        )
        .await
        {
            Ok(job_ids) if !job_ids.is_empty() => {
                match db::pipeline_logs::delete_for_jobs(&state.pool, &job_ids).await {
                    Ok(pruned) => {
                        tracing::info!(jobs = job_ids.len(), chunks = pruned, "pruned archived log chunks")
                    }
                    Err(error) => {
                        tracing::warn!(error = ?error, "failed to prune archived log chunks")
                    }
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(error = ?error, "failed to scan prunable archived jobs"),
        }
    }

    // 4b. Notification lifecycle: read rows auto-archive after their window,
    //     archived rows hard-delete past retention, and two derived
    //     conditions (stale secrets, artifacts about to expire) generate
    //     reminders. All warn-on-error; each insert path dedups itself.
    match db::notifications::auto_archive_read(
        &state.pool,
        state.config.notification_auto_archive_days,
        BATCH,
    )
    .await
    {
        Ok(0) => {}
        Ok(archived) => tracing::info!(archived, "auto-archived read notifications"),
        Err(error) => tracing::warn!(error = ?error, "failed to auto-archive read notifications"),
    }
    match db::notifications::purge_archived(
        &state.pool,
        state.config.notification_retention_days,
        BATCH,
    )
    .await
    {
        Ok(0) => {}
        Ok(purged) => tracing::info!(purged, "purged archived notifications past retention"),
        Err(error) => tracing::warn!(error = ?error, "failed to purge archived notifications"),
    }
    scan_stale_secrets(state).await;
    scan_expiring_artifacts(state).await;
    scan_queue_congestion(state).await;

    // Docker-touching cleanup below only makes sense with a live daemon
    // connection; skipping it while Docker is down avoids a warn-per-row
    // spray, and the DB-only purges still run either way (any containers
    // left behind are reclaimed by reconciliation once Docker returns).
    let live_provisioner = match &state.runner_provisioner {
        Some(provisioner) if provisioner.available().await => Some(provisioner),
        Some(_) => {
            tracing::debug!("skipping hosted-runner docker cleanup: docker unavailable");
            None
        }
        None => None,
    };

    // 5. Runner rows stuck mid-registration: the wizard's bootstrap token
    //    expired before the runner ever connected, so no permanent identity
    //    was established — nothing worth keeping. Hosted (managed) runners
    //    first get their containers deprovisioned, while the rows still
    //    hold the container ids.
    if let Some(provisioner) = live_provisioner {
        match db::runners::find_expired_bootstrap_managed(&state.pool).await {
            Ok(orphans) => {
                for (runner_id, container_id) in orphans {
                    let Some(container_id) = container_id else { continue };
                    if let Err(error) = provisioner.deprovision(runner_id, &container_id).await {
                        tracing::warn!(%runner_id, error = ?error, "failed to deprovision abandoned hosted runner");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "failed to scan abandoned hosted runners")
            }
        }
    }
    match db::runners::purge_expired_bootstrap(&state.pool).await {
        Ok(purged) if purged.is_empty() => {}
        Ok(purged) => {
            tracing::info!(purged = purged.len(), "purged abandoned runner bootstrap registrations");
            notify_expired_bootstraps(state, purged).await;
        }
        Err(error) => tracing::warn!(error = ?error, "failed to purge expired runner bootstrap tokens"),
    }

    // 5b. Managed rows that never got their bootstrap credential armed (the
    //     provisioning task died before the just-in-time mint, or its
    //     provision_error write failed). Their bootstrap_expires_at is NULL,
    //     so step 5 can't see them; age from created_at instead. Two hours
    //     comfortably exceeds every provisioning budget, so an in-flight
    //     provision is never purged from under its task.
    const STALE_PENDING_HOURS: i64 = 2;
    if let Some(provisioner) = live_provisioner {
        match db::runners::find_stale_pending_managed(&state.pool, STALE_PENDING_HOURS).await {
            Ok(stale) => {
                for (runner_id, container_id) in stale {
                    // Arm precedes container creation, so this should always
                    // be None — deprovision defensively if it isn't.
                    let Some(container_id) = container_id else { continue };
                    if let Err(error) = provisioner.deprovision(runner_id, &container_id).await {
                        tracing::warn!(%runner_id, error = ?error, "failed to deprovision stale pending hosted runner");
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "failed to scan stale pending hosted runners")
            }
        }
    }
    match db::runners::purge_stale_pending_managed(&state.pool, STALE_PENDING_HOURS).await {
        Ok(0) => {}
        Ok(purged) => tracing::info!(purged, "purged stale pending hosted runners"),
        Err(error) => tracing::warn!(error = ?error, "failed to purge stale pending hosted runners"),
    }

    // 6. Reconcile the Docker host against runner rows. Conservative and
    //    idempotent: ambiguous states are skipped, never destroyed, and the
    //    next pass retries anything that failed.
    if let Some(provisioner) = live_provisioner {
        reconcile_managed_containers(state, provisioner).await;
    }
}

/// Rotation reminders for secrets whose value is past the staleness window.
/// Derived STATE, not an event — nothing rides the audit ledger; rows insert
/// directly with a weekly re-fire guard in the candidate query, and the
/// per-user dedup upsert absorbs anything the guard misses.
async fn scan_stale_secrets(state: &AppState) {
    let candidates = match db::notifications::stale_secret_candidates(
        &state.pool,
        db::secrets::SECRET_STALE_DAYS as i32,
        BATCH,
    )
    .await
    {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::warn!(error = ?error, "failed to scan stale secrets for reminders");
            return;
        }
    };
    for secret in candidates {
        let spec = crate::services::notification::NotificationSpec {
            category: "security",
            severity: "warning",
            recipients: crate::services::notification::Recipients::Permission(
                crate::services::authz::SECRETS_MANAGE,
            ),
            suppress_actor: false,
            title: format!(
                "Secret {} has not been rotated in {}+ days",
                secret.name,
                db::secrets::SECRET_STALE_DAYS
            ),
            body: "Rotate the value to keep the workspace's credential posture healthy.".into(),
            subject_type: Some("secret".into()),
            subject_id: Some(secret.id),
            link: serde_json::json!({ "kind": "secret", "secretId": secret.id }),
            dedup_key: Some(format!("secret.stale:{}", secret.id)),
        };
        if let Err(error) = crate::services::notification::deliver_direct(
            state,
            secret.workspace_id,
            "secret.stale",
            &spec,
        )
        .await
        {
            tracing::warn!(secret_id = %secret.id, error = ?error, "failed to deliver stale-secret reminder");
        }
    }
}

/// Expiry warnings for uploaded artifacts inside their last 24 hours.
async fn scan_expiring_artifacts(state: &AppState) {
    let candidates =
        match db::notifications::expiring_artifact_candidates(&state.pool, BATCH).await {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::warn!(error = ?error, "failed to scan expiring artifacts");
                return;
            }
        };
    for artifact in candidates {
        // Warn the pipeline's triggering actor when there is one; push
        // pipelines fall back to everyone who can manage content.
        let recipients = match artifact.triggered_by {
            Some(actor) => crate::services::notification::Recipients::Direct(actor),
            None => crate::services::notification::Recipients::Permission(
                crate::services::authz::CONTENT_WRITE,
            ),
        };
        let spec = crate::services::notification::NotificationSpec {
            category: "artifact",
            severity: "warning",
            recipients,
            suppress_actor: false,
            title: format!("Artifact {} expires within 24 hours", artifact.name),
            body: format!(
                "From pipeline #{} — download it before retention removes it.",
                artifact.pipeline_number
            ),
            subject_type: Some("artifact".into()),
            subject_id: Some(artifact.id),
            link: serde_json::json!({ "kind": "artifact", "artifactId": artifact.id }),
            dedup_key: Some(format!("artifact.expiring:{}", artifact.id)),
        };
        if let Err(error) = crate::services::notification::deliver_direct(
            state,
            artifact.workspace_id,
            "artifact.expiring",
            &spec,
        )
        .await
        {
            tracing::warn!(artifact_id = %artifact.id, error = ?error, "failed to deliver artifact expiry warning");
        }
    }
}

/// Registration tokens that expired unused: one summarized warning per
/// affected workspace (the purge already happened — this is awareness that
/// a planned runner never came online). Dedup absorbs repeats while unread.
async fn notify_expired_bootstraps(state: &AppState, purged: Vec<(uuid::Uuid, String)>) {
    let mut by_workspace: std::collections::HashMap<uuid::Uuid, Vec<String>> =
        std::collections::HashMap::new();
    for (workspace_id, name) in purged {
        by_workspace.entry(workspace_id).or_default().push(name);
    }
    for (workspace_id, names) in by_workspace {
        let title = if names.len() == 1 {
            format!("Runner registration for {} expired unused", names[0])
        } else {
            format!("{} runner registrations expired unused", names.len())
        };
        let spec = crate::services::notification::NotificationSpec {
            category: "runner",
            severity: "warning",
            recipients: crate::services::notification::Recipients::Permission(
                crate::services::authz::CONTENT_WRITE,
            ),
            suppress_actor: false,
            title,
            body: "The registration token was never used to connect a runner. Create a new one from the Runners page.".into(),
            subject_type: Some("runner".into()),
            subject_id: None,
            link: serde_json::json!({ "kind": "runners" }),
            dedup_key: Some(format!("runner.bootstrap_expired:{workspace_id}")),
        };
        if let Err(error) = crate::services::notification::deliver_direct(
            state,
            workspace_id,
            "runner.bootstrap_expired",
            &spec,
        )
        .await
        {
            tracing::warn!(%workspace_id, error = ?error, "failed to deliver bootstrap-expiry notification");
        }
    }
}

/// Scheduler congestion: queued, unassigned jobs older than 15 minutes.
/// Escalates to error when no runner is online at all (nothing can drain
/// the queue). The candidate query carries a 24 h re-fire guard.
async fn scan_queue_congestion(state: &AppState) {
    const CONGESTION_MINUTES: i32 = 15;
    let candidates =
        match db::notifications::congested_workspaces(&state.pool, CONGESTION_MINUTES, BATCH)
            .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::warn!(error = ?error, "failed to scan queue congestion");
                return;
            }
        };
    for workspace in candidates {
        let none_online = workspace.online_runners == 0;
        let spec = crate::services::notification::NotificationSpec {
            category: "system",
            severity: if none_online { "error" } else { "warning" },
            recipients: crate::services::notification::Recipients::Permission(
                crate::services::authz::CONTENT_READ,
            ),
            suppress_actor: false,
            title: format!(
                "{} job{} waiting over {CONGESTION_MINUTES} minutes in the queue",
                workspace.queued_jobs,
                if workspace.queued_jobs == 1 { "" } else { "s" },
            ),
            body: if none_online {
                "No runners are online — queued pipelines cannot start until one connects.".into()
            } else {
                format!(
                    "{} runner{} online but the queue is not draining — check runner labels against the workflows' runs-on values.",
                    workspace.online_runners,
                    if workspace.online_runners == 1 { " is" } else { "s are" },
                )
            },
            subject_type: None,
            subject_id: None,
            link: serde_json::json!({ "kind": "runners" }),
            dedup_key: Some(format!("queue.congested:{}", workspace.workspace_id)),
        };
        if let Err(error) = crate::services::notification::deliver_direct(
            state,
            workspace.workspace_id,
            "queue.congested",
            &spec,
        )
        .await
        {
            tracing::warn!(workspace_id = %workspace.workspace_id, error = ?error, "failed to deliver congestion notification");
        }
    }
}

/// Cross-check `overup.managed=true` containers against live managed runner
/// rows:
///   - container without a live owning row -> deprovision (orphan);
///   - exited container with a live row    -> restart once (daemon restarts,
///     manual stops; crashes are covered by unless-stopped);
///   - row whose recorded container vanished -> flag `container_missing`,
///     but only when the runner is offline AND not connected — a live
///     runner is never touched.
async fn reconcile_managed_containers(
    state: &AppState,
    provisioner: &crate::services::runner_provisioner::RunnerProvisioner,
) {
    let containers = match provisioner.list_managed_containers().await {
        Ok(containers) => containers,
        Err(error) => {
            tracing::warn!(error = ?error, "failed to list managed runner containers");
            return;
        }
    };
    let rows = match db::runners::list_managed_live(&state.pool).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = ?error, "failed to list managed runner rows");
            return;
        }
    };
    let live_rows: std::collections::HashMap<uuid::Uuid, (Option<String>, String)> = rows
        .into_iter()
        .map(|(id, container_id, status)| (id, (container_id, status)))
        .collect();

    let mut seen_container_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut orphans_removed = 0u32;
    for container in containers {
        seen_container_ids.insert(container.container_id.clone());
        match container.runner_id {
            Some(runner_id) if live_rows.contains_key(&runner_id) => {
                if !container.running {
                    if let Err(error) = provisioner.start_container(&container.container_id).await {
                        tracing::warn!(%runner_id, error = ?error, "failed to restart stopped hosted runner container");
                    } else {
                        tracing::info!(%runner_id, "restarted stopped hosted runner container");
                    }
                }
            }
            Some(runner_id) => {
                // Row gone or revoked — the container must go too.
                if let Err(error) =
                    provisioner.deprovision(runner_id, &container.container_id).await
                {
                    tracing::warn!(%runner_id, error = ?error, "failed to deprovision orphan hosted runner container");
                } else {
                    orphans_removed += 1;
                }
            }
            None => {
                // Unparseable label: remove the container; the volume name
                // cannot be derived, so it is left for the operator.
                if let Err(error) = provisioner.remove_container(&container.container_id).await {
                    tracing::warn!(container = %container.container_id, error = ?error, "failed to remove unlabeled managed container");
                } else {
                    orphans_removed += 1;
                }
            }
        }
    }
    if orphans_removed > 0 {
        tracing::info!(orphans_removed, "removed orphan hosted runner containers");
    }

    // Rows pointing at containers that no longer exist on the host.
    for (runner_id, (container_id, status)) in live_rows {
        let Some(container_id) = container_id else { continue };
        if seen_container_ids.contains(&container_id) {
            continue;
        }
        if status != "offline" || state.runner_hub.is_connected(runner_id) {
            continue;
        }
        match db::runners::flag_container_missing(&state.pool, runner_id).await {
            Ok(true) => tracing::info!(%runner_id, "hosted runner container missing; flagged"),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%runner_id, error = ?error, "failed to flag missing hosted runner container")
            }
        }
    }
}
