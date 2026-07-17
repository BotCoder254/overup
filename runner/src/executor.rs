//! Job execution via the Docker Engine API.
//!
//! One job at a time: download and repackage the repository tarball into a
//! path-traversal-safe tar (GitHub's top-level dir stripped, symlink/hardlink
//! entries dropped), pull the container image, start one hardened keep-alive
//! container (no-new-privileges always; capabilities dropped, memory/CPU/
//! pids limits, network mode, optional non-root user and read-only rootfs
//! per config) with a daemon-managed anonymous volume at /workspace, stream
//! the source INTO that volume via the Docker archive API (`docker cp`), run
//! each step as a `docker exec` (`<shell> -c <script>`), stream stdout/stderr
//! chunks back with monotonic sequence numbers, sample container resource
//! stats for the Performance tab, pull artifacts back OUT of
//! `.overup/artifacts/` via the archive API and upload them, then remove the
//! container (with its anonymous volume) and per-job network. Cancellation
//! and the job timeout kill the container immediately.
//!
//! Source and artifacts deliberately travel over the daemon's archive API
//! rather than a host bind mount: when the runner itself runs in a container
//! sharing the host's Docker socket (hosted runners), a bind mount of a
//! runner-local path resolves against the HOST filesystem in the sibling job
//! container and comes up empty. The archive API is daemon-agnostic and works
//! identically for on-host, containerized, and remote-`DOCKER_HOST` runners.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use bollard::Docker;
use bollard::models::{ContainerCreateBody, ExecConfig, HostConfig, NetworkCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, DownloadFromContainerOptionsBuilder,
    KillContainerOptionsBuilder, RemoveContainerOptionsBuilder, StartContainerOptions,
    StatsOptionsBuilder, UploadToContainerOptionsBuilder,
};
use futures_util::StreamExt;
use protocol::{JobConclusion, JobMetrics, JobPayload, JobStep, LogStream, RunnerMsg};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::artifacts::{self, GrantWaiters};
use crate::{CurrentJob, JobIsolation, JobNetwork};

/// Repository tarballs beyond this are refused.
const MAX_TARBALL_BYTES: u64 = 1024 * 1024 * 1024;
/// The artifacts tar streamed back out of the container is buffered to a temp
/// file; beyond this it is abandoned (best-effort, never fails the job).
const MAX_ARTIFACT_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Absolute container path the repository source is unpacked into and steps
/// run from.
const WORKSPACE_DIR: &str = "/workspace";
/// Where a job leaves artifacts, relative to the workspace.
const ARTIFACTS_SUBDIR: &str = ".overup/artifacts";
/// Log chunks are split to stay comfortably under the server frame cap.
const LOG_CHUNK_BYTES: usize = 32 * 1024;
/// Where the `sudo` shim is installed. Precedes /usr/bin in the default
/// Debian/Ubuntu PATH, so it shadows the real setuid binary.
const SUDO_SHIM_DIR: &str = "/usr/local/bin";
/// The `sudo` shim installed into root job containers.
///
/// Job containers already run as uid 0, and no-new-privileges makes the
/// kernel ignore the setuid bit — so the real `sudo` can never work here,
/// while the privilege it would grant is already held. Workflows carry
/// `sudo` because GitHub's hosted runners execute as a non-root user; this
/// strips the options and runs the command as-is. It escalates nothing.
const SUDO_SHIM: &str = r#"#!/bin/sh
# Installed by the overup runner. This container already runs as root and
# real sudo cannot work under no-new-privileges (the kernel ignores the
# setuid bit), so strip sudo's options and exec the command directly.
while [ $# -gt 0 ]; do
  case "$1" in
    --) shift; break ;;
    # Options taking a separate argument. `-h` is deliberately absent: sudo
    # overloads it as both --help and --host=host, so its arity is ambiguous.
    -u|-g|-p|-C|-D|-R|-T)
      if [ $# -ge 2 ]; then shift 2; else shift; fi ;;
    -*) shift ;;
    *) break ;;
  esac
done
# Nothing left to run (`sudo -s`, `sudo -u` with no command). Never start a
# shell here: with stdin open it would block until the job times out.
[ $# -eq 0 ] && exit 0
exec env "$@"
"#;

/// Sequenced log emitter for one job. Carries the current section
/// attribution (execution phase + plan step index) so every chunk tells the
/// server which collapsible section it belongs to.
struct JobLog {
    out: mpsc::Sender<RunnerMsg>,
    job_id: Uuid,
    seq: u64,
    phase: Option<&'static str>,
    step: Option<u32>,
}

impl JobLog {
    /// Set the section every following chunk is attributed to. Phases come
    /// from protocol::LOG_PHASES; the server re-validates regardless.
    fn section(&mut self, phase: Option<&'static str>, step: Option<u32>) {
        self.phase = phase;
        self.step = step;
    }

    async fn emit(&mut self, stream: LogStream, text: &str) {
        let mut rest = text;
        while !rest.is_empty() {
            let mut cut = rest.len().min(LOG_CHUNK_BYTES);
            while cut < rest.len() && !rest.is_char_boundary(cut) {
                cut += 1;
            }
            let (piece, tail) = rest.split_at(cut);
            rest = tail;
            self.seq += 1;
            // Backpressure: a slow connection slows the job's output, never
            // unbounded memory.
            let _ = self
                .out
                .send(RunnerMsg::Log {
                    job_id: self.job_id,
                    seq: self.seq,
                    stream,
                    text: piece.to_string(),
                    step: self.step,
                    phase: self.phase.map(str::to_string),
                })
                .await;
        }
    }

    async fn system(&mut self, text: &str) {
        self.emit(LogStream::System, text).await;
    }
}

async fn stage(out: &mpsc::Sender<RunnerMsg>, job_id: Uuid, stage: &str) {
    stage_with(out, job_id, stage, None, None).await;
}

async fn stage_with(
    out: &mpsc::Sender<RunnerMsg>,
    job_id: Uuid,
    stage: &str,
    detail: Option<String>,
    step: Option<protocol::StepProgress>,
) {
    let _ = out
        .send(RunnerMsg::JobStage {
            job_id,
            stage: stage.to_string(),
            detail,
            step,
        })
        .await;
}

/// Structured step progress: rides the job_stage channel with the fixed
/// "running" stage so pre-step-progress servers treat it as a harmless
/// idempotent stage repeat.
async fn step_progress(
    out: &mpsc::Sender<RunnerMsg>,
    job_id: Uuid,
    index: u32,
    total: u32,
    status: &str,
    exit_code: Option<i32>,
) {
    stage_with(
        out,
        job_id,
        "running",
        None,
        Some(protocol::StepProgress {
            index,
            total,
            status: status.to_string(),
            exit_code,
        }),
    )
    .await;
}

enum StepOutcome {
    Done,
    Failed(Option<i32>),
    Cancelled,
    TimedOut,
}

pub async fn run_job(
    docker: Option<Docker>,
    payload: JobPayload,
    out: mpsc::Sender<RunnerMsg>,
    mut cancel: watch::Receiver<bool>,
    grants: GrantWaiters,
    isolation: JobIsolation,
    current: CurrentJob,
) {
    let job_id = payload.job_id;
    let mut log = JobLog {
        out: out.clone(),
        job_id,
        seq: 0,
        phase: None,
        step: None,
    };
    let mut metrics = JobMetrics::default();
    let started = Instant::now();

    let (conclusion, exit_code, error_category) = execute(
        docker,
        &payload,
        &out,
        &mut log,
        &mut cancel,
        &grants,
        &isolation,
        &mut metrics,
    )
    .await;
    metrics.exec_ms = Some(started.elapsed().as_millis() as u64);

    let _ = out
        .send(RunnerMsg::JobResult {
            job_id,
            conclusion,
            exit_code,
            error_category,
            metrics,
        })
        .await;

    // Free the slot: the runner is idle again.
    current.lock().unwrap().take();
    tracing::info!(%job_id, "job finished");
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    docker: Option<Docker>,
    payload: &JobPayload,
    out: &mpsc::Sender<RunnerMsg>,
    log: &mut JobLog,
    cancel: &mut watch::Receiver<bool>,
    grants: &GrantWaiters,
    isolation: &JobIsolation,
    metrics: &mut JobMetrics,
) -> (JobConclusion, Option<i32>, Option<String>) {
    let job_id = payload.job_id;
    let fail = |category: &str| {
        (
            JobConclusion::Failure,
            None,
            Some(category.to_string()),
        )
    };

    let Some(docker) = docker else {
        log.system("Docker is not available on this runner; job cannot execute")
            .await;
        return fail("container_error");
    };

    let http = match reqwest::Client::builder()
        .user_agent("overup-runner")
        .timeout(Duration::from_secs(600))
        .build()
    {
        Ok(client) => client,
        Err(_) => return fail("internal"),
    };

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(payload.timeout_seconds.max(60));

    // --- checkout (download + repackage) ------------------------------------
    // The source is uploaded INTO the container after it starts (see below) —
    // it can't be bind-mounted, because a sibling job container resolves a
    // runner-local path against the host filesystem. Download and repackage
    // now so a fetch failure fails fast before we pull an image.
    log.section(Some("checkout"), None);
    let source_tar = if let Some(checkout) = &payload.checkout {
        log.system("downloading repository archive").await;
        match fetch_and_repackage(&http, checkout).await {
            Ok(tar) => Some(tar),
            Err(error) => {
                // Error text never contains the token (it travels in a header).
                log.system(&format!("checkout failed: {error:#}")).await;
                return fail("checkout_failed");
            }
        }
    } else {
        log.system("no checkout credentials; starting with an empty workspace")
            .await;
        None
    };

    // --- image pull ---------------------------------------------------------
    log.section(Some("image_pull"), None);
    stage(out, job_id, "pulling_image").await;
    log.system(&format!("pulling image {}", payload.image)).await;
    let pull_started = Instant::now();
    let mut pull = docker.create_image(
        Some(
            CreateImageOptionsBuilder::default()
                .from_image(&payload.image)
                .build(),
        ),
        None,
        None,
    );
    // Per-layer byte counts aggregated into one throttled progress line, so
    // a large first pull reads as live progress in the streaming log rather
    // than minutes of silence that look like a hang.
    let mut layer_progress: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    let mut last_progress_log = Instant::now();
    let mut pull_error: Option<String> = None;
    while let Some(progress) = pull.next().await {
        if *cancel.borrow() {
            return (JobConclusion::Cancelled, None, None);
        }
        match progress {
            Err(error) => {
                pull_error = Some(error.to_string());
                break;
            }
            Ok(info) => {
                // bollard maps `errorDetail` frames with a message to stream
                // errors; a message-less one would otherwise slip through.
                if let Some(detail) = &info.error_detail {
                    pull_error = Some(
                        detail
                            .message
                            .clone()
                            .unwrap_or_else(|| "unknown pull error".to_string()),
                    );
                    break;
                }
                if let (Some(id), Some(detail)) = (&info.id, &info.progress_detail)
                    && let (Some(current), Some(total)) = (detail.current, detail.total)
                    && total > 0
                {
                    layer_progress.insert(id.clone(), (current.min(total), total));
                }
                if last_progress_log.elapsed() >= Duration::from_secs(2)
                    && !layer_progress.is_empty()
                {
                    let (done, total) = layer_progress
                        .values()
                        .fold((0i64, 0i64), |(d, t), (c, tot)| (d + c, t + tot));
                    if total > 0 {
                        log.system(&format!(
                            "pulling image: {}% ({} / {})",
                            done * 100 / total,
                            format_mib(done),
                            format_mib(total),
                        ))
                        .await;
                        last_progress_log = Instant::now();
                    }
                }
            }
        }
    }
    if let Some(error) = pull_error {
        // Pulls fail transiently (registry hiccups, daemon layer-extraction
        // errors) even when a usable copy of the image is already on the
        // daemon — fall back to it rather than failing the job.
        if docker.inspect_image(&payload.image).await.is_ok() {
            log.system(&format!(
                "image pull failed ({error}); using locally cached image"
            ))
            .await;
        } else {
            log.system(&format!("image pull failed: {error}")).await;
            return fail("image_pull_failed");
        }
    }
    metrics.image_pull_ms = Some(pull_started.elapsed().as_millis() as u64);
    log.system(&format!(
        "image ready in {:.1}s",
        pull_started.elapsed().as_secs_f64()
    ))
    .await;

    // --- container ----------------------------------------------------------
    log.section(Some("container"), None);
    stage(out, job_id, "starting_container").await;
    let env: Vec<String> = payload
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();

    // A throwaway bridge network isolates the job from other containers on
    // the host; it is removed in every cleanup path.
    let network: Option<String> = match isolation.network {
        JobNetwork::Isolated => {
            let name = format!("overup-job-{job_id}");
            match docker
                .create_network(NetworkCreateRequest {
                    name: name.clone(),
                    driver: Some("bridge".to_string()),
                    ..Default::default()
                })
                .await
            {
                Ok(_) => Some(name),
                Err(error) => {
                    log.system(&format!("job network creation failed: {error}")).await;
                    return fail("container_error");
                }
            }
        }
        JobNetwork::Bridge | JobNetwork::None => None,
    };
    let network_mode = match isolation.network {
        JobNetwork::Bridge => "bridge".to_string(),
        JobNetwork::None => "none".to_string(),
        JobNetwork::Isolated => network.clone().unwrap_or_else(|| "bridge".to_string()),
    };

    let host_config = HostConfig {
        // No host bind for /workspace: the source is streamed in over the
        // archive API after start, so this works even when a sibling job
        // container can't see a runner-local path. /workspace is a
        // daemon-managed anonymous volume (declared on the body below), which
        // stays writable even under a read-only rootfs.
        cap_drop: isolation.cap_drop.then(|| vec!["ALL".to_string()]),
        // Always: children can never gain privileges (setuid binaries etc.).
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        memory: (isolation.memory_bytes > 0).then_some(isolation.memory_bytes),
        nano_cpus: (isolation.nano_cpus > 0).then_some(isolation.nano_cpus),
        pids_limit: (isolation.pids_limit > 0).then_some(isolation.pids_limit),
        network_mode: Some(network_mode),
        readonly_rootfs: isolation.readonly_rootfs.then_some(true),
        // A read-only rootfs still needs a scratch /tmp; the RW workspace
        // bind covers build output.
        tmpfs: isolation.readonly_rootfs.then(|| {
            [("/tmp".to_string(), "rw,size=268435456".to_string())]
                .into_iter()
                .collect()
        }),
        ..Default::default()
    };
    let body = ContainerCreateBody {
        image: Some(payload.image.clone()),
        // Keep-alive; every step runs as an exec inside this container.
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "sleep 2147483647".to_string(),
        ]),
        env: Some(env.clone()),
        working_dir: Some(WORKSPACE_DIR.to_string()),
        // Anonymous volume at /workspace (serialized as `{"/workspace":{}}`):
        // daemon-managed and writable regardless of readonly_rootfs, removed
        // with the container via `v: true` on cleanup.
        volumes: Some(vec![WORKSPACE_DIR.to_string()]),
        user: isolation.user.clone(),
        host_config: Some(host_config),
        ..Default::default()
    };
    let container = match docker
        .create_container(None::<CreateContainerOptions>, body)
        .await
    {
        Ok(container) => container.id,
        Err(error) => {
            log.system(&format!("container creation failed: {error}")).await;
            cleanup(&docker, None, network.as_deref()).await;
            return fail("container_error");
        }
    };
    if let Err(error) = docker
        .start_container(&container, None::<StartContainerOptions>)
        .await
    {
        log.system(&format!("container start failed: {error}")).await;
        cleanup(&docker, Some(&container), network.as_deref()).await;
        return fail("container_error");
    }
    log.system(&format!("container started ({})", short_id(&container))).await;
    // Follow-up with the (short) container id so the UI can show it; still
    // the fixed-vocabulary stage, so old servers see an idempotent repeat.
    stage_with(
        out,
        job_id,
        "starting_container",
        Some(short_id(&container).to_string()),
        None,
    )
    .await;

    // --- sudo shim ----------------------------------------------------------
    // GitHub's hosted runners execute as a non-root user with passwordless
    // sudo, so workflows are written with `sudo`. Our containers run as root
    // already AND set no-new-privileges (which makes the kernel ignore the
    // setuid bit), so the real sudo fails while its privilege is redundant.
    // Shim it rather than weaken the container: root -> root grants nothing.
    if isolation.sudo_shim {
        if isolation.readonly_rootfs {
            log.system("read-only rootfs: skipping the sudo shim; `sudo` steps will not work")
                .await;
        } else if container_uid(&docker, &container).await == Some(0) {
            install_sudo_shim(&docker, &container, log).await;
        }
    }

    // --- checkout (upload source into the container) ------------------------
    // Now that /workspace exists inside the container, stream the repackaged
    // source into it over the archive API.
    if let Some(source) = source_tar {
        log.section(Some("checkout"), None);
        let files = source.files;
        if let Err(error) = upload_source(&docker, &container, source.tar).await {
            log.system(&format!("checkout failed: {error:#}")).await;
            cleanup(&docker, Some(&container), network.as_deref()).await;
            return fail("checkout_failed");
        }
        log.system(&format!("checkout complete ({files} files)")).await;
    }

    // Resource telemetry for the Performance tab; failures only cost the
    // metrics, never the job.
    let stats_agg: Arc<Mutex<StatsAgg>> = Arc::default();
    let sampler = tokio::spawn(sample_stats(
        docker.clone(),
        container.clone(),
        Arc::clone(&stats_agg),
    ));

    // --- steps ---------------------------------------------------------------
    stage(out, job_id, "running").await;
    let total_steps = payload.steps.len() as u32;
    let mut result = (JobConclusion::Success, Some(0), None);
    for (index, step) in payload.steps.iter().enumerate() {
        log.section(Some("steps"), Some(index as u32));
        log.system(&format!("▶ step {}/{}: {}", index + 1, payload.steps.len(), step.name))
            .await;
        step_progress(out, job_id, index as u32, total_steps, "started", None).await;

        // Defense-in-depth: re-validate the payload's working directory with
        // the shared protocol rules — the runner never trusts the control
        // plane for path safety — then require it to exist (GitHub fails the
        // step on a missing working-directory too).
        let workdir = match resolve_container_workdir(step.working_dir.as_deref()) {
            Ok(workdir) => workdir,
            Err(()) => {
                log.system("working directory failed validation; refusing to run the step")
                    .await;
                step_progress(out, job_id, index as u32, total_steps, "failed", None).await;
                result = (JobConclusion::Failure, None, Some("workdir_missing".to_string()));
                break;
            }
        };
        if workdir != WORKSPACE_DIR {
            log.system(&format!("working directory: {workdir}")).await;
            match dir_exists(&docker, &container, &workdir).await {
                Ok(true) => {}
                Ok(false) => {
                    log.system(&format!(
                        "working directory `{workdir}` does not exist in the workspace"
                    ))
                    .await;
                    step_progress(out, job_id, index as u32, total_steps, "failed", None).await;
                    result = (JobConclusion::Failure, None, Some("workdir_missing".to_string()));
                    break;
                }
                Err(error) => {
                    log.system(&format!("working directory check failed: {error}")).await;
                    step_progress(out, job_id, index as u32, total_steps, "failed", None).await;
                    result = (JobConclusion::Failure, None, Some("container_error".to_string()));
                    break;
                }
            }
        }

        match run_step(&docker, &container, step, &env, &workdir, log, cancel, deadline).await {
            StepOutcome::Done => {
                step_progress(out, job_id, index as u32, total_steps, "succeeded", None).await;
            }
            StepOutcome::Failed(code) => {
                log.system(&format!(
                    "step failed{}",
                    code.map(|c| format!(" (exit code {c})")).unwrap_or_default()
                ))
                .await;
                step_progress(out, job_id, index as u32, total_steps, "failed", code).await;
                result = (JobConclusion::Failure, code, Some("step_failed".to_string()));
                break;
            }
            StepOutcome::Cancelled => {
                log.system("job cancelled").await;
                result = (JobConclusion::Cancelled, None, None);
                break;
            }
            StepOutcome::TimedOut => {
                log.system("job exceeded its time budget").await;
                result = (JobConclusion::TimedOut, None, Some("timeout".to_string()));
                break;
            }
        }
    }

    // Steps are done: stop sampling and fold the aggregate into metrics.
    sampler.abort();
    {
        let agg = stats_agg.lock().unwrap();
        if agg.samples > 0 {
            metrics.cpu_peak_permille = Some(agg.cpu_peak_permille);
            metrics.cpu_avg_permille = Some(agg.cpu_sum_permille / u64::from(agg.samples));
            metrics.mem_peak_bytes = Some(agg.mem_peak_bytes);
            metrics.net_rx_bytes = Some(agg.net_rx_bytes);
            metrics.net_tx_bytes = Some(agg.net_tx_bytes);
            metrics.blkio_read_bytes = Some(agg.blkio_read_bytes);
            metrics.blkio_write_bytes = Some(agg.blkio_write_bytes);
            metrics.sample_count = Some(agg.samples);
        }
    }

    // --- artifacts (successful jobs only) ------------------------------------
    // Pull `.overup/artifacts/` back OUT of the container over the archive API
    // (symmetric with the source upload — no host bind), extract it to a temp
    // dir, then hand that dir to the existing uploader.
    log.section(Some("artifacts"), None);
    if matches!(result.0, JobConclusion::Success)
        && let Some(extracted) = download_artifacts_dir(&docker, &container).await
    {
        // The archive endpoint tars the requested directory itself, so its
        // contents land under `<tmp>/artifacts/`.
        let dir = extracted.path().join("artifacts");
        if dir.is_dir() {
            stage(out, job_id, "uploading_artifacts").await;
            let (uploaded, failed) =
                artifacts::upload_dir(&http, &dir, job_id, &payload.caps, out, grants).await;
            if uploaded + failed > 0 {
                log.system(&format!("artifacts: {uploaded} uploaded, {failed} failed"))
                    .await;
            }
        }
    }

    // --- cleanup --------------------------------------------------------------
    log.section(Some("cleanup"), None);
    stage(out, job_id, "cleaning_workspace").await;
    // Removing the container drops its anonymous /workspace volume (v: true),
    // so everything the job wrote is gone with it.
    cleanup(&docker, Some(&container), network.as_deref()).await;
    result
}

/// Best-effort teardown of everything a job materialized in Docker: the
/// container (forced) and, for isolated jobs, the per-job network. The
/// network removal must come second — Docker refuses to delete a network
/// with attached containers.
async fn cleanup(docker: &Docker, container: Option<&str>, network: Option<&str>) {
    if let Some(container) = container {
        remove_container(docker, container).await;
    }
    if let Some(network) = network
        && let Err(error) = docker.remove_network(network).await
    {
        tracing::warn!(network, error = ?error, "failed to remove job network");
    }
}

/// Running aggregate of container stats samples.
#[derive(Default)]
struct StatsAgg {
    cpu_peak_permille: u64,
    cpu_sum_permille: u64,
    mem_peak_bytes: u64,
    net_rx_bytes: u64,
    net_tx_bytes: u64,
    blkio_read_bytes: u64,
    blkio_write_bytes: u64,
    samples: u32,
}

/// Poll one-shot container stats every 5 s. CPU needs precpu deltas, so the
/// non-one-shot single read is used. Cumulative counters (network, blkio)
/// take the latest sample; gauges track peaks. Aborted by the caller.
async fn sample_stats(docker: Docker, container: String, agg: Arc<Mutex<StatsAgg>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let options = StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(false)
            .build();
        let mut stream = docker.stats(&container, Some(options));
        let Some(Ok(stats)) = stream.next().await else {
            continue;
        };

        let cpu_permille = (|| {
            let cpu = stats.cpu_stats.as_ref()?;
            let pre = stats.precpu_stats.as_ref()?;
            let cpu_delta = cpu
                .cpu_usage
                .as_ref()?
                .total_usage?
                .checked_sub(pre.cpu_usage.as_ref()?.total_usage?)?;
            let system_delta = cpu
                .system_cpu_usage?
                .checked_sub(pre.system_cpu_usage?)
                .filter(|d| *d > 0)?;
            let online = u64::from(cpu.online_cpus.unwrap_or(1).max(1));
            Some(cpu_delta.saturating_mul(online).saturating_mul(1000) / system_delta)
        })();

        let mem = stats.memory_stats.as_ref().and_then(|m| {
            m.max_usage.or(m.usage)
        });
        let (net_rx, net_tx) = stats
            .networks
            .as_ref()
            .map(|networks| {
                networks.values().fold((0u64, 0u64), |(rx, tx), iface| {
                    (
                        rx.saturating_add(iface.rx_bytes.unwrap_or(0)),
                        tx.saturating_add(iface.tx_bytes.unwrap_or(0)),
                    )
                })
            })
            .unwrap_or((0, 0));
        let (blk_read, blk_write) = stats
            .blkio_stats
            .as_ref()
            .and_then(|b| b.io_service_bytes_recursive.as_ref())
            .map(|entries| {
                entries.iter().fold((0u64, 0u64), |(read, write), entry| {
                    let value = entry.value.unwrap_or(0);
                    match entry.op.as_deref() {
                        Some(op) if op.eq_ignore_ascii_case("read") => {
                            (read.saturating_add(value), write)
                        }
                        Some(op) if op.eq_ignore_ascii_case("write") => {
                            (read, write.saturating_add(value))
                        }
                        _ => (read, write),
                    }
                })
            })
            .unwrap_or((0, 0));

        let mut agg = agg.lock().unwrap();
        agg.samples = agg.samples.saturating_add(1);
        if let Some(permille) = cpu_permille {
            agg.cpu_peak_permille = agg.cpu_peak_permille.max(permille);
            agg.cpu_sum_permille = agg.cpu_sum_permille.saturating_add(permille);
        }
        if let Some(mem) = mem {
            agg.mem_peak_bytes = agg.mem_peak_bytes.max(mem);
        }
        agg.net_rx_bytes = net_rx;
        agg.net_tx_bytes = net_tx;
        agg.blkio_read_bytes = blk_read;
        agg.blkio_write_bytes = blk_write;
    }
}

/// Resolve a step's workspace-relative working directory onto the container
/// path. Independent re-validation via the shared `protocol` rules: `..`,
/// absolute paths, and backslashes are rejected there, so the plain join can
/// never escape [`WORKSPACE_DIR`].
fn resolve_container_workdir(step_working_dir: Option<&str>) -> Result<String, ()> {
    match step_working_dir {
        None => Ok(WORKSPACE_DIR.to_string()),
        Some(raw) => match protocol::normalize_relative_path(raw) {
            Some(path) => Ok(format!("{WORKSPACE_DIR}/{path}")),
            None => Err(()),
        },
    }
}

/// Whether `path` exists as a directory inside the container. Argv-form
/// `test -d` (no shell — the path is never interpolated into a command
/// string); the exit code is the whole answer.
async fn dir_exists(
    docker: &Docker,
    container: &str,
    path: &str,
) -> Result<bool, bollard::errors::Error> {
    let exec = docker
        .create_exec(
            container,
            ExecConfig {
                cmd: Some(vec!["test".to_string(), "-d".to_string(), path.to_string()]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await?;
    if let bollard::exec::StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await?
    {
        while output.next().await.is_some() {}
    }
    let inspect = docker.inspect_exec(&exec.id).await?;
    Ok(inspect.exit_code == Some(0))
}

#[allow(clippy::too_many_arguments)]
async fn run_step(
    docker: &Docker,
    container: &str,
    step: &JobStep,
    env: &[String],
    workdir: &str,
    log: &mut JobLog,
    cancel: &mut watch::Receiver<bool>,
    deadline: tokio::time::Instant,
) -> StepOutcome {
    let exec = match docker
        .create_exec(
            container,
            ExecConfig {
                cmd: Some(vec![step.shell.clone(), "-c".to_string(), step.run.clone()]),
                env: Some(env.to_vec()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                working_dir: Some(workdir.to_string()),
                ..Default::default()
            },
        )
        .await
    {
        Ok(exec) => exec,
        Err(error) => {
            log.system(&format!("exec creation failed: {error}")).await;
            return StepOutcome::Failed(None);
        }
    };

    let mut output = match docker.start_exec(&exec.id, None).await {
        Ok(bollard::exec::StartExecResults::Attached { output, .. }) => output,
        Ok(bollard::exec::StartExecResults::Detached) => {
            return StepOutcome::Failed(None);
        }
        Err(error) => {
            log.system(&format!("exec start failed: {error}")).await;
            return StepOutcome::Failed(None);
        }
    };

    loop {
        tokio::select! {
            biased;
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    kill_container(docker, container).await;
                    return StepOutcome::Cancelled;
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                kill_container(docker, container).await;
                return StepOutcome::TimedOut;
            }
            chunk = output.next() => {
                match chunk {
                    Some(Ok(bollard::container::LogOutput::StdOut { message })) => {
                        log.emit(LogStream::Stdout, &String::from_utf8_lossy(&message)).await;
                    }
                    Some(Ok(bollard::container::LogOutput::StdErr { message })) => {
                        log.emit(LogStream::Stderr, &String::from_utf8_lossy(&message)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    match docker.inspect_exec(&exec.id).await {
        Ok(inspect) => match inspect.exit_code {
            Some(0) | None => StepOutcome::Done,
            Some(code) => StepOutcome::Failed(Some(code as i32)),
        },
        Err(_) => StepOutcome::Failed(None),
    }
}

async fn kill_container(docker: &Docker, container: &str) {
    let _ = docker
        .kill_container(
            container,
            Some(KillContainerOptionsBuilder::default().signal("SIGKILL").build()),
        )
        .await;
}

async fn remove_container(docker: &Docker, container: &str) {
    let _ = docker
        .remove_container(
            container,
            // `v(true)` also removes the anonymous /workspace volume.
            Some(
                RemoveContainerOptionsBuilder::default()
                    .force(true)
                    .v(true)
                    .build(),
            ),
        )
        .await;
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(12)]
}

fn format_mib(bytes: i64) -> String {
    format!("{:.0} MiB", bytes.max(0) as f64 / (1024.0 * 1024.0))
}

/// A repackaged source archive: the uncompressed tar bytes plus how many
/// regular files survived filtering. Zero files means the tarball had an
/// unexpected layout — uploading it would leave /workspace empty while the
/// log claims a successful checkout, so callers treat it as a failure.
struct RepackagedSource {
    tar: Vec<u8>,
    files: u64,
}

/// Download the repository tarball (size-capped) and repackage it into a
/// plain tar with the GitHub top-level directory stripped, ready to stream
/// into the container's /workspace. Only plain relative path segments survive
/// — entries with `..`, absolute paths, or prefixes are dropped, as are
/// symlink/hardlink entries (directory-traversal defense; the path check
/// can't validate a link TARGET). Fails if no files survive.
async fn fetch_and_repackage(
    http: &reqwest::Client,
    checkout: &protocol::Checkout,
) -> anyhow::Result<RepackagedSource> {
    let response = http
        .get(&checkout.tarball_url)
        .bearer_auth(&checkout.token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .context("tarball request failed")?
        .error_for_status()
        .context("tarball request rejected")?;

    let tarball = tempfile::NamedTempFile::new().context("temp file creation failed")?;
    let mut file = tokio::fs::File::create(tarball.path())
        .await
        .context("temp file open failed")?;
    let mut stream = response.bytes_stream();
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("tarball download interrupted")?;
        total += chunk.len() as u64;
        if total > MAX_TARBALL_BYTES {
            anyhow::bail!("repository tarball exceeds the size limit");
        }
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);

    let tar_path = tarball.path().to_path_buf();
    let source = tokio::task::spawn_blocking(move || repackage_stripped(&tar_path))
        .await
        .context("repackage task panicked")??;
    if source.files == 0 {
        anyhow::bail!("repository archive contained no usable files (unexpected tarball layout)");
    }
    Ok(source)
}

/// Read the downloaded gzip tarball and re-emit an uncompressed tar with the
/// top-level `{owner}-{repo}-{sha}/` component stripped and unsafe entries
/// dropped. Entry paths are rebuilt with `/` separators so the archive is
/// valid for a Linux container regardless of the runner's own platform.
fn repackage_stripped(tar_path: &Path) -> anyhow::Result<RepackagedSource> {
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut builder = tar::Builder::new(Vec::new());
    let mut skipped_links: u64 = 0;
    let mut files: u64 = 0;
    // git's tar writer stores any path longer than the 100-byte ustar name
    // field in a pax extended header (a type-`x` pseudo-entry that applies
    // to the NEXT entry) — it never uses the ustar prefix field. Carry that
    // path across so long-path files keep their real names, and never copy
    // the pseudo-entries into the output: their record data still holds the
    // ORIGINAL unstripped path, which Docker's extractor would re-apply,
    // nesting those files under the wrapper dir inside /workspace.
    let mut pax_path: Option<PathBuf> = None;
    for entry in archive.entries()? {
        let mut entry = entry?;

        match entry.header().entry_type() {
            tar::EntryType::XHeader => {
                if let Some(extensions) = entry.pax_extensions()? {
                    for extension in extensions.flatten() {
                        if extension.key().ok() == Some("path")
                            && let Ok(value) = extension.value()
                        {
                            pax_path = Some(PathBuf::from(value));
                        }
                    }
                }
                continue;
            }
            // Global pax headers and GNU long-name pseudo-entries are
            // metadata, not files — never part of the output.
            tar::EntryType::XGlobalHeader
            | tar::EntryType::GNULongName
            | tar::EntryType::GNULongLink => continue,
            // Symlink/hardlink entries are dropped outright: a link TARGET
            // can't be validated by the path check below, and one pointing
            // outside the workspace would let later entries (or the job)
            // escape it. Source archives from the Contents API don't need
            // them.
            tar::EntryType::Symlink | tar::EntryType::Link => {
                skipped_links += 1;
                pax_path = None;
                continue;
            }
            _ => {}
        }

        let path = match pax_path.take() {
            Some(path) => path,
            None => entry.path()?.into_owned(),
        };
        // Strip GitHub's top-level wrapper, keep only plain segments, and
        // rebuild the path with `/` so it is portable into the container.
        // A leading `./` (some tar producers emit it) is consumed first so
        // the strip removes the wrapper dir, not the no-op dot.
        let mut components = path.components().peekable();
        if matches!(components.peek(), Some(Component::CurDir)) {
            components.next();
        }
        components.next();
        let mut rel = String::new();
        let mut safe = true;
        for component in components {
            match component {
                Component::Normal(segment) => {
                    if !rel.is_empty() {
                        rel.push('/');
                    }
                    rel.push_str(&segment.to_string_lossy());
                }
                _ => {
                    safe = false;
                    break;
                }
            }
        }
        if !safe || rel.is_empty() {
            continue;
        }

        // The cloned header carries the correct size/mode/mtime; append_data
        // sets the (stripped) path and copies exactly `size` bytes.
        let mut header = entry.header().clone();
        if header.entry_type().is_file() {
            files += 1;
        }
        builder.append_data(&mut header, &rel, &mut entry)?;
    }
    if skipped_links > 0 {
        tracing::warn!(count = skipped_links, "skipped link entries in repository tarball");
    }
    let tar = builder.into_inner().context("finalizing source tar failed")?;
    Ok(RepackagedSource { tar, files })
}

/// Build the one-entry tar carrying the `sudo` shim, mode 0755.
///
/// `exec env "$@"` covers both plain commands (`sudo apt-get install -y x`)
/// and the env-prefix form (`sudo FOO=bar cmd`) in a single path. Options
/// that take a separate argument consume two slots; the rest are dropped.
fn sudo_shim_tar() -> anyhow::Result<Vec<u8>> {
    let mut header = tar::Header::new_gnu();
    header.set_size(SUDO_SHIM.len() as u64);
    header.set_mode(0o755);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();

    let mut builder = tar::Builder::new(Vec::new());
    builder
        .append_data(&mut header, "sudo", SUDO_SHIM.as_bytes())
        .context("building the sudo shim tar failed")?;
    builder.into_inner().context("finalizing the sudo shim tar failed")
}

/// The container's numeric uid, via argv-form `id -u`. Any failure reads as
/// `None` — callers treat that as "not known to be root" and skip the shim,
/// so a probe failure can never install it where it does not belong.
async fn container_uid(docker: &Docker, container: &str) -> Option<u32> {
    let exec = docker
        .create_exec(
            container,
            ExecConfig {
                cmd: Some(vec!["id".to_string(), "-u".to_string()]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .ok()?;
    let mut stdout = String::new();
    if let bollard::exec::StartExecResults::Attached { mut output, .. } =
        docker.start_exec(&exec.id, None).await.ok()?
    {
        while let Some(Ok(chunk)) = output.next().await {
            if let bollard::container::LogOutput::StdOut { message } = chunk {
                stdout.push_str(&String::from_utf8_lossy(&message));
            }
        }
    }
    if docker.inspect_exec(&exec.id).await.ok()?.exit_code != Some(0) {
        return None;
    }
    stdout.trim().parse().ok()
}

/// Install the `sudo` shim into a root job container over the archive API.
///
/// `/usr/local/bin` precedes `/usr/bin` in the default Debian/Ubuntu PATH,
/// so the shim shadows the real setuid binary without replacing it. Purely
/// best-effort: a job that does not use sudo must never fail because this
/// did not install.
async fn install_sudo_shim(docker: &Docker, container: &str, log: &mut JobLog) {
    let tar = match sudo_shim_tar() {
        Ok(tar) => tar,
        Err(error) => {
            tracing::warn!(%error, "building the sudo shim failed");
            return;
        }
    };
    match docker
        .upload_to_container(
            container,
            Some(
                UploadToContainerOptionsBuilder::default()
                    .path(SUDO_SHIM_DIR)
                    .build(),
            ),
            bollard::body_full(bytes::Bytes::from(tar)),
        )
        .await
    {
        Ok(()) => {
            log.system("sudo shim installed (container runs as root)").await;
        }
        Err(error) => {
            log.system(&format!(
                "sudo shim installation failed ({error}); `sudo` steps will not work"
            ))
            .await;
        }
    }
}

/// Stream a repackaged source tar into the container's /workspace over the
/// Docker archive API (`PUT /containers/{id}/archive`).
async fn upload_source(docker: &Docker, container: &str, tar: Vec<u8>) -> anyhow::Result<()> {
    docker
        .upload_to_container(
            container,
            Some(
                UploadToContainerOptionsBuilder::default()
                    .path(WORKSPACE_DIR)
                    .build(),
            ),
            bollard::body_full(bytes::Bytes::from(tar)),
        )
        .await
        .context("uploading source into the container failed")?;
    Ok(())
}

/// Pull `/workspace/.overup/artifacts` back out of the container over the
/// archive API and extract it to a fresh temp dir. Best-effort: a missing
/// directory (404) or any transport/extraction error yields `None` (no
/// artifacts), never a job failure.
async fn download_artifacts_dir(docker: &Docker, container: &str) -> Option<tempfile::TempDir> {
    let mut stream = docker.download_from_container(
        container,
        Some(
            DownloadFromContainerOptionsBuilder::default()
                .path(&format!("{WORKSPACE_DIR}/{ARTIFACTS_SUBDIR}"))
                .build(),
        ),
    );

    let tarball = tempfile::NamedTempFile::new().ok()?;
    let mut file = tokio::fs::File::create(tarball.path()).await.ok()?;
    let mut total: u64 = 0;
    while let Some(chunk) = stream.next().await {
        // The daemon returns 404 as a stream error when the path is absent —
        // that just means the job produced no artifacts.
        let chunk = chunk.ok()?;
        total += chunk.len() as u64;
        if total > MAX_ARTIFACT_ARCHIVE_BYTES {
            tracing::warn!("artifacts archive exceeds the size limit; skipping upload");
            return None;
        }
        file.write_all(&chunk).await.ok()?;
    }
    file.flush().await.ok()?;
    drop(file);
    if total == 0 {
        return None;
    }

    let dir = tempfile::tempdir().ok()?;
    let tar_path = tarball.path().to_path_buf();
    let dest = dir.path().to_path_buf();
    // The archive endpoint returns an uncompressed tar; `unpack` guards
    // against path-traversal entries.
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&tar_path)?;
        tar::Archive::new(file).unpack(&dest)
    })
    .await
    .ok()?
    .ok()?;
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a gzip tarball wrapped in a `repo-sha/` top-level dir with a mix
    /// of a root file, a nested file, and a symlink — the shape of a GitHub
    /// source archive plus a link we must drop.
    fn sample_source_tarball() -> tempfile::NamedTempFile {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(tmp.path()).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(gz);
        for (name, contents) in [
            ("repo-sha/package.json", &b"{}"[..]),
            ("repo-sha/src/main.rs", b"fn main() {}"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append_data(&mut header, name, contents).unwrap();
        }
        // A symlink entry that must be dropped.
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_cksum();
        builder
            .append_link(&mut link, "repo-sha/evil", "/etc/passwd")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        tmp
    }

    #[test]
    fn sudo_shim_tar_carries_one_executable_entry() {
        let tar = sudo_shim_tar().unwrap();
        let mut archive = tar::Archive::new(&tar[..]);
        let entries: Vec<_> = archive
            .entries()
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let path = entry.path().unwrap().to_string_lossy().into_owned();
                (path, entry.header().mode().unwrap())
            })
            .collect();

        // Unpacked at /usr/local/bin, this lands on /usr/local/bin/sudo —
        // ahead of the real /usr/bin/sudo on the default PATH — and must be
        // executable to shadow it at all.
        assert_eq!(entries, vec![("sudo".to_string(), 0o755)]);
    }

    #[test]
    fn sudo_shim_execs_the_command_without_escalating() {
        // The shim's whole safety argument: it never invokes the real setuid
        // sudo and never changes user — it execs what it was handed.
        assert!(SUDO_SHIM.starts_with("#!/bin/sh\n"));
        assert!(SUDO_SHIM.contains(r#"exec env "$@""#));
        assert!(!SUDO_SHIM.contains("/usr/bin/sudo"));
        // An interactive shell here would block until the job timed out.
        assert!(!SUDO_SHIM.contains("exec /bin/sh"));
        // Options taking a separate argument must consume both slots, or the
        // shim would exec the argument (`sudo -u root apt-get` -> `root …`).
        assert!(SUDO_SHIM.contains("-u|-g|-p|-C|-D|-R|-T)"));
    }

    #[test]
    fn repackage_strips_top_level_and_drops_links() {
        let src = sample_source_tarball();
        let source = repackage_stripped(src.path()).unwrap();

        let mut archive = tar::Archive::new(&source.tar[..]);
        let mut paths: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        paths.sort();

        // Top-level `repo-sha/` stripped; the symlink is gone.
        assert_eq!(paths, vec!["package.json".to_string(), "src/main.rs".to_string()]);
        assert_eq!(source.files, 2);
    }

    #[test]
    fn repackaged_source_unpacks_to_workspace_root() {
        let src = sample_source_tarball();
        let source = repackage_stripped(src.path()).unwrap();

        let dest = tempfile::tempdir().unwrap();
        tar::Archive::new(&source.tar[..]).unpack(dest.path()).unwrap();

        // Exactly where a step running in /workspace expects them.
        assert_eq!(
            std::fs::read(dest.path().join("package.json")).unwrap(),
            b"{}"
        );
        assert!(dest.path().join("src/main.rs").is_file());
        assert!(!dest.path().join("evil").exists());
        // Subdirectories materialize on unpack — the contract the per-step
        // working-directory existence check (`test -d`) relies on.
        assert!(dest.path().join("src").is_dir());
    }

    #[test]
    fn resolve_container_workdir_stays_inside_the_workspace() {
        assert_eq!(
            resolve_container_workdir(None).as_deref(),
            Ok(WORKSPACE_DIR)
        );
        assert_eq!(
            resolve_container_workdir(Some("scripts/build")).as_deref(),
            Ok("/workspace/scripts/build")
        );
        // Normalization mirrors the control plane's.
        assert_eq!(
            resolve_container_workdir(Some("./scripts/")).as_deref(),
            Ok("/workspace/scripts")
        );
        // The runner re-validates independently — a hostile payload never
        // directs execution outside the workspace volume.
        for bad in ["../x", "/abs", "a\\b", "a/../b", "", "${{ x }}"] {
            assert_eq!(resolve_container_workdir(Some(bad)), Err(()), "{bad:?}");
        }
        assert_eq!(resolve_container_workdir(Some(&"a".repeat(256))), Err(()));
    }

    /// `./`-prefixed entries (`./repo-sha/…`) must strip the wrapper dir, not
    /// the no-op dot — otherwise files land at /workspace/repo-sha/… and
    /// steps see an empty workspace root.
    #[test]
    fn repackage_handles_dot_prefixed_entries() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(tmp.path()).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(gz);
        for (name, contents) in [
            ("./repo-sha/package.json", &b"{}"[..]),
            ("./repo-sha/src/main.rs", b"fn main() {}"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder.append_data(&mut header, name, contents).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();

        let source = repackage_stripped(tmp.path()).unwrap();
        assert_eq!(source.files, 2);

        let dest = tempfile::tempdir().unwrap();
        tar::Archive::new(&source.tar[..]).unpack(dest.path()).unwrap();
        assert!(dest.path().join("package.json").is_file());
        assert!(dest.path().join("src/main.rs").is_file());
        assert!(!dest.path().join("repo-sha").exists());
    }

    /// One pax record in the `%len key=value\n` wire format.
    fn pax_record(key: &str, value: &str) -> Vec<u8> {
        // len counts its own digits plus ` key=value\n`.
        let base = key.len() + value.len() + 3;
        let mut len = base;
        loop {
            let candidate = base + len.to_string().len();
            if candidate == len {
                break;
            }
            len = candidate;
        }
        format!("{len} {key}={value}\n").into_bytes()
    }

    /// git's tar writer never uses the ustar prefix field: a path longer
    /// than the 100-byte name field arrives as a pax extended header
    /// (type `x`) followed by a regular entry with a truncated name. The
    /// repackager must restore the real path and drop the pseudo-entry —
    /// copying it through made Docker re-apply the ORIGINAL unstripped
    /// path, nesting 176 of this repo's own files under the wrapper dir
    /// (the empty-looking /workspace CI failure of 2026-07-16).
    #[test]
    fn repackage_applies_pax_long_paths_and_drops_pseudo_entries() {
        let long_rel = format!("src/{}/useRepositories.ts", "d".repeat(110));
        let long_path = format!("repo-sha/{long_rel}");

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(tmp.path()).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(gz);

        // Global pax header, the way git leads every archive.
        let comment = pax_record("comment", "abc123");
        let mut global = tar::Header::new_ustar();
        global.set_entry_type(tar::EntryType::XGlobalHeader);
        global.set_size(comment.len() as u64);
        global.set_cksum();
        builder
            .append_data(&mut global, "pax_global_header", &comment[..])
            .unwrap();

        // Extended header carrying the real path, then the truncated entry.
        let record = pax_record("path", &long_path);
        let mut xheader = tar::Header::new_ustar();
        xheader.set_entry_type(tar::EntryType::XHeader);
        xheader.set_size(record.len() as u64);
        xheader.set_cksum();
        builder
            .append_data(&mut xheader, "repo-sha/paxheader", &record[..])
            .unwrap();
        let contents = b"export {};";
        let mut file = tar::Header::new_ustar();
        file.set_entry_type(tar::EntryType::Regular);
        file.set_size(contents.len() as u64);
        file.set_mode(0o644);
        file.set_cksum();
        builder
            .append_data(&mut file, "repo-sha/truncated", &contents[..])
            .unwrap();

        // A short-path sibling must be unaffected by the carried pax path.
        let mut plain = tar::Header::new_ustar();
        plain.set_entry_type(tar::EntryType::Regular);
        plain.set_size(2);
        plain.set_mode(0o644);
        plain.set_cksum();
        builder
            .append_data(&mut plain, "repo-sha/package.json", &b"{}"[..])
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let source = repackage_stripped(tmp.path()).unwrap();
        assert_eq!(source.files, 2);

        // No pseudo-entries survive into the output archive.
        let mut archive = tar::Archive::new(&source.tar[..]);
        for entry in archive.entries().unwrap() {
            let entry = entry.unwrap();
            assert!(matches!(
                entry.header().entry_type(),
                tar::EntryType::Regular
            ));
        }

        let dest = tempfile::tempdir().unwrap();
        tar::Archive::new(&source.tar[..]).unpack(dest.path()).unwrap();
        assert_eq!(
            std::fs::read(dest.path().join(&long_rel)).unwrap(),
            contents
        );
        assert!(dest.path().join("package.json").is_file());
        assert!(!dest.path().join("truncated").exists());
        assert!(!dest.path().join("repo-sha").exists());
    }

    /// A tarball whose entries all get filtered out (here: links only) must
    /// report zero files so the caller fails the checkout instead of
    /// uploading an empty archive and logging success.
    #[test]
    fn repackage_reports_zero_files_when_everything_is_filtered() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(tmp.path()).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(gz);
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_cksum();
        builder
            .append_link(&mut link, "repo-sha/evil", "/etc/passwd")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let source = repackage_stripped(tmp.path()).unwrap();
        assert_eq!(source.files, 0);
    }
}
