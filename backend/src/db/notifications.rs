//! Notification Center data layer. Writes happen only from the audit-tail
//! projector and the janitor's derived-condition scans — request handlers
//! only flip per-user read/archive state on rows already scoped to the
//! caller (`user_id = $current` in every predicate, so a forged id can
//! never touch another user's rows). Every filter arrives pre-validated
//! (allow-listed or pre-escaped) from the handler.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

use crate::models::notification::{NotificationRow, PreferencesRow};

/// Validated list filters (the pipelines-ledger pattern): allow-listed
/// category/severity, pre-escaped ILIKE pattern, parsed cursor.
pub struct ListFilter {
    pub unread: Option<bool>,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub repository_id: Option<Uuid>,
    pub search_pattern: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    /// Validated to one of `exclude` (default), `include`, `only`.
    pub archived: &'static str,
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Insert payload built by the notification service; all text is
/// server-rendered from static templates — never runner/upstream input.
pub struct NewNotification<'a> {
    pub workspace_id: Uuid,
    pub user_id: Uuid,
    pub action: &'a str,
    pub category: &'a str,
    pub severity: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub subject_type: Option<&'a str>,
    pub subject_id: Option<Uuid>,
    pub link: &'a serde_json::Value,
    pub dedup_key: Option<&'a str>,
}

/// Upsert result: the live row plus whether it was freshly inserted
/// (`false` = merged into an existing unread row via the dedup key).
#[derive(Debug, sqlx::FromRow)]
pub struct UpsertOutcome {
    #[sqlx(flatten)]
    pub row: NotificationRow,
    pub inserted: bool,
}

/// Insert one per-recipient notification, merging into the live unread row
/// when the dedup key already has one (`occurrence_count + 1`, refreshed
/// presentation). The partial unique index makes concurrent batches
/// race-free by construction; `xmax = 0` distinguishes insert from merge.
pub async fn upsert<'e, E>(executor: E, n: &NewNotification<'_>) -> sqlx::Result<UpsertOutcome>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query_as::<_, UpsertOutcome>(
        r#"
        INSERT INTO notifications
            (workspace_id, user_id, action, category, severity, title, body,
             subject_type, subject_id, link, dedup_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (user_id, dedup_key)
            WHERE read_at IS NULL AND archived_at IS NULL AND dedup_key IS NOT NULL
        DO UPDATE SET
            occurrence_count = notifications.occurrence_count + 1,
            severity   = EXCLUDED.severity,
            title      = EXCLUDED.title,
            body       = EXCLUDED.body,
            link       = EXCLUDED.link,
            updated_at = now()
        RETURNING id, workspace_id, user_id, action, category, severity, title, body,
                  subject_type, subject_id, link, occurrence_count, read_at,
                  archived_at, created_at, updated_at, (xmax = 0) AS inserted
        "#,
    )
    .bind(n.workspace_id)
    .bind(n.user_id)
    .bind(n.action)
    .bind(n.category)
    .bind(n.severity)
    .bind(n.title)
    .bind(n.body)
    .bind(n.subject_type)
    .bind(n.subject_id)
    .bind(n.link)
    .bind(n.dedup_key)
    .fetch_one(executor)
    .await
}

/// Workspace members who should receive an event: holders of `permission`,
/// minus the acting user, with per-user preferences applied at write time
/// (mute, disabled categories, min severity). Critical always delivers —
/// an outage alert must not be silenced by a stale snooze.
pub async fn fan_out_recipients(
    pool: &PgPool,
    workspace_id: Uuid,
    permission: &str,
    exclude_actor: Option<Uuid>,
    category: &str,
    severity: &str,
) -> sqlx::Result<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT DISTINCT m.user_id
        FROM workspace_members m
        JOIN role_permissions rp ON rp.role_id = m.role_id AND rp.permission = $2
        LEFT JOIN notification_preferences p
               ON p.user_id = m.user_id AND p.workspace_id = m.workspace_id
        WHERE m.workspace_id = $1
          AND ($3::uuid IS NULL OR m.user_id <> $3)
          AND ( $5 = 'critical'
                OR ( (p.muted_until IS NULL OR p.muted_until < now())
                     AND NOT ($4 = ANY(COALESCE(p.disabled_categories, '{}')))
                     AND array_position(ARRAY['info','success','warning','error','critical'], $5)
                         >= array_position(ARRAY['info','success','warning','error','critical'],
                                           COALESCE(p.min_severity, 'info')) ) )
        "#,
    )
    .bind(workspace_id)
    .bind(permission)
    .bind(exclude_actor)
    .bind(category)
    .bind(severity)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Keyset-paginated list for the caller, newest first.
pub async fn list(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    filter: &ListFilter,
) -> sqlx::Result<Vec<NotificationRow>> {
    fetch(pool, workspace_id, user_id, filter, filter.limit.clamp(1, 100)).await
}

async fn fetch(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    filter: &ListFilter,
    limit: i64,
) -> sqlx::Result<Vec<NotificationRow>> {
    let (cursor_at, cursor_id) = match filter.cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, NotificationRow>(
        r#"
        SELECT id, workspace_id, user_id, action, category, severity, title, body,
               subject_type, subject_id, link, occurrence_count, read_at,
               archived_at, created_at, updated_at
        FROM notifications
        WHERE workspace_id = $1
          AND user_id = $2
          AND ($3::bool IS NULL OR $3 = (read_at IS NULL))
          AND ($4::text IS NULL OR category = $4)
          AND ($5::text IS NULL OR severity = $5)
          AND ($6::uuid IS NULL OR link->>'repositoryId' = ($6::uuid)::text)
          AND ($7::text IS NULL OR title ILIKE $7 ESCAPE '\'
                                OR body  ILIKE $7 ESCAPE '\')
          AND ($8::timestamptz IS NULL OR created_at >= $8)
          AND ($9::timestamptz IS NULL OR created_at <= $9)
          AND ( $10::text = 'include'
                OR ($10 = 'only'    AND archived_at IS NOT NULL)
                OR ($10 = 'exclude' AND archived_at IS NULL) )
          AND ($11::timestamptz IS NULL OR (created_at, id) < ($11, $12))
        ORDER BY created_at DESC, id DESC
        LIMIT $13
        "#,
    )
    .bind(workspace_id)
    .bind(user_id)
    .bind(filter.unread)
    .bind(&filter.category)
    .bind(&filter.severity)
    .bind(filter.repository_id)
    .bind(&filter.search_pattern)
    .bind(filter.created_after)
    .bind(filter.created_before)
    .bind(filter.archived)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Hard ceiling on one CSV-export response (the activity-export pattern):
/// large histories are narrowed with filters, not streamed.
pub const EXPORT_MAX_ROWS: i64 = 5000;

/// Same filters as [`list`] under the export ceiling — one bounded pass for
/// the CSV download, still self-scoped to the caller. The cursor filter
/// composes normally (it is simply absent from the export handler's query).
pub async fn export(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    filter: &ListFilter,
) -> sqlx::Result<Vec<NotificationRow>> {
    fetch(pool, workspace_id, user_id, filter, EXPORT_MAX_ROWS).await
}

/// Unread badge count; rides the partial `notifications_unread_idx`.
pub async fn unread_count(pool: &PgPool, workspace_id: Uuid, user_id: Uuid) -> sqlx::Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM notifications
        WHERE user_id = $1 AND workspace_id = $2
          AND read_at IS NULL AND archived_at IS NULL
        "#,
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Mark one of the caller's notifications read; false = no such live row.
pub async fn mark_read(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE notifications SET read_at = now(), updated_at = now()
        WHERE id = $1 AND user_id = $2 AND workspace_id = $3 AND read_at IS NULL
        "#,
    )
    .bind(id)
    .bind(user_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Mark every (optionally category-scoped) unread notification read inside
/// the caller's transaction — the audit row rides the same tx.
pub async fn mark_all_read(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    workspace_id: Uuid,
    user_id: Uuid,
    category: Option<&str>,
) -> sqlx::Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE notifications SET read_at = now(), updated_at = now()
        WHERE user_id = $1 AND workspace_id = $2
          AND read_at IS NULL AND archived_at IS NULL
          AND ($3::text IS NULL OR category = $3)
        "#,
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(category)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Bulk state change over the caller's own rows. `action` is one of the
/// handler's validated verbs; ids are capped by the handler (≤100).
pub async fn bulk_update(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    workspace_id: Uuid,
    user_id: Uuid,
    action: &str,
    ids: &[Uuid],
) -> sqlx::Result<u64> {
    let query = match action {
        "read" => {
            r#"
            UPDATE notifications SET read_at = now(), updated_at = now()
            WHERE id = ANY($1) AND user_id = $2 AND workspace_id = $3 AND read_at IS NULL
            "#
        }
        "unread" => {
            r#"
            UPDATE notifications SET read_at = NULL, updated_at = now()
            WHERE id = ANY($1) AND user_id = $2 AND workspace_id = $3
              AND read_at IS NOT NULL AND archived_at IS NULL
            "#
        }
        // Archiving implies read: an archived-but-unread row would count
        // toward nothing and be unreachable in the default views.
        "archive" => {
            r#"
            UPDATE notifications
            SET archived_at = now(), read_at = COALESCE(read_at, now()), updated_at = now()
            WHERE id = ANY($1) AND user_id = $2 AND workspace_id = $3 AND archived_at IS NULL
            "#
        }
        _ => return Ok(0),
    };
    let result = sqlx::query(query)
        .bind(ids)
        .bind(user_id)
        .bind(workspace_id)
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected())
}

/// The caller's preferences row, if one exists (absent = all defaults).
pub async fn get_preferences(
    pool: &PgPool,
    workspace_id: Uuid,
    user_id: Uuid,
) -> sqlx::Result<Option<PreferencesRow>> {
    sqlx::query_as::<_, PreferencesRow>(
        r#"
        SELECT muted_until, disabled_categories, min_severity, updated_at
        FROM notification_preferences
        WHERE user_id = $1 AND workspace_id = $2
        "#,
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

/// Upsert the caller's preferences inside the handler's transaction (the
/// audit row rides the same tx).
pub async fn put_preferences(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    workspace_id: Uuid,
    user_id: Uuid,
    muted_until: Option<DateTime<Utc>>,
    disabled_categories: &[String],
    min_severity: &str,
) -> sqlx::Result<PreferencesRow> {
    sqlx::query_as::<_, PreferencesRow>(
        r#"
        INSERT INTO notification_preferences
            (user_id, workspace_id, muted_until, disabled_categories, min_severity)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_id, workspace_id) DO UPDATE SET
            muted_until = EXCLUDED.muted_until,
            disabled_categories = EXCLUDED.disabled_categories,
            min_severity = EXCLUDED.min_severity,
            updated_at = now()
        RETURNING muted_until, disabled_categories, min_severity, updated_at
        "#,
    )
    .bind(user_id)
    .bind(workspace_id)
    .bind(muted_until)
    .bind(disabled_categories)
    .bind(min_severity)
    .fetch_one(&mut **tx)
    .await
}

// ---------------------------------------------------------------------------
// Projector: audit tail + checkpoint.
// ---------------------------------------------------------------------------

/// One `audit_logs` row as seen by the projector.
#[derive(Debug, sqlx::FromRow)]
pub struct AuditTailRow {
    pub id: Uuid,
    pub workspace_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub action: String,
    pub subject_id: Option<Uuid>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// The projector's checkpoint (singleton row, seeded by the migration).
pub async fn load_cursor(pool: &PgPool) -> sqlx::Result<(DateTime<Utc>, Uuid)> {
    let (at, id): (DateTime<Utc>, Uuid) =
        sqlx::query_as("SELECT last_at, last_id FROM notification_cursor WHERE singleton")
            .fetch_one(pool)
            .await?;
    Ok((at, id))
}

/// Advance the checkpoint inside the projector's batch transaction so
/// notification writes and the cursor move together (exactly-once).
pub async fn advance_cursor(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    at: DateTime<Utc>,
    id: Uuid,
) -> sqlx::Result<()> {
    sqlx::query("UPDATE notification_cursor SET last_at = $1, last_id = $2 WHERE singleton")
        .bind(at)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Ledger rows past the checkpoint, oldest first, one bounded batch.
pub async fn tail_audit(
    pool: &PgPool,
    after: (DateTime<Utc>, Uuid),
    limit: i64,
) -> sqlx::Result<Vec<AuditTailRow>> {
    sqlx::query_as::<_, AuditTailRow>(
        r#"
        SELECT id, workspace_id, actor_user_id, action, subject_id,
               metadata, created_at
        FROM audit_logs
        WHERE (created_at, id) > ($1, $2)
        ORDER BY created_at ASC, id ASC
        LIMIT $3
        "#,
    )
    .bind(after.0)
    .bind(after.1)
    .bind(limit)
    .fetch_all(pool)
    .await
}

// ---------------------------------------------------------------------------
// Janitor: retention + derived-condition scans.
// ---------------------------------------------------------------------------

/// Archive read notifications whose `read_at` is older than `days`.
pub async fn auto_archive_read(pool: &PgPool, days: i32, batch: i64) -> sqlx::Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE notifications SET archived_at = now(), updated_at = now()
        WHERE id IN (
            SELECT id FROM notifications
            WHERE read_at IS NOT NULL AND archived_at IS NULL
              AND read_at < now() - make_interval(days => $1)
            LIMIT $2
        )
        "#,
    )
    .bind(days)
    .bind(batch)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Hard-delete archived notifications past the retention window.
pub async fn purge_archived(pool: &PgPool, days: i32, batch: i64) -> sqlx::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM notifications
        WHERE id IN (
            SELECT id FROM notifications
            WHERE archived_at IS NOT NULL
              AND archived_at < now() - make_interval(days => $1)
            LIMIT $2
        )
        "#,
    )
    .bind(days)
    .bind(batch)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// A secret due a rotation reminder.
#[derive(Debug, sqlx::FromRow)]
pub struct StaleSecretRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
}

/// Secrets whose value hasn't rotated in `stale_days`, excluding those
/// already reminded within the last 7 days (read or not) — a dismissed
/// reminder re-fires weekly, not hourly.
pub async fn stale_secret_candidates(
    pool: &PgPool,
    stale_days: i32,
    batch: i64,
) -> sqlx::Result<Vec<StaleSecretRow>> {
    sqlx::query_as::<_, StaleSecretRow>(
        r#"
        SELECT s.id, s.workspace_id, s.name
        FROM secrets s
        WHERE s.value_set_at < now() - make_interval(days => $1)
          AND NOT EXISTS (
              SELECT 1 FROM notifications n
              WHERE n.dedup_key = 'secret.stale:' || s.id::text
                AND n.created_at > now() - interval '7 days'
          )
        ORDER BY s.value_set_at ASC
        LIMIT $2
        "#,
    )
    .bind(stale_days)
    .bind(batch)
    .fetch_all(pool)
    .await
}

/// A workspace whose job queue is congested.
#[derive(Debug, sqlx::FromRow)]
pub struct CongestedWorkspaceRow {
    pub workspace_id: Uuid,
    pub queued_jobs: i64,
    pub online_runners: i64,
}

/// Workspaces with queued, unassigned jobs older than `minutes`, excluding
/// those already warned within 24 hours (read or not) — congestion is a
/// sustained state, so one reminder a day is signal, more is noise. Rides
/// the partial `pipeline_jobs_claimable_idx`.
pub async fn congested_workspaces(
    pool: &PgPool,
    minutes: i32,
    batch: i64,
) -> sqlx::Result<Vec<CongestedWorkspaceRow>> {
    sqlx::query_as::<_, CongestedWorkspaceRow>(
        r#"
        SELECT p.workspace_id,
               COUNT(*) AS queued_jobs,
               (SELECT COUNT(*) FROM runners r
                WHERE r.workspace_id = p.workspace_id
                  AND r.status <> 'offline' AND r.revoked_at IS NULL) AS online_runners
        FROM pipeline_jobs j
        JOIN pipelines p ON p.id = j.pipeline_id
        WHERE j.status = 'queued'
          AND j.runner_id IS NULL
          AND j.queued_at < now() - make_interval(mins => $1)
          AND NOT EXISTS (
              SELECT 1 FROM notifications n
              WHERE n.dedup_key = 'queue.congested:' || p.workspace_id::text
                AND n.created_at > now() - interval '24 hours'
          )
        GROUP BY p.workspace_id
        LIMIT $2
        "#,
    )
    .bind(minutes)
    .bind(batch)
    .fetch_all(pool)
    .await
}

/// An artifact expiring within the warning window.
#[derive(Debug, sqlx::FromRow)]
pub struct ExpiringArtifactRow {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub pipeline_number: i32,
    pub triggered_by: Option<Uuid>,
}

/// Uploaded artifacts expiring in the next 24 hours that haven't been
/// warned about yet (one-shot: the dedup key row, read or not, suppresses
/// a repeat — the artifact is gone soon either way).
pub async fn expiring_artifact_candidates(
    pool: &PgPool,
    batch: i64,
) -> sqlx::Result<Vec<ExpiringArtifactRow>> {
    sqlx::query_as::<_, ExpiringArtifactRow>(
        r#"
        SELECT a.id, a.workspace_id, a.name,
               p.number AS pipeline_number, p.triggered_by
        FROM artifacts a
        JOIN pipelines p ON p.id = a.pipeline_id
        WHERE a.status = 'uploaded'
          AND a.expires_at IS NOT NULL
          AND a.expires_at BETWEEN now() AND now() + interval '24 hours'
          AND NOT EXISTS (
              SELECT 1 FROM notifications n
              WHERE n.dedup_key = 'artifact.expiring:' || a.id::text
          )
        ORDER BY a.expires_at ASC
        LIMIT $1
        "#,
    )
    .bind(batch)
    .fetch_all(pool)
    .await
}
