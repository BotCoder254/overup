use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::artifact::Artifact;

/// Record a pending artifact before the presigned PUT is handed out. A
/// re-request for the same (job, name) resets the existing row and keeps its
/// original r2_key so the object location is stable.
#[allow(clippy::too_many_arguments)]
pub async fn insert_pending(
    pool: &PgPool,
    workspace_id: Uuid,
    pipeline_id: Uuid,
    job_id: Uuid,
    name: &str,
    r2_key: &str,
    size_bytes: i64,
    content_type: &str,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
) -> sqlx::Result<Artifact> {
    sqlx::query_as::<_, Artifact>(
        r#"
        INSERT INTO artifacts
            (workspace_id, pipeline_id, job_id, name, r2_key, size_bytes, content_type, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        ON CONFLICT ON CONSTRAINT artifacts_job_name_key
        DO UPDATE SET size_bytes = EXCLUDED.size_bytes,
                      content_type = EXCLUDED.content_type,
                      status = 'pending',
                      checksum_sha256 = NULL
        RETURNING *
        "#,
    )
    .bind(workspace_id)
    .bind(pipeline_id)
    .bind(job_id)
    .bind(name)
    .bind(r2_key)
    .bind(size_bytes)
    .bind(content_type)
    .bind(expires_at)
    .fetch_one(pool)
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

/// Flip to uploaded once the server has verified the object (HeadObject).
pub async fn mark_uploaded(
    pool: &PgPool,
    job_id: Uuid,
    name: &str,
    size_bytes: i64,
    checksum_sha256: &str,
) -> sqlx::Result<Option<Artifact>> {
    sqlx::query_as::<_, Artifact>(
        r#"
        UPDATE artifacts
        SET status = 'uploaded', size_bytes = $3, checksum_sha256 = $4
        WHERE job_id = $1 AND name = $2 AND status = 'pending'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(name)
    .bind(size_bytes)
    .bind(checksum_sha256)
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
}

const CATALOG_SELECT: &str = r#"
    SELECT a.*,
           p.number       AS pipeline_number,
           p.repository_id AS repository_id,
           r.full_name    AS repo_full_name,
           p.workflow_id  AS workflow_id,
           p.workflow_name AS workflow_name,
           p.git_ref      AS git_ref,
           p.commit_sha   AS commit_sha,
           j.job_key      AS job_key,
           j.name         AS job_name,
           ru.name        AS runner_name
    FROM artifacts a
    JOIN pipelines p ON p.id = a.pipeline_id
    JOIN repositories r ON r.id = p.repository_id
    JOIN pipeline_jobs j ON j.id = a.job_id
    LEFT JOIN runners ru ON ru.id = j.runner_id
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
        {CATALOG_SELECT}
        WHERE a.workspace_id = $1
          AND ($2::uuid IS NULL OR p.repository_id = $2)
          AND ($3::uuid IS NULL OR p.workflow_id = $3)
          AND ($4::uuid IS NULL OR a.pipeline_id = $4)
          AND ($5::uuid IS NULL OR a.job_id = $5)
          AND ($6::text IS NULL OR a.status = $6)
          AND ($7::text IS NULL OR a.name ILIKE $7 ESCAPE '\')
          AND ($8::timestamptz IS NULL OR a.created_at >= $8)
          AND ($9::timestamptz IS NULL OR a.created_at <= $9)
          AND ($10::timestamptz IS NULL OR (a.created_at, a.id) < ($10, $11))
        ORDER BY a.created_at DESC, a.id DESC
        LIMIT $12
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
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(filter.limit.clamp(1, 50))
    .fetch_all(pool)
    .await
}

/// One catalog row with provenance, workspace-scoped.
pub async fn find_catalog_row(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<ArtifactCatalogRow>> {
    sqlx::query_as::<_, ArtifactCatalogRow>(&format!(
        "{CATALOG_SELECT} WHERE a.workspace_id = $1 AND a.id = $2",
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
}

pub async fn summary_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<ArtifactSummary> {
    sqlx::query_as::<_, ArtifactSummary>(
        r#"
        SELECT COUNT(*)                                            AS total,
               COUNT(*) FILTER (WHERE status = 'uploaded')          AS uploaded,
               COUNT(*) FILTER (WHERE status = 'pending')           AS pending,
               COUNT(*) FILTER (WHERE status = 'failed')            AS failed,
               COUNT(*) FILTER (WHERE status = 'uploaded'
                                AND expires_at IS NOT NULL
                                AND expires_at < now() + interval '7 days') AS expiring_soon,
               COALESCE(SUM(size_bytes) FILTER (WHERE status = 'uploaded'), 0)::bigint AS total_bytes
        FROM artifacts
        WHERE workspace_id = $1
        "#,
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await
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
