//! Cloudflare R2 flavor of the shared S3 object store.
//!
//! R2 presigning only works against the account-scoped S3 endpoint
//! (`https://<account>.r2.cloudflarestorage.com`), with the literal region
//! `auto` and virtual-host addressing. Everything else — presigned PUT/GET,
//! HeadObject verification, deletes — is the generic
//! [`crate::services::object_store::S3Store`]. R2 stays fully supported:
//! it is the primary store when it is the only one configured, and the
//! fallback when MinIO is also present.

use crate::config::R2Config;
use crate::services::object_store::{S3Store, StorageBackend};

pub fn store(config: &R2Config) -> S3Store {
    S3Store::new(
        StorageBackend::R2,
        &format!("https://{}.r2.cloudflarestorage.com", config.account_id),
        "auto",
        &config.access_key_id,
        &config.secret_access_key,
        &config.bucket,
        false,
    )
}
