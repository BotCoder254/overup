//! Job execution via the Docker Engine API.
//!
//! One job at a time: prepare an isolated temporary workspace, download and
//! extract the repository tarball (path-traversal-safe, symlink entries
//! dropped), pull the container image, start one hardened keep-alive
//! container (no-new-privileges always; capabilities dropped, memory/CPU/
//! pids limits, network mode, optional non-root user and read-only rootfs
//! per config) with the workspace bind-mounted at /workspace, run each step
//! as a `docker exec` (`<shell> -c <script>`), stream stdout/stderr chunks
//! back with monotonic sequence numbers, sample container resource stats
//! for the Performance tab, upload artifacts from `.overup/artifacts/`,
//! then remove the container (and per-job network) and delete the
//! workspace. Cancellation and the job timeout kill the container
//! immediately.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use bollard::Docker;
use bollard::models::{ContainerCreateBody, ExecConfig, HostConfig, NetworkCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptionsBuilder, KillContainerOptionsBuilder,
    RemoveContainerOptionsBuilder, StartContainerOptions, StatsOptionsBuilder,
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
/// Log chunks are split to stay comfortably under the server frame cap.
const LOG_CHUNK_BYTES: usize = 32 * 1024;

/// Sequenced log emitter for one job.
struct JobLog {
    out: mpsc::Sender<RunnerMsg>,
    job_id: Uuid,
    seq: u64,
}

impl JobLog {
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
                })
                .await;
        }
    }

    async fn system(&mut self, text: &str) {
        self.emit(LogStream::System, text).await;
    }
}

async fn stage(out: &mpsc::Sender<RunnerMsg>, job_id: Uuid, stage: &str) {
    let _ = out
        .send(RunnerMsg::JobStage {
            job_id,
            stage: stage.to_string(),
            detail: None,
        })
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

    let workspace = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(error) => {
            tracing::warn!(error = ?error, "failed to create workspace");
            return fail("internal");
        }
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

    // --- checkout -----------------------------------------------------------
    if let Some(checkout) = &payload.checkout {
        log.system("downloading repository archive").await;
        if let Err(error) = fetch_and_extract(&http, checkout, workspace.path()).await {
            // Error text never contains the token (it travels in a header).
            log.system(&format!("checkout failed: {error:#}")).await;
            return fail("checkout_failed");
        }
        log.system("checkout complete").await;
    } else {
        log.system("no checkout credentials; starting with an empty workspace")
            .await;
    }

    // --- image pull ---------------------------------------------------------
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
    while let Some(progress) = pull.next().await {
        if *cancel.borrow() {
            return (JobConclusion::Cancelled, None, None);
        }
        match progress {
            Err(error) => {
                log.system(&format!("image pull failed: {error}")).await;
                return fail("image_pull_failed");
            }
            Ok(info) => {
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
    metrics.image_pull_ms = Some(pull_started.elapsed().as_millis() as u64);
    log.system(&format!(
        "image ready in {:.1}s",
        pull_started.elapsed().as_secs_f64()
    ))
    .await;

    // --- container ----------------------------------------------------------
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
        binds: Some(vec![format!("{}:/workspace", workspace.path().display())]),
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
        working_dir: Some("/workspace".to_string()),
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
    let mut result = (JobConclusion::Success, Some(0), None);
    for (index, step) in payload.steps.iter().enumerate() {
        log.system(&format!("▶ step {}/{}: {}", index + 1, payload.steps.len(), step.name))
            .await;
        match run_step(&docker, &container, step, &env, log, cancel, deadline).await {
            StepOutcome::Done => {}
            StepOutcome::Failed(code) => {
                log.system(&format!(
                    "step failed{}",
                    code.map(|c| format!(" (exit code {c})")).unwrap_or_default()
                ))
                .await;
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
    if matches!(result.0, JobConclusion::Success) {
        let dir = workspace.path().join(".overup").join("artifacts");
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
    stage(out, job_id, "cleaning_workspace").await;
    cleanup(&docker, Some(&container), network.as_deref()).await;
    // The temp workspace (and everything the job wrote) is deleted on drop.
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

async fn run_step(
    docker: &Docker,
    container: &str,
    step: &JobStep,
    env: &[String],
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
                working_dir: Some("/workspace".to_string()),
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
            Some(RemoveContainerOptionsBuilder::default().force(true).build()),
        )
        .await;
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(12)]
}

fn format_mib(bytes: i64) -> String {
    format!("{:.0} MiB", bytes.max(0) as f64 / (1024.0 * 1024.0))
}

/// Download the repository tarball (size-capped) and extract it with the
/// GitHub top-level directory stripped. Only plain relative path segments
/// are unpacked — entries with `..`, absolute paths, or prefixes are
/// silently dropped (directory-traversal defense).
async fn fetch_and_extract(
    http: &reqwest::Client,
    checkout: &protocol::Checkout,
    dest: &Path,
) -> anyhow::Result<()> {
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
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || extract_stripped(&tar_path, &dest))
        .await
        .context("extraction task panicked")??;
    Ok(())
}

fn extract_stripped(tar_path: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(tar_path)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut skipped_links: u64 = 0;
    for entry in archive.entries()? {
        let mut entry = entry?;

        // Symlink/hardlink entries are dropped outright: the entry path
        // check below cannot validate a link TARGET, and a link pointing
        // outside the workspace would let later entries (or the job itself)
        // escape it. Source archives from the Contents API don't need them.
        match entry.header().entry_type() {
            tar::EntryType::Symlink | tar::EntryType::Link => {
                skipped_links += 1;
                continue;
            }
            _ => {}
        }

        let path = entry.path()?.into_owned();
        // GitHub tarballs wrap everything in `{owner}-{repo}-{sha}/`.
        let stripped: PathBuf = path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        if stripped
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            continue;
        }
        let target = dest.join(&stripped);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
    }
    if skipped_links > 0 {
        tracing::warn!(count = skipped_links, "skipped link entries in repository tarball");
    }
    Ok(())
}
