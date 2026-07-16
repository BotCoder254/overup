use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::pipeline::{Pipeline, PipelineJob, PipelineListRow};
use crate::services::pipeline_plan::PlannedJob;

/// Everything needed to persist a new pipeline. Snapshot fields survive
/// later workflow deletion or resync.
pub struct NewPipeline<'a> {
    pub workspace_id: Uuid,
    pub repository_id: Uuid,
    pub workflow_id: Uuid,
    pub workflow_name: &'a str,
    pub workflow_path: &'a str,
    pub trigger: &'a str,
    pub triggered_by: Option<Uuid>,
    pub commit_sha: &'a str,
    pub commit_message: Option<&'a str>,
    pub commit_author: Option<&'a str>,
    pub actor_login: Option<&'a str>,
    pub actor_avatar_url: Option<&'a str>,
    pub git_ref: &'a str,
    pub trigger_inputs: Option<&'a serde_json::Value>,
    /// Pull request number when trigger = 'pull_request'.
    pub pr_number: Option<i32>,
    pub timeout_seconds: i32,
    pub job_timeout_seconds: i32,
    pub request_id: Option<&'a str>,
}

/// Create the pipeline, its jobs, the creation ledger entry, and the audit
/// row in one transaction. The per-repository number is claimed race-free
/// through pipeline_counters.
pub async fn create(
    pool: &PgPool,
    new: &NewPipeline<'_>,
    jobs: &[PlannedJob],
) -> sqlx::Result<(Pipeline, Vec<PipelineJob>)> {
    let mut tx = pool.begin().await?;

    let (number,): (i32,) = sqlx::query_as(
        r#"
        INSERT INTO pipeline_counters (repository_id, next_number)
        VALUES ($1, 2)
        ON CONFLICT (repository_id)
        DO UPDATE SET next_number = pipeline_counters.next_number + 1
        RETURNING next_number - 1
        "#,
    )
    .bind(new.repository_id)
    .fetch_one(&mut *tx)
    .await?;

    let pipeline = sqlx::query_as::<_, Pipeline>(
        r#"
        INSERT INTO pipelines
            (workspace_id, repository_id, workflow_id, workflow_name, workflow_path,
             number, trigger, triggered_by, commit_sha, commit_message, commit_author,
             actor_login, actor_avatar_url, git_ref, trigger_inputs, pr_number,
             timeout_seconds)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
        RETURNING *
        "#,
    )
    .bind(new.workspace_id)
    .bind(new.repository_id)
    .bind(new.workflow_id)
    .bind(new.workflow_name)
    .bind(new.workflow_path)
    .bind(number)
    .bind(new.trigger)
    .bind(new.triggered_by)
    .bind(new.commit_sha)
    .bind(new.commit_message)
    .bind(new.commit_author)
    .bind(new.actor_login)
    .bind(new.actor_avatar_url)
    .bind(new.git_ref)
    .bind(new.trigger_inputs)
    .bind(new.pr_number)
    .bind(new.timeout_seconds)
    .fetch_one(&mut *tx)
    .await?;

    let mut job_rows = Vec::with_capacity(jobs.len());
    for job in jobs {
        let row = sqlx::query_as::<_, PipelineJob>(
            r#"
            INSERT INTO pipeline_jobs
                (pipeline_id, job_key, name, runs_on, needs, plan, timeout_seconds, position)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(pipeline.id)
        .bind(&job.key)
        .bind(&job.name)
        .bind(&job.runs_on)
        .bind(&job.needs)
        .bind(&job.plan)
        .bind(new.job_timeout_seconds)
        .bind(job.position)
        .fetch_one(&mut *tx)
        .await?;
        job_rows.push(row);
    }

    sqlx::query(
        r#"
        INSERT INTO pipeline_events (pipeline_id, event_type, to_state, actor_user_id, payload)
        VALUES ($1, 'pipeline.created', 'queued', $2, $3)
        "#,
    )
    .bind(pipeline.id)
    .bind(new.triggered_by)
    .bind(serde_json::json!({
        "trigger": new.trigger,
        "commitSha": new.commit_sha,
        "jobs": jobs.len(),
    }))
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'pipeline.created', 'pipeline', $3, $4, $5)
        "#,
    )
    .bind(new.workspace_id)
    .bind(new.triggered_by)
    .bind(pipeline.id)
    .bind(serde_json::json!({
        "workflow": new.workflow_path,
        "trigger": new.trigger,
        "number": number,
    }))
    .bind(new.request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((pipeline, job_rows))
}

pub struct ListFilter {
    pub repository_id: Option<Uuid>,
    pub workflow_id: Option<Uuid>,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub trigger: Option<String>,
    /// Full ref (`refs/heads/<branch>`), exact match.
    pub git_ref: Option<String>,
    pub triggered_by: Option<Uuid>,
    pub runner_id: Option<Uuid>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    /// Pre-escaped ILIKE pattern (`%...%` with \, %, _ escaped) built by the
    /// handler — never raw user input.
    pub search_pattern: Option<String>,
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Keyset-paginated workspace listing, newest first. Every predicate ANDs
/// ahead of the cursor tuple comparison, so filters and pagination compose.
pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &ListFilter,
) -> sqlx::Result<Vec<PipelineListRow>> {
    let (cursor_at, cursor_id) = match filter.cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, PipelineListRow>(
        r#"
        SELECT p.*, r.full_name AS repo_full_name
        FROM pipelines p
        JOIN repositories r ON r.id = p.repository_id
        WHERE p.workspace_id = $1
          AND ($2::uuid IS NULL OR p.repository_id = $2)
          AND ($3::text IS NULL OR p.status = $3)
          AND ($4::timestamptz IS NULL OR (p.created_at, p.id) < ($4, $5))
          AND ($7::uuid IS NULL OR p.workflow_id = $7)
          AND ($8::text IS NULL OR p.conclusion = $8)
          AND ($9::text IS NULL OR p.trigger = $9)
          AND ($10::text IS NULL OR p.git_ref = $10)
          AND ($11::uuid IS NULL OR p.triggered_by = $11)
          AND ($12::uuid IS NULL OR EXISTS (
                SELECT 1 FROM pipeline_jobs j
                WHERE j.pipeline_id = p.id AND j.runner_id = $12))
          AND ($13::timestamptz IS NULL OR p.created_at >= $13)
          AND ($14::timestamptz IS NULL OR p.created_at <= $14)
          AND ($15::text IS NULL OR
                p.commit_message ILIKE $15 ESCAPE '\'
                OR p.commit_sha ILIKE $15 ESCAPE '\'
                OR p.workflow_name ILIKE $15 ESCAPE '\')
        ORDER BY p.created_at DESC, p.id DESC
        LIMIT $6
        "#,
    )
    .bind(workspace_id)
    .bind(filter.repository_id)
    .bind(&filter.status)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(filter.limit.clamp(1, 100))
    .bind(filter.workflow_id)
    .bind(&filter.conclusion)
    .bind(&filter.trigger)
    .bind(&filter.git_ref)
    .bind(filter.triggered_by)
    .bind(filter.runner_id)
    .bind(filter.created_after)
    .bind(filter.created_before)
    .bind(&filter.search_pattern)
    .fetch_all(pool)
    .await
}

pub async fn find_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Pipeline>> {
    sqlx::query_as::<_, Pipeline>("SELECT * FROM pipelines WHERE workspace_id = $1 AND id = $2")
        .bind(workspace_id)
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn find_row_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<PipelineListRow>> {
    sqlx::query_as::<_, PipelineListRow>(
        r#"
        SELECT p.*, r.full_name AS repo_full_name
        FROM pipelines p
        JOIN repositories r ON r.id = p.repository_id
        WHERE p.workspace_id = $1 AND p.id = $2
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<Pipeline>> {
    sqlx::query_as::<_, Pipeline>("SELECT * FROM pipelines WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// First job started: the pipeline is in progress. Idempotent.
pub async fn mark_in_progress(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<Pipeline>> {
    sqlx::query_as::<_, Pipeline>(
        r#"
        UPDATE pipelines
        SET status = 'in_progress', started_at = COALESCE(started_at, now())
        WHERE id = $1 AND status = 'queued'
        RETURNING *
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Terminal transition; the guard makes double-finalization a no-op.
pub async fn finalize(
    pool: &PgPool,
    id: Uuid,
    conclusion: &str,
) -> sqlx::Result<Option<Pipeline>> {
    sqlx::query_as::<_, Pipeline>(
        r#"
        UPDATE pipelines
        SET status = 'completed', conclusion = $2,
            started_at = COALESCE(started_at, now()), finished_at = now()
        WHERE id = $1 AND status <> 'completed'
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(conclusion)
    .fetch_optional(pool)
    .await
}

/// Everything the Checks API reporter needs to address GitHub for one
/// pipeline, resolved in a single join. Returns None when the pipeline (or
/// its repository/installation) is gone — the reporter then no-ops.
#[derive(Debug, sqlx::FromRow)]
pub struct ChecksContext {
    pub workflow_name: String,
    pub commit_sha: String,
    pub trigger: String,
    pub conclusion: Option<String>,
    pub check_run_id: Option<i64>,
    pub repo_owner: String,
    pub repo_name: String,
    pub installation_id: i64,
    pub workspace_slug: String,
}

pub async fn checks_context(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<ChecksContext>> {
    sqlx::query_as::<_, ChecksContext>(
        r#"
        SELECT p.workflow_name, p.commit_sha, p.trigger, p.conclusion,
               p.check_run_id,
               r.owner AS repo_owner, r.name AS repo_name,
               gi.installation_id,
               w.slug AS workspace_slug
        FROM pipelines p
        JOIN repositories r ON r.id = p.repository_id
        JOIN github_installations gi ON gi.id = r.installation_id
        JOIN workspaces w ON w.id = p.workspace_id
        WHERE p.id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Record the GitHub check-run id after a successful create. Guarded so a
/// duplicate create (retry race) never overwrites the first linkage.
pub async fn set_check_run_id(pool: &PgPool, id: Uuid, check_run_id: i64) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE pipelines SET check_run_id = $2 WHERE id = $1 AND check_run_id IS NULL",
    )
    .bind(id)
    .bind(check_run_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Pipelines whose wall-clock budget lapsed; the scheduler sweep times
/// them out.
pub async fn find_timed_out(pool: &PgPool) -> sqlx::Result<Vec<Pipeline>> {
    sqlx::query_as::<_, Pipeline>(
        r#"
        SELECT * FROM pipelines
        WHERE status <> 'completed'
          AND created_at + make_interval(secs => timeout_seconds::double precision) < now()
        "#,
    )
    .fetch_all(pool)
    .await
}
