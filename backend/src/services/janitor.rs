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
    //    was established — nothing worth keeping.
    match db::runners::purge_expired_bootstrap(&state.pool).await {
        Ok(0) => {}
        Ok(purged) => tracing::info!(purged, "purged abandoned runner bootstrap registrations"),
        Err(error) => tracing::warn!(error = ?error, "failed to purge expired runner bootstrap tokens"),
    }
}
