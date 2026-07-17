//! The durable webhook queue. The HTTP handler persists a normalized,
//! server-built payload (never the raw body) and acks GitHub immediately;
//! `services/webhook_processor.rs` claims rows one at a time and processes
//! them asynchronously. `delivery_id` (GitHub's `X-GitHub-Delivery`) is the
//! idempotency key — redeliveries keep the original id, so replays collapse.
//! Per-repo event ordering holds only under a SINGLE consumer process
//! (deployment runs `replicas: 1`); see `claim_next` below.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// One claimed delivery, ready for processing.
#[derive(Debug, sqlx::FromRow)]
pub struct ClaimedDelivery {
    pub delivery_id: String,
    pub event: String,
    pub action: Option<String>,
    pub installation_id: Option<i64>,
    pub github_repo_id: Option<i64>,
    pub payload: Option<serde_json::Value>,
    pub retry_count: i32,
    pub received_at: DateTime<Utc>,
}

/// Record a delivery (the idempotency gate). Returns false when this
/// delivery was already recorded — GitHub redeliveries become no-ops, EXCEPT
/// that a manual redelivery of a terminally `failed` row revives it for
/// another processing round (GitHub's redelivery keeps the original id).
#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &PgPool,
    delivery_id: &str,
    event: &str,
    action: Option<&str>,
    installation_id: Option<i64>,
    github_repo_id: Option<i64>,
    status: &str,
    payload: Option<&serde_json::Value>,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO webhook_deliveries
            (delivery_id, event, action, installation_id, github_repo_id, status, payload)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (delivery_id) DO UPDATE
            SET status = 'pending', retry_count = 0, payload = EXCLUDED.payload
            WHERE webhook_deliveries.status = 'failed'
        "#,
    )
    .bind(delivery_id)
    .bind(event)
    .bind(action)
    .bind(installation_id)
    .bind(github_repo_id)
    .bind(status)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Claim the oldest pending delivery for processing. `FOR UPDATE SKIP
/// LOCKED` keeps concurrent claimers (accidental multiple backend
/// instances) from blocking each other, but it does NOT preserve ordering
/// across instances — per-repo event ordering is guaranteed only by the
/// single sequential in-process consumer (deployment mandates
/// `replicas: 1`; see docs/deploy-backend-dokploy.md).
pub async fn claim_next(pool: &PgPool) -> sqlx::Result<Option<ClaimedDelivery>> {
    sqlx::query_as::<_, ClaimedDelivery>(
        r#"
        UPDATE webhook_deliveries
        SET status = 'processing', last_attempt_at = now()
        WHERE delivery_id = (
            SELECT delivery_id FROM webhook_deliveries
            WHERE status = 'pending'
            ORDER BY received_at, delivery_id
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING delivery_id, event, action, installation_id, github_repo_id,
                  payload, retry_count, received_at
        "#,
    )
    .fetch_optional(pool)
    .await
}

/// Terminal success ('processed') or deliberate drop ('ignored'). The
/// consumed payload is nulled so the table stays bounded.
pub async fn finish(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    delivery_id: &str,
    status: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = $2, processed_at = now(), payload = NULL
        WHERE delivery_id = $1
        "#,
    )
    .bind(delivery_id)
    .bind(status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Processing failed: requeue with a bumped retry count, or park the row as
/// terminally 'failed' once the budget is spent (payload dropped — a manual
/// GitHub redelivery revives it with a fresh payload).
pub async fn record_failure(
    pool: &PgPool,
    delivery_id: &str,
    max_retries: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET retry_count = retry_count + 1,
            status = CASE WHEN retry_count + 1 >= $2 THEN 'failed' ELSE 'pending' END,
            payload = CASE WHEN retry_count + 1 >= $2 THEN NULL ELSE payload END,
            processed_at = CASE WHEN retry_count + 1 >= $2 THEN now() ELSE processed_at END
        WHERE delivery_id = $1
        "#,
    )
    .bind(delivery_id)
    .bind(max_retries)
    .execute(pool)
    .await?;
    Ok(())
}

/// Crash recovery: a row stuck in 'processing' past the window (its worker
/// died mid-flight) goes back to 'pending' for the next drain.
pub async fn revert_stuck(pool: &PgPool, older_than_secs: i64) -> sqlx::Result<u64> {
    let result = sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = 'pending'
        WHERE status = 'processing'
          AND last_attempt_at < now() - make_interval(secs => $1::double precision)
        "#,
    )
    .bind(older_than_secs)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Live (unprocessed) deliveries attributed to one GitHub repository — the
/// repository health panel's "pending events" figure.
pub async fn pending_count_for_repo(pool: &PgPool, github_repo_id: i64) -> sqlx::Result<i64> {
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM webhook_deliveries
        WHERE github_repo_id = $1 AND status IN ('pending', 'processing')
        "#,
    )
    .bind(github_repo_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}
