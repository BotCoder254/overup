use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Catalog row: one workflow joined with its repository identity.
#[derive(Debug, sqlx::FromRow)]
pub struct WorkflowSummaryRow {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub path: String,
    pub name: String,
    pub triggers: Vec<String>,
    pub validation_status: String,
    pub job_count: i32,
    pub last_commit_sha: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummaryResponse {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub path: String,
    pub name: String,
    pub triggers: Vec<String>,
    pub validation_status: String,
    pub job_count: i32,
    pub last_commit_sha: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl From<WorkflowSummaryRow> for WorkflowSummaryResponse {
    fn from(row: WorkflowSummaryRow) -> Self {
        Self {
            id: row.id,
            repository_id: row.repository_id,
            repo_full_name: row.repo_full_name,
            path: row.path,
            name: row.name,
            triggers: row.triggers,
            validation_status: row.validation_status,
            job_count: row.job_count,
            last_commit_sha: row.last_commit_sha,
            last_commit_at: row.last_commit_at,
            updated_at: row.updated_at,
        }
    }
}

/// Full detail row, raw YAML included.
#[derive(Debug, sqlx::FromRow)]
pub struct WorkflowDetailRow {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub default_branch: String,
    pub path: String,
    pub name: String,
    pub file_size: i32,
    pub raw_content: String,
    pub triggers: Vec<String>,
    pub metadata: serde_json::Value,
    pub job_count: i32,
    pub validation_status: String,
    pub validation_errors: serde_json::Value,
    pub last_commit_sha: Option<String>,
    pub last_commit_message: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct WorkflowJobRow {
    pub job_key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    pub uses: Option<String>,
    pub strategy: Option<serde_json::Value>,
    pub step_count: i32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowJobResponse {
    pub key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    pub uses: Option<String>,
    pub strategy: Option<serde_json::Value>,
    pub step_count: i32,
}

impl From<WorkflowJobRow> for WorkflowJobResponse {
    fn from(row: WorkflowJobRow) -> Self {
        Self {
            key: row.job_key,
            name: row.name,
            runs_on: row.runs_on,
            needs: row.needs,
            uses: row.uses,
            strategy: row.strategy,
            step_count: row.step_count,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDetailResponse {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub default_branch: String,
    pub path: String,
    pub name: String,
    pub file_size: i32,
    pub raw_content: String,
    pub triggers: Vec<String>,
    pub metadata: serde_json::Value,
    pub job_count: i32,
    pub validation_status: String,
    pub validation_errors: serde_json::Value,
    pub last_commit_sha: Option<String>,
    pub last_commit_message: Option<String>,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub jobs: Vec<WorkflowJobResponse>,
}

impl WorkflowDetailResponse {
    pub fn from_rows(row: WorkflowDetailRow, jobs: Vec<WorkflowJobRow>) -> Self {
        Self {
            id: row.id,
            repository_id: row.repository_id,
            repo_full_name: row.repo_full_name,
            default_branch: row.default_branch,
            path: row.path,
            name: row.name,
            file_size: row.file_size,
            raw_content: row.raw_content,
            triggers: row.triggers,
            metadata: row.metadata,
            job_count: row.job_count,
            validation_status: row.validation_status,
            validation_errors: row.validation_errors,
            last_commit_sha: row.last_commit_sha,
            last_commit_message: row.last_commit_message,
            last_commit_at: row.last_commit_at,
            updated_at: row.updated_at,
            jobs: jobs.into_iter().map(WorkflowJobResponse::from).collect(),
        }
    }
}
