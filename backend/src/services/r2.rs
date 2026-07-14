//! Cloudflare R2 storage (artifacts + log archives) via the S3 API.
//!
//! The control plane never proxies artifact bytes: runners upload through
//! short-lived presigned PUT URLs and browsers download through presigned
//! GET URLs. Presigning happens client-side against the R2 S3 endpoint
//! (`https://<account>.r2.cloudflarestorage.com`) — the only domain R2
//! presigning supports. Uploads are verified server-side with HeadObject
//! before the artifact row flips to `uploaded`. Log archives are small
//! (bounded by the per-job log cap) and are PUT server-side after a job
//! completes; expired objects are deleted by the janitor.

use std::time::Duration;

use anyhow::Context;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_s3::presigning::PresigningConfig;
use uuid::Uuid;

/// Presigned PUT grants expire quickly; the runner uploads immediately.
pub const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);
/// Download links are minted per request.
pub const DOWNLOAD_URL_TTL: Duration = Duration::from_secs(10 * 60);

pub struct R2 {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl R2 {
    pub fn new(account_id: &str, access_key_id: &str, secret_access_key: &str, bucket: &str) -> Self {
        let credentials = Credentials::new(access_key_id, secret_access_key, None, None, "r2-static");
        let config = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("auto"))
            .endpoint_url(format!("https://{account_id}.r2.cloudflarestorage.com"))
            .credentials_provider(credentials)
            .build();
        Self {
            client: aws_sdk_s3::Client::from_conf(config),
            bucket: bucket.to_string(),
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

    /// Server-side upload for control-plane-generated objects (log
    /// archives). Bodies are small — bounded by the per-job log cap.
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
            .context("failed to upload object to R2")?;
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
            .context("failed to delete object from R2")?;
        Ok(())
    }

    pub async fn presign_put(&self, key: &str, content_type: &str) -> anyhow::Result<String> {
        let presigned = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
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
}
