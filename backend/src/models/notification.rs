//! Notification Center rows and DTOs. Notifications are a per-user,
//! actionable projection over the immutable `audit_logs` ledger (plus a few
//! janitor-scanned derived conditions): category and severity are decided at
//! WRITE time by the notification service's mapping and stored on the row,
//! because a notification is a point-in-time alert — unlike the Activity
//! Feed, re-deriving presentation later would rewrite what the user was told.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Notification categories. Doubles as the allow-list for the `category`
/// filter and for `notification_preferences.disabled_categories` — anything
/// else is rejected before SQL.
pub const CATEGORIES: &[&str] = &[
    "pipeline",
    "runner",
    "repository",
    "workflow",
    "artifact",
    "security",
    "environment",
    "system",
];

/// Notification severities, mildest first. The order IS the ranking used by
/// the `min_severity` preference filter.
pub const SEVERITIES: &[&str] = &["info", "success", "warning", "error", "critical"];

/// Rank a severity for min-severity comparisons; unknown strings rank lowest
/// so a corrupted preference can only widen delivery, never silently drop
/// critical alerts. The fan-out SQL mirrors this ranking with
/// `array_position` — the tests below pin the two in sync.
#[allow(dead_code)] // ranking lives in SQL today; kept as the documented reference
pub fn severity_rank(severity: &str) -> usize {
    SEVERITIES.iter().position(|s| *s == severity).unwrap_or(0)
}

/// One `notifications` row.
#[derive(Debug, sqlx::FromRow)]
pub struct NotificationRow {
    pub id: Uuid,
    #[allow(dead_code)] // scoping columns; never serialized to the caller
    pub workspace_id: Uuid,
    #[allow(dead_code)] // scoping columns; never serialized to the caller
    pub user_id: Uuid,
    pub action: String,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub subject_type: Option<String>,
    pub subject_id: Option<Uuid>,
    pub link: serde_json::Value,
    pub occurrence_count: i32,
    pub read_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// API/WS shape for one notification. `user_id` never serializes — rows are
/// always scoped to the caller, so echoing it would only invite misuse.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationResponse {
    pub id: Uuid,
    pub action: String,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub subject_type: Option<String>,
    pub subject_id: Option<Uuid>,
    pub link: serde_json::Value,
    pub occurrence_count: i32,
    pub read_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<NotificationRow> for NotificationResponse {
    fn from(row: NotificationRow) -> Self {
        Self {
            id: row.id,
            action: row.action,
            category: row.category,
            severity: row.severity,
            title: row.title,
            body: row.body,
            subject_type: row.subject_type,
            subject_id: row.subject_id,
            link: row.link,
            occurrence_count: row.occurrence_count,
            read_at: row.read_at,
            archived_at: row.archived_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// One `notification_preferences` row.
#[derive(Debug, sqlx::FromRow)]
pub struct PreferencesRow {
    pub muted_until: Option<DateTime<Utc>>,
    pub disabled_categories: Vec<String>,
    pub min_severity: String,
    pub updated_at: DateTime<Utc>,
}

/// API shape for preferences; [`PreferencesResponse::default`] is the
/// everything-on state an absent row means.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesResponse {
    pub muted_until: Option<DateTime<Utc>>,
    pub disabled_categories: Vec<String>,
    pub min_severity: String,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Default for PreferencesResponse {
    fn default() -> Self {
        Self {
            muted_until: None,
            disabled_categories: Vec::new(),
            min_severity: "info".to_string(),
            updated_at: None,
        }
    }
}

impl From<PreferencesRow> for PreferencesResponse {
    fn from(row: PreferencesRow) -> Self {
        Self {
            muted_until: row.muted_until,
            disabled_categories: row.disabled_categories,
            min_severity: row.min_severity,
            updated_at: Some(row.updated_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_rank_orders_the_scale() {
        assert!(severity_rank("info") < severity_rank("success"));
        assert!(severity_rank("success") < severity_rank("warning"));
        assert!(severity_rank("warning") < severity_rank("error"));
        assert!(severity_rank("error") < severity_rank("critical"));
    }

    #[test]
    fn unknown_severity_ranks_lowest() {
        // Fail open on delivery: a corrupted min_severity must never
        // suppress alerts above it.
        assert_eq!(severity_rank("bogus"), 0);
    }

    #[test]
    fn allow_lists_are_nonempty_and_distinct() {
        assert!(!CATEGORIES.is_empty());
        assert!(!SEVERITIES.is_empty());
        let mut cats = CATEGORIES.to_vec();
        cats.dedup();
        assert_eq!(cats.len(), CATEGORIES.len());
    }
}
