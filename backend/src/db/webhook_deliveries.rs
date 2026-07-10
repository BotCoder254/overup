use sqlx::PgPool;

/// Record a delivery id (the idempotency gate). Returns false when this
/// delivery was already processed — GitHub redeliveries become no-ops.
pub async fn insert(
    pool: &PgPool,
    delivery_id: &str,
    event: &str,
    action: Option<&str>,
    installation_id: Option<i64>,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        INSERT INTO webhook_deliveries (delivery_id, event, action, installation_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (delivery_id) DO NOTHING
        "#,
    )
    .bind(delivery_id)
    .bind(event)
    .bind(action)
    .bind(installation_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn set_status(pool: &PgPool, delivery_id: &str, status: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE webhook_deliveries SET status = $2 WHERE delivery_id = $1")
        .bind(delivery_id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}
