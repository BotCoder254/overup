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
    if let Err(error) = db::sessions::delete_expired(&state.pool).await {
        tracing::warn!(error = ?error, "failed to purge expired sessions");
    }
    if let Err(error) = db::oauth_states::delete_expired(&state.pool).await {
        tracing::warn!(error = ?error, "failed to purge expired oauth states");
    }

    // 2. Uploaded artifacts whose retention lapsed: delete the R2 object
    //    best-effort, then flip the row so downloads stop immediately even
    //    if the delete failed (the next pass retries nothing — the object
    //    becomes unreachable garbage at worst, never a live leak).
    match db::artifacts::find_expired(&state.pool, BATCH).await {
        Ok(expired) => {
            for artifact in expired {
                if let Some(r2) = &state.r2
                    && let Err(error) = r2.delete_object(&artifact.r2_key).await
                {
                    tracing::warn!(
                        artifact_id = %artifact.id,
                        error = ?error,
                        "failed to delete expired artifact object from R2"
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
                if let Some(r2) = &state.r2
                    && let Err(error) = r2.delete_object(&artifact.r2_key).await
                {
                    tracing::warn!(
                        artifact_id = %artifact.id,
                        error = ?error,
                        "failed to delete abandoned artifact object from R2"
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

    // 4. Hot log chunks whose R2 archive exists and whose hot window has
    //    lapsed. Only runs with R2 configured — without it, Postgres is the
    //    only copy and is never pruned.
    if state.r2.is_some() {
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

    // 5. Runner rows stuck mid-registration: the wizard's bootstrap token
    //    expired before the runner ever connected, so no permanent identity
    //    was established — nothing worth keeping. Hosted (managed) runners
    //    first get their containers deprovisioned, while the rows still
    //    hold the container ids.
    if let Some(provisioner) = &state.runner_provisioner {
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
        Ok(0) => {}
        Ok(purged) => tracing::info!(purged, "purged abandoned runner bootstrap registrations"),
        Err(error) => tracing::warn!(error = ?error, "failed to purge expired runner bootstrap tokens"),
    }

    // 5b. Managed rows that never got their bootstrap credential armed (the
    //     provisioning task died before the just-in-time mint, or its
    //     provision_error write failed). Their bootstrap_expires_at is NULL,
    //     so step 5 can't see them; age from created_at instead. Two hours
    //     comfortably exceeds every provisioning budget, so an in-flight
    //     provision is never purged from under its task.
    const STALE_PENDING_HOURS: i64 = 2;
    if let Some(provisioner) = &state.runner_provisioner {
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
    if let Some(provisioner) = &state.runner_provisioner {
        reconcile_managed_containers(state, provisioner).await;
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
