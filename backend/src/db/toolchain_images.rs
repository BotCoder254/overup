//! Access to `installed_toolchain_images` — the persisted set of toolchain
//! images a user has installed from the UI. The set is global (one runner
//! Docker daemon per deployment); `list_active_images` feeds the provisioner's
//! prewarm union so installed toolchains re-warm on every reconnect. Audit
//! entries for install/uninstall are written by the handler (they carry the
//! calling workspace + actor); this module only owns the row lifecycle.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// One installed-toolchain row.
#[derive(Debug, sqlx::FromRow)]
pub struct InstalledToolchainRow {
    pub toolchain_key: String,
    #[allow(dead_code)]
    pub image: String,
    pub status: String,
    pub error: Option<String>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

/// Every install row (any status) — for the catalog read.
pub async fn list(pool: &PgPool) -> sqlx::Result<Vec<InstalledToolchainRow>> {
    sqlx::query_as::<_, InstalledToolchainRow>(
        "SELECT toolchain_key, image, status, error, created_at, updated_at \
         FROM installed_toolchain_images",
    )
    .fetch_all(pool)
    .await
}

/// Images to prewarm: installed, plus pending (a pull already in flight — warm
/// it too so a reconnect during install doesn't drop it).
pub async fn list_active_images(pool: &PgPool) -> sqlx::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT image FROM installed_toolchain_images \
         WHERE status IN ('installed', 'pending')",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(image,)| image).collect())
}

/// Mark a toolchain as install-pending (idempotent: a re-install or retry
/// resets an existing row to pending and clears any prior error).
pub async fn upsert_pending(pool: &PgPool, key: &str, image: &str) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO installed_toolchain_images (toolchain_key, image, status) \
         VALUES ($1, $2, 'pending') \
         ON CONFLICT (toolchain_key) DO UPDATE \
         SET image = EXCLUDED.image, status = 'pending', error = NULL, updated_at = now()",
    )
    .bind(key)
    .bind(image)
    .execute(pool)
    .await?;
    Ok(())
}

/// Flip a row to installed after a successful pull. Guards on the row still
/// existing (a race with uninstall drops the update harmlessly).
pub async fn mark_installed(pool: &PgPool, key: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE installed_toolchain_images \
         SET status = 'installed', error = NULL, updated_at = now() \
         WHERE toolchain_key = $1",
    )
    .bind(key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Flip a row to failed with a static error category.
pub async fn mark_failed(pool: &PgPool, key: &str, error: &str) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE installed_toolchain_images \
         SET status = 'failed', error = $2, updated_at = now() \
         WHERE toolchain_key = $1",
    )
    .bind(key)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a row (uninstall); returns the removed image so the caller can rmi it.
pub async fn delete(pool: &PgPool, key: &str) -> sqlx::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "DELETE FROM installed_toolchain_images WHERE toolchain_key = $1 RETURNING image",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(image,)| image))
}
