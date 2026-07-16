use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::artifact::Artifact;

/// Record a pending artifact before the presigned PUT is handed out. A
/// re-request for the same (job, name) resets the existing row and keeps its
/// original r2_key so the object location is stable; the manifest columns
/// and expiry are reset too so a re-upload never keeps a stale manifest or
/// an old retention window.
#[allow(clippy::too_many_arguments)]
pub async fn insert_pending(
    pool: &PgPool,
    workspace_id: Uuid,
    pipeline_id: Uuid,
    job_id: Uuid,
    name: &str,
    r2_key: &str,
    storage_backend: &str,
    size_bytes: i64,
    content_type: &str,
    kind: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> sqlx::Result<Artifact> {
    sqlx::query_as::<_, Artifact>(
        r#"
        INSERT INTO artifacts
            (workspace_id, pipeline_id, job_id, name, r2_key, storage_backend, size_bytes, content_type, kind, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT ON CONSTRAINT artifacts_job_name_key
        DO UPDATE SET size_bytes = EXCLUDED.size_bytes,
                      content_type = EXCLUDED.content_type,
                      kind = EXCLUDED.kind,
                      storage_backend = EXCLUDED.storage_backend,
                      status = 'pending',
                      checksum_sha256 = NULL,
                      uncompressed_bytes = NULL,
                      file_count = NULL,
                      entries = NULL,
                      expires_at = EXCLUDED.expires_at
        RETURNING *
        "#,
    )
    .bind(workspace_id)
    .bind(pipeline_id)
    .bind(job_id)
    .bind(name)
    .bind(r2_key)
    .bind(storage_backend)
    .bind(size_bytes)
    .bind(content_type)
    .bind(kind)
    .bind(expires_at)
    .fetch_one(pool)
    .await
}

/// The pending row for one (job, name) pair — the verification path needs
/// its key and storage-backend marker before HeadObject.
pub async fn find_pending(
    pool: &PgPool,
    job_id: Uuid,
    name: &str,
) -> sqlx::Result<Option<Artifact>> {
    sqlx::query_as::<_, Artifact>(
        "SELECT * FROM artifacts WHERE job_id = $1 AND name = $2 AND status = 'pending'",
    )
    .bind(job_id)
    .bind(name)
    .fetch_optional(pool)
    .await
}

pub async fn count_for_job(pool: &PgPool, job_id: Uuid) -> sqlx::Result<i64> {
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM artifacts WHERE job_id = $1")
            .bind(job_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

/// Optional archive introspection reported with `artifact_done`. The caller
/// (runner_ws) has already validated and capped every field — over-cap
/// manifests are dropped there, never stored.
pub struct UploadManifest {
    pub uncompressed_bytes: Option<i64>,
    pub file_count: Option<i32>,
    pub entries: Option<serde_json::Value>,
}

/// Flip to uploaded once the server has verified the object (HeadObject).
pub async fn mark_uploaded(
    pool: &PgPool,
    job_id: Uuid,
    name: &str,
    size_bytes: i64,
    checksum_sha256: &str,
    manifest: Option<&UploadManifest>,
) -> sqlx::Result<Option<Artifact>> {
    sqlx::query_as::<_, Artifact>(
        r#"
        UPDATE artifacts
        SET status = 'uploaded', size_bytes = $3, checksum_sha256 = $4,
            uncompressed_bytes = $5, file_count = $6, entries = $7
        WHERE job_id = $1 AND name = $2 AND status = 'pending'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(name)
    .bind(size_bytes)
    .bind(checksum_sha256)
    .bind(manifest.and_then(|m| m.uncompressed_bytes))
    .bind(manifest.and_then(|m| m.file_count))
    .bind(manifest.and_then(|m| m.entries.clone()))
    .fetch_optional(pool)
    .await
}

pub async fn mark_failed(pool: &PgPool, job_id: Uuid, name: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE artifacts SET status = 'failed' WHERE job_id = $1 AND name = $2 AND status = 'pending'",
    )
    .bind(job_id)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_for_pipeline(pool: &PgPool, pipeline_id: Uuid) -> sqlx::Result<Vec<Artifact>> {
    sqlx::query_as::<_, Artifact>(
        "SELECT * FROM artifacts WHERE pipeline_id = $1 ORDER BY created_at",
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await
}

pub async fn list_for_job(pool: &PgPool, job_id: Uuid) -> sqlx::Result<Vec<Artifact>> {
    sqlx::query_as::<_, Artifact>("SELECT * FROM artifacts WHERE job_id = $1 ORDER BY created_at")
        .bind(job_id)
        .fetch_all(pool)
        .await
}

pub async fn find_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Artifact>> {
    sqlx::query_as::<_, Artifact>("SELECT * FROM artifacts WHERE workspace_id = $1 AND id = $2")
        .bind(workspace_id)
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Validated filters for the workspace-wide artifact catalog. Every string
/// field is allow-listed or pre-escaped by the handler — never raw input.
pub struct CatalogFilter {
    pub repository_id: Option<Uuid>,
    pub workflow_id: Option<Uuid>,
    pub pipeline_id: Option<Uuid>,
    pub job_id: Option<Uuid>,
    /// Pre-validated against the status allow-list.
    pub status: Option<String>,
    /// Pre-escaped ILIKE pattern (`%...%` with \, %, _ escaped).
    pub search_pattern: Option<String>,
    /// Full ref (`refs/heads/<branch>`), built by the handler.
    pub git_ref: Option<String>,
    /// Pre-validated against the kind allow-list.
    pub kind: Option<String>,
    /// Pre-validated: `active` | `expiring_soon` | `expired`.
    pub retention: Option<String>,
    pub min_size_bytes: Option<i64>,
    pub max_size_bytes: Option<i64>,
    /// Pre-escaped ILIKE pattern matched against job key AND job name.
    pub job_pattern: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Catalog row: the artifact plus the provenance of the execution that
/// produced it (pipeline / repository / job, runner on the detail path).
#[derive(Debug, sqlx::FromRow)]
pub struct ArtifactCatalogRow {
    #[sqlx(flatten)]
    pub artifact: Artifact,
    pub pipeline_number: i32,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub workflow_id: Option<Uuid>,
    pub workflow_name: String,
    pub git_ref: String,
    pub commit_sha: String,
    pub job_key: String,
    pub job_name: Option<String>,
    pub runner_name: Option<String>,
    pub job_image: Option<String>,
}

/// Provenance columns + joins shared by the list and detail selects.
const CATALOG_PROVENANCE: &str = r#"
           p.number       AS pipeline_number,
           p.repository_id AS repository_id,
           r.full_name    AS repo_full_name,
           p.workflow_id  AS workflow_id,
           p.workflow_name AS workflow_name,
           p.git_ref      AS git_ref,
           p.commit_sha   AS commit_sha,
           j.job_key      AS job_key,
           j.name         AS job_name,
           ru.name        AS runner_name,
           j.plan->>'image' AS job_image
    FROM artifacts a
    JOIN pipelines p ON p.id = a.pipeline_id
    JOIN repositories r ON r.id = p.repository_id
    JOIN pipeline_jobs j ON j.id = a.job_id
    LEFT JOIN runners ru ON ru.id = j.runner_id
"#;

/// List select: never fetches `entries` (up to ~64 KB per row) — page loads
/// stay cheap and the manifest remains a detail-only payload.
const CATALOG_LIST_COLUMNS: &str = r#"
    SELECT a.id, a.workspace_id, a.pipeline_id, a.job_id, a.name, a.r2_key,
           a.storage_backend,
           a.size_bytes, a.content_type, a.checksum_sha256, a.status, a.kind,
           a.uncompressed_bytes, a.file_count, NULL::jsonb AS entries,
           a.created_at, a.expires_at,
"#;

/// Keyset-paginated workspace catalog, newest first. Every predicate ANDs
/// ahead of the cursor tuple comparison, so filters and pagination compose.
pub async fn list_catalog(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &CatalogFilter,
) -> sqlx::Result<Vec<ArtifactCatalogRow>> {
    let (cursor_at, cursor_id) = match filter.cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, ArtifactCatalogRow>(&format!(
        r#"
        {CATALOG_LIST_COLUMNS}
        {CATALOG_PROVENANCE}
        WHERE a.workspace_id = $1
          AND ($2::uuid IS NULL OR p.repository_id = $2)
          AND ($3::uuid IS NULL OR p.workflow_id = $3)
          AND ($4::uuid IS NULL OR a.pipeline_id = $4)
          AND ($5::uuid IS NULL OR a.job_id = $5)
          AND ($6::text IS NULL OR a.status = $6)
          AND ($7::text IS NULL OR a.name ILIKE $7 ESCAPE '\')
          AND ($8::timestamptz IS NULL OR a.created_at >= $8)
          AND ($9::timestamptz IS NULL OR a.created_at <= $9)
          AND ($10::text IS NULL OR p.git_ref = $10)
          AND ($11::text IS NULL OR a.kind = $11)
          AND ($12::text IS NULL
               OR ($12 = 'expired'       AND a.status = 'expired')
               OR ($12 = 'expiring_soon' AND a.status = 'uploaded'
                                         AND a.expires_at IS NOT NULL
                                         AND a.expires_at < now() + interval '7 days')
               OR ($12 = 'active'        AND a.status = 'uploaded'
                                         AND (a.expires_at IS NULL
                                              OR a.expires_at >= now() + interval '7 days')))
          AND ($13::bigint IS NULL OR a.size_bytes >= $13)
          AND ($14::bigint IS NULL OR a.size_bytes <= $14)
          AND ($15::text IS NULL OR j.job_key ILIKE $15 ESCAPE '\'
                                 OR j.name ILIKE $15 ESCAPE '\')
          AND ($16::timestamptz IS NULL OR (a.created_at, a.id) < ($16, $17))
        ORDER BY a.created_at DESC, a.id DESC
        LIMIT $18
        "#,
    ))
    .bind(workspace_id)
    .bind(filter.repository_id)
    .bind(filter.workflow_id)
    .bind(filter.pipeline_id)
    .bind(filter.job_id)
    .bind(&filter.status)
    .bind(&filter.search_pattern)
    .bind(filter.created_after)
    .bind(filter.created_before)
    .bind(&filter.git_ref)
    .bind(&filter.kind)
    .bind(&filter.retention)
    .bind(filter.min_size_bytes)
    .bind(filter.max_size_bytes)
    .bind(&filter.job_pattern)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(filter.limit.clamp(1, 50))
    .fetch_all(pool)
    .await
}

/// One catalog row with provenance (including the archive entry manifest),
/// workspace-scoped.
pub async fn find_catalog_row(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<ArtifactCatalogRow>> {
    sqlx::query_as::<_, ArtifactCatalogRow>(&format!(
        "SELECT a.*, {CATALOG_PROVENANCE} WHERE a.workspace_id = $1 AND a.id = $2",
    ))
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Workspace storage summary in one aggregate pass.
#[derive(Debug, sqlx::FromRow)]
pub struct ArtifactSummary {
    pub total: i64,
    pub uploaded: i64,
    pub pending: i64,
    pub failed: i64,
    pub expiring_soon: i64,
    pub total_bytes: i64,
    pub recent_24h: i64,
    pub expiring_bytes_7d: i64,
}

/// Uploaded storage grouped by classified kind.
#[derive(Debug, sqlx::FromRow)]
pub struct KindUsage {
    pub kind: String,
    pub count: i64,
    pub bytes: i64,
}

/// The biggest stored artifacts, for the summary strip.
#[derive(Debug, sqlx::FromRow)]
pub struct LargestArtifact {
    pub id: Uuid,
    pub name: String,
    pub size_bytes: i64,
}

pub async fn summary_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<(ArtifactSummary, Vec<KindUsage>, Vec<LargestArtifact>)> {
    let summary = sqlx::query_as::<_, ArtifactSummary>(
        r#"
        SELECT COUNT(*)                                            AS total,
               COUNT(*) FILTER (WHERE status = 'uploaded')          AS uploaded,
               COUNT(*) FILTER (WHERE status = 'pending')           AS pending,
               COUNT(*) FILTER (WHERE status = 'failed')            AS failed,
               COUNT(*) FILTER (WHERE status = 'uploaded'
                                AND expires_at IS NOT NULL
                                AND expires_at < now() + interval '7 days') AS expiring_soon,
               COALESCE(SUM(size_bytes) FILTER (WHERE status = 'uploaded'), 0)::bigint AS total_bytes,
               COUNT(*) FILTER (WHERE status = 'uploaded'
                                AND created_at > now() - interval '24 hours') AS recent_24h,
               COALESCE(SUM(size_bytes) FILTER (WHERE status = 'uploaded'
                                AND expires_at IS NOT NULL
                                AND expires_at < now() + interval '7 days'), 0)::bigint AS expiring_bytes_7d
        FROM artifacts
        WHERE workspace_id = $1
        "#,
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;

    let by_kind = sqlx::query_as::<_, KindUsage>(
        r#"
        SELECT kind,
               COUNT(*) AS count,
               COALESCE(SUM(size_bytes), 0)::bigint AS bytes
        FROM artifacts
        WHERE workspace_id = $1 AND status = 'uploaded'
        GROUP BY kind
        ORDER BY bytes DESC
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    let largest = sqlx::query_as::<_, LargestArtifact>(
        r#"
        SELECT id, name, size_bytes
        FROM artifacts
        WHERE workspace_id = $1 AND status = 'uploaded' AND size_bytes IS NOT NULL
        ORDER BY size_bytes DESC, id DESC
        LIMIT 3
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    Ok((summary, by_kind, largest))
}

/// Operator delete: hard-remove the row and record an immutable audit entry
/// in the same transaction. Returns the deleted row so the caller can clean
/// up the R2 object (it already did, best-effort, before calling this).
pub async fn delete_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<Option<Artifact>> {
    let mut tx = pool.begin().await?;
    let deleted = sqlx::query_as::<_, Artifact>(
        "DELETE FROM artifacts WHERE workspace_id = $1 AND id = $2 RETURNING *",
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(artifact) = deleted else {
        return Ok(None);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'artifact.deleted', 'artifact', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(artifact.id)
    .bind(serde_json::json!({
        "name": artifact.name,
        "sizeBytes": artifact.size_bytes,
        "status": artifact.status,
        "pipelineId": artifact.pipeline_id,
    }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(artifact))
}

/// Uploaded artifacts whose retention lapsed — janitor batch.
pub async fn find_expired(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<Artifact>> {
    sqlx::query_as::<_, Artifact>(
        r#"
        SELECT * FROM artifacts
        WHERE status = 'uploaded' AND expires_at IS NOT NULL AND expires_at < now()
        ORDER BY expires_at
        LIMIT $1
        "#,
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
}

/// Guarded flip to expired after the R2 object was (best-effort) deleted.
pub async fn mark_expired(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE artifacts SET status = 'expired' WHERE id = $1 AND status = 'uploaded'")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Pending rows whose upload was never confirmed — janitor batch. The grant
/// URL itself expires after 15 minutes, so anything this old is abandoned.
pub async fn find_stale_pending(
    pool: &PgPool,
    older_than_hours: i64,
    limit: i64,
) -> sqlx::Result<Vec<Artifact>> {
    sqlx::query_as::<_, Artifact>(
        r#"
        SELECT * FROM artifacts
        WHERE status = 'pending' AND created_at < now() - ($1 * interval '1 hour')
        ORDER BY created_at
        LIMIT $2
        "#,
    )
    .bind(older_than_hours.max(1))
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await
}

/// Remove an abandoned pending row (it was never announced to browsers).
pub async fn delete_stale_pending(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM artifacts WHERE id = $1 AND status = 'pending'")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
