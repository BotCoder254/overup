use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::user::User;

pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
    ip: Option<&str>,
    user_agent: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO sessions (token_hash, user_id, expires_at, ip, user_agent)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(expires_at)
    .bind(ip)
    .bind(user_agent)
    .execute(pool)
    .await?;
    Ok(())
}

/// One row of the active-sessions list. The token hash never leaves the
/// database — `is_current` is computed inside the query instead.
#[derive(Debug, sqlx::FromRow)]
pub struct SessionRow {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub is_current: bool,
}

/// All live (unexpired) sessions for a user, most recently seen first.
pub async fn list_for_user(
    pool: &PgPool,
    user_id: Uuid,
    current_hash: &str,
) -> sqlx::Result<Vec<SessionRow>> {
    sqlx::query_as::<_, SessionRow>(
        r#"
        SELECT id, created_at, last_seen_at, expires_at, ip, user_agent,
               (token_hash = $2) AS is_current
        FROM sessions
        WHERE user_id = $1 AND expires_at > now()
        ORDER BY (token_hash = $2) DESC, last_seen_at DESC NULLS LAST, created_at DESC
        "#,
    )
    .bind(user_id)
    .bind(current_hash)
    .fetch_all(pool)
    .await
}

/// Revoke one session by id, scoped to its owner. Returns rows affected
/// (0 = not found / not yours — same flat outcome either way).
pub async fn delete_by_id_for_user(
    pool: &PgPool,
    user_id: Uuid,
    session_id: Uuid,
) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE id = $1 AND user_id = $2")
        .bind(session_id)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Revoke every session for the user except the one presenting `keep_hash`.
pub async fn delete_all_for_user_except(
    pool: &PgPool,
    user_id: Uuid,
    keep_hash: &str,
) -> sqlx::Result<u64> {
    let result = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token_hash <> $2")
        .bind(user_id)
        .bind(keep_hash)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
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
