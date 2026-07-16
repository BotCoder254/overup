use anyhow::Context;

use crate::services::runner_profiles::ResourceProfile;

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
    /// True when this deployment sits behind a trusted reverse proxy
    /// (Traefik/Dokploy/nginx). Enables reading the client IP for session
    /// metadata from the RIGHTMOST X-Forwarded-For hop (the one appended by
    /// the trusted proxy — the leftmost is client-spoofable). Display-only;
    /// rate limiting deliberately stays keyed on the socket peer address.
    pub trust_proxy: bool,
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
    /// Report pipeline status back to GitHub via the Checks API (queued →
    /// in_progress → completed). Requires the App's Checks (Read & write)
    /// permission; without it reporting degrades to an edge-triggered warn.
    pub github_checks_enabled: bool,
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
    /// Days an ARCHIVED notification is kept before the janitor hard-deletes
    /// it. Independent from audit retention: notifications are operational
    /// awareness, the ledger is the permanent record.
    pub notification_retention_days: i32,
    /// Days after a notification is read before the janitor auto-archives it.
    pub notification_auto_archive_days: i32,
    /// Idle timeout for sessions: a session unused for this many hours is
    /// invalid even before its absolute expiry (0 disables). Uses the
    /// `last_seen_at` column that is already touched on every request.
    pub session_idle_timeout_hours: i64,
    /// MinIO (or any S3-compatible endpoint) storage — all-or-none optional
    /// group. When configured, MinIO is the DEFAULT / primary object store;
    /// R2 becomes the fallback.
    pub minio: Option<MinioConfig>,
    /// Cloudflare R2 storage (artifacts + gzip'd log archives) — all-or-none
    /// optional group; without it (and without MinIO), artifact grants are
    /// cleanly denied and logs simply stay in Postgres.
    pub r2: Option<R2Config>,
    /// Hosted-runner provisioning (Docker containers spawned by the control
    /// plane) — optional group enabled with RUNNER_PROVISIONER=docker;
    /// without it, hosted-runner creation is cleanly denied.
    pub runner_provisioner: Option<RunnerProvisionerConfig>,
    /// Master key wrapping per-secret data-encryption keys
    /// (SECRETS_MASTER_KEY: exactly 32 bytes, hex- or base64-encoded).
    /// Optional: without it the Secrets API denies mutations cleanly and
    /// dispatch fails closed for repositories that have stored secrets.
    /// Losing the key makes existing secrets permanently undecryptable
    /// (no plaintext escrow by design) — replace values to recover.
    pub secrets_master_key: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct R2Config {
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
}

#[derive(Clone)]
pub struct MinioConfig {
    /// Full http(s) endpoint, e.g. http://localhost:9000. Plain http on a
    /// non-loopback host draws a startup warning (credentials in cleartext).
    pub endpoint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    /// MinIO's own default region (MINIO_REGION, default us-east-1).
    pub region: String,
    /// Path-style addressing (MINIO_FORCE_PATH_STYLE, default true) — the
    /// standard MinIO deployment shape.
    pub force_path_style: bool,
}

#[derive(Clone)]
pub struct RunnerProvisionerConfig {
    /// Runner image to run, e.g. ghcr.io/botcoder254/overup-runner:latest
    /// (RUNNER_IMAGE).
    pub image: String,
    /// URL provisioned containers use to reach this control plane
    /// (RUNNER_PROVISIONER_OVERUP_URL). Required: a container cannot assume
    /// the operator's localhost.
    pub overup_url: String,
    /// DOCKER_HOST value passed through into runner containers so they can
    /// execute jobs (RUNNER_PROVISIONER_DOCKER_HOST). Empty/unset = mount
    /// /var/run/docker.sock into the container instead — note that the
    /// socket mount is root-equivalent on the host. This is NOT the daemon
    /// the provisioner itself talks to — that's `docker_socket` below.
    pub runner_docker_host: Option<String>,
    /// Explicit Docker endpoint for the provisioner's OWN connection
    /// (RUNNER_PROVISIONER_DOCKER_SOCKET): a unix socket path (with or
    /// without a unix:// prefix; e.g. rootless Docker's
    /// $XDG_RUNTIME_DIR/docker.sock) or a Windows named pipe. Remote TCP
    /// daemons must use DOCKER_HOST (+ DOCKER_TLS_VERIFY/DOCKER_CERT_PATH)
    /// so TLS handling stays on the audited bollard path. Unset = probe
    /// DOCKER_HOST, then the well-known local socket locations.
    pub docker_socket: Option<String>,
    /// Auto-create one hosted runner when a workspace is created
    /// (RUNNER_AUTO_PROVISION, default true whenever the provisioner is on).
    pub auto_provision: bool,
    /// Hosted-runner quotas: managed, non-revoked runners counted per
    /// workspace (HOSTED_RUNNERS_PER_WORKSPACE) and across the whole
    /// deployment (HOSTED_RUNNERS_GLOBAL). Both must be >= 1 — disabling
    /// hosted runners is RUNNER_PROVISIONER=off, not a zero quota.
    pub max_per_workspace: i64,
    pub max_global: i64,
    /// Docker network runner containers join (RUNNER_PROVISIONER_NETWORK,
    /// default `overup-runners`). Set to `bridge` to opt out of the dedicated
    /// network; anything else is created at startup if missing.
    pub network: String,
    /// Resource profile applied when a request doesn't pick one — including
    /// workspace auto-provisioning (RUNNER_PROVISIONER_DEFAULT_PROFILE,
    /// default `standard`).
    pub default_profile: ResourceProfile,
    /// Job images pre-pulled into the daemon after every successful
    /// provisioner (re)connect (RUNNER_PREPULL_IMAGES, comma-separated;
    /// default = DEFAULT_JOB_IMAGE). The runner image itself is always
    /// warmed in addition. Pull failures are warn-only — they never affect
    /// hosted-runner availability.
    pub prepull_images: Vec<String>,
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

        // Parity with the signing-key rule: a guessable webhook secret would
        // let anyone forge GitHub deliveries, so a weak one fails at boot.
        let github_webhook_secret = required("GITHUB_WEBHOOK_SECRET")?;
        if github_webhook_secret.len() < 16 {
            anyhow::bail!(
                "GITHUB_WEBHOOK_SECRET must be at least 16 bytes (generate one with `openssl rand -hex 32` and set it on the GitHub App too)"
            );
        }

        let session_idle_timeout_hours: i64 = optional("SESSION_IDLE_TIMEOUT_HOURS", "72")
            .parse()
            .context("SESSION_IDLE_TIMEOUT_HOURS must be an integer (0 disables)")?;
        if session_idle_timeout_hours < 0 {
            anyhow::bail!("SESSION_IDLE_TIMEOUT_HOURS must be 0 (disabled) or positive");
        }

        // MinIO settings are all-or-none, mirroring the R2 group. MinIO is
        // the default/primary object store whenever it is configured.
        let minio_keys = [
            "MINIO_ENDPOINT",
            "MINIO_ACCESS_KEY_ID",
            "MINIO_SECRET_ACCESS_KEY",
            "MINIO_BUCKET",
        ];
        let minio_present = minio_keys
            .iter()
            .filter(|key| std::env::var(key).is_ok())
            .count();
        let minio = match minio_present {
            0 => None,
            4 => {
                let endpoint = required("MINIO_ENDPOINT")?.trim_end_matches('/').to_string();
                validate_minio_endpoint(&endpoint)?;
                let force_path_style: bool = optional("MINIO_FORCE_PATH_STYLE", "true")
                    .parse()
                    .context("MINIO_FORCE_PATH_STYLE must be true or false")?;
                Some(MinioConfig {
                    endpoint,
                    access_key_id: required("MINIO_ACCESS_KEY_ID")?,
                    secret_access_key: required("MINIO_SECRET_ACCESS_KEY")?,
                    bucket: required("MINIO_BUCKET")?,
                    region: optional("MINIO_REGION", "us-east-1"),
                    force_path_style,
                })
            }
            _ => anyhow::bail!(
                "MinIO configuration is incomplete: set all of MINIO_ENDPOINT, MINIO_ACCESS_KEY_ID, MINIO_SECRET_ACCESS_KEY, MINIO_BUCKET or none"
            ),
        };

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

        // Parsed ahead of the provisioner group: it doubles as the default
        // pre-pull list, so the image jobs fall back to is always warm.
        // catthehacker/ubuntu:act-* is the GitHub-runner-compatible family
        // nektos/act and Gitea Actions default to — bare ubuntu images have
        // no toolchains, so real workflows die on `npm: not found`.
        let default_job_image = optional("DEFAULT_JOB_IMAGE", "catthehacker/ubuntu:act-latest");

        // Hosted-runner provisioning is opt-in: the server needs Docker
        // access and a URL that provisioned containers can reach it on.
        let runner_provisioner = match optional("RUNNER_PROVISIONER", "").as_str() {
            "" | "off" | "false" => None,
            "docker" => {
                let auto_provision: bool = optional("RUNNER_AUTO_PROVISION", "true")
                    .parse()
                    .context("RUNNER_AUTO_PROVISION must be true or false")?;
                let max_per_workspace: i64 = optional("HOSTED_RUNNERS_PER_WORKSPACE", "3")
                    .parse()
                    .context("HOSTED_RUNNERS_PER_WORKSPACE must be an integer")?;
                let max_global: i64 = optional("HOSTED_RUNNERS_GLOBAL", "20")
                    .parse()
                    .context("HOSTED_RUNNERS_GLOBAL must be an integer")?;
                if max_per_workspace < 1 || max_global < 1 {
                    anyhow::bail!(
                        "HOSTED_RUNNERS_PER_WORKSPACE and HOSTED_RUNNERS_GLOBAL must be at least 1 \
                         (disable hosted runners with RUNNER_PROVISIONER=off instead)"
                    );
                }
                let default_profile_raw =
                    optional("RUNNER_PROVISIONER_DEFAULT_PROFILE", "standard");
                let Some(default_profile) = ResourceProfile::from_str(&default_profile_raw) else {
                    anyhow::bail!(
                        "RUNNER_PROVISIONER_DEFAULT_PROFILE must be small, standard, or large \
                         (got {default_profile_raw})"
                    );
                };
                let network = optional("RUNNER_PROVISIONER_NETWORK", "overup-runners");
                if network.is_empty() {
                    anyhow::bail!(
                        "RUNNER_PROVISIONER_NETWORK must not be empty (use 'bridge' to opt out \
                         of the dedicated network)"
                    );
                }
                // Job images warmed into the daemon on every provisioner
                // (re)connect (RUNNER_PREPULL_IMAGES, comma-separated).
                // Default: the default job image, so the common
                // `pulling_image` stage is near-instant. Validated here so a
                // malformed deployment fails loudly; pulls themselves are
                // warn-only at runtime.
                let prepull_images =
                    parse_prepull_list(&optional("RUNNER_PREPULL_IMAGES", &default_job_image))?;
                Some(RunnerProvisionerConfig {
                    image: optional("RUNNER_IMAGE", "ghcr.io/botcoder254/overup-runner:latest"),
                    overup_url: required("RUNNER_PROVISIONER_OVERUP_URL")
                        .context(
                            "RUNNER_PROVISIONER=docker requires RUNNER_PROVISIONER_OVERUP_URL — \
                             the URL runner containers use to reach this control plane",
                        )?
                        .trim_end_matches('/')
                        .to_string(),
                    runner_docker_host: std::env::var("RUNNER_PROVISIONER_DOCKER_HOST")
                        .ok()
                        .filter(|v| !v.is_empty()),
                    docker_socket: std::env::var("RUNNER_PROVISIONER_DOCKER_SOCKET")
                        .ok()
                        .filter(|v| !v.is_empty()),
                    auto_provision,
                    max_per_workspace,
                    max_global,
                    network,
                    default_profile,
                    prepull_images,
                })
            }
            other => anyhow::bail!("RUNNER_PROVISIONER must be 'docker' or unset (got {other})"),
        };

        // Secrets master key: exactly 32 decoded bytes. 64 hex chars is the
        // documented form (openssl rand -hex 32); base64 of 32 bytes is also
        // accepted. Anything else is a deployment mistake, not a toggle.
        let secrets_master_key = match std::env::var("SECRETS_MASTER_KEY") {
            Err(_) => None,
            Ok(raw) => {
                let raw = raw.trim();
                let decoded = if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
                    hex::decode(raw).context("SECRETS_MASTER_KEY is not valid hex")?
                } else {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD
                        .decode(raw)
                        .context("SECRETS_MASTER_KEY is not valid hex or base64")?
                };
                if decoded.len() != 32 {
                    anyhow::bail!(
                        "SECRETS_MASTER_KEY must decode to exactly 32 bytes (generate one with `openssl rand -hex 32`)"
                    );
                }
                Some(decoded)
            }
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
        let notification_retention_days: i32 = optional("NOTIFICATION_RETENTION_DAYS", "90")
            .parse()
            .context("NOTIFICATION_RETENTION_DAYS must be an integer")?;
        if notification_retention_days < 1 {
            anyhow::bail!("NOTIFICATION_RETENTION_DAYS must be at least 1");
        }
        let notification_auto_archive_days: i32 =
            optional("NOTIFICATION_AUTO_ARCHIVE_DAYS", "14")
                .parse()
                .context("NOTIFICATION_AUTO_ARCHIVE_DAYS must be an integer")?;
        if notification_auto_archive_days < 1 {
            anyhow::bail!("NOTIFICATION_AUTO_ARCHIVE_DAYS must be at least 1");
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
            trust_proxy: optional("TRUST_PROXY", "false")
                .parse()
                .context("TRUST_PROXY must be true or false")?,
            github_app_client_id: required("GITHUB_APP_CLIENT_ID")?,
            github_app_private_key_pem,
            github_webhook_secret,
            github_app_slug: required("GITHUB_APP_SLUG")?,
            github_checks_enabled: optional("GITHUB_CHECKS_ENABLED", "true")
                .parse()
                .context("GITHUB_CHECKS_ENABLED must be true or false")?,
            runner_job_signing_key,
            default_job_image,
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
            notification_retention_days,
            notification_auto_archive_days,
            session_idle_timeout_hours,
            minio,
            r2,
            runner_provisioner,
            secrets_master_key,
        })
    }
}

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("missing required environment variable {key}"))
}

/// MINIO_ENDPOINT must be a syntactically sane http(s) URL. Plain http is
/// allowed (the standard local `docker compose` shape) but draws a loud
/// warning on non-loopback hosts: S3 credentials would cross the network in
/// cleartext.
fn validate_minio_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let rest = if let Some(rest) = endpoint.strip_prefix("https://") {
        rest
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        let host = rest
            .split(['/', ':'])
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !loopback {
            tracing::warn!(
                "MINIO_ENDPOINT uses plain http on a non-loopback host — S3 credentials and \
                 presigned uploads cross the network in cleartext; put MinIO behind TLS"
            );
        }
        rest
    } else {
        anyhow::bail!("MINIO_ENDPOINT must start with http:// or https://");
    };
    if rest.is_empty() || rest.starts_with('/') {
        anyhow::bail!("MINIO_ENDPOINT is missing a host");
    }
    // A bare host with no explicit port defaults to 80/443 — almost always the
    // MinIO *console*, not the S3 API. Bucket/object operations against the
    // console fail with "S3 API Requests must be made to API port." Warn (never
    // fail: a fronting proxy on 443 may legitimately route to the API).
    let authority = rest.split('/').next().unwrap_or_default();
    let after_bracket = authority.rsplit(']').next().unwrap_or(authority);
    if !after_bracket.contains(':') {
        tracing::warn!(
            "MINIO_ENDPOINT has no explicit port — a bare host defaults to 80/443, which is \
             usually the MinIO console, not the S3 API. If bucket creation fails with \
             \"S3 API Requests must be made to API port.\", point MINIO_ENDPOINT at the API \
             endpoint (e.g. host:9000)"
        );
    }
    Ok(())
}

fn optional(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Parse the comma-separated RUNNER_PREPULL_IMAGES value: trim entries, drop
/// empties, dedupe preserving order, and enforce sane caps so a malformed
/// deployment fails at startup instead of spraying pull warnings forever.
fn parse_prepull_list(raw: &str) -> anyhow::Result<Vec<String>> {
    let mut images: Vec<String> = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.len() > 256 {
            anyhow::bail!(
                "RUNNER_PREPULL_IMAGES entries must be at most 256 characters (got one of {} \
                 characters)",
                entry.len()
            );
        }
        if !images.iter().any(|existing| existing == entry) {
            images.push(entry.to_string());
        }
    }
    if images.len() > 20 {
        anyhow::bail!(
            "RUNNER_PREPULL_IMAGES supports at most 20 images (got {})",
            images.len()
        );
    }
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::{parse_prepull_list, validate_minio_endpoint};

    #[test]
    fn minio_endpoint_accepts_http_and_https_urls() {
        assert!(validate_minio_endpoint("http://localhost:9000").is_ok());
        assert!(validate_minio_endpoint("http://127.0.0.1:9000").is_ok());
        assert!(validate_minio_endpoint("https://minio.example.com").is_ok());
        // Non-loopback http is allowed (warn-only at startup).
        assert!(validate_minio_endpoint("http://10.0.0.5:9000").is_ok());
    }

    #[test]
    fn minio_endpoint_rejects_malformed_urls() {
        assert!(validate_minio_endpoint("localhost:9000").is_err());
        assert!(validate_minio_endpoint("ftp://minio.example.com").is_err());
        assert!(validate_minio_endpoint("http://").is_err());
        assert!(validate_minio_endpoint("https:///bucket").is_err());
    }

    #[test]
    fn prepull_list_trims_dedupes_and_drops_empties() {
        let parsed =
            parse_prepull_list("ubuntu:24.04, ,debian:bookworm-slim,ubuntu:24.04").unwrap();
        assert_eq!(parsed, vec!["ubuntu:24.04", "debian:bookworm-slim"]);
    }

    #[test]
    fn prepull_list_single_default() {
        assert_eq!(parse_prepull_list("ubuntu:24.04").unwrap(), vec!["ubuntu:24.04"]);
    }

    #[test]
    fn prepull_list_rejects_oversized_entry() {
        assert!(parse_prepull_list(&"x".repeat(257)).is_err());
    }

    #[test]
    fn prepull_list_rejects_too_many_images() {
        let raw = (0..21).map(|i| format!("image-{i}")).collect::<Vec<_>>().join(",");
        assert!(parse_prepull_list(&raw).is_err());
    }
}
