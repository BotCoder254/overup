//! Artifact upload: request a presigned R2 PUT grant from the control
//! plane, upload the bytes directly to storage, then report completion with
//! a SHA-256 checksum. By convention, files a job leaves in
//! `.overup/artifacts/` inside its workspace are uploaded.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::RunnerMsg;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// How long to wait for the control plane to answer an artifact request.
const GRANT_TIMEOUT: Duration = Duration::from_secs(30);

/// Grant outcome: Ok(presigned PUT url) or Err(denial reason).
type GrantResult = Result<String, String>;
type WaiterMap = HashMap<(Uuid, String), oneshot::Sender<GrantResult>>;

/// Pending grant requests, resolved by the WebSocket task when the server
/// answers with artifact_grant / artifact_deny.
#[derive(Clone, Default)]
pub struct GrantWaiters(Arc<Mutex<WaiterMap>>);

impl GrantWaiters {
    pub fn expect(&self, job_id: Uuid, name: &str) -> oneshot::Receiver<GrantResult> {
        let (tx, rx) = oneshot::channel();
        self.0
            .lock()
            .unwrap()
            .insert((job_id, name.to_string()), tx);
        rx
    }

    pub fn resolve(&self, job_id: Uuid, name: &str, result: GrantResult) {
        if let Some(tx) = self.0.lock().unwrap().remove(&(job_id, name.to_string())) {
            let _ = tx.send(result);
        }
    }
}

/// Upload every regular file directly inside `dir`. Failures are reported
/// per file and never abort the job. Returns (uploaded, failed) counts.
pub async fn upload_dir(
    http: &reqwest::Client,
    dir: &Path,
    job_id: Uuid,
    caps: &protocol::JobCaps,
    out: &mpsc::Sender<RunnerMsg>,
    grants: &GrantWaiters,
) -> (u32, u32) {
    let mut uploaded = 0u32;
    let mut failed = 0u32;

    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return (0, 0); // no artifacts directory: nothing to do
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        if uploaded + failed >= caps.max_artifacts {
            break;
        }
        let Ok(meta) = entry.metadata().await else { continue };
        if !meta.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let size = meta.len();
        if size == 0 || size > caps.max_artifact_bytes {
            failed += 1;
            continue;
        }

        match upload_one(http, &entry.path(), job_id, &name, size, out, grants).await {
            Ok(()) => uploaded += 1,
            Err(error) => {
                tracing::warn!(artifact = %name, error = ?error, "artifact upload failed");
                failed += 1;
            }
        }
    }
    (uploaded, failed)
}

async fn upload_one(
    http: &reqwest::Client,
    path: &Path,
    job_id: Uuid,
    name: &str,
    size: u64,
    out: &mpsc::Sender<RunnerMsg>,
    grants: &GrantWaiters,
) -> anyhow::Result<()> {
    let grant_rx = grants.expect(job_id, name);
    out.send(RunnerMsg::ArtifactRequest {
        job_id,
        name: name.to_string(),
        size_bytes: size,
        content_type: "application/octet-stream".to_string(),
    })
    .await
    .map_err(|_| anyhow::anyhow!("connection closed"))?;

    let put_url = tokio::time::timeout(GRANT_TIMEOUT, grant_rx)
        .await
        .map_err(|_| anyhow::anyhow!("artifact grant timed out"))?
        .map_err(|_| anyhow::anyhow!("artifact grant dropped"))?
        .map_err(|reason| anyhow::anyhow!("artifact denied: {reason}"))?;

    // Size is capped, so an in-memory read is fine for phase 1.
    let bytes = tokio::fs::read(path).await?;
    let checksum = hex::encode(Sha256::digest(&bytes));

    let response = http
        .put(put_url)
        .header("content-type", "application/octet-stream")
        .body(bytes)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("storage returned {}", response.status());
    }

    out.send(RunnerMsg::ArtifactDone {
        job_id,
        name: name.to_string(),
        size_bytes: size,
        checksum_sha256: checksum,
    })
    .await
    .map_err(|_| anyhow::anyhow!("connection closed"))?;
    Ok(())
}
