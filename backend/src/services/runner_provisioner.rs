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
//! Docker access: `connect_with_defaults` honors DOCKER_HOST /
//! DOCKER_TLS_VERIFY / DOCKER_CERT_PATH, same as the runner crate. Note the
//! trust boundary: whoever controls that Docker daemon controls the host —
//! only point this at the daemon the control plane itself runs on, or a
//! TLS-secured one (never an unauthenticated tcp://2375).

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
    docker: Docker,
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
    ImagePull(bollard::errors::Error),
    ContainerCreate(bollard::errors::Error),
    ContainerStart(bollard::errors::Error),
}

impl ProvisionError {
    pub fn category(&self) -> &'static str {
        match self {
            Self::ImagePull(_) => "image_pull_failed",
            Self::ContainerCreate(_) => "container_create_failed",
            Self::ContainerStart(_) => "container_start_failed",
        }
    }

    pub fn detail(&self) -> &bollard::errors::Error {
        match self {
            Self::ImagePull(e) | Self::ContainerCreate(e) | Self::ContainerStart(e) => e,
        }
    }
}

fn container_name(runner_id: Uuid) -> String {
    format!("overup-runner-{runner_id}")
}

fn volume_name(runner_id: Uuid) -> String {
    format!("overup-runner-{runner_id}-data")
}

impl RunnerProvisioner {
    /// Connect and ping at startup. Degrades cleanly (like R2): when Docker
    /// is unreachable the feature is simply unavailable, never a crash.
    pub async fn init(cfg: Option<RunnerProvisionerConfig>) -> Option<std::sync::Arc<Self>> {
        let mut cfg = cfg?;
        let docker = match Docker::connect_with_defaults() {
            Ok(docker) => docker,
            Err(error) => {
                tracing::warn!(error = ?error, "hosted-runner provisioner disabled: docker connection failed");
                return None;
            }
        };
        if let Err(error) = docker.ping().await {
            tracing::warn!(error = ?error, "hosted-runner provisioner disabled: docker unreachable");
            return None;
        }

        // Preflight the connect-back URL. Misconfiguration here is otherwise
        // silent: every hosted runner provisions fine but never connects.
        // Warnings only — the server-side view can't fully prove what a
        // container will see, so this is never fatal.
        let url = &cfg.overup_url;
        if url.contains("localhost") || url.contains("127.0.0.1") || url.contains("[::1]") {
            tracing::warn!(
                overup_url = %url,
                "RUNNER_PROVISIONER_OVERUP_URL points at localhost — inside a container that is \
                 the container itself, not this server; hosted runners will provision but never \
                 connect (use the server's LAN/public URL or host.docker.internal)"
            );
        } else {
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

        // Ensure the dedicated runner network exists (a user-defined bridge
        // isolates runner containers from unrelated ones on the default
        // bridge). Failure degrades to the default bridge with a warning —
        // it never disables the whole feature.
        if cfg.network != "bridge" && !Self::ensure_network(&docker, &cfg.network).await {
            cfg.network = "bridge".to_string();
        }

        tracing::info!(image = %cfg.image, network = %cfg.network, "hosted-runner provisioner ready");
        Some(std::sync::Arc::new(Self { docker, cfg }))
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
        let mut pull = self.docker.create_image(
            Some(
                CreateImageOptionsBuilder::default()
                    .from_image(&self.cfg.image)
                    .build(),
            ),
            None,
            None,
        );
        while let Some(progress) = pull.next().await {
            progress.map_err(ProvisionError::ImagePull)?;
        }
        Ok(())
    }

    /// Create and start one runner container. The image must already be
    /// present ([`Self::ensure_image`]). Returns the container id.
    pub async fn provision(&self, params: ProvisionParams<'_>) -> Result<String, ProvisionError> {
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
            network_mode: Some(self.cfg.network.clone()),
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

        let container = self
            .docker
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
        if let Err(error) = self
            .docker
            .start_container(&container, None::<StartContainerOptions>)
            .await
        {
            // Never leave a created-but-unstartable container behind.
            let _ = self
                .docker
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
        let _ = self
            .docker
            .remove_container(
                &container_name(runner_id),
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        let _ = self
            .docker
            .remove_volume(&volume_name(runner_id), None::<RemoveVolumeOptions>)
            .await;
    }

    /// Every container this provisioner ever created (running or not),
    /// identified by the `overup.managed=true` label — the janitor's
    /// reconciliation input.
    pub async fn list_managed_containers(&self) -> anyhow::Result<Vec<ManagedContainer>> {
        let containers = self
            .docker
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
        self.docker
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
        self.docker
            .start_container(container_id, None::<StartContainerOptions>)
            .await
            .context("starting managed runner container failed")?;
        Ok(())
    }

    /// Best-effort teardown: stop (10 s grace), force-remove the container,
    /// then its data volume.
    pub async fn deprovision(&self, runner_id: Uuid, container_id: &str) -> anyhow::Result<()> {
        let _ = self
            .docker
            .stop_container(
                container_id,
                Some(StopContainerOptionsBuilder::default().t(10).build()),
            )
            .await;
        self.docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await
            .context("runner container removal failed")?;
        if let Err(error) = self
            .docker
            .remove_volume(&volume_name(runner_id), None::<RemoveVolumeOptions>)
            .await
        {
            tracing::warn!(%runner_id, error = ?error, "failed to remove hosted runner data volume");
        }
        tracing::info!(%runner_id, "hosted runner deprovisioned");
        Ok(())
    }
}
