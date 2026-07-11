//! Artifact upload: request a presigned R2 PUT grant from the control
//! plane, upload the bytes directly to storage, then report completion with
//! a SHA-256 checksum. By convention, files a job leaves in
//! `.overup/artifacts/` inside its workspace are uploaded. Archive artifacts
//! (`.zip`, `.tar.gz`, `.tgz`) additionally get a capped entry manifest so
//! browsers can list their contents without server-side extraction.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::{ArtifactEntry, RunnerMsg};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// How long to wait for the control plane to answer an artifact request.
const GRANT_TIMEOUT: Duration = Duration::from_secs(30);

/// Manifest caps: the whole `artifact_done` frame must stay well under the
/// server's 128 KB inbound limit, so entries stop at 1000 items or ~64 KB
/// of serialized paths (whichever comes first). Totals keep counting past
/// the entry cap so file_count/uncompressed_bytes stay truthful.
const MAX_MANIFEST_ENTRIES: usize = 1000;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Rough serialized overhead per entry beyond the path itself
/// (`{"path":"…","size_bytes":N},`).
const ENTRY_JSON_OVERHEAD: usize = 32;

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
    // The same string must go in the request, the presign, and the PUT
    // header — R2 rejects a signature/header content-type mismatch.
    let content_type = content_type_for(name);

    let grant_rx = grants.expect(job_id, name);
    out.send(RunnerMsg::ArtifactRequest {
        job_id,
        name: name.to_string(),
        size_bytes: size,
        content_type: content_type.to_string(),
    })
    .await
    .map_err(|_| anyhow::anyhow!("connection closed"))?;

    let put_url = tokio::time::timeout(GRANT_TIMEOUT, grant_rx)
        .await
        .map_err(|_| anyhow::anyhow!("artifact grant timed out"))?
        .map_err(|_| anyhow::anyhow!("artifact grant dropped"))?
        .map_err(|reason| anyhow::anyhow!("artifact denied: {reason}"))?;

    // Archive introspection is best-effort: any error just means no
    // manifest, never a failed upload.
    let manifest = {
        let path = path.to_path_buf();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || inspect_archive(&path, &name))
            .await
            .unwrap_or(None)
    };

    // Size is capped, so an in-memory read is fine for phase 1.
    let bytes = tokio::fs::read(path).await?;
    let checksum = hex::encode(Sha256::digest(&bytes));

    let response = http
        .put(put_url)
        .header("content-type", content_type)
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
        uncompressed_bytes: manifest.as_ref().map(|m| m.uncompressed_bytes),
        file_count: manifest.as_ref().map(|m| m.file_count),
        entries: manifest.map(|m| m.entries),
    })
    .await
    .map_err(|_| anyhow::anyhow!("connection closed"))?;
    Ok(())
}

/// Static suffix -> MIME map so stored artifacts carry a real content type.
/// Unknown extensions stay `application/octet-stream`.
fn content_type_for(name: &str) -> &'static str {
    const MAP: &[(&str, &str)] = &[
        // Compound suffixes first — the map is scanned in order.
        (".tar.gz", "application/gzip"),
        (".tar.bz2", "application/x-bzip2"),
        (".tgz", "application/gzip"),
        (".gz", "application/gzip"),
        (".tar", "application/x-tar"),
        (".zip", "application/zip"),
        (".7z", "application/x-7z-compressed"),
        (".json", "application/json"),
        (".xml", "application/xml"),
        (".html", "text/html"),
        (".htm", "text/html"),
        (".txt", "text/plain"),
        (".log", "text/plain"),
        (".md", "text/markdown"),
        (".csv", "text/csv"),
        (".pdf", "application/pdf"),
        (".png", "image/png"),
        (".jpg", "image/jpeg"),
        (".jpeg", "image/jpeg"),
        (".gif", "image/gif"),
        (".svg", "image/svg+xml"),
        (".webp", "image/webp"),
        (".wasm", "application/wasm"),
    ];
    let lower = name.to_ascii_lowercase();
    for (suffix, mime) in MAP {
        if lower.ends_with(suffix) {
            return mime;
        }
    }
    "application/octet-stream"
}

struct ArchiveManifest {
    uncompressed_bytes: u64,
    file_count: u32,
    entries: Vec<ArtifactEntry>,
}

/// Read archive metadata without extracting anything. Only regular-file
/// entries are listed; totals keep accumulating after the entry caps hit.
/// Any parse error returns `None` — the upload itself always proceeds.
fn inspect_archive(path: &Path, name: &str) -> Option<ArchiveManifest> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        inspect_tar_gz(path)
    } else if lower.ends_with(".zip") {
        inspect_zip(path)
    } else {
        None
    }
}

/// Accumulates entries under the manifest caps; totals are unconditional.
struct ManifestBuilder {
    manifest: ArchiveManifest,
    serialized_estimate: usize,
}

impl ManifestBuilder {
    fn new() -> Self {
        Self {
            manifest: ArchiveManifest {
                uncompressed_bytes: 0,
                file_count: 0,
                entries: Vec::new(),
            },
            serialized_estimate: 0,
        }
    }

    fn add(&mut self, path: String, size: u64) {
        self.manifest.uncompressed_bytes = self.manifest.uncompressed_bytes.saturating_add(size);
        self.manifest.file_count = self.manifest.file_count.saturating_add(1);
        let cost = path.len() + ENTRY_JSON_OVERHEAD;
        if self.manifest.entries.len() < MAX_MANIFEST_ENTRIES
            && self.serialized_estimate + cost <= MAX_MANIFEST_BYTES
        {
            self.serialized_estimate += cost;
            self.manifest.entries.push(ArtifactEntry {
                path,
                size_bytes: size,
            });
        }
    }

    fn finish(self) -> Option<ArchiveManifest> {
        (self.manifest.file_count > 0).then_some(self.manifest)
    }
}

fn inspect_tar_gz(path: &Path) -> Option<ArchiveManifest> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut builder = ManifestBuilder::new();
    for entry in archive.entries().ok()? {
        let Ok(entry) = entry else { return None };
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let entry_path = entry.path().ok()?.to_string_lossy().into_owned();
        builder.add(entry_path, entry.header().size().unwrap_or(0));
    }
    builder.finish()
}

fn inspect_zip(path: &Path) -> Option<ArchiveManifest> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut builder = ManifestBuilder::new();
    for index in 0..archive.len() {
        // Raw access reads central-directory metadata only — entries are
        // never decompressed.
        let entry = archive.by_index_raw(index).ok()?;
        if entry.is_dir() {
            continue;
        }
        builder.add(entry.name().to_string(), entry.size());
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn content_types_match_extensions() {
        assert_eq!(content_type_for("dist.tar.gz"), "application/gzip");
        assert_eq!(content_type_for("bundle.ZIP"), "application/zip");
        assert_eq!(content_type_for("report.json"), "application/json");
        assert_eq!(content_type_for("index.html"), "text/html");
        assert_eq!(content_type_for("mystery"), "application/octet-stream");
        // Plain .tar must not be swallowed by the .tar.gz rule.
        assert_eq!(content_type_for("layer.tar"), "application/x-tar");
    }

    #[test]
    fn inspects_tar_gz_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("out.tar.gz");

        let file = std::fs::File::create(&archive_path).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(gz);
        for (name, contents) in [("bin/app", &b"binarybytes"[..]), ("README.md", b"docs")] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append_data(&mut header, name, contents).unwrap();
        }
        tar_builder.into_inner().unwrap().finish().unwrap();

        let manifest = inspect_archive(&archive_path, "out.tar.gz").expect("manifest");
        assert_eq!(manifest.file_count, 2);
        assert_eq!(manifest.uncompressed_bytes, 11 + 4);
        assert_eq!(manifest.entries.len(), 2);
        assert!(manifest.entries.iter().any(|e| e.path == "bin/app"));
    }

    #[test]
    fn inspects_zip_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("out.zip");

        let file = std::fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("report.txt", options).unwrap();
        writer.write_all(b"hello world").unwrap();
        writer.finish().unwrap();

        let manifest = inspect_archive(&archive_path, "out.zip").expect("manifest");
        assert_eq!(manifest.file_count, 1);
        assert_eq!(manifest.uncompressed_bytes, 11);
        assert_eq!(manifest.entries[0].path, "report.txt");
    }

    #[test]
    fn non_archives_and_garbage_yield_no_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, b"plain file").unwrap();
        assert!(inspect_archive(&path, "notes.txt").is_none());

        // A .zip that isn't a zip parses to None, never panics.
        let bogus = dir.path().join("fake.zip");
        std::fs::write(&bogus, b"definitely not a zip").unwrap();
        assert!(inspect_archive(&bogus, "fake.zip").is_none());
    }

    #[test]
    fn manifest_entries_are_capped_but_totals_keep_counting() {
        let mut builder = ManifestBuilder::new();
        for i in 0..(MAX_MANIFEST_ENTRIES + 50) {
            builder.add(format!("file-{i}.txt"), 10);
        }
        let manifest = builder.finish().unwrap();
        assert_eq!(manifest.entries.len(), MAX_MANIFEST_ENTRIES);
        assert_eq!(manifest.file_count as usize, MAX_MANIFEST_ENTRIES + 50);
        assert_eq!(manifest.uncompressed_bytes, 10 * (MAX_MANIFEST_ENTRIES as u64 + 50));
    }
}
