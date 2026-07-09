use anyhow::Context;

/// Application configuration, loaded once at startup from environment
/// variables. Secrets never appear in Debug output or logs.
#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub github_client_id: String,
    pub github_client_secret: String,
    /// Exact redirect URL registered with the GitHub OAuth App.
    /// Acts as an allow-list of one — never derived from request input.
    pub oauth_redirect_url: String,
    /// Origin of the React app; used for CORS and post-login redirects.
    pub frontend_url: String,
    pub bind_addr: String,
    pub cookie_secure: bool,
    pub cookie_name: String,
    pub session_ttl_hours: i64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            database_url: required("DATABASE_URL")?,
            github_client_id: required("GITHUB_CLIENT_ID")?,
            github_client_secret: required("GITHUB_CLIENT_SECRET")?,
            oauth_redirect_url: optional(
                "OAUTH_REDIRECT_URL",
                "http://localhost:8080/auth/github/callback",
            ),
            frontend_url: optional("FRONTEND_URL", "http://localhost:3000")
                .trim_end_matches('/')
                .to_string(),
            bind_addr: optional("BIND_ADDR", "0.0.0.0:8080"),
            cookie_secure: optional("COOKIE_SECURE", "false")
                .parse()
                .context("COOKIE_SECURE must be true or false")?,
            cookie_name: optional("COOKIE_NAME", "overup_session"),
            session_ttl_hours: optional("SESSION_TTL_HOURS", "168")
                .parse()
                .context("SESSION_TTL_HOURS must be an integer")?,
        })
    }
}

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("missing required environment variable {key}"))
}

fn optional(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
