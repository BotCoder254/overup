//! Hosted-runner provisioning: the control plane spawns runner containers
//! on its own Docker host ("create and wait" — no manual install step).
//!
//! Opt-in via RUNNER_PROVISIONER=docker (see [`crate::config`]). The
//! provisioned container receives a one-time bootstrap token in its
//! environment (single-use, exchanged for a permanent credential within
//! seconds of first connect and persisted in the container's data volume);
//! the job-payload signing key is NOT injected — it is delivered over the
//! authenticated WebSocket in hello_ack. Tokens are never logged.
//!
//! Docker access: an ordered candidate probe — the explicit
//! RUNNER_PROVISIONER_DOCKER_SOCKET endpoint, then DOCKER_HOST (+
//! DOCKER_TLS_VERIFY / DOCKER_CERT_PATH via bollard's standard resolution,
//! same as the runner crate), then the well-known local socket locations
//! including rootless Docker's. The connection is held behind a reconnect
//! loop: Docker being down never disables the feature for the process
//! lifetime, it just makes hosted runners unavailable until the daemon is
//! reachable again. Note the trust boundary: whoever controls that Docker
//! daemon controls the host — only point this at the daemon the control
//! plane itself runs on, or a TLS-secured one (never an unauthenticated
//! tcp://2375).

use anyhow::Context;
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, HostConfig, NetworkCreateRequest, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectNetworkOptions,
    ListContainersOptionsBuilder, RemoveContainerOptionsBuilder, RemoveVolumeOptions,
    StartContainerOptions, StopContainerOptionsBuilder,
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::config::RunnerProvisionerConfig;
use crate::services::runner_profiles::ResourceLimits;

pub struct RunnerProvisioner {
    /// Live daemon handle; `None` while disconnected. Populated (and
    /// re-populated after an outage) by [`Self::run_reconnect_loop`].
    docker: tokio::sync::RwLock<Option<Docker>>,
    /// Network runner containers actually join: `cfg.network` when it exists
    /// or could be created on the current connection, `bridge` as the
    /// per-connection fallback (re-evaluated on every reconnect).
    network: tokio::sync::RwLock<String>,
    /// Guards the post-connect image warm-up so rapid reconnects can't stack
    /// concurrent pulls of the same list.
    prepull_running: std::sync::atomic::AtomicBool,
    cfg: RunnerProvisionerConfig,
}

/// One `overup.managed=true` container as seen on the Docker host.
pub struct ManagedContainer {
    pub container_id: String,
    /// Parsed from the `overup.runner_id` label; `None` when the label is
    /// missing or unparseable (the reconciler removes such containers but
    /// cannot guess their data-volume name).
    pub runner_id: Option<Uuid>,
    pub running: bool,
}

/// Everything needed to spawn one hosted runner container.
pub struct ProvisionParams<'a> {
    pub runner_id: Uuid,
    pub workspace_id: Uuid,
    pub name: &'a str,
    pub labels: &'a [String],
    /// One-time bootstrap credential; goes straight into the container env,
    /// never into a response or a log line.
    pub bootstrap_token: &'a str,
    /// Profile-derived limits applied to the runner container's own cgroup
    /// AND forwarded to its job containers via RUNNER_JOB_* env. Job
    /// containers run as siblings on the host daemon — outside the runner's
    /// cgroup — so the forwarded env is what actually bounds jobs.
    pub limits: ResourceLimits,
}

/// Provisioning failure with a static category safe to persist and show to
/// users; the Docker detail stays in tracing logs only.
#[derive(Debug)]
pub enum ProvisionError {
    /// The provisioner currently has no live Docker connection (the
    /// reconnect loop will restore it once the daemon is reachable).
    DockerUnavailable,
    ImagePull(bollard::errors::Error),
    ContainerCreate(bollard::errors::Error),
    ContainerStart(bollard::errors::Error),
}

impl ProvisionError {
    pub fn category(&self) -> &'static str {
        match self {
            Self::DockerUnavailable => "docker_unavailable",
            Self::ImagePull(_) => "image_pull_failed",
            Self::ContainerCreate(_) => "container_create_failed",
            Self::ContainerStart(_) => "container_start_failed",
        }
    }

    /// Human-readable detail for tracing logs only — never persisted or
    /// sent to clients.
    pub fn detail(&self) -> String {
        match self {
            Self::DockerUnavailable => "no live docker connection".to_string(),
            Self::ImagePull(e) | Self::ContainerCreate(e) | Self::ContainerStart(e) => {
                format!("{e:?}")
            }
        }
    }
}

fn container_name(runner_id: Uuid) -> String {
    format!("overup-runner-{runner_id}")
}

fn volume_name(runner_id: Uuid) -> String {
    format!("overup-runner-{runner_id}-data")
}

/// Drain one `create_image` pull to completion (idempotent: an image already
/// present in the daemon resolves immediately).
async fn pull_image(docker: &Docker, image: &str) -> Result<(), bollard::errors::Error> {
    let mut pull = docker.create_image(
        Some(CreateImageOptionsBuilder::default().from_image(image).build()),
        None,
        None,
    );
    while let Some(progress) = pull.next().await {
        progress?;
    }
    Ok(())
}

/// How long a disconnected provisioner waits between connection attempts.
const RECONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// How often a connected provisioner pings the daemon to detect an outage.
const HEALTH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// Connect timeout (seconds) for explicit socket/pipe endpoints — matches
/// bollard's own default.
const CONNECT_TIMEOUT_SECS: u64 = 120;

/// Connect + ping one candidate; a handle that cannot answer a ping is as
/// good as no handle.
async fn ping_checked(docker: Docker) -> Result<Docker, String> {
    match docker.ping().await {
        Ok(_) => Ok(docker),
        Err(error) => Err(format!("connected but ping failed ({error})")),
    }
}

/// bollard's standard resolution: DOCKER_HOST (+ DOCKER_TLS_VERIFY /
/// DOCKER_CERT_PATH) or the platform default socket/pipe.
async fn connect_defaults() -> Result<Docker, String> {
    match Docker::connect_with_defaults() {
        Ok(docker) => ping_checked(docker).await,
        Err(error) => Err(error.to_string()),
    }
}

/// The explicit RUNNER_PROVISIONER_DOCKER_SOCKET endpoint: a unix socket
/// path (optionally unix://-prefixed) or, on Windows dev machines, a named
/// pipe. Remote TCP daemons must go through DOCKER_HOST so TLS handling
/// stays on bollard's audited path.
async fn connect_explicit(raw: &str) -> Result<Docker, String> {
    #[cfg(unix)]
    return match Docker::connect_with_unix(
        raw.strip_prefix("unix://").unwrap_or(raw),
        CONNECT_TIMEOUT_SECS,
        bollard::API_DEFAULT_VERSION,
    ) {
        Ok(docker) => ping_checked(docker).await,
        Err(error) => Err(error.to_string()),
    };
    #[cfg(windows)]
    return match Docker::connect_with_named_pipe(
        raw.strip_prefix("npipe://").unwrap_or(raw),
        CONNECT_TIMEOUT_SECS,
        bollard::API_DEFAULT_VERSION,
    ) {
        Ok(docker) => ping_checked(docker).await,
        Err(error) => Err(error.to_string()),
    };
}

/// Whether this process is itself running inside a container. Drives the
/// remediation wording: "join the docker group" advice is useless when the
/// real problem is that the host's daemon socket was never mounted into the
/// backend container — the host's /usr/bin/docker is invisible from here.
#[cfg(unix)]
fn running_in_container() -> bool {
    if std::path::Path::new("/.dockerenv").exists() {
        return true;
    }
    std::fs::read_to_string("/proc/1/cgroup")
        .map(|cgroup| {
            ["docker", "containerd", "kubepods", "libpod", "buildkit"]
                .iter()
                .any(|marker| cgroup.contains(marker))
        })
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn running_in_container() -> bool {
    false
}

/// Make an EACCES on an existing socket actionable: bollard's raw
/// "Permission denied (os error 13)" doesn't say whose permission or how to
/// grant it, and it's the exact failure a non-root backend hits once the
/// socket IS mounted but the group wasn't added.
fn annotate_permission_denied(reason: String) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("permission denied") || lower.contains("os error 13") {
        format!(
            "{reason} (socket exists but this user cannot open it — add the backend user to \
             the socket's group: usermod -aG docker, or for a containerized backend \
             --group-add $(stat -c %g /var/run/docker.sock) / compose group_add)"
        )
    } else {
        reason
    }
}

/// Well-known local socket locations, probed when neither the explicit
/// endpoint nor DOCKER_HOST produced a connection: the standard daemon
/// sockets, then rootless Docker's per-user runtime sockets.
#[cfg(unix)]
fn unix_socket_candidates() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut paths = vec![
        PathBuf::from("/var/run/docker.sock"),
        PathBuf::from("/run/docker.sock"),
    ];
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        paths.push(PathBuf::from(dir).join("docker.sock"));
    }
    // Rootless Docker under a systemd service that doesn't export
    // XDG_RUNTIME_DIR: derive the runtime dir from our own uid.
    if let Ok(meta) = std::fs::metadata("/proc/self") {
        use std::os::unix::fs::MetadataExt;
        paths.push(PathBuf::from(format!("/run/user/{}/docker.sock", meta.uid())));
    }
    paths.dedup();
    paths
}

/// Try every candidate in order; `Ok((handle, via))` on the first ping
/// success, `Err(attempts)` with one line per endpoint tried otherwise.
async fn try_connect(cfg: &RunnerProvisionerConfig) -> Result<(Docker, String), Vec<String>> {
    let mut attempts: Vec<String> = Vec::new();

    if let Some(raw) = cfg.docker_socket.as_deref() {
        let label = format!("RUNNER_PROVISIONER_DOCKER_SOCKET={raw}");
        match connect_explicit(raw).await {
            Ok(docker) => return Ok((docker, label)),
            Err(reason) => {
                attempts.push(format!("{label}: {}", annotate_permission_denied(reason)));
            }
        }
    }

    if let Ok(host) = std::env::var("DOCKER_HOST")
        && !host.is_empty()
    {
        let label = format!("DOCKER_HOST={host}");
        match connect_defaults().await {
            Ok(docker) => return Ok((docker, label)),
            Err(reason) => attempts.push(format!("{label}: {reason}")),
        }
    }

    #[cfg(unix)]
    for path in unix_socket_candidates() {
        let label = path.display().to_string();
        if !path.exists() {
            attempts.push(format!("{label}: socket not present"));
            continue;
        }
        match connect_explicit(&label).await {
            Ok(docker) => return Ok((docker, label)),
            Err(reason) => {
                attempts.push(format!("{label}: {}", annotate_permission_denied(reason)));
            }
        }
    }

    // Windows dev fallback: the platform default named pipe.
    #[cfg(windows)]
    {
        let label = "local Docker named pipe".to_string();
        match connect_defaults().await {
            Ok(docker) => return Ok((docker, label)),
            Err(reason) => attempts.push(format!("{label}: {reason}")),
        }
    }

    Err(attempts)
}

impl RunnerProvisioner {
    /// Construct without connecting: liveness is owned by
    /// [`Self::run_reconnect_loop`], so a Docker outage at boot (or any time
    /// after) degrades cleanly to "hosted runners unavailable" instead of
    /// disabling the feature for the process lifetime.
    pub fn new(cfg: RunnerProvisionerConfig) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            docker: tokio::sync::RwLock::new(None),
            network: tokio::sync::RwLock::new(cfg.network.clone()),
            prepull_running: std::sync::atomic::AtomicBool::new(false),
            cfg,
        })
    }

    /// Whether a live Docker connection currently exists — the signal behind
    /// `hostedAvailable` and every janitor/auto-provision gate.
    pub async fn available(&self) -> bool {
        self.docker.read().await.is_some()
    }

    /// Clone out the current handle (bollard's `Docker` is a cheap clone
    /// around a shared transport).
    async fn handle(&self) -> Option<Docker> {
        self.docker.read().await.clone()
    }

    /// Owns the connection lifecycle: while disconnected, probe the
    /// candidates every [`RECONNECT_INTERVAL`]; while connected, ping every
    /// [`HEALTH_INTERVAL`] and drop the handle on failure. Logging is
    /// edge-triggered — one warn per outage, one info per recovery.
    pub async fn run_reconnect_loop(self: std::sync::Arc<Self>) {
        let mut warned = false;
        loop {
            if let Some(docker) = self.handle().await {
                tokio::time::sleep(HEALTH_INTERVAL).await;
                if let Err(error) = docker.ping().await {
                    *self.docker.write().await = None;
                    tracing::warn!(
                        error = %error,
                        "hosted-runner provisioner: docker connection lost — reconnecting"
                    );
                }
                continue;
            }

            match try_connect(&self.cfg).await {
                Ok((docker, via)) => {
                    // Ensure the dedicated runner network exists (a
                    // user-defined bridge isolates runner containers from
                    // unrelated ones on the default bridge). Failure degrades
                    // to the default bridge — it never disables the feature.
                    let mut network = self.cfg.network.clone();
                    if network != "bridge" && !Self::ensure_network(&docker, &network).await {
                        network = "bridge".to_string();
                    }
                    *self.network.write().await = network.clone();
                    *self.docker.write().await = Some(docker.clone());
                    warned = false;
                    tracing::info!(
                        via = %via,
                        image = %self.cfg.image,
                        network = %network,
                        "hosted-runner provisioner connected"
                    );
                    self.preflight_overup_url().await;
                    std::sync::Arc::clone(&self).spawn_prepull(docker);
                }
                Err(attempts) => {
                    if !warned {
                        warned = true;
                        // Docker being installed on the HOST doesn't help a
                        // containerized backend — tailor the fix to where we
                        // actually run.
                        let remediation = if running_in_container() {
                            "the backend itself runs inside a container, so the host's Docker \
                             daemon (and /usr/bin/docker) is not visible here. Bind-mount the \
                             socket into THIS container (-v \
                             /var/run/docker.sock:/var/run/docker.sock — in Dokploy/Portainer \
                             etc. add a Mounts entry) AND grant the non-root backend user the \
                             socket's group (--group-add $(stat -c %g /var/run/docker.sock) or \
                             compose group_add), then redeploy; alternatively point DOCKER_HOST \
                             at a TLS-secured remote daemon (+ DOCKER_TLS_VERIFY / \
                             DOCKER_CERT_PATH)"
                        } else {
                            "ensure the docker daemon is running (systemctl status docker) and \
                             the backend user is in the `docker` group; for rootless Docker set \
                             RUNNER_PROVISIONER_DOCKER_SOCKET=$XDG_RUNTIME_DIR/docker.sock; a \
                             remote daemon needs DOCKER_HOST (+ DOCKER_TLS_VERIFY / \
                             DOCKER_CERT_PATH)"
                        };
                        tracing::warn!(
                            attempts = %attempts.join("; "),
                            remediation = %remediation,
                            "hosted-runner provisioner: docker unreachable — retrying every 30s"
                        );
                    }
                    tokio::time::sleep(RECONNECT_INTERVAL).await;
                }
            }
        }
    }

    /// Preflight the connect-back URL after every successful (re)connect.
    /// Misconfiguration here is otherwise silent: every hosted runner
    /// provisions fine but never connects. Warnings only — the server-side
    /// view can't fully prove what a container will see, so never fatal.
    async fn preflight_overup_url(&self) {
        let url = &self.cfg.overup_url;
        if url.contains("localhost") || url.contains("127.0.0.1") || url.contains("[::1]") {
            tracing::warn!(
                overup_url = %url,
                "RUNNER_PROVISIONER_OVERUP_URL points at localhost — inside a container that is \
                 the container itself, not this server; hosted runners will provision but never \
                 connect (use the server's LAN/public URL or host.docker.internal)"
            );
            return;
        }
        let probe = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .ok();
        if let Some(client) = probe {
            match client.get(format!("{url}/healthz")).send().await {
                Ok(resp) if resp.status().is_success() => {}
                Ok(resp) => tracing::warn!(
                    overup_url = %url,
                    status = %resp.status(),
                    "RUNNER_PROVISIONER_OVERUP_URL healthz probe returned a non-success status"
                ),
                Err(error) => tracing::warn!(
                    overup_url = %url,
                    error = %error,
                    "RUNNER_PROVISIONER_OVERUP_URL healthz probe failed — hosted runner \
                     containers may not be able to reach the control plane on this URL"
                ),
            }
        }
    }

    /// Make sure the named runner network exists. Returns `false` when it
    /// could neither be found nor created.
    async fn ensure_network(docker: &Docker, name: &str) -> bool {
        if docker
            .inspect_network(name, None::<InspectNetworkOptions>)
            .await
            .is_ok()
        {
            return true;
        }
        match docker
            .create_network(NetworkCreateRequest {
                name: name.to_string(),
                driver: Some("bridge".to_string()),
                labels: Some(
                    [("overup.managed".to_string(), "true".to_string())]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            })
            .await
        {
            Ok(_) => {
                tracing::info!(network = %name, "created hosted-runner network");
                true
            }
            // Lost a create race: the network exists now, which is all we need.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 409, ..
            }) => true,
            Err(error) => {
                tracing::warn!(
                    network = %name,
                    error = ?error,
                    "failed to create hosted-runner network; falling back to the default bridge"
                );
                false
            }
        }
    }

    /// Pull the runner image if missing (idempotent: a present image resolves
    /// immediately). Kept separate from [`Self::provision`] so the caller can
    /// finish the potentially minutes-long pull BEFORE minting any bootstrap
    /// credential — no plaintext token ever waits on this.
    pub async fn ensure_image(&self) -> Result<(), ProvisionError> {
        let Some(docker) = self.handle().await else {
            return Err(ProvisionError::DockerUnavailable);
        };
        pull_image(&docker, &self.cfg.image)
            .await
            .map_err(ProvisionError::ImagePull)
    }

    /// Warm the daemon's image cache after a successful (re)connect: the
    /// runner image plus every RUNNER_PREPULL_IMAGES entry, so the runner's
    /// `pulling_image` stage (and the first hosted-runner create) resolves
    /// from local layers. Spawned — a multi-minute pull must never block the
    /// health-ping loop — and warn-only: a bad image name never affects
    /// hosted-runner availability. In the default setup the runner containers
    /// share this daemon via the socket mount, so warming here warms job
    /// execution too; with RUNNER_PROVISIONER_DOCKER_HOST pointed at a
    /// different daemon this only helps runner-container creation.
    fn spawn_prepull(self: std::sync::Arc<Self>, docker: Docker) {
        use std::sync::atomic::Ordering;
        if self.prepull_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let this = self;
        tokio::spawn(async move {
            let mut images: Vec<&str> = vec![this.cfg.image.as_str()];
            for image in &this.cfg.prepull_images {
                if !images.contains(&image.as_str()) {
                    images.push(image);
                }
            }
            for image in images {
                let started = std::time::Instant::now();
                match pull_image(&docker, image).await {
                    Ok(()) => tracing::info!(
                        image = %image,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "pre-pulled image into the docker daemon"
                    ),
                    Err(error) => tracing::warn!(
                        image = %image,
                        error = ?error,
                        "image pre-pull failed — jobs needing it will pull on demand"
                    ),
                }
            }
            this.prepull_running.store(false, Ordering::SeqCst);
        });
    }

    /// Create and start one runner container. The image must already be
    /// present ([`Self::ensure_image`]). Returns the container id.
    pub async fn provision(&self, params: ProvisionParams<'_>) -> Result<String, ProvisionError> {
        let Some(docker) = self.handle().await else {
            return Err(ProvisionError::DockerUnavailable);
        };
        let ProvisionParams {
            runner_id,
            workspace_id,
            name,
            labels,
            bootstrap_token,
            limits,
        } = params;

        let mut env = vec![
            format!("OVERUP_URL={}", self.cfg.overup_url),
            format!("RUNNER_TOKEN={bootstrap_token}"),
            // The permanent token from the bootstrap exchange survives
            // restarts in the container's data volume.
            "RUNNER_TOKEN_FILE=/data/token".to_string(),
            format!("RUNNER_NAME={name}"),
            format!("RUNNER_LABELS={}", labels.join(",")),
            // Forward the profile to job containers — they run as siblings
            // on the host daemon, outside this container's cgroup, so these
            // env knobs are what actually bounds them.
            format!("RUNNER_JOB_MEMORY_BYTES={}", limits.memory_bytes),
            format!("RUNNER_JOB_NANO_CPUS={}", limits.nano_cpus),
            format!("RUNNER_JOB_PIDS_LIMIT={}", limits.pids_limit),
        ];
        let mut binds = vec![format!("{}:/data", volume_name(runner_id))];
        match &self.cfg.runner_docker_host {
            // The runner needs a Docker daemon of its own to execute jobs.
            Some(docker_host) => env.push(format!("DOCKER_HOST={docker_host}")),
            // Fallback: share the host daemon via the socket. Root-equivalent
            // on the host — documented in .env.example and deploy docs.
            None => binds.push("/var/run/docker.sock:/var/run/docker.sock".to_string()),
        }

        // Hardened like job containers, with one deliberate difference: no
        // cap-drop — the runner drives a Docker daemon and the image already
        // runs as a non-root user; job-container caps are dropped by the
        // runner crate itself.
        let host_config = HostConfig {
            binds: Some(binds),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                ..Default::default()
            }),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            memory: Some(limits.memory_bytes),
            nano_cpus: Some(limits.nano_cpus),
            pids_limit: Some(limits.pids_limit),
            network_mode: Some(self.network.read().await.clone()),
            ..Default::default()
        };
        let body = ContainerCreateBody {
            image: Some(self.cfg.image.clone()),
            env: Some(env),
            labels: Some(
                [
                    ("overup.managed".to_string(), "true".to_string()),
                    ("overup.runner_id".to_string(), runner_id.to_string()),
                    ("overup.workspace_id".to_string(), workspace_id.to_string()),
                ]
                .into_iter()
                .collect(),
            ),
            host_config: Some(host_config),
            ..Default::default()
        };

        let container = docker
            .create_container(
                Some(
                    CreateContainerOptionsBuilder::default()
                        .name(&container_name(runner_id))
                        .build(),
                ),
                body,
            )
            .await
            .map_err(ProvisionError::ContainerCreate)?
            .id;
        if let Err(error) = docker
            .start_container(&container, None::<StartContainerOptions>)
            .await
        {
            // Never leave a created-but-unstartable container behind.
            let _ = docker
                .remove_container(
                    &container,
                    Some(RemoveContainerOptionsBuilder::default().force(true).build()),
                )
                .await;
            return Err(ProvisionError::ContainerStart(error));
        }

        tracing::info!(%runner_id, container = %&container[..container.len().min(12)], "hosted runner provisioned");
        Ok(container)
    }

    /// Best-effort cleanup after an abandoned provision attempt (e.g. the
    /// caller's timeout cancelled the future mid-flight, so no container id
    /// was ever returned). Container names are deterministic per runner, so
    /// any partially-created container/volume can still be found and removed.
    pub async fn cleanup_partial(&self, runner_id: Uuid) {
        let Some(docker) = self.handle().await else {
            return;
        };
        let _ = docker
            .remove_container(
                &container_name(runner_id),
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        let _ = docker
            .remove_volume(&volume_name(runner_id), None::<RemoveVolumeOptions>)
            .await;
    }

    /// Every container this provisioner ever created (running or not),
    /// identified by the `overup.managed=true` label — the janitor's
    /// reconciliation input.
    pub async fn list_managed_containers(&self) -> anyhow::Result<Vec<ManagedContainer>> {
        let Some(docker) = self.handle().await else {
            anyhow::bail!("docker daemon unavailable");
        };
        let containers = docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(
                        &[("label", vec!["overup.managed=true"])]
                            .into_iter()
                            .collect(),
                    )
                    .build(),
            ))
            .await
            .context("listing managed runner containers failed")?;

        Ok(containers
            .into_iter()
            .filter_map(|c| {
                let container_id = c.id?;
                let runner_id = c
                    .labels
                    .as_ref()
                    .and_then(|labels| labels.get("overup.runner_id"))
                    .and_then(|raw| Uuid::parse_str(raw).ok());
                let running = c.state.is_some_and(|s| {
                    matches!(s, bollard::models::ContainerSummaryStateEnum::RUNNING)
                });
                Some(ManagedContainer {
                    container_id,
                    runner_id,
                    running,
                })
            })
            .collect())
    }

    /// Force-remove one container without touching any volume — for orphans
    /// whose `overup.runner_id` label is missing/unparseable, where the
    /// data-volume name cannot be derived.
    pub async fn remove_container(&self, container_id: &str) -> anyhow::Result<()> {
        let Some(docker) = self.handle().await else {
            anyhow::bail!("docker daemon unavailable");
        };
        docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .context("removing managed runner container failed")?;
        Ok(())
    }

    /// Restart a stopped managed container (daemon restarts / manual stops;
    /// crashes are already covered by the unless-stopped restart policy).
    pub async fn start_container(&self, container_id: &str) -> anyhow::Result<()> {
        let Some(docker) = self.handle().await else {
            anyhow::bail!("docker daemon unavailable");
        };
        docker
            .start_container(container_id, None::<StartContainerOptions>)
            .await
            .context("starting managed runner container failed")?;
        Ok(())
    }

    /// Best-effort teardown: stop (10 s grace), force-remove the container,
    /// then its data volume.
    pub async fn deprovision(&self, runner_id: Uuid, container_id: &str) -> anyhow::Result<()> {
        let Some(docker) = self.handle().await else {
            anyhow::bail!("docker daemon unavailable");
        };
        let _ = docker
            .stop_container(
                container_id,
                Some(StopContainerOptionsBuilder::default().t(10).build()),
            )
            .await;
        docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .context("runner container removal failed")?;
        if let Err(error) = docker
            .remove_volume(&volume_name(runner_id), None::<RemoveVolumeOptions>)
            .await
        {
            tracing::warn!(%runner_id, error = ?error, "failed to remove hosted runner data volume");
        }
        tracing::info!(%runner_id, "hosted runner deprovisioned");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::annotate_permission_denied;

    #[test]
    fn permission_denied_gets_group_remediation() {
        let annotated =
            annotate_permission_denied("Permission denied (os error 13)".to_string());
        assert!(annotated.contains("group"));
        assert!(annotated.starts_with("Permission denied (os error 13)"));
    }

    #[test]
    fn other_errors_pass_through_untouched() {
        let reason = "connected but ping failed (timeout)".to_string();
        assert_eq!(annotate_permission_denied(reason.clone()), reason);
    }
}
