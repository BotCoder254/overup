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
