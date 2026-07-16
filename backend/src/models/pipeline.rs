use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Database row for a pipeline (one workflow execution).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Pipeline {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub repository_id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub workflow_name: String,
    pub workflow_path: String,
    pub number: i32,
    pub trigger: String,
    #[allow(dead_code)] // audit column mapped ahead of first read
    pub triggered_by: Option<Uuid>,
    pub commit_sha: String,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub git_ref: String,
    pub trigger_inputs: Option<serde_json::Value>,
    pub status: String,
    pub conclusion: Option<String>,
    #[allow(dead_code)] // enforced in SQL sweeps, mapped for completeness
    pub timeout_seconds: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// List-view row: pipeline plus repository identity.
#[derive(Debug, sqlx::FromRow)]
pub struct PipelineListRow {
    #[sqlx(flatten)]
    pub pipeline: Pipeline,
    pub repo_full_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineResponse {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub workflow_id: Option<Uuid>,
    pub workflow_name: String,
    pub workflow_path: String,
    pub number: i32,
    pub trigger: String,
    pub commit_sha: String,
    pub commit_message: Option<String>,
    pub commit_author: Option<String>,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub git_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_inputs: Option<serde_json::Value>,
    pub status: String,
    pub conclusion: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl PipelineResponse {
    pub fn from_row(pipeline: Pipeline, repo_full_name: String) -> Self {
        Self {
            id: pipeline.id,
            repository_id: pipeline.repository_id,
            repo_full_name,
            workflow_id: pipeline.workflow_id,
            workflow_name: pipeline.workflow_name,
            workflow_path: pipeline.workflow_path,
            number: pipeline.number,
            trigger: pipeline.trigger,
            commit_sha: pipeline.commit_sha,
            commit_message: pipeline.commit_message,
            commit_author: pipeline.commit_author,
            actor_login: pipeline.actor_login,
            actor_avatar_url: pipeline.actor_avatar_url,
            git_ref: pipeline.git_ref,
            trigger_inputs: pipeline.trigger_inputs,
            status: pipeline.status,
            conclusion: pipeline.conclusion,
            created_at: pipeline.created_at,
            started_at: pipeline.started_at,
            finished_at: pipeline.finished_at,
        }
    }
}

impl From<PipelineListRow> for PipelineResponse {
    fn from(row: PipelineListRow) -> Self {
        Self::from_row(row.pipeline, row.repo_full_name)
    }
}

/// Database row for one job inside a pipeline.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PipelineJob {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub job_key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    pub plan: serde_json::Value,
    pub status: String,
    pub conclusion: Option<String>,
    pub stage: String,
    pub runner_id: Option<Uuid>,
    pub attempt: i32,
    pub exit_code: Option<i32>,
    pub error_category: Option<String>,
    pub timeout_seconds: i32,
    pub log_bytes: i64,
    pub position: i32,
    /// Runner-reported resource metrics, clamped server-side before storage.
    pub metrics: Option<serde_json::Value>,
    /// Set once the full log has been archived to object storage; NULL
    /// chunks are the only copy and must never be pruned.
    pub logs_archived_at: Option<DateTime<Utc>>,
    /// Which store holds the archive ('minio' | 'r2'); NULL legacy rows
    /// read as 'r2'. Internal routing marker, never serialized outward.
    pub logs_archive_backend: Option<String>,
    pub queued_at: DateTime<Utc>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineJobResponse {
    pub id: Uuid,
    pub key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    /// Executable plan with confidential-looking env values masked.
    pub plan: serde_json::Value,
    pub status: String,
    pub conclusion: Option<String>,
    pub stage: String,
    pub runner_id: Option<Uuid>,
    pub attempt: i32,
    pub exit_code: Option<i32>,
    pub error_category: Option<String>,
    pub log_bytes: i64,
    pub position: i32,
    /// Resource metrics reported by the runner (camelCase keys), if any.
    pub metrics: Option<serde_json::Value>,
    pub queued_at: DateTime<Utc>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl From<PipelineJob> for PipelineJobResponse {
    fn from(row: PipelineJob) -> Self {
        Self {
            id: row.id,
            key: row.job_key,
            name: row.name,
            runs_on: row.runs_on,
            needs: row.needs,
            plan: sanitize_plan(row.plan),
            status: row.status,
            conclusion: row.conclusion,
            stage: row.stage,
            runner_id: row.runner_id,
            attempt: row.attempt,
            exit_code: row.exit_code,
            error_category: row.error_category,
            log_bytes: row.log_bytes,
            position: row.position,
            metrics: row.metrics,
            queued_at: row.queued_at,
            assigned_at: row.assigned_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
        }
    }
}

/// Queue-view row: an active job joined to its pipeline/repository identity
/// plus the dependency-blocked flag computed in SQL (the same NOT-EXISTS
/// predicate the scheduler's eligibility query uses).
#[derive(Debug, sqlx::FromRow)]
pub struct QueueJobRow {
    #[sqlx(flatten)]
    pub job: PipelineJob,
    pub pipeline_number: i32,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub pipeline_workflow_id: Option<Uuid>,
    pub workflow_name: String,
    pub git_ref: String,
    pub trigger: String,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub blocked_by_needs: bool,
}

/// Slim queue DTO: deliberately excludes `plan` (no env surface at all) and
/// carries a static, server-computed `queue_reason` category string.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueJobResponse {
    pub id: Uuid,
    pub pipeline_id: Uuid,
    pub key: String,
    pub name: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    pub status: String,
    pub conclusion: Option<String>,
    pub stage: String,
    pub runner_id: Option<Uuid>,
    pub attempt: i32,
    pub error_category: Option<String>,
    pub queued_at: DateTime<Utc>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub pipeline_number: i32,
    pub repository_id: Uuid,
    pub repo_full_name: String,
    pub workflow_id: Option<Uuid>,
    pub workflow_name: String,
    pub git_ref: String,
    pub trigger: String,
    pub actor_login: Option<String>,
    pub actor_avatar_url: Option<String>,
    pub queue_reason: &'static str,
}

impl QueueJobResponse {
    pub fn from_row(row: QueueJobRow, queue_reason: &'static str) -> Self {
        Self {
            id: row.job.id,
            pipeline_id: row.job.pipeline_id,
            key: row.job.job_key,
            name: row.job.name,
            runs_on: row.job.runs_on,
            needs: row.job.needs,
            status: row.job.status,
            conclusion: row.job.conclusion,
            stage: row.job.stage,
            runner_id: row.job.runner_id,
            attempt: row.job.attempt,
            error_category: row.job.error_category,
            queued_at: row.job.queued_at,
            assigned_at: row.job.assigned_at,
            started_at: row.job.started_at,
            pipeline_number: row.pipeline_number,
            repository_id: row.repository_id,
            repo_full_name: row.repo_full_name,
            workflow_id: row.pipeline_workflow_id,
            workflow_name: row.workflow_name,
            git_ref: row.git_ref,
            trigger: row.trigger,
            actor_login: row.actor_login,
            actor_avatar_url: row.actor_avatar_url,
            queue_reason,
        }
    }
}

/// Env keys that look confidential are masked in API responses even though
/// plan env only ever comes from workflow YAML (defense in depth).
pub fn looks_confidential(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    ["token", "secret", "password", "key", "credential"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn sanitize_plan(mut plan: serde_json::Value) -> serde_json::Value {
    if let Some(env) = plan.get_mut("env").and_then(|e| e.as_object_mut()) {
        for (key, value) in env.iter_mut() {
            if looks_confidential(key) {
                *value = serde_json::Value::String("***".into());
            }
        }
    }
    plan
}

/// Database row for one entry in the append-only execution ledger.
#[derive(Debug, sqlx::FromRow)]
pub struct PipelineEvent {
    pub id: i64,
    #[allow(dead_code)] // scope column, filtered in queries
    pub pipeline_id: Uuid,
    pub job_id: Option<Uuid>,
    pub event_type: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub runner_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineEventResponse {
    pub id: i64,
    pub job_id: Option<Uuid>,
    pub event_type: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub runner_id: Option<Uuid>,
    pub actor_user_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl From<PipelineEvent> for PipelineEventResponse {
    fn from(row: PipelineEvent) -> Self {
        Self {
            id: row.id,
            job_id: row.job_id,
            event_type: row.event_type,
            from_state: row.from_state,
            to_state: row.to_state,
            runner_id: row.runner_id,
            actor_user_id: row.actor_user_id,
            payload: row.payload,
            created_at: row.created_at,
        }
    }
}

/// Database row for one persisted (already masked) log chunk.
#[derive(Debug, sqlx::FromRow)]
pub struct LogChunk {
    pub seq: i64,
    pub stream: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// 0-based plan step this chunk belongs to (NULL = unsectioned output).
    pub step_index: Option<i16>,
    /// One of protocol::LOG_PHASES (CHECK-constrained in SQL).
    pub phase: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogChunkResponse {
    pub seq: i64,
    pub stream: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub step_index: Option<i16>,
    pub phase: Option<String>,
}

impl From<LogChunk> for LogChunkResponse {
    fn from(row: LogChunk) -> Self {
        Self {
            seq: row.seq,
            stream: row.stream,
            content: row.content,
            created_at: row.created_at,
            step_index: row.step_index,
            phase: row.phase,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_env_masking() {
        let plan = serde_json::json!({
            "image": "ubuntu:24.04",
            "env": { "CI": "true", "NPM_TOKEN": "abc123", "ApiKey": "xyz" }
        });
        let sanitized = sanitize_plan(plan);
        assert_eq!(sanitized["env"]["CI"], "true");
        assert_eq!(sanitized["env"]["NPM_TOKEN"], "***");
        assert_eq!(sanitized["env"]["ApiKey"], "***");
    }
}
