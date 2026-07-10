use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Database row for a connected repository.
#[derive(Debug, sqlx::FromRow)]
pub struct Repository {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub installation_id: Uuid,
    pub github_repo_id: i64,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub language: Option<String>,
    pub description: Option<String>,
    pub sync_status: String,
    pub sync_error: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    #[allow(dead_code)] // audit column mapped ahead of first read
    pub imported_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

/// List-view row: repository plus its workflow count.
#[derive(Debug, sqlx::FromRow)]
pub struct RepositoryWithCount {
    #[sqlx(flatten)]
    pub repository: Repository,
    pub workflow_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryResponse {
    pub id: Uuid,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub language: Option<String>,
    pub description: Option<String>,
    pub sync_status: String,
    pub sync_error: Option<String>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub workflow_count: i64,
    pub created_at: DateTime<Utc>,
}

impl RepositoryResponse {
    pub fn from_row(repository: Repository, workflow_count: i64) -> Self {
        Self {
            id: repository.id,
            owner: repository.owner,
            name: repository.name,
            full_name: repository.full_name,
            private: repository.private,
            default_branch: repository.default_branch,
            language: repository.language,
            description: repository.description,
            sync_status: repository.sync_status,
            sync_error: repository.sync_error,
            last_synced_at: repository.last_synced_at,
            workflow_count,
            created_at: repository.created_at,
        }
    }
}

impl From<RepositoryWithCount> for RepositoryResponse {
    fn from(row: RepositoryWithCount) -> Self {
        Self::from_row(row.repository, row.workflow_count)
    }
}

/// A repository visible through a linked installation but not necessarily
/// imported yet. Built from the live GitHub response — never persisted.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableRepoResponse {
    pub github_repo_id: i64,
    pub installation_id: Uuid,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub connected: bool,
}

#[derive(Debug, sqlx::FromRow)]
pub struct RepoBranch {
    pub name: String,
    pub commit_sha: String,
    pub is_default: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchResponse {
    pub name: String,
    pub commit_sha: String,
    pub is_default: bool,
    pub updated_at: DateTime<Utc>,
}

impl From<RepoBranch> for BranchResponse {
    fn from(row: RepoBranch) -> Self {
        Self {
            name: row.name,
            commit_sha: row.commit_sha,
            is_default: row.is_default,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct RepoSyncRun {
    pub id: Uuid,
    pub trigger: String,
    pub status: String,
    pub error: Option<String>,
    pub stats: serde_json::Value,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRunResponse {
    pub id: Uuid,
    pub trigger: String,
    pub status: String,
    pub error: Option<String>,
    pub stats: serde_json::Value,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<RepoSyncRun> for SyncRunResponse {
    fn from(row: RepoSyncRun) -> Self {
        Self {
            id: row.id,
            trigger: row.trigger,
            status: row.status,
            error: row.error,
            stats: row.stats,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}
