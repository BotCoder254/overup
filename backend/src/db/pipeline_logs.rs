use sqlx::PgPool;
use uuid::Uuid;

use crate::models::pipeline::LogChunk;

/// One masked, truncated chunk ready to persist.
pub struct NewChunk {
    pub seq: i64,
    pub stream: &'static str,
    pub content: String,
    pub byte_len: i32,
}

/// Batch insert via UNNEST; redelivered sequence numbers are no-ops so a
/// runner reconnect can safely resend its tail.
pub async fn insert_chunks(pool: &PgPool, job_id: Uuid, chunks: &[NewChunk]) -> sqlx::Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    let seqs: Vec<i64> = chunks.iter().map(|c| c.seq).collect();
    let streams: Vec<&str> = chunks.iter().map(|c| c.stream).collect();
    let contents: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let byte_lens: Vec<i32> = chunks.iter().map(|c| c.byte_len).collect();

    sqlx::query(
        r#"
        INSERT INTO pipeline_log_chunks (job_id, seq, stream, content, byte_len)
        SELECT $1, t.seq, t.stream, t.content, t.byte_len
        FROM UNNEST($2::bigint[], $3::text[], $4::text[], $5::int[])
            AS t(seq, stream, content, byte_len)
        ON CONFLICT (job_id, seq) DO NOTHING
        "#,
    )
    .bind(job_id)
    .bind(&seqs)
    .bind(&streams)
    .bind(&contents)
    .bind(&byte_lens)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_range(
    pool: &PgPool,
    job_id: Uuid,
    from_seq: i64,
    limit: i64,
) -> sqlx::Result<Vec<LogChunk>> {
    sqlx::query_as::<_, LogChunk>(
        r#"
        SELECT seq, stream, content, created_at
        FROM pipeline_log_chunks
        WHERE job_id = $1 AND seq >= $2
        ORDER BY seq
        LIMIT $3
        "#,
    )
    .bind(job_id)
    .bind(from_seq)
    .bind(limit.clamp(1, 2000))
    .fetch_all(pool)
    .await
}

/// Full log for raw download. Bounded by the per-job log byte cap enforced
/// at ingest, so this can never grow without limit.
pub async fn fetch_all(pool: &PgPool, job_id: Uuid) -> sqlx::Result<Vec<LogChunk>> {
    sqlx::query_as::<_, LogChunk>(
        r#"
        SELECT seq, stream, content, created_at
        FROM pipeline_log_chunks
        WHERE job_id = $1
        ORDER BY seq
        "#,
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
}

/// Prune the hot copies of logs that have been archived to R2. Only ever
/// called with job ids whose `logs_archived_at` is set.
pub async fn delete_for_jobs(pool: &PgPool, job_ids: &[Uuid]) -> sqlx::Result<u64> {
    if job_ids.is_empty() {
        return Ok(0);
    }
    let result = sqlx::query("DELETE FROM pipeline_log_chunks WHERE job_id = ANY($1)")
        .bind(job_ids)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
