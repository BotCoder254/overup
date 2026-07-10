use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Database row for artifact metadata. Blobs live in R2; the r2_key is an
/// internal locator and is never exposed to clients.
#[derive(Debug, sqlx::FromRow)]
pub struct Artifact {
    pub id: Uuid,
    #[allow(dead_code)] // authorization scope column, checked in queries
    pub workspace_id: Uuid,
    pub pipeline_id: Uuid,
    pub job_id: Uuid,
    pub name: String,
    pub r2_key: String,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
    pub checksum_sha256: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactResponse {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub job_id: Uuid,
    pub name: String,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
    pub checksum_sha256: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<Artifact> for ArtifactResponse {
    fn from(row: Artifact) -> Self {
        Self {
            id: row.id,
            pipeline_id: row.pipeline_id,
            job_id: row.job_id,
            name: row.name,
            size_bytes: row.size_bytes,
            content_type: row.content_type,
            checksum_sha256: row.checksum_sha256,
            status: row.status,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}
