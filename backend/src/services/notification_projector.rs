//! Audit-tail notification projector.
//!
//! Tails the append-only `audit_logs` ledger past a persisted singleton
//! checkpoint and materializes per-user notification rows through the
//! mapping in [`super::notification`]. The poke from `WorkspaceHub::publish`
//! is a LATENCY optimization only (the search-indexer pattern) — a 5 s
//! safety tick is the correctness backstop. Each batch commits its inserts
//! and the cursor advance in ONE transaction, so notifications are
//! exactly-once across crashes, and nothing here ever runs inside a request
//! path or blocks one.

use std::time::Duration;

use tokio::sync::Notify;

use crate::db;
use crate::services::notification;
use crate::services::notification_hub::NotificationEvent;
use crate::state::AppState;

/// Coalesce bursts of ledger writes before draining.
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Safety tick: drains the tail even if every poke was lost.
const TICK: Duration = Duration::from_secs(5);
/// Ledger rows per batch transaction.
const BATCH: i64 = 200;

/// Wake handle held in `AppState`; pure signal, no payload — the cursor in
/// Postgres is the only source of truth for what's been projected.
#[derive(Default)]
pub struct NotificationProjector {
    notify: Notify,
}

impl NotificationProjector {
    /// Queue a drain; cheap and callable from anywhere.
    pub fn poke(&self) {
        self.notify.notify_one();
    }
}

/// The projector loop, spawned once from main.
pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = state.notification_projector.notify.notified() => {
                tokio::time::sleep(DEBOUNCE).await;
            }
            _ = tick.tick() => {}
        }
        // Drain until the tail is shorter than one batch; errors warn and
        // leave the cursor untouched, so the next tick retries cleanly.
        loop {
            match drain_batch(&state).await {
                Ok(processed) if processed >= BATCH as usize => continue,
                Ok(_) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, "notification projection batch failed; will retry");
                    break;
                }
            }
        }
    }
}

/// Project one bounded ledger batch. Returns how many ledger rows were
/// covered (not how many notifications were written).
async fn drain_batch(state: &AppState) -> anyhow::Result<usize> {
    let cursor = db::notifications::load_cursor(&state.pool).await?;
    let rows = db::notifications::tail_audit(&state.pool, cursor, BATCH).await?;
    let Some(last) = rows.last() else {
        return Ok(0);
    };
    let (last_at, last_id) = (last.created_at, last.id);

    // Phase 1 (read-only, outside the tx): route + render + resolve
    // recipients. A row that fails to route is skipped, never fatal — the
    // ledger must keep advancing past malformed or orphaned entries.
    let mut prepared = Vec::new();
    for row in &rows {
        // Never project our own lifecycle entries — no feedback loop.
        if row.action.starts_with("notification.") {
            continue;
        }
        let Some(workspace_id) = row.workspace_id else {
            continue;
        };
        let spec = match notification::route(&state.pool, row).await {
            Ok(Some(spec)) => spec,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(error = ?error, audit_id = %row.id, "notification routing failed; entry skipped");
                continue;
            }
        };
        let recipients =
            match notification::recipients_for(&state.pool, workspace_id, row.actor_user_id, &spec)
                .await
            {
                Ok(recipients) => recipients,
                Err(error) => {
                    tracing::warn!(error = ?error, audit_id = %row.id, "notification fan-out failed; entry skipped");
                    continue;
                }
            };
        if !recipients.is_empty() {
            prepared.push((row, workspace_id, spec, recipients));
        }
    }

    // Phase 2: inserts + cursor advance in one transaction (exactly-once).
    let mut tx = state.pool.begin().await?;
    let mut outcomes = Vec::new();
    for (row, workspace_id, spec, recipients) in prepared {
        outcomes.extend(
            notification::insert_for_recipients(&mut tx, workspace_id, &row.action, &spec, &recipients)
                .await?,
        );
    }
    db::notifications::advance_cursor(&mut tx, last_at, last_id).await?;
    tx.commit().await?;

    // Phase 3 (after commit): push live frames. The client bumps its badge
    // for fresh rows and patches merged rows by id.
    for (user_id, outcome) in outcomes {
        state.notification_hub.publish(
            user_id,
            NotificationEvent::Notification {
                inserted: outcome.inserted,
                notification: Box::new(outcome.row.into()),
            },
        );
    }

    Ok(rows.len())
}
