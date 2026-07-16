//! Post-completion log archival to object storage.
//!
//! When a job reaches a terminal state and object storage is configured
//! (MinIO primary and/or R2 fallback), its persisted (already masked) log
//! chunks are concatenated, gzip-compressed, and stored at
//! `logs/{workspace}/{pipeline}/{job}-{attempt}.log.gz`. Postgres remains
//! the source of truth until the janitor prunes chunks whose hot-retention
//! window has lapsed — a failed upload just leaves the job unarchived, and
//! unarchived jobs are never pruned. The write falls back to the secondary
//! store automatically; whichever store accepted the archive is recorded on
//! the job row so downloads presign against the right host.

use std::io::Write;

use uuid::Uuid;

use crate::db;
use crate::models::pipeline::PipelineJob;
use crate::services::object_store;
use crate::state::AppState;

/// Fire-and-forget archival for one finished job. Skipped jobs and jobs
/// that never produced output are left alone.
pub fn spawn_archive(state: &AppState, job: &PipelineJob) {
    if state.storage.is_none() {
        return;
    }
    if job.conclusion.as_deref() == Some("skipped") || job.log_bytes == 0 {
        return;
    }
    let state = state.clone();
    let job = job.clone();
    tokio::spawn(async move {
        if let Err(error) = archive_job(&state, &job).await {
            tracing::warn!(
                job_id = %job.id,
                error = ?error,
                "log archival failed; chunks stay hot in Postgres"
            );
        }
    });
}

async fn archive_job(state: &AppState, job: &PipelineJob) -> anyhow::Result<()> {
    let Some(storage) = &state.storage else {
        return Ok(());
    };

    let pipeline = db::pipelines::find_by_id(&state.pool, job.pipeline_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("pipeline row vanished"))?;

    let chunks = db::pipeline_logs::fetch_all(&state.pool, job.id).await?;
    if chunks.is_empty() {
        return Ok(());
    }

    // Same concatenation as the raw download endpoint, so the archive and
    // the hot path produce byte-identical files.
    let mut body = String::new();
    for chunk in &chunks {
        body.push_str(&chunk.content);
        if !chunk.content.ends_with('\n') {
            body.push('\n');
        }
    }

    // Bounded by the per-job log cap (~10 MiB), so in-memory compression on
    // a blocking thread is fine.
    let compressed = tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body.as_bytes())?;
        encoder.finish()
    })
    .await??;

    let key = log_key_for(pipeline.workspace_id, job);
    let backend = storage
        .put_object(&key, compressed, "application/gzip")
        .await?;
    db::pipeline_jobs::mark_logs_archived(&state.pool, job.id, backend.as_str()).await?;

    tracing::debug!(job_id = %job.id, key = %key, backend = %backend, "job log archived");
    Ok(())
}

pub fn log_key_for(workspace_id: Uuid, job: &PipelineJob) -> String {
    object_store::log_key(workspace_id, job.pipeline_id, job.id, job.attempt)
}
