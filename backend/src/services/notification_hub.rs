//! Per-USER live fan-out for the Notification Center bell.
//!
//! Unlike [`super::workspace_hub::WorkspaceHub`] (workspace-keyed), channels
//! here are keyed by user id: notifications are a per-user projection, and a
//! workspace-wide broadcast would leak one member's preference-filtered feed
//! to every other member's socket. Payloads are already caller-scoped
//! [`NotificationResponse`] DTOs (server-rendered static text, no secret
//! values by construction), so this module is pure fan-out.

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::models::notification::NotificationResponse;

/// These are per-user alert deltas, not a log stream — a dropped frame is
/// always corrected by the next REST refetch, so a modest depth is enough.
const CHANNEL_CAPACITY: usize = 64;

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEvent {
    /// A new notification (or a dedup merge — the client patches by id and
    /// picks up the bumped `occurrenceCount`). `inserted` distinguishes a
    /// fresh row (badge +1) from a merge (badge unchanged).
    Notification {
        notification: Box<NotificationResponse>,
        inserted: bool,
    },
    /// Authoritative unread count, pushed after any read/archive mutation so
    /// sibling tabs converge without refetching.
    #[serde(rename_all = "camelCase")]
    UnreadCount { count: i64 },
}

#[derive(Default)]
pub struct NotificationHub {
    channels: DashMap<Uuid, broadcast::Sender<NotificationEvent>>,
}

impl NotificationHub {
    pub fn subscribe(&self, user_id: Uuid) -> broadcast::Receiver<NotificationEvent> {
        self.channels
            .entry(user_id)
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// Fan an event out to one user's sockets; idle channels are pruned
    /// lazily (the WorkspaceHub pattern).
    pub fn publish(&self, user_id: Uuid, event: NotificationEvent) {
        if let Some(tx) = self.channels.get(&user_id)
            && tx.send(event).is_err()
        {
            drop(tx);
            self.channels
                .remove_if(&user_id, |_, tx| tx.receiver_count() == 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(user: &str) -> NotificationEvent {
        NotificationEvent::Notification {
            inserted: true,
            notification: Box::new(NotificationResponse {
                id: Uuid::new_v4(),
                action: "pipeline.completed".into(),
                category: "pipeline".into(),
                severity: "error".into(),
                title: format!("for {user}"),
                body: String::new(),
                subject_type: Some("pipeline".into()),
                subject_id: Some(Uuid::new_v4()),
                link: serde_json::json!({}),
                occurrence_count: 1,
                read_at: None,
                archived_at: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }),
        }
    }

    #[test]
    fn subscribe_receives_publish() {
        let hub = NotificationHub::default();
        let user_id = Uuid::new_v4();
        let mut rx = hub.subscribe(user_id);
        hub.publish(user_id, NotificationEvent::UnreadCount { count: 3 });
        assert!(matches!(
            rx.try_recv().expect("delivered"),
            NotificationEvent::UnreadCount { count: 3 }
        ));
    }

    #[test]
    fn publish_without_subscribers_is_a_no_op() {
        let hub = NotificationHub::default();
        hub.publish(Uuid::new_v4(), sample("nobody"));
        assert!(hub.channels.is_empty());
    }

    #[test]
    fn events_are_isolated_per_user() {
        let hub = NotificationHub::default();
        let (user_a, user_b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut rx_a = hub.subscribe(user_a);
        let mut rx_b = hub.subscribe(user_b);

        hub.publish(user_a, sample("a"));

        assert!(rx_a.try_recv().is_ok());
        assert!(
            rx_b.try_recv().is_err(),
            "user b must never see user a's notifications"
        );
    }

    #[test]
    fn channel_is_pruned_after_last_subscriber_drops() {
        let hub = NotificationHub::default();
        let user_id = Uuid::new_v4();
        let rx = hub.subscribe(user_id);
        drop(rx);
        hub.publish(user_id, NotificationEvent::UnreadCount { count: 0 });
        assert!(hub.channels.is_empty());
    }
}
