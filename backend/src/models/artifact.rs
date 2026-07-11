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
    /// Server-classified category (package/report/docs/archive/...), never
    /// runner-supplied — see services/artifact_kind.rs.
    pub kind: String,
    pub uncompressed_bytes: Option<i64>,
    pub file_count: Option<i32>,
    /// Archive entry manifest — detail responses only, never list DTOs.
    pub entries: Option<serde_json::Value>,
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
    pub kind: String,
    pub uncompressed_bytes: Option<i64>,
    pub file_count: Option<i32>,
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
            kind: row.kind,
            uncompressed_bytes: row.uncompressed_bytes,
            file_count: row.file_count,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

/// Catalog entry: the artifact plus the provenance of the execution that
/// produced it. `r2_key` stays internal, as everywhere else.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactCatalogResponse {
    #[serde(flatten)]
    pub artifact: ArtifactResponse,
    pub pipeline_number: i32,
    pub repository_id: Uuid,
    pub repository_full_name: String,
    pub workflow_id: Option<Uuid>,
    pub workflow_name: String,
    /// Short branch name (refs/heads/ stripped); other refs pass through.
    pub branch: String,
    pub commit_sha: String,
    pub job_key: String,
    pub job_name: Option<String>,
    pub runner_name: Option<String>,
    /// Docker image the producing job ran in (from the job plan snapshot).
    pub job_image: Option<String>,
}

impl From<crate::db::artifacts::ArtifactCatalogRow> for ArtifactCatalogResponse {
    fn from(row: crate::db::artifacts::ArtifactCatalogRow) -> Self {
        let branch = row
            .git_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&row.git_ref)
            .to_string();
        Self {
            artifact: ArtifactResponse::from(row.artifact),
            pipeline_number: row.pipeline_number,
            repository_id: row.repository_id,
            repository_full_name: row.repo_full_name,
            workflow_id: row.workflow_id,
            workflow_name: row.workflow_name,
            branch,
            commit_sha: row.commit_sha,
            job_key: row.job_key,
            job_name: row.job_name,
            runner_name: row.runner_name,
            job_image: row.job_image,
        }
    }
}
