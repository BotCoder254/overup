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
    /// GitHub App client ID — the JWT `iss` claim (GitHub recommends the
    /// client ID over the numeric app ID).
    pub github_app_client_id: String,
    /// RSA private key PEM for signing GitHub App JWTs. Loaded once at
    /// startup from a file path or base64 env var; bytes never appear in
    /// Debug output or logs.
    pub github_app_private_key_pem: Vec<u8>,
    /// Shared secret for HMAC-SHA256 verification of GitHub webhooks.
    pub github_webhook_secret: String,
    /// Public slug of the GitHub App; builds the install URL
    /// https://github.com/apps/{slug}/installations/new.
    pub github_app_slug: String,
    /// Shared key (>= 32 bytes) for HMAC-SHA256 signatures over job payloads
    /// sent to runners. Never logged.
    pub runner_job_signing_key: Vec<u8>,
    /// Container image used when a job declares no container and no known
    /// runs-on label.
    pub default_job_image: String,
    pub job_timeout_seconds: i32,
    pub pipeline_timeout_seconds: i32,
    pub max_log_bytes_per_job: i64,
    pub max_artifact_bytes: i64,
    pub max_artifacts_per_job: i64,
    /// Days an uploaded artifact stays downloadable before the janitor
    /// expires it and deletes the R2 object.
    pub artifact_retention_days: i64,
    /// Hours a `pending` artifact row may linger (upload never confirmed)
    /// before the janitor deletes it and any half-uploaded R2 object.
    pub artifact_pending_ttl_hours: i64,
    /// Days a finished job's log chunks stay hot in Postgres after they have
    /// been archived to R2. Only relevant when R2 is configured — without it,
    /// chunks are never pruned.
    pub log_hot_retention_days: i64,
    /// Cloudflare R2 storage (artifacts + gzip'd log archives) — all-or-none
    /// optional group; without it, artifact grants are cleanly denied and
    /// logs simply stay in Postgres.
    pub r2: Option<R2Config>,
}

#[derive(Clone)]
pub struct R2Config {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let cookie_secure: bool = optional("COOKIE_SECURE", "false")
            .parse()
            .context("COOKIE_SECURE must be true or false")?;

        // On HTTPS deployments the __Host- prefix binds the cookie to this
        // exact host (Secure, Path=/, no Domain), so a compromised subdomain
        // can't plant or fixate a session cookie. The prerequisites already
        // hold: build_cookie sets Secure + Path=/ and never sets Domain.
        let mut cookie_name = optional("COOKIE_NAME", "overup_session");
        if cookie_secure && !cookie_name.starts_with("__Host-") {
            cookie_name = format!("__Host-{cookie_name}");
        }

        let github_app_private_key_pem = match std::env::var("GITHUB_APP_PRIVATE_KEY_PATH") {
            Ok(path) => std::fs::read(&path)
                .with_context(|| format!("failed to read GITHUB_APP_PRIVATE_KEY_PATH {path}"))?,
            Err(_) => {
                use base64::Engine;
                let b64 = required("GITHUB_APP_PRIVATE_KEY_B64").context(
                    "set GITHUB_APP_PRIVATE_KEY_PATH or GITHUB_APP_PRIVATE_KEY_B64 for the GitHub App",
                )?;
                base64::engine::general_purpose::STANDARD
                    .decode(b64.trim())
                    .context("GITHUB_APP_PRIVATE_KEY_B64 is not valid base64")?
            }
        };

        let runner_job_signing_key = required("RUNNER_JOB_SIGNING_KEY")?.into_bytes();
        if runner_job_signing_key.len() < 32 {
            anyhow::bail!("RUNNER_JOB_SIGNING_KEY must be at least 32 bytes");
        }

        // R2 settings are all-or-none: a partial configuration is a
        // deployment mistake, not a feature toggle.
        let r2_keys = [
            "R2_ACCOUNT_ID",
            "R2_ACCESS_KEY_ID",
            "R2_SECRET_ACCESS_KEY",
            "R2_BUCKET",
        ];
        let r2_present = r2_keys
            .iter()
            .filter(|key| std::env::var(key).is_ok())
            .count();
        let r2 = match r2_present {
            0 => None,
            4 => Some(R2Config {
                account_id: required("R2_ACCOUNT_ID")?,
                access_key_id: required("R2_ACCESS_KEY_ID")?,
                secret_access_key: required("R2_SECRET_ACCESS_KEY")?,
                bucket: required("R2_BUCKET")?,
            }),
            _ => anyhow::bail!(
                "R2 configuration is incomplete: set all of R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_BUCKET or none"
            ),
        };

        let artifact_retention_days: i64 = optional("ARTIFACT_RETENTION_DAYS", "30")
            .parse()
            .context("ARTIFACT_RETENTION_DAYS must be an integer")?;
        if artifact_retention_days < 1 {
            anyhow::bail!("ARTIFACT_RETENTION_DAYS must be at least 1");
        }
        let artifact_pending_ttl_hours: i64 = optional("ARTIFACT_PENDING_TTL_HOURS", "24")
            .parse()
            .context("ARTIFACT_PENDING_TTL_HOURS must be an integer")?;
        if artifact_pending_ttl_hours < 1 {
            anyhow::bail!("ARTIFACT_PENDING_TTL_HOURS must be at least 1");
        }
        let log_hot_retention_days: i64 = optional("LOG_HOT_RETENTION_DAYS", "7")
            .parse()
            .context("LOG_HOT_RETENTION_DAYS must be an integer")?;
        if log_hot_retention_days < 1 {
            anyhow::bail!("LOG_HOT_RETENTION_DAYS must be at least 1");
        }

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
            cookie_secure,
            cookie_name,
            session_ttl_hours: optional("SESSION_TTL_HOURS", "168")
                .parse()
                .context("SESSION_TTL_HOURS must be an integer")?,
            github_app_client_id: required("GITHUB_APP_CLIENT_ID")?,
            github_app_private_key_pem,
            github_webhook_secret: required("GITHUB_WEBHOOK_SECRET")?,
            github_app_slug: required("GITHUB_APP_SLUG")?,
            runner_job_signing_key,
            default_job_image: optional("DEFAULT_JOB_IMAGE", "ubuntu:24.04"),
            job_timeout_seconds: optional("JOB_TIMEOUT_SECONDS", "3600")
                .parse()
                .context("JOB_TIMEOUT_SECONDS must be an integer")?,
            pipeline_timeout_seconds: optional("PIPELINE_TIMEOUT_SECONDS", "7200")
                .parse()
                .context("PIPELINE_TIMEOUT_SECONDS must be an integer")?,
            max_log_bytes_per_job: optional("MAX_LOG_BYTES_PER_JOB", "10485760")
                .parse()
                .context("MAX_LOG_BYTES_PER_JOB must be an integer")?,
            max_artifact_bytes: optional("MAX_ARTIFACT_BYTES", "104857600")
                .parse()
                .context("MAX_ARTIFACT_BYTES must be an integer")?,
            max_artifacts_per_job: optional("MAX_ARTIFACTS_PER_JOB", "10")
                .parse()
                .context("MAX_ARTIFACTS_PER_JOB must be an integer")?,
            artifact_retention_days,
            artifact_pending_ttl_hours,
            log_hot_retention_days,
            r2,
        })
    }
}

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("missing required environment variable {key}"))
}

fn optional(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
