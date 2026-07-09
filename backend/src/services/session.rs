use axum_extra::extract::cookie::{Cookie, SameSite};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use rand::TryRngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::Config;
use crate::db;
use crate::error::AppResult;

/// Generate a fresh session token. Returns `(cookie_value, token_hash)` —
/// the raw value goes to the browser, only the SHA-256 hex digest is stored.
pub fn generate_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .expect("operating system RNG unavailable");
    let value = URL_SAFE_NO_PAD.encode(bytes);
    let hash = hash_token(&value);
    (value, hash)
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Create a session row and return the cookie to hand to the browser.
pub async fn create_session(
    pool: &sqlx::PgPool,
    config: &Config,
    user_id: Uuid,
) -> AppResult<Cookie<'static>> {
    let (value, hash) = generate_token();
    let expires_at = Utc::now() + Duration::hours(config.session_ttl_hours);
    db::sessions::create(pool, user_id, &hash, expires_at).await?;
    Ok(build_cookie(config, value, config.session_ttl_hours))
}

/// Invalidate the session behind a cookie value (if any) and return an
/// expired cookie that clears it from the browser.
pub async fn destroy_session(
    pool: &sqlx::PgPool,
    config: &Config,
    token: &str,
) -> AppResult<Cookie<'static>> {
    db::sessions::delete_by_token_hash(pool, &hash_token(token)).await?;
    Ok(build_clear_cookie(config))
}

fn build_cookie(config: &Config, value: String, ttl_hours: i64) -> Cookie<'static> {
    Cookie::build((config.cookie_name.clone(), value))
        .path("/")
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::hours(ttl_hours))
        .build()
}

pub fn build_clear_cookie(config: &Config) -> Cookie<'static> {
    Cookie::build((config.cookie_name.clone(), ""))
        .path("/")
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::ZERO)
        .build()
}
