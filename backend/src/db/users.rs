use sqlx::PgPool;
use uuid::Uuid;

use crate::models::user::User;
use crate::services::github::GitHubProfile;

/// Create the user on first sign-in, refresh the mirrored GitHub profile on
/// every subsequent one. Email is only overwritten when GitHub returns one.
/// Fields the user has customized in Settings are never clobbered; the avatar
/// always mirrors GitHub (it is the source of truth for the profile image).
pub async fn upsert_by_github(pool: &PgPool, profile: &GitHubProfile) -> sqlx::Result<User> {
    sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (github_id, username, display_name, email, avatar_url)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (github_id) DO UPDATE SET
            username = EXCLUDED.username,
            display_name = CASE WHEN users.display_name_customized
                THEN users.display_name ELSE EXCLUDED.display_name END,
            email = CASE WHEN users.email_customized
                THEN users.email ELSE COALESCE(EXCLUDED.email, users.email) END,
            avatar_url = EXCLUDED.avatar_url,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(profile.id)
    .bind(&profile.login)
    .bind(&profile.name)
    .bind(&profile.email)
    .bind(&profile.avatar_url)
    .fetch_one(pool)
    .await
}

/// Apply an explicit profile edit. Only the provided fields move, and each
/// one flips its `*_customized` flag so future GitHub logins stop mirroring it.
pub async fn update_profile(
    pool: &PgPool,
    user_id: Uuid,
    display_name: Option<&str>,
    email: Option<&str>,
) -> sqlx::Result<Option<User>> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE users SET
            display_name = COALESCE($2, display_name),
            display_name_customized = display_name_customized OR $2 IS NOT NULL,
            email = COALESCE($3, email),
            email_customized = email_customized OR $3 IS NOT NULL,
            updated_at = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(user_id)
    .bind(display_name)
    .bind(email)
    .fetch_optional(pool)
    .await
}

/// Delete the account and everything it owns, in one transaction. The
/// workspace's audit ledger is deleted explicitly first (its workspace_id
/// FK is ON DELETE SET NULL, which would otherwise strand de-identified
/// rows invisible to every reader — retention policy is full removal).
/// Then the workspace goes: `workspaces.created_by` is a RESTRICT foreign
/// key, and deleting the workspace cascades to every workspace-scoped
/// resource (members, repositories, pipelines, secrets, environments,
/// runners). Sessions cascade with the user row itself. Returns the logo
/// keys of deleted workspaces so the caller can best-effort remove the R2
/// objects.
pub async fn delete_account(pool: &PgPool, user_id: Uuid) -> sqlx::Result<Vec<String>> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        DELETE FROM audit_logs
        WHERE workspace_id IN (SELECT id FROM workspaces WHERE created_by = $1)
        "#,
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let logo_keys: Vec<String> = sqlx::query_scalar(
        "DELETE FROM workspaces WHERE created_by = $1 RETURNING COALESCE(logo_key, '')",
    )
    .bind(user_id)
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(logo_keys.into_iter().filter(|k| !k.is_empty()).collect())
}
