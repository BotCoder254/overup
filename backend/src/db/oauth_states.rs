use chrono::{DateTime, Utc};
use sqlx::PgPool;

pub async fn insert(
    pool: &PgPool,
    state_hash: &str,
    pkce_verifier: &str,
    expires_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO oauth_states (state_hash, pkce_verifier, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(state_hash)
    .bind(pkce_verifier)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Consume a pending login transaction: single-use by construction
/// (DELETE ... RETURNING), and only honored before it expires.
pub async fn take(pool: &PgPool, state_hash: &str) -> sqlx::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "DELETE FROM oauth_states WHERE state_hash = $1 AND expires_at > now() RETURNING pkce_verifier",
    )
    .bind(state_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(verifier,)| verifier))
}

pub async fn delete_expired(pool: &PgPool) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM oauth_states WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
