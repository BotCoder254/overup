//! Global Search data layer: ranked queries over the denormalized
//! `search_documents` index plus the per-entity upserts the async indexer
//! (`services/search_indexer.rs`) runs. Strictly parameterized: the tsquery
//! string arrives pre-built from quoted prefix lexemes (see
//! `handlers::search::build_tsquery`) and the ILIKE boost pattern is
//! pre-escaped by the handler — this module never sees raw user text.
//!
//! Secret VALUES never transit here: the secrets upsert selects name,
//! description, and scope columns only (the ciphertext/DEK columns are
//! never in any SELECT list in this module).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::search::{SearchCategoryCount, SearchHit};

/// Most-recent documents kept per workspace for the high-volume entity
/// types; the reconcile pass trims past this so the index stays bounded.
pub const VOLUME_CAP: i64 = 5000;
/// Activity documents indexed per pass (append-only cursor batches).
pub const ACTIVITY_BATCH: i64 = 1000;

/// Validated query inputs. `tsquery` is built from quoted prefix lexemes,
/// `prefix_pattern` is a pre-escaped ILIKE pattern (`escape_like(q) || '%'`),
/// `permissions` is the caller's actual permission set intersected with the
/// indexable trio — never raw user input.
pub struct SearchFilter {
    pub tsquery: String,
    pub raw_query: String,
    pub prefix_pattern: String,
    pub permissions: Vec<String>,
    pub category: Option<String>,
    pub cursor: Option<(f64, DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// The ranked match set as one inner subquery: GIN-pruned by `@@`, filtered
/// by workspace + the caller's permissions BEFORE ranking, scored with
/// ts_rank_cd plus exact/prefix title boosts. Both query shapes below build
/// on it so ranking is identical everywhere.
///
/// Keyset note: the cursor is a tuple filter over the *computed*
/// (score, source_updated_at, id) triple — deterministic for a fixed query,
/// so pages are stable; each page re-scans the matched set, which is
/// acceptable because it is workspace-scoped, GIN-pruned, and volume-capped.
const RANKED_MATCHES: &str = r#"
    SELECT sd.id, sd.entity_type, sd.entity_id, sd.title, sd.subtitle,
           sd.meta, sd.source_updated_at,
           (ts_rank_cd(sd.search, q.query)::float8
             + CASE WHEN lower(sd.title) = lower($3) THEN 2.0
                    WHEN sd.title ILIKE $4 ESCAPE '\' THEN 0.5
                    ELSE 0.0 END) AS score
    FROM search_documents sd,
         to_tsquery('simple', $2) AS q(query)
    WHERE sd.workspace_id = $1
      AND sd.search @@ q.query
      AND sd.permission = ANY($5)
      AND ($6::text IS NULL OR sd.entity_type = $6)
"#;

/// Flat, score-ordered page for the dedicated search page.
pub async fn query_flat(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &SearchFilter,
) -> sqlx::Result<Vec<SearchHit>> {
    let (cursor_score, cursor_at, cursor_id) = match filter.cursor {
        Some((score, at, id)) => (Some(score), Some(at), Some(id)),
        None => (None, None, None),
    };
    let sql = format!(
        r#"
        SELECT s.id, s.entity_type, s.entity_id, s.title, s.subtitle,
               s.meta, s.source_updated_at, s.score
        FROM ({RANKED_MATCHES}) s
        WHERE ($7::float8 IS NULL OR (s.score, s.source_updated_at, s.id) < ($7, $8, $9))
        ORDER BY s.score DESC, s.source_updated_at DESC, s.id DESC
        LIMIT $10
        "#
    );
    sqlx::query_as::<_, SearchHit>(&sql)
        .bind(workspace_id)
        .bind(&filter.tsquery)
        .bind(&filter.raw_query)
        .bind(&filter.prefix_pattern)
        .bind(&filter.permissions)
        .bind(&filter.category)
        .bind(cursor_score)
        .bind(cursor_at)
        .bind(cursor_id)
        .bind(filter.limit.clamp(1, 50))
        .fetch_all(pool)
        .await
}

/// Palette mode: the top N per category in one pass, so every category is
/// represented even when one floods the match set.
pub async fn query_grouped(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &SearchFilter,
    per_category: i64,
) -> sqlx::Result<Vec<SearchHit>> {
    let sql = format!(
        r#"
        SELECT ranked.id, ranked.entity_type, ranked.entity_id, ranked.title,
               ranked.subtitle, ranked.meta, ranked.source_updated_at, ranked.score
        FROM (
            SELECT s.*, ROW_NUMBER() OVER (
                PARTITION BY s.entity_type
                ORDER BY s.score DESC, s.source_updated_at DESC, s.id DESC
            ) AS rn
            FROM ({RANKED_MATCHES}) s
        ) ranked
        WHERE ranked.rn <= $7
        ORDER BY ranked.score DESC, ranked.source_updated_at DESC, ranked.id DESC
        "#
    );
    sqlx::query_as::<_, SearchHit>(&sql)
        .bind(workspace_id)
        .bind(&filter.tsquery)
        .bind(&filter.raw_query)
        .bind(&filter.prefix_pattern)
        .bind(&filter.permissions)
        .bind(&filter.category)
        .bind(per_category.clamp(1, 10))
        .fetch_all(pool)
        .await
}

/// Per-category totals over the whole matched, permission-filtered set.
pub async fn counts(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &SearchFilter,
) -> sqlx::Result<Vec<SearchCategoryCount>> {
    sqlx::query_as::<_, SearchCategoryCount>(
        r#"
        SELECT sd.entity_type, COUNT(*) AS total
        FROM search_documents sd,
             to_tsquery('simple', $2) AS q(query)
        WHERE sd.workspace_id = $1
          AND sd.search @@ q.query
          AND sd.permission = ANY($3)
        GROUP BY sd.entity_type
        "#,
    )
    .bind(workspace_id)
    .bind(&filter.tsquery)
    .bind(&filter.permissions)
    .fetch_all(pool)
    .await
}

/// The caller's live permission set, restricted to the three permissions
/// documents can carry — bound as the `ANY` filter above so unauthorized
/// categories never match, rank, or count.
pub async fn caller_permissions(
    pool: &PgPool,
    user_id: Uuid,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT rp.permission
        FROM workspace_members m
        JOIN role_permissions rp ON rp.role_id = m.role_id
        WHERE m.user_id = $1 AND m.workspace_id = $2
          AND rp.permission IN ('content.read', 'secrets.read', 'audit.read')
        "#,
    )
    .bind(user_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(p,)| p).collect())
}

// ---------------------------------------------------------------------------
// Indexer upserts. All follow one idiom: INSERT … SELECT from the source
// table(s), ON CONFLICT DO UPDATE guarded by a full-tuple IS DISTINCT FROM so
// unchanged rows are no-ops regardless of timestamp churn. `since` (None =
// full pass) narrows the scan where the source has a reliable change clock.
// ---------------------------------------------------------------------------

const UPSERT_GUARD: &str = r#"
    ON CONFLICT (workspace_id, entity_type, entity_id) DO UPDATE SET
        title = EXCLUDED.title,
        subtitle = EXCLUDED.subtitle,
        body = EXCLUDED.body,
        meta = EXCLUDED.meta,
        source_updated_at = EXCLUDED.source_updated_at,
        updated_at = now()
    WHERE (search_documents.title, search_documents.subtitle,
           search_documents.body, search_documents.meta,
           search_documents.source_updated_at)
          IS DISTINCT FROM
          (EXCLUDED.title, EXCLUDED.subtitle, EXCLUDED.body,
           EXCLUDED.meta, EXCLUDED.source_updated_at)
"#;

async fn run_upsert(
    pool: &PgPool,
    select_sql: &str,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    let sql = format!(
        "INSERT INTO search_documents \
         (workspace_id, entity_type, entity_id, permission, title, subtitle, body, meta, source_updated_at) \
         {select_sql} {UPSERT_GUARD}"
    );
    sqlx::query(&sql)
        .bind(workspace_id)
        .bind(since)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn upsert_repositories(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    run_upsert(
        pool,
        r#"
        SELECT r.workspace_id, 'repository', r.id, 'content.read',
               r.full_name,
               coalesce(r.description, ''),
               trim(concat_ws(' ', r.default_branch, r.language)),
               jsonb_strip_nulls(jsonb_build_object(
                   'language', r.language, 'private', r.private)),
               r.updated_at
        FROM repositories r
        WHERE r.workspace_id = $1
          AND ($2::timestamptz IS NULL OR r.updated_at > $2)
        "#,
        workspace_id,
        since,
    )
    .await
}

pub async fn upsert_workflows(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    run_upsert(
        pool,
        r#"
        SELECT r.workspace_id, 'workflow', w.id, 'content.read',
               w.name,
               r.full_name || ' · ' || w.path,
               coalesce(w.last_commit_message, ''),
               jsonb_build_object('repository', r.full_name, 'path', w.path),
               w.updated_at
        FROM workflows w
        JOIN repositories r ON r.id = w.repository_id
        WHERE r.workspace_id = $1
          AND ($2::timestamptz IS NULL OR w.updated_at > $2)
        "#,
        workspace_id,
        since,
    )
    .await
}

pub async fn upsert_pipelines(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    // No updated_at column: status flips always set started_at/finished_at,
    // so GREATEST over the three timestamps is the change clock. Bounded to
    // the most recent VOLUME_CAP rows (the trim below enforces the same cap
    // on the document side).
    run_upsert(
        pool,
        r#"
        SELECT p.workspace_id, 'pipeline', p.id, 'content.read',
               p.workflow_name || ' #' || p.number,
               coalesce(split_part(p.commit_message, E'\n', 1), ''),
               trim(concat_ws(' ', p.commit_sha, p.git_ref, p.commit_author,
                              p.trigger, p.conclusion)),
               jsonb_strip_nulls(jsonb_build_object(
                   'status', p.status, 'conclusion', p.conclusion,
                   'number', p.number, 'repository', r.full_name)),
               GREATEST(p.created_at, coalesce(p.started_at, p.created_at),
                        coalesce(p.finished_at, p.created_at))
        FROM pipelines p
        JOIN repositories r ON r.id = p.repository_id
        WHERE p.workspace_id = $1
          AND ($2::timestamptz IS NULL
               OR GREATEST(p.created_at, coalesce(p.started_at, p.created_at),
                           coalesce(p.finished_at, p.created_at)) > $2)
        ORDER BY p.created_at DESC
        LIMIT 5000
        "#,
        workspace_id,
        since,
    )
    .await
}

pub async fn upsert_runners(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    // Revoked runners are dropped by the reconcile anti-join; renames have
    // no timestamp, so runner passes are always full (`since` narrows
    // nothing here — the table is small) and rely on the tuple guard.
    let _ = since;
    run_upsert(
        pool,
        r#"
        SELECT rn.workspace_id, 'runner', rn.id, 'content.read',
               rn.name,
               '',
               trim(concat_ws(' ', array_to_string(rn.labels, ' '), rn.status)),
               jsonb_build_object('status', rn.status,
                                  'labels', to_jsonb(rn.labels),
                                  'managed', rn.managed),
               coalesce(rn.last_seen_at, rn.created_at)
        FROM runners rn
        WHERE rn.workspace_id = $1 AND rn.revoked_at IS NULL
          AND ($2::timestamptz IS NULL OR TRUE)
        "#,
        workspace_id,
        None,
    )
    .await
}

pub async fn upsert_artifacts(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    // No updated_at and status flips (pending → uploaded → expired) carry no
    // timestamp, so artifact passes are always full over the most recent
    // VOLUME_CAP rows; the tuple guard makes unchanged rows free.
    let _ = since;
    run_upsert(
        pool,
        r#"
        SELECT a.workspace_id, 'artifact', a.id, 'content.read',
               a.name,
               r.full_name || ' · ' || p.workflow_name || ' #' || p.number,
               a.kind,
               jsonb_strip_nulls(jsonb_build_object(
                   'kind', a.kind, 'status', a.status,
                   'sizeBytes', a.size_bytes, 'pipelineId', a.pipeline_id)),
               a.created_at
        FROM artifacts a
        JOIN pipelines p ON p.id = a.pipeline_id
        JOIN repositories r ON r.id = p.repository_id
        WHERE a.workspace_id = $1
          AND ($2::timestamptz IS NULL OR TRUE)
        ORDER BY a.created_at DESC
        LIMIT 5000
        "#,
        workspace_id,
        None,
    )
    .await
}

pub async fn upsert_environments(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    run_upsert(
        pool,
        r#"
        SELECT e.workspace_id, 'environment', e.id, 'content.read',
               e.name,
               coalesce(e.description, ''),
               '',
               '{}'::jsonb,
               e.updated_at
        FROM environments e
        WHERE e.workspace_id = $1
          AND ($2::timestamptz IS NULL OR e.updated_at > $2)
        "#,
        workspace_id,
        since,
    )
    .await
}

/// Secrets metadata ONLY: name, description, and scope. The ciphertext,
/// nonce, and DEK columns are deliberately absent from this SELECT — the
/// write-only contract extends to the search index.
pub async fn upsert_secrets(
    pool: &PgPool,
    workspace_id: Uuid,
    since: Option<DateTime<Utc>>,
) -> sqlx::Result<()> {
    run_upsert(
        pool,
        r#"
        SELECT s.workspace_id, 'secret', s.id, 'secrets.read',
               s.name,
               coalesce(s.description, ''),
               CASE WHEN s.environment_id IS NOT NULL THEN 'environment'
                    WHEN s.repository_id IS NOT NULL THEN 'repository'
                    ELSE 'workspace' END,
               jsonb_build_object('scope',
                   CASE WHEN s.environment_id IS NOT NULL THEN 'environment'
                        WHEN s.repository_id IS NOT NULL THEN 'repository'
                        ELSE 'workspace' END),
               s.updated_at
        FROM secrets s
        WHERE s.workspace_id = $1
          AND ($2::timestamptz IS NULL OR s.updated_at > $2)
        "#,
        workspace_id,
        since,
    )
    .await
}

/// Append-only activity documents by (created_at, id) cursor, one batch per
/// call. Returns the advanced cursor and how many ledger rows the batch
/// covered (callers loop while it equals the batch size). Audit metadata is
/// value-free by construction (the feed's ILIKE precedent), so indexing its
/// text leaks nothing.
pub async fn append_activity(
    pool: &PgPool,
    workspace_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
) -> sqlx::Result<(Option<(DateTime<Utc>, Uuid)>, i64)> {
    let (cursor_at, cursor_id) = match cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    let row: (Option<DateTime<Utc>>, Option<Uuid>, i64) = sqlx::query_as(
        r#"
        WITH batch AS (
            SELECT a.id, a.workspace_id, a.action, a.subject_type,
                   a.metadata, a.actor_user_id, a.created_at
            FROM audit_logs a
            WHERE a.workspace_id = $1
              AND ($2::timestamptz IS NULL OR (a.created_at, a.id) > ($2, $3))
            ORDER BY a.created_at, a.id
            LIMIT $4
        ), ins AS (
            INSERT INTO search_documents
                (workspace_id, entity_type, entity_id, permission,
                 title, subtitle, body, meta, source_updated_at)
            SELECT b.workspace_id, 'activity', b.id, 'audit.read',
                   b.action,
                   coalesce(u.username, 'system'),
                   trim(concat_ws(' ', b.subject_type, b.metadata::text)),
                   jsonb_strip_nulls(jsonb_build_object(
                       'action', b.action, 'actor', u.username)),
                   b.created_at
            FROM batch b
            LEFT JOIN users u ON u.id = b.actor_user_id
            ON CONFLICT (workspace_id, entity_type, entity_id) DO NOTHING
        )
        SELECT max(b.created_at),
               (SELECT b2.id FROM batch b2
                ORDER BY b2.created_at DESC, b2.id DESC LIMIT 1),
               count(*)
        FROM batch b
        "#,
    )
    .bind(workspace_id)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(ACTIVITY_BATCH)
    .fetch_one(pool)
    .await?;

    let advanced = match (row.0, row.1) {
        (Some(at), Some(id)) => Some((at, id)),
        _ => cursor,
    };
    Ok((advanced, row.2))
}

pub async fn load_index_state(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Option<(DateTime<Utc>, Uuid)>> {
    let row: Option<(Option<DateTime<Utc>>, Option<Uuid>)> = sqlx::query_as(
        "SELECT audit_cursor_at, audit_cursor_id FROM search_index_state WHERE workspace_id = $1",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(at, id)| match (at, id) {
        (Some(at), Some(id)) => Some((at, id)),
        _ => None,
    }))
}

pub async fn save_index_state(
    pool: &PgPool,
    workspace_id: Uuid,
    cursor: Option<(DateTime<Utc>, Uuid)>,
    reconciled: bool,
) -> sqlx::Result<()> {
    let (at, id) = match cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query(
        r#"
        INSERT INTO search_index_state (workspace_id, audit_cursor_at, audit_cursor_id, reconciled_at)
        VALUES ($1, $2, $3, CASE WHEN $4 THEN now() ELSE NULL END)
        ON CONFLICT (workspace_id) DO UPDATE SET
            audit_cursor_at = COALESCE($2, search_index_state.audit_cursor_at),
            audit_cursor_id = COALESCE($3, search_index_state.audit_cursor_id),
            reconciled_at = CASE WHEN $4 THEN now() ELSE search_index_state.reconciled_at END
        "#,
    )
    .bind(workspace_id)
    .bind(at)
    .bind(id)
    .bind(reconciled)
    .execute(pool)
    .await?;
    Ok(())
}

/// Drop documents whose source row is gone (or, for runners, revoked).
/// Activity documents are immutable and trimmed by the volume cap instead.
pub async fn reconcile_deletes(pool: &PgPool, workspace_id: Uuid) -> sqlx::Result<()> {
    let anti_joins: &[(&str, &str)] = &[
        ("repository", "SELECT 1 FROM repositories r WHERE r.id = sd.entity_id"),
        ("workflow", "SELECT 1 FROM workflows w WHERE w.id = sd.entity_id"),
        ("pipeline", "SELECT 1 FROM pipelines p WHERE p.id = sd.entity_id"),
        (
            "runner",
            "SELECT 1 FROM runners rn WHERE rn.id = sd.entity_id AND rn.revoked_at IS NULL",
        ),
        ("artifact", "SELECT 1 FROM artifacts a WHERE a.id = sd.entity_id"),
        ("environment", "SELECT 1 FROM environments e WHERE e.id = sd.entity_id"),
        ("secret", "SELECT 1 FROM secrets s WHERE s.id = sd.entity_id"),
    ];
    for (entity_type, exists) in anti_joins {
        let sql = format!(
            "DELETE FROM search_documents sd \
             WHERE sd.workspace_id = $1 AND sd.entity_type = $2 \
               AND NOT EXISTS ({exists})"
        );
        sqlx::query(&sql)
            .bind(workspace_id)
            .bind(entity_type)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Trim the high-volume entity types to the most recent [`VOLUME_CAP`]
/// documents per workspace.
pub async fn enforce_caps(pool: &PgPool, workspace_id: Uuid) -> sqlx::Result<()> {
    for entity_type in ["pipeline", "activity", "artifact"] {
        sqlx::query(
            r#"
            DELETE FROM search_documents
            WHERE workspace_id = $1 AND entity_type = $2
              AND id IN (
                  SELECT id FROM search_documents
                  WHERE workspace_id = $1 AND entity_type = $2
                  ORDER BY source_updated_at DESC, id DESC
                  OFFSET $3
              )
            "#,
        )
        .bind(workspace_id)
        .bind(entity_type)
        .bind(VOLUME_CAP)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Boot check: an empty index triggers the full backfill pass.
pub async fn is_empty(pool: &PgPool) -> sqlx::Result<bool> {
    let (exists,): (bool,) =
        sqlx::query_as("SELECT NOT EXISTS (SELECT 1 FROM search_documents)")
            .fetch_one(pool)
            .await?;
    Ok(exists)
}

/// All workspace ids, for the backfill/reconcile passes.
pub async fn workspace_ids(pool: &PgPool) -> sqlx::Result<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM workspaces")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Per-entity-type change watermark used to narrow incremental scans.
pub async fn watermark(
    pool: &PgPool,
    workspace_id: Uuid,
    entity_type: &str,
) -> sqlx::Result<Option<DateTime<Utc>>> {
    let (max,): (Option<DateTime<Utc>>,) = sqlx::query_as(
        "SELECT max(source_updated_at) FROM search_documents \
         WHERE workspace_id = $1 AND entity_type = $2",
    )
    .bind(workspace_id)
    .bind(entity_type)
    .fetch_one(pool)
    .await?;
    Ok(max)
}
