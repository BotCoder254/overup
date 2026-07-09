use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::user::User;

pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
        .bind(token_hash)
        .bind(user_id)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Resolve a session token hash to its user, only while the session is alive.
/// Touches `last_seen_at` in the same round-trip.
pub async fn find_valid_user(pool: &PgPool, token_hash: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE sessions s
        SET last_seen_at = now()
        FROM users u
        WHERE s.token_hash = $1
          AND s.expires_at > now()
          AND u.id = s.user_id
        RETURNING u.*
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub async fn delete_by_token_hash(pool: &PgPool, token_hash: &str) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_expired(pool: &PgPool) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
