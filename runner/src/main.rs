//! overup reference runner.
//!
//! An orchestration daemon: it holds one outbound WebSocket to the control
//! plane, receives HMAC-signed job payloads, and executes each job inside a
//! Docker container via the Docker Engine API (bollard). The runner itself
//! never compiles or tests anything — Docker provides process, filesystem,
//! and network isolation; the runner streams logs and lifecycle events back
//! and cleans everything up afterwards.
//!
//! Configuration (environment):
//!   OVERUP_URL                  control plane origin (default http://localhost:8080)
//!   RUNNER_TOKEN                registration token (shown once at creation).
//!                               When it is a short-lived bootstrap token
//!                               (guided wizard registration), the runner
//!                               receives a permanent token on first
//!                               connect and persists it to RUNNER_TOKEN_FILE.
//!   RUNNER_TOKEN_FILE           optional path to persist a permanent token
//!                               received from a bootstrap exchange; read on
//!                               startup in preference to RUNNER_TOKEN once
//!                               it exists
//!   RUNNER_JOB_SIGNING_KEY      optional shared HMAC key. When unset the
//!                               runner uses the key the control plane
//!                               delivers in hello_ack (in memory only,
//!                               re-received on every connect); when set it
//!                               must match the control plane and takes
//!                               precedence over the delivered key
//!   RUNNER_NAME                 display name sent in hello (default overup-runner)
//!   RUNNER_LABELS               comma-separated labels (default self-hosted)
//!   RUNNER_CAP_DROP             drop ALL Linux capabilities (default true; set
//!                               false only for workloads that need capabilities)
//!   RUNNER_JOB_MEMORY_BYTES     per-job memory limit (default 2 GiB; 0 = unlimited)
//!   RUNNER_JOB_NANO_CPUS        per-job CPU limit in 1e-9 CPUs (default 2 CPUs;
//!                               0 = unlimited)
//!   RUNNER_JOB_PIDS_LIMIT       per-job process cap (default 512; 0 = unlimited)
//!   RUNNER_JOB_NETWORK          bridge | none | isolated (default bridge;
//!                               isolated = throwaway per-job bridge network)
//!   RUNNER_JOB_USER             container user, e.g. 1000:1000 (default: image user)
//!   RUNNER_JOB_READONLY_ROOTFS  read-only root filesystem + tmpfs /tmp (default false)
//!   RUNNER_SUDO_SHIM            install a `sudo` shim in root job containers so
//!                               GitHub-authored `sudo …` steps run (default true;
//!                               see JobIsolation::sudo_shim)
//!
//! Docker connection: honors DOCKER_HOST (unix://, npipe://, tcp://…) plus
//! DOCKER_TLS_VERIFY=1 and DOCKER_CERT_PATH (ca.pem/cert.pem/key.pem) for
//! remote TLS-secured daemons; defaults to the local socket / named pipe.

mod artifacts;
mod executor;
mod health;
mod ws;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct RunnerConfig {
    pub server_url: String,
    pub token: String,
    pub name: String,
    pub labels: Vec<String>,
    /// Locally pinned job-payload verification key. `None` means "use the
    /// key the control plane delivers in hello_ack".
    pub signing_key: Option<Vec<u8>>,
    pub isolation: JobIsolation,
}

/// Container hardening applied to every job. Defaults follow least
/// privilege: all capabilities dropped, bounded memory/CPU/pids, and
/// no-new-privileges is always set (not configurable).
#[derive(Clone)]
pub struct JobIsolation {
    pub cap_drop: bool,
    /// 0 disables the limit.
    pub memory_bytes: i64,
    /// 0 disables the limit.
    pub nano_cpus: i64,
    /// 0 disables the limit.
    pub pids_limit: i64,
    pub network: JobNetwork,
    pub user: Option<String>,
    pub readonly_rootfs: bool,
    /// Install a `sudo` shim into job containers that already run as root.
    /// no-new-privileges makes the kernel ignore the setuid bit, so the real
    /// setuid `sudo` can never work here; the shim strips sudo's options and
    /// execs the command directly. Compatibility only — the job is already
    /// uid 0, so this grants nothing. Skipped for non-root containers.
    pub sudo_shim: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobNetwork {
    /// Docker's default bridge — outbound network access.
    Bridge,
    /// No network at all.
    None,
    /// A throwaway per-job bridge network, removed after the job.
    Isolated,
}

/// The single job this runner is currently executing (one at a time).
pub struct ActiveJob {
    pub job_id: uuid::Uuid,
    pub cancel: tokio::sync::watch::Sender<bool>,
}

pub type CurrentJob = Arc<Mutex<Option<ActiveJob>>>;

/// This binary's identity: `0.1.0+<git sha>`, stamped by `build.rs`.
///
/// Reported in `hello` (stored on the runner row, shown in the Runners UI)
/// and logged at startup. `CARGO_PKG_VERSION` alone is the same string in
/// every build ever made, so it cannot distinguish a stale runner image from
/// a current one — the build ref is what makes that answerable.
pub fn runner_version() -> String {
    format!("{}+{}", env!("CARGO_PKG_VERSION"), env!("RUNNER_BUILD_REF"))
}

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("missing required environment variable {key}"))
}

fn optional(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Read a credential from the environment, treating set-but-blank exactly like
/// unset.
///
/// `std::env::var` returns `Ok("")` for `RUNNER_TOKEN=`, so without this guard
/// the runner starts, sends `Authorization: Bearer `, and loops forever on a
/// 401 it can never recover from — while the control plane reports
/// `missing_token`. Failing at boot turns an unbounded retry storm into one
/// actionable line. Trimming matters for the same reason: a trailing
/// newline (heredocs, secret files pasted into env) hashes to a different
/// token and earns a baffling 401.
fn token_from_env(key: &str) -> anyhow::Result<String> {
    let value = required(key)?.trim().to_string();
    if value.is_empty() {
        anyhow::bail!(
            "{key} is set but empty. Set it to the token shown when you registered the \
             runner, or point RUNNER_TOKEN_FILE at a file holding one. Hosted runners need \
             neither: the control plane provisions them and injects the token itself — set \
             RUNNER_PROVISIONER=docker there and create the runner from the Runners page \
             instead of starting this container by hand."
        );
    }
    Ok(value)
}

/// Write a freshly-issued permanent token to disk, restricted to the owner
/// (same convention as the `.env` file holding RUNNER_TOKEN today).
fn persist_token(path: &str, token: &str) -> anyhow::Result<()> {
    std::fs::write(path, token).with_context(|| format!("writing {path}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("chmod 600 {path}"))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // First line in the log, before anything can fail: which build is this?
    // A deployment that forgot to rebuild is the most common cause of "the
    // fix didn't work" (docs/fix-empty-workspace-stale-runner.md), and this
    // answers it without inferring from log ordering.
    tracing::info!(version = %runner_version(), "overup runner starting");

    // Optional since the control plane delivers the key in hello_ack; a
    // locally set key is validated here and takes precedence.
    let signing_key = match std::env::var("RUNNER_JOB_SIGNING_KEY") {
        Ok(key) => {
            let key = key.into_bytes();
            if key.len() < 32 {
                anyhow::bail!("RUNNER_JOB_SIGNING_KEY must be at least 32 bytes");
            }
            Some(key)
        }
        Err(_) => {
            tracing::info!(
                "no local RUNNER_JOB_SIGNING_KEY; using the key delivered by the control plane"
            );
            None
        }
    };

    let parse_limit = |key: &str, default: &str| -> anyhow::Result<i64> {
        let value: i64 = optional(key, default)
            .parse()
            .with_context(|| format!("{key} must be an integer"))?;
        if value < 0 {
            anyhow::bail!("{key} must be >= 0 (0 disables the limit)");
        }
        Ok(value)
    };

    let network = match optional("RUNNER_JOB_NETWORK", "bridge").as_str() {
        "bridge" => JobNetwork::Bridge,
        "none" => JobNetwork::None,
        "isolated" => JobNetwork::Isolated,
        other => anyhow::bail!("RUNNER_JOB_NETWORK must be bridge, none, or isolated (got {other})"),
    };

    // A prior bootstrap exchange may have persisted a permanent token here;
    // it takes priority over RUNNER_TOKEN (which, after a bootstrap-token
    // first run, holds a now-consumed one-time credential).
    let token_file = std::env::var("RUNNER_TOKEN_FILE").ok().filter(|p| !p.is_empty());
    let initial_token = token_file
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map_or_else(|| token_from_env("RUNNER_TOKEN"), Ok)?;

    // RUNNER_LABELS set-but-blank must not produce an empty label set: an
    // empty set can never satisfy a labeled runs-on, so the runner would sit
    // idle while jobs queue forever. Mirror GitHub's unremovable default
    // labels (jobs run in containers picked from runs-on, so these describe
    // the execution environment, not the host).
    let mut labels: Vec<String> = optional("RUNNER_LABELS", "self-hosted")
        .split(',')
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if labels.is_empty() {
        labels = ["self-hosted", "linux", "x64", "ubuntu-latest"]
            .map(String::from)
            .to_vec();
        tracing::warn!(?labels, "RUNNER_LABELS is empty; falling back to default labels");
    }

    let mut config = RunnerConfig {
        server_url: optional("OVERUP_URL", "http://localhost:8080")
            .trim_end_matches('/')
            .to_string(),
        token: initial_token,
        name: optional("RUNNER_NAME", "overup-runner"),
        labels,
        signing_key,
        isolation: JobIsolation {
            // Least privilege by default: opting OUT requires an explicit
            // RUNNER_CAP_DROP=false.
            cap_drop: optional("RUNNER_CAP_DROP", "true") != "false",
            memory_bytes: parse_limit("RUNNER_JOB_MEMORY_BYTES", "2147483648")?,
            nano_cpus: parse_limit("RUNNER_JOB_NANO_CPUS", "2000000000")?,
            pids_limit: parse_limit("RUNNER_JOB_PIDS_LIMIT", "512")?,
            network,
            user: std::env::var("RUNNER_JOB_USER").ok().filter(|u| !u.is_empty()),
            readonly_rootfs: optional("RUNNER_JOB_READONLY_ROOTFS", "false") == "true",
            // On by default so GitHub-authored `sudo …` steps work; opting
            // OUT requires an explicit RUNNER_SUDO_SHIM=false.
            sudo_shim: optional("RUNNER_SUDO_SHIM", "true") != "false",
        },
    };

    // Probe Docker once at startup; jobs fail cleanly if it is unavailable.
    // connect_with_defaults honors DOCKER_HOST / DOCKER_TLS_VERIFY /
    // DOCKER_CERT_PATH, so both the local socket/pipe and a remote
    // TLS-secured daemon work. Endpoint only — never certificate material —
    // is logged.
    let docker_host =
        std::env::var("DOCKER_HOST").unwrap_or_else(|_| "local socket/pipe".to_string());
    let docker = match bollard::Docker::connect_with_defaults() {
        Ok(docker) => match docker.ping().await {
            Ok(_) => {
                tracing::info!(endpoint = %docker_host, "docker engine reachable");
                Some(docker)
            }
            Err(error) => {
                tracing::warn!(endpoint = %docker_host, error = ?error, "docker engine unreachable; jobs will fail");
                None
            }
        },
        Err(error) => {
            tracing::warn!(endpoint = %docker_host, error = ?error, "docker engine unreachable; jobs will fail");
            None
        }
    };

    let mut backoff = Duration::from_secs(1);
    loop {
        let connected_at = Instant::now();
        let mut new_token = None;
        match ws::run_connection(&config, docker.clone(), &mut new_token).await {
            Ok(ws::Disconnect::Revoked) => {
                tracing::error!("this runner was revoked by the control plane; exiting");
                return Ok(());
            }
            Ok(ws::Disconnect::Lost) => {
                tracing::warn!("connection to control plane lost");
            }
            Err(error) => {
                tracing::warn!(error = ?error, "connection attempt failed");
            }
        }

        if let Some(token) = new_token {
            config.token = token.clone();
            match &token_file {
                Some(path) => match persist_token(path, &token) {
                    Ok(()) => tracing::info!(path = %path, "permanent token persisted"),
                    Err(error) => tracing::warn!(
                        path = %path,
                        error = ?error,
                        "could not persist the new permanent token; it will only be used for this process"
                    ),
                },
                None => tracing::warn!(
                    "received a permanent token but RUNNER_TOKEN_FILE is not set; it will only be used for this process"
                ),
            }
        }

        // A session that survived a while earns a fresh backoff.
        if connected_at.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        tracing::info!(delay = ?backoff, "reconnecting");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_version_carries_a_build_ref() {
        let version = runner_version();
        // `0.1.0` alone is identical in every build ever made; the ref after
        // `+` is the only part that can identify a stale deployed runner.
        let (pkg, build_ref) = version.split_once('+').expect("version must carry a build ref");
        assert_eq!(pkg, env!("CARGO_PKG_VERSION"));
        assert!(!build_ref.is_empty());
    }

    /// The outage regression: `RUNNER_TOKEN=` (set, blank) must fail at boot
    /// rather than start and loop forever on an unrecoverable 401. Uses a
    /// uniquely-named var — env is process-global and tests run in parallel.
    #[test]
    fn blank_env_token_is_an_error_not_an_empty_string() {
        const VAR: &str = "OVERUP_TEST_RUNNER_TOKEN_BLANK";
        for blank in ["", "   ", "\n", "\t \r\n"] {
            unsafe { std::env::set_var(VAR, blank) };
            let error = token_from_env(VAR)
                .expect_err("a blank token must not resolve to an empty credential")
                .to_string();
            assert!(error.contains(VAR), "error must name the variable: {error}");
        }
        unsafe { std::env::remove_var(VAR) };
    }

    /// Unset is the same failure class as blank — both mean "no credential".
    #[test]
    fn unset_env_token_is_an_error() {
        const VAR: &str = "OVERUP_TEST_RUNNER_TOKEN_UNSET";
        unsafe { std::env::remove_var(VAR) };
        assert!(token_from_env(VAR).is_err());
    }

    /// A trailing newline hashes to a different token and earns a baffling
    /// 401, so the value is trimmed on the way in.
    #[test]
    fn env_token_is_trimmed() {
        const VAR: &str = "OVERUP_TEST_RUNNER_TOKEN_TRIM";
        unsafe { std::env::set_var(VAR, "  abc123\n") };
        assert_eq!(token_from_env(VAR).unwrap(), "abc123");
        unsafe { std::env::remove_var(VAR) };
    }

    /// The blank-token guidance must point at the hosted path, since that is
    /// the misconfiguration behind a hand-started container with no token.
    #[test]
    fn blank_token_error_mentions_the_hosted_alternative() {
        const VAR: &str = "OVERUP_TEST_RUNNER_TOKEN_HOSTED_HINT";
        unsafe { std::env::set_var(VAR, "") };
        let error = token_from_env(VAR).unwrap_err().to_string();
        assert!(error.contains("RUNNER_PROVISIONER=docker"));
        assert!(error.contains("RUNNER_TOKEN_FILE"));
        unsafe { std::env::remove_var(VAR) };
    }
}
