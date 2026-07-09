use sqlx::PgPool;

use crate::models::user::User;
use crate::services::github::GitHubProfile;

/// Create the user on first sign-in, refresh the mirrored GitHub profile on
/// every subsequent one. Email is only overwritten when GitHub returns one.
pub async fn upsert_by_github(pool: &PgPool, profile: &GitHubProfile) -> sqlx::Result<User> {
    sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (github_id, username, display_name, email, avatar_url)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (github_id) DO UPDATE SET
            username = EXCLUDED.username,
            display_name = EXCLUDED.display_name,
            email = COALESCE(EXCLUDED.email, users.email),
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
