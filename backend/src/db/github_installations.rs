use sqlx::PgPool;
use uuid::Uuid;

use crate::models::github_installation::GitHubInstallation;

/// Outcome of linking an installation to a workspace. The named unique
/// constraint on installation_id is the authority: an installation already
/// claimed by another workspace loses cleanly instead of being stolen.
pub enum LinkOutcome {
    Linked(GitHubInstallation),
    ClaimedElsewhere,
}

pub async fn link(
    pool: &PgPool,
    workspace_id: Uuid,
    installation_id: i64,
    account_login: &str,
    account_type: &str,
    account_avatar_url: Option<&str>,
    linked_by: Uuid,
) -> sqlx::Result<LinkOutcome> {
    // Re-linking the same installation to the same workspace refreshes the
    // account snapshot; a claim by a different workspace matches no row.
    let row = sqlx::query_as::<_, GitHubInstallation>(
        r#"
        INSERT INTO github_installations
            (workspace_id, installation_id, account_login, account_type,
             account_avatar_url, linked_by)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT ON CONSTRAINT github_installations_installation_id_key
        DO UPDATE SET
            account_login = EXCLUDED.account_login,
            account_type = EXCLUDED.account_type,
            account_avatar_url = EXCLUDED.account_avatar_url,
            suspended_at = NULL,
            updated_at = now()
        WHERE github_installations.workspace_id = EXCLUDED.workspace_id
        RETURNING *
        "#,
    )
    .bind(workspace_id)
    .bind(installation_id)
    .bind(account_login)
    .bind(account_type)
    .bind(account_avatar_url)
    .bind(linked_by)
    .fetch_optional(pool)
    .await?;

    Ok(match row {
        Some(installation) => LinkOutcome::Linked(installation),
        None => LinkOutcome::ClaimedElsewhere,
    })
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<GitHubInstallation>> {
    sqlx::query_as::<_, GitHubInstallation>(
        "SELECT * FROM github_installations WHERE workspace_id = $1 ORDER BY created_at",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn find_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<GitHubInstallation>> {
    sqlx::query_as::<_, GitHubInstallation>(
        "SELECT * FROM github_installations WHERE workspace_id = $1 AND id = $2",
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Unlink; returns the numeric installation id so the caller can evict its
/// cached token. Connected repositories cascade.
pub async fn delete_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<i64>> {
    let row: Option<(i64,)> = sqlx::query_as(
        "DELETE FROM github_installations WHERE workspace_id = $1 AND id = $2
         RETURNING installation_id",
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Webhook-driven removal (installation deleted on GitHub).
pub async fn delete_by_installation_id(pool: &PgPool, installation_id: i64) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM github_installations WHERE installation_id = $1")
        .bind(installation_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM github_installation_events WHERE installation_id = $1")
        .bind(installation_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_suspended(
    pool: &PgPool,
    installation_id: i64,
    suspended: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE github_installations
        SET suspended_at = CASE WHEN $2 THEN now() ELSE NULL END, updated_at = now()
        WHERE installation_id = $1
        "#,
    )
    .bind(installation_id)
    .bind(suspended)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record an `installation.created` webhook so an org install can later be
/// claimed by the member who performed it.
pub async fn record_created_event(
    pool: &PgPool,
    installation_id: i64,
    account_login: &str,
    sender_login: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO github_installation_events (installation_id, account_login, sender_login)
        VALUES ($1, $2, $3)
        ON CONFLICT (installation_id)
        DO UPDATE SET account_login = EXCLUDED.account_login,
                      sender_login = EXCLUDED.sender_login,
                      received_at = now()
        "#,
    )
    .bind(installation_id)
    .bind(account_login)
    .bind(sender_login)
    .execute(pool)
    .await?;
    Ok(())
}

/// The recorded installer login for an installation, if a created event
/// was received.
pub async fn find_event_sender(
    pool: &PgPool,
    installation_id: i64,
) -> sqlx::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT sender_login FROM github_installation_events WHERE installation_id = $1",
    )
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(login,)| login))
}
