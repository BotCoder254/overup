//! The immutable per-repository event timeline: one row per processed
//! webhook delivery that concerned a connected repository, recording what
//! arrived and what it caused. `outcome` / `ignored_reason` hold STATIC
//! category strings only (consts in `services/webhook_processor.rs`) —
//! never upstream text.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug)]
pub struct NewRepositoryEvent<'a> {
    pub repository_id: Uuid,
    pub delivery_id: &'a str,
    pub event: &'a str,
    pub action: Option<&'a str>,
    pub git_ref: Option<&'a str>,
    pub head_sha: Option<&'a str>,
    pub actor_login: Option<&'a str>,
    pub actor_avatar_url: Option<&'a str>,
    /// Static category: pipelines_created | sync_scheduled |
    /// pipelines_and_sync | ignored | failed.
    pub outcome: &'a str,
    /// Static category, only when outcome = 'ignored'.
    pub ignored_reason: Option<&'a str>,
    pub pipeline_ids: &'a [Uuid],
    pub sync_run_id: Option<Uuid>,
    /// Server-built, capped JSON (skipped workflows, PR number, merged flag).
    pub summary: &'a serde_json::Value,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct RepositoryEventRow {
    pub id: Uuid,
    pub event: String,
    pub action: Option<String>,
    pub git_ref: Option<String>,
    pub head_sha: Option<String>,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub outcome: String,
    pub ignored_reason: Option<String>,
    pub pipeline_ids: Vec<Uuid>,
    pub sync_run_id: Option<Uuid>,
    pub summary: serde_json::Value,
    pub received_at: DateTime<Utc>,
    pub processed_at: DateTime<Utc>,
}

/// Insert one timeline row inside the delivery-completion transaction.
pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ev: &NewRepositoryEvent<'_>,
) -> sqlx::Result<Uuid> {
    let (id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO repository_events
            (repository_id, delivery_id, event, action, git_ref, head_sha,
             actor_login, actor_avatar_url, outcome, ignored_reason,
             pipeline_ids, sync_run_id, summary, received_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        RETURNING id
        "#,
    )
    .bind(ev.repository_id)
    .bind(ev.delivery_id)
    .bind(ev.event)
    .bind(ev.action)
    .bind(ev.git_ref)
    .bind(ev.head_sha)
    .bind(ev.actor_login)
    .bind(ev.actor_avatar_url)
    .bind(ev.outcome)
    .bind(ev.ignored_reason)
    .bind(ev.pipeline_ids)
    .bind(ev.sync_run_id)
    .bind(ev.summary)
    .bind(ev.received_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(id)
}

/// Keyset-paginated timeline for one repository, newest first. The cursor is
/// the (processed_at, id) tuple of the last row seen (the pipelines-list
/// pattern).
pub async fn list_for_repo(
    pool: &PgPool,
    repository_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> sqlx::Result<Vec<RepositoryEventRow>> {
    let (cursor_at, cursor_id) = match cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, RepositoryEventRow>(
        r#"
        SELECT id, event, action, git_ref, head_sha, actor_login, actor_avatar_url,
               outcome, ignored_reason, pipeline_ids, sync_run_id, summary,
               received_at, processed_at
        FROM repository_events
        WHERE repository_id = $1
          AND ($2::timestamptz IS NULL OR (processed_at, id) < ($2, $3))
        ORDER BY processed_at DESC, id DESC
        LIMIT $4
        "#,
    )
    .bind(repository_id)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await
}

/// Webhook-health aggregate for the repository sync panel: the latest
/// event's timestamp/outcome plus the failure count over the last 24 h.
#[derive(Debug, sqlx::FromRow)]
pub struct EventsOverviewRow {
    pub last_event_at: Option<DateTime<Utc>>,
    pub last_event_outcome: Option<String>,
    pub failed_events_24h: i64,
}

pub async fn overview(pool: &PgPool, repository_id: Uuid) -> sqlx::Result<EventsOverviewRow> {
    sqlx::query_as::<_, EventsOverviewRow>(
        r#"
        SELECT
            (SELECT processed_at FROM repository_events
             WHERE repository_id = $1
             ORDER BY processed_at DESC, id DESC LIMIT 1) AS last_event_at,
            (SELECT outcome FROM repository_events
             WHERE repository_id = $1
             ORDER BY processed_at DESC, id DESC LIMIT 1) AS last_event_outcome,
            (SELECT COUNT(*) FROM repository_events
             WHERE repository_id = $1
               AND outcome = 'failed'
               AND processed_at > now() - interval '24 hours') AS failed_events_24h
        "#,
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
}
