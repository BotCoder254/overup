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
//!   RUNNER_TOKEN                registration token (shown once at creation)
//!   RUNNER_JOB_SIGNING_KEY      shared HMAC key — must match the control plane
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
//!
//! Docker connection: honors DOCKER_HOST (unix://, npipe://, tcp://…) plus
//! DOCKER_TLS_VERIFY=1 and DOCKER_CERT_PATH (ca.pem/cert.pem/key.pem) for
//! remote TLS-secured daemons; defaults to the local socket / named pipe.

mod artifacts;
mod executor;
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
    pub signing_key: Vec<u8>,
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

fn required(key: &str) -> anyhow::Result<String> {
    std::env::var(key).with_context(|| format!("missing required environment variable {key}"))
}

fn optional(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let signing_key = required("RUNNER_JOB_SIGNING_KEY")?.into_bytes();
    if signing_key.len() < 32 {
        anyhow::bail!("RUNNER_JOB_SIGNING_KEY must be at least 32 bytes");
    }

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

    let config = RunnerConfig {
        server_url: optional("OVERUP_URL", "http://localhost:8080")
            .trim_end_matches('/')
            .to_string(),
        token: required("RUNNER_TOKEN")?,
        name: optional("RUNNER_NAME", "overup-runner"),
        labels: optional("RUNNER_LABELS", "self-hosted")
            .split(',')
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
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
        match ws::run_connection(&config, docker.clone()).await {
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
        // A session that survived a while earns a fresh backoff.
        if connected_at.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        tracing::info!(delay = ?backoff, "reconnecting");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}
