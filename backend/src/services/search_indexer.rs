//! Event-driven Global Search indexer.
//!
//! Mutating paths mark workspaces dirty through [`SearchIndexer::mark_dirty`]
//! (wired into `WorkspaceHub::publish`, the single choke point every
//! searchable mutation already goes through); this loop drains the dirty set
//! and re-upserts each workspace's documents incrementally. The poke is a
//! LATENCY optimization only — correctness never depends on it: a 60 s
//! safety tick drains anything a poke missed, and a periodic full reconcile
//! pass (upserts without watermarks + delete anti-joins + volume-cap trims)
//! self-heals gaps, renames without timestamps, and deletions — the janitor
//! pattern. Failures re-mark the workspace dirty and warn; they never panic
//! and never block request paths (no indexing happens inside a request).
//!
//! Secret VALUES and log content are never indexed; see `db::search` for the
//! per-entity SELECT lists.

use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::db;
use crate::state::AppState;

/// Coalesce bursts of mutations before draining the dirty set.
const DEBOUNCE: Duration = Duration::from_secs(2);
/// Safety tick: drains dirty workspaces even if a poke was lost.
const TICK: Duration = Duration::from_secs(60);
/// Full self-heal pass (deletes, caps, watermark-free upserts).
const RECONCILE: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct SearchIndexer {
    notify: Notify,
    dirty: DashMap<Uuid, ()>,
}

impl SearchIndexer {
    /// Queue a workspace for re-indexing; cheap and callable from anywhere.
    pub fn mark_dirty(&self, workspace_id: Uuid) {
        self.dirty.insert(workspace_id, ());
        self.notify.notify_one();
    }

    fn drain(&self) -> Vec<Uuid> {
        let ids: Vec<Uuid> = self.dirty.iter().map(|entry| *entry.key()).collect();
        for id in &ids {
            self.dirty.remove(id);
        }
        ids
    }
}

/// The indexer loop, spawned once from main.
pub async fn run(state: AppState) {
    if let Err(error) = backfill_if_empty(&state).await {
        tracing::warn!(error = ?error, "search index backfill failed; reconcile will retry");
    }

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut reconcile = tokio::time::interval(RECONCILE);
    reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = state.search_indexer.notify.notified() => {
                tokio::time::sleep(DEBOUNCE).await;
            }
            _ = tick.tick() => {}
            _ = reconcile.tick() => {
                if let Err(error) = full_reconcile(&state).await {
                    tracing::warn!(error = ?error, "search reconcile pass failed");
                }
                continue;
            }
        }
        for workspace_id in state.search_indexer.drain() {
            if let Err(error) = reindex_workspace(&state, workspace_id, false).await {
                tracing::warn!(error = ?error, %workspace_id, "search reindex failed; requeued");
                state.search_indexer.mark_dirty(workspace_id);
            }
        }
    }
}

/// One bounded set of parameterized upserts per workspace. Incremental mode
/// narrows each scan to rows changed since the stored watermark (with an
/// overlap window absorbing clock skew); full mode re-reads everything and
/// relies on the tuple guard to make unchanged rows no-ops.
async fn reindex_workspace(
    state: &AppState,
    workspace_id: Uuid,
    full: bool,
) -> anyhow::Result<()> {
    let pool = &state.pool;
    let since = |entity_type: &'static str| async move {
        if full {
            Ok::<_, sqlx::Error>(None)
        } else {
            Ok(db::search::watermark(pool, workspace_id, entity_type)
                .await?
                .map(|at| at - chrono::Duration::minutes(5)))
        }
    };

    db::search::upsert_repositories(pool, workspace_id, since("repository").await?).await?;
    db::search::upsert_workflows(pool, workspace_id, since("workflow").await?).await?;
    db::search::upsert_pipelines(pool, workspace_id, since("pipeline").await?).await?;
    db::search::upsert_runners(pool, workspace_id, None).await?;
    db::search::upsert_artifacts(pool, workspace_id, None).await?;
    db::search::upsert_environments(pool, workspace_id, since("environment").await?).await?;
    db::search::upsert_secrets(pool, workspace_id, since("secret").await?).await?;

    // Append-only ledger rides its own (created_at, id) cursor.
    let mut cursor = db::search::load_index_state(pool, workspace_id).await?;
    loop {
        let (advanced, covered) = db::search::append_activity(pool, workspace_id, cursor).await?;
        cursor = advanced;
        db::search::save_index_state(pool, workspace_id, cursor, false).await?;
        if covered < db::search::ACTIVITY_BATCH {
            break;
        }
    }
    Ok(())
}

/// Self-heal pass over every workspace: watermark-free upserts, source
/// anti-join deletes, and volume-cap trims.
async fn full_reconcile(state: &AppState) -> anyhow::Result<()> {
    for workspace_id in db::search::workspace_ids(&state.pool).await? {
        reindex_workspace(state, workspace_id, true).await?;
        db::search::reconcile_deletes(&state.pool, workspace_id).await?;
        db::search::enforce_caps(&state.pool, workspace_id).await?;
        db::search::save_index_state(&state.pool, workspace_id, None, true).await?;
    }
    Ok(())
}

/// Boot: populate a brand-new (or dropped-and-recreated) index in one pass.
async fn backfill_if_empty(state: &AppState) -> anyhow::Result<()> {
    if !db::search::is_empty(&state.pool).await? {
        return Ok(());
    }
    tracing::info!("search index empty; running initial backfill");
    full_reconcile(state).await
}
