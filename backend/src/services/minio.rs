//! MinIO flavor of the shared S3 object store.
//!
//! MinIO is a drop-in S3 replacement: only construction differs — an
//! explicit endpoint URL (nothing is derived from an account id) and
//! path-style addressing (`http://host:9000/bucket/key`), which MinIO's
//! standard deployment requires. When configured, MinIO is the DEFAULT /
//! primary object store; R2 (when also configured) becomes the fallback.

use crate::config::MinioConfig;
use crate::services::object_store::{S3Store, StorageBackend};

pub fn store(config: &MinioConfig) -> S3Store {
    S3Store::new(
        StorageBackend::Minio,
        &config.endpoint,
        &config.region,
        &config.access_key_id,
        &config.secret_access_key,
        &config.bucket,
        config.force_path_style,
    )
}
