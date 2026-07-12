//! Workspace-wide reader over the immutable `audit_logs` ledger — the
//! Activity Feed's data layer. Strictly read-only: writes happen inline at
//! each mutating call site (the existing audit pattern) and are never
//! routed through here. Every filter arrives pre-validated (allow-listed or
//! pre-escaped) from the handler; queries ride the
//! `audit_logs_ws_created_id_idx` keyset index.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::activity::{ActivityRow, ActivitySummaryRow};

/// Validated feed filters. `action_prefixes` comes from the category
/// allow-list (e.g. `pipeline` → `["pipeline", "job"]`), `action` from the
/// full action allow-list, and `search_pattern` is a pre-escaped ILIKE
/// pattern — the SQL below never sees raw user text.
pub struct FeedFilter {
    pub action_prefixes: Option<Vec<String>>,
    pub action: Option<String>,
    pub actor_id: Option<Uuid>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub search_pattern: Option<String>,
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Hard ceiling on one compliance-export response. Exports are a single
/// bounded query — large histories are narrowed with filters, not streamed.
pub const EXPORT_MAX_ROWS: i64 = 5000;

/// Keyset-paginated feed, newest first, with the actor's public profile
/// joined in (`NULL` actor = system action, e.g. webhook sync or scheduler).
pub async fn list_feed(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &FeedFilter,
) -> sqlx::Result<Vec<ActivityRow>> {
    fetch_feed(pool, workspace_id, filter, filter.limit.clamp(1, 50)).await
}

/// Same query as [`list_feed`] under the export ceiling — one bounded pass
/// for the CSV download, honoring every validated filter.
pub async fn export_feed(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &FeedFilter,
) -> sqlx::Result<Vec<ActivityRow>> {
    fetch_feed(pool, workspace_id, filter, EXPORT_MAX_ROWS).await
}

async fn fetch_feed(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &FeedFilter,
    limit: i64,
) -> sqlx::Result<Vec<ActivityRow>> {
    let (cursor_at, cursor_id) = match filter.cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, ActivityRow>(
        r#"
        SELECT a.id, a.action, a.subject_type, a.subject_id, a.actor_user_id,
               u.username AS actor_login, u.avatar_url AS actor_avatar_url,
               a.metadata, a.request_id, a.created_at
        FROM audit_logs a
        LEFT JOIN users u ON u.id = a.actor_user_id
        WHERE a.workspace_id = $1
          AND ($2::text[] IS NULL OR split_part(a.action, '.', 1) = ANY($2))
          AND ($3::text IS NULL OR a.action = $3)
          AND ($4::uuid IS NULL OR a.actor_user_id = $4)
          AND ($5::timestamptz IS NULL OR a.created_at >= $5)
          AND ($6::timestamptz IS NULL OR a.created_at <= $6)
          AND ($7::text IS NULL OR a.action ILIKE $7 ESCAPE '\'
                                OR a.metadata::text ILIKE $7 ESCAPE '\'
                                OR u.username ILIKE $7 ESCAPE '\')
          AND ($8::timestamptz IS NULL OR (a.created_at, a.id) < ($8, $9))
        ORDER BY a.created_at DESC, a.id DESC
        LIMIT $10
        "#,
    )
    .bind(workspace_id)
    .bind(&filter.action_prefixes)
    .bind(&filter.action)
    .bind(filter.actor_id)
    .bind(filter.created_after)
    .bind(filter.created_before)
    .bind(&filter.search_pattern)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Feed KPIs + category distribution in one aggregate pass. Only `total`
/// is all-time; every other figure is windowed (24 h / 30 d) so the strip
/// reads as operational posture rather than lifetime trivia.
pub async fn summary(pool: &PgPool, workspace_id: Uuid) -> sqlx::Result<ActivitySummaryRow> {
    sqlx::query_as::<_, ActivitySummaryRow>(
        r#"
        SELECT COUNT(*) AS total,
               COUNT(*) FILTER (WHERE created_at > now() - interval '24 hours') AS last_24h,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND (action LIKE 'secret.%'
                        OR action IN ('runner.token_regenerated', 'runner.revoked',
                                      'installation.linked', 'installation.unlinked')))
                   AS security_30d,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND (action = 'runner.provision_failed'
                        OR (action = 'pipeline.completed'
                            AND metadata->>'conclusion' IS DISTINCT FROM 'success')))
                   AS failures_30d,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) IN ('pipeline', 'job'))   AS cat_pipeline,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'runner')               AS cat_runner,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'repository')           AS cat_repository,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'artifact')             AS cat_artifact,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'secret')               AS cat_secret,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'environment')          AS cat_environment,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'installation')         AS cat_integration,
               COUNT(*) FILTER (WHERE created_at > now() - interval '30 days'
                   AND split_part(action, '.', 1) = 'workspace')            AS cat_workspace
        FROM audit_logs
        WHERE workspace_id = $1
        "#,
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await
}
