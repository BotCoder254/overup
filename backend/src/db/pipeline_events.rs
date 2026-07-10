use sqlx::PgPool;
use uuid::Uuid;

use crate::models::pipeline::PipelineEvent;

/// One appended ledger entry, returned so callers can broadcast it live.
/// Payloads carry contextual metadata only — never env or secret values.
#[allow(clippy::too_many_arguments)]
pub async fn append(
    pool: &PgPool,
    pipeline_id: Uuid,
    job_id: Option<Uuid>,
    event_type: &str,
    from_state: Option<&str>,
    to_state: Option<&str>,
    runner_id: Option<Uuid>,
    actor_user_id: Option<Uuid>,
    payload: serde_json::Value,
) -> sqlx::Result<PipelineEvent> {
    sqlx::query_as::<_, PipelineEvent>(
        r#"
        INSERT INTO pipeline_events
            (pipeline_id, job_id, event_type, from_state, to_state,
             runner_id, actor_user_id, payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING *
        "#,
    )
    .bind(pipeline_id)
    .bind(job_id)
    .bind(event_type)
    .bind(from_state)
    .bind(to_state)
    .bind(runner_id)
    .bind(actor_user_id)
    .bind(payload)
    .fetch_one(pool)
    .await
}

/// Ledger for one pipeline, oldest first, sanity-capped.
pub async fn list_for_pipeline(
    pool: &PgPool,
    pipeline_id: Uuid,
) -> sqlx::Result<Vec<PipelineEvent>> {
    sqlx::query_as::<_, PipelineEvent>(
        r#"
        SELECT * FROM pipeline_events
        WHERE pipeline_id = $1
        ORDER BY id
        LIMIT 1000
        "#,
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await
}

/// Ledger entries for one job of a pipeline, oldest first, sanity-capped.
/// Filtered by pipeline too so a job id can never read across pipelines.
pub async fn list_for_job(
    pool: &PgPool,
    pipeline_id: Uuid,
    job_id: Uuid,
) -> sqlx::Result<Vec<PipelineEvent>> {
    sqlx::query_as::<_, PipelineEvent>(
        r#"
        SELECT * FROM pipeline_events
        WHERE pipeline_id = $1 AND job_id = $2
        ORDER BY id
        LIMIT 500
        "#,
    )
    .bind(pipeline_id)
    .bind(job_id)
    .fetch_all(pool)
    .await
}
