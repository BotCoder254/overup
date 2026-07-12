//! Workspace-wide live fan-out for the Dashboard and Runner Management pages.
//!
//! One broadcast channel per workspace carries typed [`WorkspaceEvent`]s to
//! every subscribed browser WebSocket. Unlike [`super::log_hub::LogHub`],
//! there is no masking or capping here: every payload is already a sanitized,
//! already-public-to-workspace-members DTO (a [`crate::models::runner::RunnerResponse`],
//! already-clamped health JSON, or a handful of pipeline status fields) by
//! the time it reaches this hub, so this module is pure fan-out.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::runner::RunnerResponse;
use crate::services::search_indexer::SearchIndexer;

/// Per-workspace broadcast depth. These are idempotent state deltas, not a
/// log stream — a dropped frame is always superseded by the next event or a
/// normal REST refetch, so a modest capacity is enough.
const CHANNEL_CAPACITY: usize = 128;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceEvent {
    #[serde(rename_all = "camelCase")]
    PipelineUpdate {
        id: Uuid,
        status: String,
        conclusion: Option<String>,
        started_at: Option<DateTime<Utc>>,
        finished_at: Option<DateTime<Utc>>,
    },
    RunnerUpdate {
        runner: RunnerResponse,
    },
    #[serde(rename_all = "camelCase")]
    RunnerHealth {
        runner_id: Uuid,
        health: serde_json::Value,
        last_seen_at: DateTime<Utc>,
    },
    /// An artifact became available (or changed status) somewhere in the
    /// workspace — the catalog page refetches on it.
    #[serde(rename_all = "camelCase")]
    ArtifactUpdate {
        id: Uuid,
        pipeline_id: Uuid,
        name: String,
        status: String,
        kind: String,
    },
    /// The audit ledger moved in a category that has no dedicated frame
    /// (secrets, environments, repositories, installations). Carries only
    /// the feed category — subscribers invalidate and refetch over REST,
    /// so nothing sensitive can transit this variant.
    ActivityUpdate { category: String },
}

pub struct WorkspaceHub {
    channels: DashMap<Uuid, broadcast::Sender<WorkspaceEvent>>,
    /// Every publish marks its workspace dirty for the Global Search
    /// indexer — this hub is the single choke point all searchable
    /// mutations already flow through. The poke is a latency optimization
    /// only; the indexer's tick + reconcile passes are the correctness
    /// backstop.
    indexer: Arc<SearchIndexer>,
}

impl WorkspaceHub {
    pub fn new(indexer: Arc<SearchIndexer>) -> Self {
        Self {
            channels: DashMap::new(),
            indexer,
        }
    }

    pub fn subscribe(&self, workspace_id: Uuid) -> broadcast::Receiver<WorkspaceEvent> {
        self.channels
            .entry(workspace_id)
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Fan an event out to subscribers; idle channels are pruned lazily.
    pub fn publish(&self, workspace_id: Uuid, event: WorkspaceEvent) {
        // Unconditionally, BEFORE the no-subscriber early-out: search
        // freshness must not depend on someone watching the dashboard.
        self.indexer.mark_dirty(workspace_id);
        if let Some(tx) = self.channels.get(&workspace_id)
            && tx.send(event).is_err()
        {
            drop(tx);
            // Nobody is listening — reclaim the entry.
            self.channels
                .remove_if(&workspace_id, |_, tx| tx.receiver_count() == 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hub() -> WorkspaceHub {
        WorkspaceHub::new(Arc::new(SearchIndexer::default()))
    }

    #[test]
    fn subscribe_creates_channel_and_receives_publish() {
        let hub = test_hub();
        let workspace_id = Uuid::new_v4();
        let mut rx = hub.subscribe(workspace_id);

        hub.publish(
            workspace_id,
            WorkspaceEvent::RunnerHealth {
                runner_id: Uuid::new_v4(),
                health: serde_json::json!({"cpuPermille": 500}),
                last_seen_at: Utc::now(),
            },
        );

        let event = rx.try_recv().expect("event should be delivered");
        assert!(matches!(event, WorkspaceEvent::RunnerHealth { .. }));
    }

    #[test]
    fn publish_without_subscribers_is_a_no_op() {
        let hub = test_hub();
        let workspace_id = Uuid::new_v4();
        // No subscribe() call — publishing must not panic and must not
        // create a lingering channel entry.
        hub.publish(
            workspace_id,
            WorkspaceEvent::PipelineUpdate {
                id: Uuid::new_v4(),
                status: "queued".into(),
                conclusion: None,
                started_at: None,
                finished_at: None,
            },
        );
        assert!(hub.channels.is_empty());
    }

    #[test]
    fn channel_is_pruned_after_last_subscriber_drops() {
        let hub = test_hub();
        let workspace_id = Uuid::new_v4();
        let rx = hub.subscribe(workspace_id);
        assert!(!hub.channels.is_empty());

        drop(rx);
        // The next publish finds no live receivers and reclaims the entry.
        hub.publish(
            workspace_id,
            WorkspaceEvent::PipelineUpdate {
                id: Uuid::new_v4(),
                status: "completed".into(),
                conclusion: Some("success".into()),
                started_at: None,
                finished_at: None,
            },
        );
        assert!(hub.channels.is_empty());
    }

    #[test]
    fn events_are_isolated_per_workspace() {
        let hub = test_hub();
        let ws_a = Uuid::new_v4();
        let ws_b = Uuid::new_v4();
        let mut rx_a = hub.subscribe(ws_a);
        let mut rx_b = hub.subscribe(ws_b);

        hub.publish(
            ws_a,
            WorkspaceEvent::PipelineUpdate {
                id: Uuid::new_v4(),
                status: "queued".into(),
                conclusion: None,
                started_at: None,
                finished_at: None,
            },
        );

        assert!(rx_a.try_recv().is_ok());
        assert!(rx_b.try_recv().is_err(), "workspace b must not see workspace a's events");
    }
}
