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
use bollard::models::{ContainerCreateBody, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptions, StartContainerOptions, StopContainerOptionsBuilder,
};
use futures_util::StreamExt;
use uuid::Uuid;

use crate::config::RunnerProvisionerConfig;

pub struct RunnerProvisioner {
    docker: Docker,
    cfg: RunnerProvisionerConfig,
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
        let cfg = cfg?;
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

        tracing::info!(image = %cfg.image, "hosted-runner provisioner ready");
        Some(std::sync::Arc::new(Self { docker, cfg }))
    }

    /// Pull the runner image if missing, then create and start one runner
    /// container. Returns the container id.
    pub async fn provision(&self, params: ProvisionParams<'_>) -> Result<String, ProvisionError> {
        let ProvisionParams {
            runner_id,
            workspace_id,
            name,
            labels,
            bootstrap_token,
        } = params;

        // Pull is idempotent: a present image resolves immediately.
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

        let mut env = vec![
            format!("OVERUP_URL={}", self.cfg.overup_url),
            format!("RUNNER_TOKEN={bootstrap_token}"),
            // The permanent token from the bootstrap exchange survives
            // restarts in the container's data volume.
            "RUNNER_TOKEN_FILE=/data/token".to_string(),
            format!("RUNNER_NAME={name}"),
            format!("RUNNER_LABELS={}", labels.join(",")),
        ];
        let mut binds = vec![format!("{}:/data", volume_name(runner_id))];
        match &self.cfg.runner_docker_host {
            // The runner needs a Docker daemon of its own to execute jobs.
            Some(docker_host) => env.push(format!("DOCKER_HOST={docker_host}")),
            // Fallback: share the host daemon via the socket. Root-equivalent
            // on the host — documented in .env.example and deploy docs.
            None => binds.push("/var/run/docker.sock:/var/run/docker.sock".to_string()),
        }

        let host_config = HostConfig {
            binds: Some(binds),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                ..Default::default()
            }),
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
