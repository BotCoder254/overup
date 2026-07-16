//! S3-compatible object storage (artifacts, log archives, workspace logos).
//!
//! Two backends share one generic [`S3Store`] client: **MinIO** (any
//! S3-compatible endpoint — the default/primary store when configured) and
//! **Cloudflare R2** (fully supported; acts as the fallback when both are
//! configured, or as the primary when it is the only one). The control plane
//! never proxies artifact bytes: runners upload through short-lived presigned
//! PUT URLs and browsers download through presigned GET URLs, both verified
//! server-side (HeadObject) before rows flip to `uploaded`.
//!
//! Presigned URLs are host-specific, so every stored object carries a
//! `storage_backend` marker ('minio' | 'r2') in its row; reads, deletes and
//! verification always route to the store that actually holds the object
//! ([`Storage::store_for`]). Server-side writes (log archives, logos) fall
//! back to the secondary store when the primary write fails; presigned
//! upload grants pick the primary while a cached HeadBucket probe says it is
//! healthy and the fallback otherwise.

use std::time::{Duration, Instant};

use anyhow::Context;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use uuid::Uuid;

/// Presigned PUT grants expire quickly; the runner uploads immediately.
pub const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
/// Download links are minted per request.
pub const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(10 * 60);
/// How long one primary-health probe result is trusted before re-probing.
const HEALTH_PROBE_TTL: Duration = Duration::from_secs(60);

/// Which physical store an object lives in. The string forms are the DB
/// values of every `storage_backend` marker column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    Minio,
    R2,
}

impl StorageBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            StorageBackend::Minio => "minio",
            StorageBackend::R2 => "r2",
        }
    }

    /// Parse a stored marker. Unknown / legacy values read as R2 — every
    /// pre-marker row was written when R2 was the only store.
    pub fn parse(value: &str) -> StorageBackend {
        match value {
            "minio" => StorageBackend::Minio,
            _ => StorageBackend::R2,
        }
    }
}

impl std::fmt::Display for StorageBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Deterministic object key; every segment is a UUID and the name is
/// allow-list validated by the caller before it gets here.
pub fn artifact_key(workspace_id: Uuid, pipeline_id: Uuid, job_id: Uuid, name: &str) -> String {
    format!("artifacts/{workspace_id}/{pipeline_id}/{job_id}/{name}")
}

/// Deterministic key for a job's archived (gzip) log; every segment is a
/// UUID or an integer.
pub fn log_key(workspace_id: Uuid, pipeline_id: Uuid, job_id: Uuid, attempt: i32) -> String {
    format!("logs/{workspace_id}/{pipeline_id}/{job_id}-{attempt}.log.gz")
}

/// Server-generated key for a workspace logo. Every segment is a UUID;
/// the extension comes from server-side magic-byte detection, never from
/// the client's filename or declared content type.
pub fn logo_key(workspace_id: Uuid, ext: &str) -> String {
    format!("logos/{workspace_id}/{}.{ext}", Uuid::new_v4())
}

/// One S3-compatible store. Generic over the endpoint so MinIO and R2 share
/// every code path; only construction differs (see the [`Self::new`] callers
/// in `services/minio.rs` and `services/r2.rs`).
pub struct S3Store {
    client: aws_sdk_s3::Client,
    bucket: String,
    backend: StorageBackend,
}

impl S3Store {
    /// `force_path_style` is required by MinIO's standard addressing
    /// (`http://host:9000/bucket/key`); R2 uses virtual-host addressing and
    /// leaves it off. Credentials never appear in Debug output or logs.
    pub fn new(
        backend: StorageBackend,
        endpoint_url: &str,
        region: &str,
        access_key_id: &str,
        secret_access_key: &str,
        bucket: &str,
        force_path_style: bool,
    ) -> Self {
        let credentials = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            match backend {
                StorageBackend::Minio => "minio-static",
                StorageBackend::R2 => "r2-static",
            },
        );
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region.to_string()))
            .endpoint_url(endpoint_url)
            .credentials_provider(credentials)
            .force_path_style(force_path_style)
            .build();
        Self {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: bucket.to_string(),
            backend,
        }
    }

    pub fn backend(&self) -> StorageBackend {
        self.backend
    }

    /// Server-side upload for control-plane-generated objects (log
    /// archives, logos). Bodies are small — bounded by the per-job log cap.
    pub async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> anyhow::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(body.into())
            .send()
            .await
            .with_context(|| format!("failed to upload object to {} storage", self.backend))?;
        Ok(())
    }

    /// Best-effort delete; a missing object counts as success (DeleteObject
    /// is idempotent on S3-compatible stores).
    pub async fn delete_object(&self, key: &str) -> anyhow::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .with_context(|| format!("failed to delete object from {} storage", self.backend))?;
        Ok(())
    }

    /// Presigned PUT bound to the exact content type AND content length the
    /// runner declared — both ride the signed headers, so the store rejects
    /// a body of any other size or type at the edge (the after-the-fact
    /// HeadObject check remains as defense-in-depth).
    pub async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
        content_length: i64,
    ) -> anyhow::Result<String> {
        let presigned = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .content_length(content_length)
            .presigned(PresigningConfig::expires_in(UPLOAD_URL_TTL)?)
            .await
            .context("failed to presign artifact upload")?;
        Ok(presigned.uri().to_string())
    }

    pub async fn presign_get(&self, key: &str, filename: &str) -> anyhow::Result<String> {
        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .response_content_disposition(format!("attachment; filename=\"{filename}\""))
            .presigned(PresigningConfig::expires_in(DOWNLOAD_URL_TTL)?)
            .await
            .context("failed to presign artifact download")?;
        Ok(presigned.uri().to_string())
    }

    /// Presigned GET for inline display (no attachment disposition) — used
    /// for workspace logos rendered in `<img>` tags. Same short TTL; the SPA
    /// re-mints through the authenticated logo-url endpoint.
    pub async fn presign_get_inline(&self, key: &str) -> anyhow::Result<String> {
        let presigned = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .response_content_disposition("inline")
            .presigned(PresigningConfig::expires_in(DOWNLOAD_URL_TTL)?)
            .await
            .context("failed to presign inline download")?;
        Ok(presigned.uri().to_string())
    }

    /// Size of the uploaded object, or None when it does not exist.
    pub async fn head_size(&self, key: &str) -> anyhow::Result<Option<i64>> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => Ok(output.content_length()),
            Err(err) => {
                if let aws_sdk_s3::error::SdkError::ServiceError(service_err) = &err
                    && service_err.err().is_not_found()
                {
                    return Ok(None);
                }
                Err(anyhow::Error::new(err).context("artifact HeadObject failed"))
            }
        }
    }

    /// Liveness probe: can this store answer for its bucket right now?
    async fn bucket_reachable(&self) -> bool {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
    }

    /// Create the bucket when it does not exist yet (MinIO deployments
    /// commonly start empty; R2 buckets are dashboard-managed, so callers
    /// only invoke this for MinIO). Warn-only at the call site.
    pub async fn ensure_bucket(&self) -> anyhow::Result<bool> {
        if self.bucket_reachable().await {
            return Ok(false);
        }
        match self
            .client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            // A concurrent boot may have won the race — owned is success.
            Err(aws_sdk_s3::error::SdkError::ServiceError(service_err))
                if matches!(
                    service_err.err(),
                    aws_sdk_s3::operation::create_bucket::CreateBucketError::BucketAlreadyOwnedByYou(_)
                        | aws_sdk_s3::operation::create_bucket::CreateBucketError::BucketAlreadyExists(_)
                ) =>
            {
                Ok(false)
            }
            Err(err) => Err(anyhow::Error::new(err)
                .context(format!("failed to create bucket on {} storage", self.backend))),
        }
    }
}

struct HealthCache {
    checked_at: Option<Instant>,
    healthy: bool,
}

/// The storage router held in `AppState`. `None` in state keeps every
/// clean-denial path exactly as before; when present, `primary` is MinIO
/// whenever MinIO is configured (R2 otherwise) and `fallback` is R2 when
/// both are configured.
pub struct Storage {
    primary: S3Store,
    fallback: Option<S3Store>,
    /// Cached primary liveness so grant routing never probes per-request.
    /// A tokio Mutex on purpose: concurrent expirees coalesce into one probe.
    primary_health: tokio::sync::Mutex<HealthCache>,
}

impl Storage {
    pub fn new(primary: S3Store, fallback: Option<S3Store>) -> Self {
        Self {
            primary,
            fallback,
            primary_health: tokio::sync::Mutex::new(HealthCache {
                checked_at: None,
                healthy: true,
            }),
        }
    }

    pub fn primary_backend(&self) -> StorageBackend {
        self.primary.backend()
    }

    /// The store that owns an object, by its row's `storage_backend` marker.
    /// Unknown/legacy values read as R2; a marker whose store is no longer
    /// configured falls back to the primary (the operation then fails or
    /// misses cleanly instead of panicking on a missing client).
    pub fn store_for(&self, backend: &str) -> &S3Store {
        let wanted = StorageBackend::parse(backend);
        if self.primary.backend() == wanted {
            return &self.primary;
        }
        if let Some(fallback) = &self.fallback
            && fallback.backend() == wanted
        {
            return fallback;
        }
        tracing::debug!(
            wanted = %wanted,
            using = %self.primary.backend(),
            "storage backend marker points at an unconfigured store; using primary"
        );
        &self.primary
    }

    /// Server-side write with reactive fallback: try the primary; if it
    /// fails and a fallback exists, try that. Returns the backend that
    /// accepted the object so the caller can record the marker.
    pub async fn put_object(
        &self,
        key: &str,
        body: Vec<u8>,
        content_type: &str,
    ) -> anyhow::Result<StorageBackend> {
        let Some(fallback) = &self.fallback else {
            self.primary.put_object(key, body, content_type).await?;
            return Ok(self.primary.backend());
        };
        match self.primary.put_object(key, body.clone(), content_type).await {
            Ok(()) => Ok(self.primary.backend()),
            Err(primary_error) => {
                tracing::warn!(
                    key = %key,
                    error = ?primary_error,
                    primary = %self.primary.backend(),
                    fallback = %fallback.backend(),
                    "primary storage write failed; trying fallback"
                );
                fallback
                    .put_object(key, body, content_type)
                    .await
                    .with_context(|| {
                        format!(
                            "both storage backends rejected the write \
                             (primary {}: {primary_error:#})",
                            self.primary.backend()
                        )
                    })?;
                Ok(fallback.backend())
            }
        }
    }

    /// The store presigned upload grants should target. Presigning is an
    /// offline signature (it cannot fail over reactively), so routing keys
    /// off a cached HeadBucket probe of the primary: healthy → primary,
    /// unreachable → fallback. Without a fallback the primary is always
    /// used — exactly the single-store behavior.
    pub async fn store_for_upload(&self) -> &S3Store {
        let Some(fallback) = &self.fallback else {
            return &self.primary;
        };
        let mut cache = self.primary_health.lock().await;
        let stale = cache
            .checked_at
            .is_none_or(|at| at.elapsed() >= HEALTH_PROBE_TTL);
        if stale {
            let healthy = self.primary.bucket_reachable().await;
            if healthy != cache.healthy {
                tracing::warn!(
                    primary = %self.primary.backend(),
                    healthy,
                    "primary storage health changed"
                );
            }
            cache.checked_at = Some(Instant::now());
            cache.healthy = healthy;
        }
        if cache.healthy { &self.primary } else { fallback }
    }

    /// Startup bucket bootstrap: MinIO buckets are created when missing
    /// (a fresh `docker compose` MinIO starts empty); R2 buckets are
    /// dashboard-managed and left alone. Warn-only — storage stays enabled
    /// either way and writes surface their own errors.
    pub async fn ensure_buckets(&self) {
        for store in std::iter::once(&self.primary).chain(self.fallback.as_ref()) {
            if store.backend() != StorageBackend::Minio {
                continue;
            }
            match store.ensure_bucket().await {
                Ok(true) => tracing::info!("created MinIO storage bucket"),
                Ok(false) => {}
                Err(error) => tracing::warn!(
                    error = ?error,
                    "could not verify/create the MinIO bucket; uploads may fail until it exists"
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(backend: StorageBackend) -> S3Store {
        S3Store::new(
            backend,
            "http://localhost:9000",
            "us-east-1",
            "test",
            "test-secret",
            "overup",
            true,
        )
    }

    #[test]
    fn backend_parse_defaults_legacy_values_to_r2() {
        assert_eq!(StorageBackend::parse("minio"), StorageBackend::Minio);
        assert_eq!(StorageBackend::parse("r2"), StorageBackend::R2);
        assert_eq!(StorageBackend::parse(""), StorageBackend::R2);
        assert_eq!(StorageBackend::parse("something-else"), StorageBackend::R2);
    }

    #[test]
    fn store_for_routes_by_marker_and_falls_back_to_primary() {
        let storage = Storage::new(
            store(StorageBackend::Minio),
            Some(store(StorageBackend::R2)),
        );
        assert_eq!(storage.store_for("minio").backend(), StorageBackend::Minio);
        assert_eq!(storage.store_for("r2").backend(), StorageBackend::R2);
        // Legacy/unknown markers read as R2.
        assert_eq!(storage.store_for("legacy").backend(), StorageBackend::R2);

        // R2-only deployment: a minio marker has no store — primary wins.
        let r2_only = Storage::new(store(StorageBackend::R2), None);
        assert_eq!(r2_only.store_for("minio").backend(), StorageBackend::R2);
        assert_eq!(r2_only.store_for("r2").backend(), StorageBackend::R2);
    }

    #[test]
    fn object_keys_are_stable() {
        let ws = Uuid::nil();
        let p = Uuid::nil();
        let j = Uuid::nil();
        assert_eq!(
            artifact_key(ws, p, j, "dist.tar.gz"),
            format!("artifacts/{ws}/{p}/{j}/dist.tar.gz")
        );
        assert_eq!(log_key(ws, p, j, 2), format!("logs/{ws}/{p}/{j}-2.log.gz"));
        assert!(logo_key(ws, "png").starts_with(&format!("logos/{ws}/")));
    }
}
