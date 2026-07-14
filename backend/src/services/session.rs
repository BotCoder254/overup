use std::net::SocketAddr;

use axum::http::HeaderMap;
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

/// Upper bound persisted for a client user-agent string.
const MAX_USER_AGENT_LEN: usize = 256;

/// Client fingerprint captured at session creation for the active-sessions
/// list. Best-effort metadata only — authorization never keys off it.
#[derive(Debug, Default)]
pub struct ClientInfo {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

/// Derive the client fingerprint from the connection.
///
/// Without `trust_proxy` the IP is the socket peer address — an
/// X-Forwarded-For header is client-controlled and deliberately ignored.
/// With `trust_proxy` (deployment behind exactly one trusted reverse proxy,
/// e.g. Traefik/Dokploy) the IP comes from the RIGHTMOST valid entry of
/// X-Forwarded-For — the hop the trusted proxy itself appended; the leftmost
/// entries are whatever the client chose to send. Falls back to X-Real-Ip,
/// then the peer address. Display-only metadata: rate limiting stays keyed
/// on the socket peer address regardless.
pub fn client_info(headers: &HeaderMap, addr: SocketAddr, trust_proxy: bool) -> ClientInfo {
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|ua| {
            ua.chars()
                .filter(|c| !c.is_control())
                .take(MAX_USER_AGENT_LEN)
                .collect::<String>()
        })
        .filter(|ua| !ua.is_empty());

    let forwarded_ip = trust_proxy
        .then(|| forwarded_client_ip(headers))
        .flatten();
    ClientInfo {
        ip: Some(
            forwarded_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| addr.ip().to_string()),
        ),
        user_agent,
    }
}

/// The rightmost parseable IP in X-Forwarded-For, else X-Real-Ip — the hop
/// the trusted proxy itself appended, never a client-chosen leftmost entry.
/// Shared with the rate-limiter key extractor (routes) so both surfaces
/// resolve the same client identity behind TRUST_PROXY.
pub fn forwarded_client_ip(headers: &HeaderMap) -> Option<std::net::IpAddr> {
    let from_xff = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .rsplit(',')
                .map(str::trim)
                .find_map(|hop| hop.parse::<std::net::IpAddr>().ok())
        });
    from_xff.or_else(|| {
        headers
            .get("x-real-ip")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<std::net::IpAddr>().ok())
    })
}

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
    client: &ClientInfo,
) -> AppResult<Cookie<'static>> {
    let (value, hash) = generate_token();
    let expires_at = Utc::now() + Duration::hours(config.session_ttl_hours);
    db::sessions::create(
        pool,
        user_id,
        &hash,
        expires_at,
        client.ip.as_deref(),
        client.user_agent.as_deref(),
    )
    .await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn addr() -> SocketAddr {
        "10.0.0.9:443".parse().unwrap()
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                axum::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn untrusted_proxy_ignores_forwarded_headers() {
        let info = client_info(
            &headers(&[("x-forwarded-for", "203.0.113.7"), ("x-real-ip", "203.0.113.7")]),
            addr(),
            false,
        );
        assert_eq!(info.ip.as_deref(), Some("10.0.0.9"));
    }

    #[test]
    fn trusted_proxy_takes_rightmost_xff_hop() {
        // The leftmost entry is client-supplied spoof; the trusted proxy
        // appended the real client at the end.
        let info = client_info(
            &headers(&[("x-forwarded-for", "1.2.3.4, 203.0.113.7")]),
            addr(),
            true,
        );
        assert_eq!(info.ip.as_deref(), Some("203.0.113.7"));
    }

    #[test]
    fn trusted_proxy_skips_garbage_and_falls_back() {
        // Rightmost entries that don't parse are skipped right-to-left.
        let info = client_info(
            &headers(&[("x-forwarded-for", "203.0.113.7, not-an-ip")]),
            addr(),
            true,
        );
        assert_eq!(info.ip.as_deref(), Some("203.0.113.7"));

        // All-garbage XFF → X-Real-Ip.
        let info = client_info(
            &headers(&[("x-forwarded-for", "junk"), ("x-real-ip", "198.51.100.4")]),
            addr(),
            true,
        );
        assert_eq!(info.ip.as_deref(), Some("198.51.100.4"));

        // No forwarded headers at all → peer address.
        let info = client_info(&headers(&[]), addr(), true);
        assert_eq!(info.ip.as_deref(), Some("10.0.0.9"));
    }

    #[test]
    fn user_agent_is_capped_and_control_stripped() {
        // Tab is a control character that is still legal inside a header
        // value, so it exercises the stripping path.
        let long_ua = "M\tozilla ".repeat(100);
        let info = client_info(&headers(&[("user-agent", long_ua.as_str())]), addr(), false);
        let ua = info.user_agent.unwrap();
        assert!(ua.chars().count() <= 256);
        assert!(!ua.chars().any(char::is_control));
    }
}
