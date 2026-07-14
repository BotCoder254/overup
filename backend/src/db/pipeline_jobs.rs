use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::pipeline::{PipelineJob, QueueJobRow};

pub async fn list_for_pipeline(pool: &PgPool, pipeline_id: Uuid) -> sqlx::Result<Vec<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        "SELECT * FROM pipeline_jobs WHERE pipeline_id = $1 ORDER BY position",
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await
}

pub async fn find_for_pipeline(
    pool: &PgPool,
    pipeline_id: Uuid,
    job_id: Uuid,
) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        "SELECT * FROM pipeline_jobs WHERE pipeline_id = $1 AND id = $2",
    )
    .bind(pipeline_id)
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

/// A job as reported by a runner: it must actually be assigned to that
/// runner — runner-supplied job ids are never trusted on their own.
pub async fn find_assigned(
    pool: &PgPool,
    job_id: Uuid,
    runner_id: Uuid,
) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        "SELECT * FROM pipeline_jobs WHERE id = $1 AND runner_id = $2 AND status = 'in_progress'",
    )
    .bind(job_id)
    .bind(runner_id)
    .fetch_optional(pool)
    .await
}

/// Jobs ready to schedule: queued, in a live pipeline, with every `needs`
/// dependency concluded successfully. Failed dependencies never appear here
/// because the state machine skips dependents eagerly.
pub async fn find_eligible(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        SELECT j.*
        FROM pipeline_jobs j
        JOIN pipelines p ON p.id = j.pipeline_id
        WHERE j.status = 'queued'
          AND p.status <> 'completed'
          AND NOT EXISTS (
              SELECT 1 FROM pipeline_jobs d
              WHERE d.pipeline_id = j.pipeline_id
                AND d.job_key = ANY(j.needs)
                AND (d.conclusion IS DISTINCT FROM 'success')
          )
        ORDER BY j.queued_at
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Validated filters for the workspace queue view. `search_pattern` is a
/// pre-escaped ILIKE pattern built by the handler — never raw user input.
pub struct QueueFilter {
    pub status: Option<String>,
    pub repository_id: Option<Uuid>,
    pub workflow_id: Option<Uuid>,
    pub runner_id: Option<Uuid>,
    pub label: Option<String>,
    pub search_pattern: Option<String>,
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Active (queued or in-progress) jobs across every live pipeline of a
/// workspace, in scheduler order (oldest queued first). The ascending keyset
/// cursor composes with every filter; `blocked_by_needs` mirrors the
/// scheduler's eligibility predicate so the handler can explain waits.
pub async fn list_queue_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &QueueFilter,
) -> sqlx::Result<Vec<QueueJobRow>> {
    let (cursor_at, cursor_id) = match filter.cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, QueueJobRow>(
        r#"
        SELECT j.*,
               p.number AS pipeline_number,
               p.repository_id AS repository_id,
               r.full_name AS repo_full_name,
               p.workflow_id AS pipeline_workflow_id,
               p.workflow_name AS workflow_name,
               p.git_ref AS git_ref,
               p.trigger AS trigger,
               p.actor_login AS actor_login,
               p.actor_avatar_url AS actor_avatar_url,
               EXISTS (
                   SELECT 1 FROM pipeline_jobs d
                   WHERE d.pipeline_id = j.pipeline_id
                     AND d.job_key = ANY(j.needs)
                     AND (d.conclusion IS DISTINCT FROM 'success')
               ) AS blocked_by_needs
        FROM pipeline_jobs j
        JOIN pipelines p ON p.id = j.pipeline_id
        JOIN repositories r ON r.id = p.repository_id
        WHERE p.workspace_id = $1
          AND p.status <> 'completed'
          AND j.status IN ('queued', 'in_progress')
          AND ($2::text IS NULL OR j.status = $2)
          AND ($3::uuid IS NULL OR p.repository_id = $3)
          AND ($4::uuid IS NULL OR p.workflow_id = $4)
          AND ($5::uuid IS NULL OR j.runner_id = $5)
          AND ($6::text IS NULL OR $6 = ANY(j.runs_on))
          AND ($7::text IS NULL OR
                j.job_key ILIKE $7 ESCAPE '\'
                OR j.name ILIKE $7 ESCAPE '\'
                OR p.workflow_name ILIKE $7 ESCAPE '\')
          AND ($8::timestamptz IS NULL OR (j.queued_at, j.id) > ($8, $9))
        ORDER BY j.queued_at, j.id
        LIMIT $10
        "#,
    )
    .bind(workspace_id)
    .bind(&filter.status)
    .bind(filter.repository_id)
    .bind(filter.workflow_id)
    .bind(filter.runner_id)
    .bind(&filter.label)
    .bind(&filter.search_pattern)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(filter.limit.clamp(1, 100))
    .fetch_all(pool)
    .await
}

/// Aggregate scheduler metrics for the Job Queue page's summary strip,
/// computed over active jobs plus the current runner fleet (same
/// COUNT-FILTER shape as the Dashboard summary).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub queued_total: i64,
    pub queued_blocked: i64,
    pub queued_waiting_runner: i64,
    pub in_progress: i64,
    pub avg_queue_wait_secs: Option<f64>,
    pub max_queue_wait_secs: Option<f64>,
    pub oldest_queued_at: Option<DateTime<Utc>>,
    pub runners_idle: i64,
    pub runners_busy: i64,
    pub runners_offline: i64,
    pub runners_disabled: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct QueueCounts {
    queued_total: i64,
    queued_blocked: i64,
    queued_waiting_runner: i64,
    in_progress: i64,
    avg_queue_wait_secs: Option<f64>,
    max_queue_wait_secs: Option<f64>,
    oldest_queued_at: Option<DateTime<Utc>>,
}

pub async fn queue_summary(
    pool: &PgPool,
    workspace_id: Uuid,
    connected_runner_ids: &[Uuid],
) -> sqlx::Result<QueueSummary> {
    let counts = sqlx::query_as::<_, QueueCounts>(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'queued') AS queued_total,
            COUNT(*) FILTER (WHERE status = 'queued' AND blocked) AS queued_blocked,
            COUNT(*) FILTER (WHERE status = 'queued' AND NOT blocked) AS queued_waiting_runner,
            COUNT(*) FILTER (WHERE status = 'in_progress') AS in_progress,
            AVG(EXTRACT(EPOCH FROM (now() - queued_at))::double precision)
                FILTER (WHERE status = 'queued') AS avg_queue_wait_secs,
            MAX(EXTRACT(EPOCH FROM (now() - queued_at))::double precision)
                FILTER (WHERE status = 'queued') AS max_queue_wait_secs,
            MIN(queued_at) FILTER (WHERE status = 'queued') AS oldest_queued_at
        FROM (
            SELECT j.status, j.queued_at,
                   EXISTS (
                       SELECT 1 FROM pipeline_jobs d
                       WHERE d.pipeline_id = j.pipeline_id
                         AND d.job_key = ANY(j.needs)
                         AND (d.conclusion IS DISTINCT FROM 'success')
                   ) AS blocked
            FROM pipeline_jobs j
            JOIN pipelines p ON p.id = j.pipeline_id
            WHERE p.workspace_id = $1
              AND p.status <> 'completed'
              AND j.status IN ('queued', 'in_progress')
        ) active
        "#,
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;

    // Fleet counts mirror scheduler eligibility, not raw row status: a
    // runner only counts as idle/busy while its socket is live in the hub
    // (`connected_runner_ids`) — a dead connection the stale sweep hasn't
    // reaped yet must not present as claimable capacity (it makes the
    // "idle runners exist, check labels" banner cry wolf). Everything
    // non-revoked that is neither claimable nor disabled reads as offline.
    #[derive(sqlx::FromRow)]
    struct FleetCounts {
        idle: i64,
        busy: i64,
        offline: i64,
        disabled: i64,
    }
    let fleet = sqlx::query_as::<_, FleetCounts>(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'idle' AND draining_at IS NULL
                               AND id = ANY($2::uuid[])) AS idle,
            COUNT(*) FILTER (WHERE status = 'busy'
                               AND id = ANY($2::uuid[])) AS busy,
            COUNT(*) FILTER (WHERE status = 'disabled') AS disabled,
            COUNT(*) FILTER (WHERE status <> 'disabled'
                               AND NOT (status = 'idle' AND draining_at IS NULL
                                          AND id = ANY($2::uuid[]))
                               AND NOT (status = 'busy'
                                          AND id = ANY($2::uuid[]))) AS offline
        FROM runners
        WHERE workspace_id = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(workspace_id)
    .bind(connected_runner_ids)
    .fetch_one(pool)
    .await?;

    Ok(QueueSummary {
        queued_total: counts.queued_total,
        queued_blocked: counts.queued_blocked,
        queued_waiting_runner: counts.queued_waiting_runner,
        in_progress: counts.in_progress,
        avg_queue_wait_secs: counts.avg_queue_wait_secs,
        max_queue_wait_secs: counts.max_queue_wait_secs,
        oldest_queued_at: counts.oldest_queued_at,
        runners_idle: fleet.idle,
        runners_busy: fleet.busy,
        runners_offline: fleet.offline,
        runners_disabled: fleet.disabled,
    })
}

/// Atomically claim one job for one runner: both the job row and the runner
/// row flip inside a single transaction, so concurrent schedulers (or future
/// multi-instance deployments) can never double-assign either side.
pub async fn claim_for_runner(
    pool: &PgPool,
    job_id: Uuid,
    runner_id: Uuid,
) -> sqlx::Result<Option<PipelineJob>> {
    let mut tx = pool.begin().await?;

    let Some(job) = sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'in_progress', stage = 'assigned', runner_id = $2, assigned_at = now()
        WHERE id = $1 AND status = 'queued' AND runner_id IS NULL
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(runner_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };

    let claimed_runner: Option<(Uuid,)> = sqlx::query_as(
        r#"
        UPDATE runners SET status = 'busy'
        WHERE id = $1 AND status = 'idle' AND revoked_at IS NULL
        RETURNING id
        "#,
    )
    .bind(runner_id)
    .fetch_optional(&mut *tx)
    .await?;

    if claimed_runner.is_none() {
        // Runner vanished between selection and claim — abandon both updates.
        return Ok(None);
    }

    tx.commit().await?;
    Ok(Some(job))
}

/// Undo a claim whose job_assign never reached the runner. Only valid while
/// still unacknowledged.
pub async fn unclaim(pool: &PgPool, job_id: Uuid) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'queued', stage = 'queued', runner_id = NULL, assigned_at = NULL
        WHERE id = $1 AND status = 'in_progress' AND stage = 'assigned'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

/// Runner acknowledged the assignment: execution begins.
pub async fn ack(
    pool: &PgPool,
    job_id: Uuid,
    runner_id: Uuid,
) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET stage = 'preparing', started_at = COALESCE(started_at, now())
        WHERE id = $1 AND runner_id = $2 AND status = 'in_progress' AND stage = 'assigned'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(runner_id)
    .fetch_optional(pool)
    .await
}

/// Assignments the runner never acknowledged go back to the queue; the
/// previous runner id is returned so the scheduler can free it.
pub async fn revert_unacked(
    pool: &PgPool,
    cutoff_secs: i64,
) -> sqlx::Result<Vec<(Uuid, Uuid, Option<Uuid>)>> {
    let rows: Vec<(Uuid, Uuid, Option<Uuid>)> = sqlx::query_as(
        r#"
        WITH stale AS (
            SELECT id, runner_id FROM pipeline_jobs
            WHERE status = 'in_progress' AND stage = 'assigned'
              AND assigned_at < now() - make_interval(secs => $1::double precision)
            FOR UPDATE SKIP LOCKED
        )
        UPDATE pipeline_jobs j
        SET status = 'queued', stage = 'queued', runner_id = NULL, assigned_at = NULL
        FROM stale
        WHERE j.id = stale.id
        RETURNING j.id, j.pipeline_id, stale.runner_id
        "#,
    )
    .bind(cutoff_secs as f64)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Stage telemetry from the runner. Fails closed: only the assigned runner
/// can advance an in-progress job.
pub async fn set_stage(
    pool: &PgPool,
    job_id: Uuid,
    runner_id: Uuid,
    stage: &str,
) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET stage = $3
        WHERE id = $1 AND runner_id = $2 AND status = 'in_progress'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(runner_id)
    .bind(stage)
    .fetch_optional(pool)
    .await
}

/// Terminal transition reported by the assigned runner. `metrics` is the
/// server-validated resource telemetry object (never raw runner input).
pub async fn finish_from_runner(
    pool: &PgPool,
    job_id: Uuid,
    runner_id: Uuid,
    conclusion: &str,
    exit_code: Option<i32>,
    error_category: Option<&str>,
    metrics: Option<&serde_json::Value>,
) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = $3, stage = 'done',
            exit_code = $4, error_category = $5,
            metrics = COALESCE($6, metrics),
            started_at = COALESCE(started_at, now()), finished_at = now()
        WHERE id = $1 AND runner_id = $2 AND status = 'in_progress'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(runner_id)
    .bind(conclusion)
    .bind(exit_code)
    .bind(error_category)
    .bind(metrics)
    .fetch_optional(pool)
    .await
}

/// Terminal transition forced by the control plane (timeout with an
/// unresponsive runner, cancellation cleanup).
pub async fn force_finish(
    pool: &PgPool,
    job_id: Uuid,
    conclusion: &str,
    error_category: Option<&str>,
) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = $2, stage = 'done', error_category = $3,
            started_at = COALESCE(started_at, now()), finished_at = now()
        WHERE id = $1 AND status <> 'completed'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .bind(conclusion)
    .bind(error_category)
    .fetch_optional(pool)
    .await
}

/// Skip queued jobs (failed/skipped dependencies). Ids are computed by the
/// state machine's transitive closure.
pub async fn mark_skipped(pool: &PgPool, ids: &[Uuid]) -> sqlx::Result<Vec<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = 'skipped', stage = 'done', finished_at = now()
        WHERE id = ANY($1::uuid[]) AND status = 'queued'
        RETURNING *
        "#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await
}

/// Cancel a single queued job. The status guard makes the transition safe
/// against a concurrent claim: an already-assigned job returns None and the
/// caller falls through to the in-progress cancel path.
pub async fn cancel_one_queued(pool: &PgPool, job_id: Uuid) -> sqlx::Result<Option<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = 'cancelled', stage = 'done', finished_at = now()
        WHERE id = $1 AND status = 'queued'
        RETURNING *
        "#,
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
}

/// Cancel every still-queued job of a pipeline.
pub async fn cancel_queued(pool: &PgPool, pipeline_id: Uuid) -> sqlx::Result<Vec<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = 'cancelled', stage = 'done', finished_at = now()
        WHERE pipeline_id = $1 AND status = 'queued'
        RETURNING *
        "#,
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await
}

pub async fn running_for_pipeline(
    pool: &PgPool,
    pipeline_id: Uuid,
) -> sqlx::Result<Vec<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        "SELECT * FROM pipeline_jobs WHERE pipeline_id = $1 AND status = 'in_progress'",
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await
}

/// Orphan recovery for one disconnected runner: first attempts requeue,
/// repeat offenders fail with a static category.
pub async fn requeue_orphans(
    pool: &PgPool,
    runner_id: Uuid,
) -> sqlx::Result<(Vec<PipelineJob>, Vec<PipelineJob>)> {
    let requeued = sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'queued', stage = 'queued', runner_id = NULL,
            assigned_at = NULL, started_at = NULL, attempt = attempt + 1
        WHERE runner_id = $1 AND status = 'in_progress' AND attempt < 2
        RETURNING *
        "#,
    )
    .bind(runner_id)
    .fetch_all(pool)
    .await?;

    let failed = sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = 'failure', stage = 'done',
            error_category = 'runner_lost',
            started_at = COALESCE(started_at, now()), finished_at = now()
        WHERE runner_id = $1 AND status = 'in_progress'
        RETURNING *
        "#,
    )
    .bind(runner_id)
    .fetch_all(pool)
    .await?;

    Ok((requeued, failed))
}

/// Boot-time orphan recovery: with no runners connected, nothing can
/// legitimately be in progress.
pub async fn requeue_all_orphans(
    pool: &PgPool,
) -> sqlx::Result<(Vec<PipelineJob>, Vec<PipelineJob>)> {
    let requeued = sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'queued', stage = 'queued', runner_id = NULL,
            assigned_at = NULL, started_at = NULL, attempt = attempt + 1
        WHERE status = 'in_progress' AND attempt < 2
        RETURNING *
        "#,
    )
    .fetch_all(pool)
    .await?;

    let failed = sqlx::query_as::<_, PipelineJob>(
        r#"
        UPDATE pipeline_jobs
        SET status = 'completed', conclusion = 'failure', stage = 'done',
            error_category = 'runner_lost',
            started_at = COALESCE(started_at, now()), finished_at = now()
        WHERE status = 'in_progress'
        RETURNING *
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok((requeued, failed))
}

/// Running jobs whose per-job budget lapsed.
pub async fn find_timed_out(pool: &PgPool) -> sqlx::Result<Vec<PipelineJob>> {
    sqlx::query_as::<_, PipelineJob>(
        r#"
        SELECT * FROM pipeline_jobs
        WHERE status = 'in_progress' AND started_at IS NOT NULL
          AND started_at + make_interval(secs => timeout_seconds::double precision) < now()
        "#,
    )
    .fetch_all(pool)
    .await
}

/// Track persisted log volume; the caller enforces the cap on the returned
/// running total.
pub async fn add_log_bytes(pool: &PgPool, job_id: Uuid, delta: i64) -> sqlx::Result<i64> {
    let (total,): (i64,) = sqlx::query_as(
        "UPDATE pipeline_jobs SET log_bytes = log_bytes + $2 WHERE id = $1 RETURNING log_bytes",
    )
    .bind(job_id)
    .bind(delta)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

/// Guarded archive marker: set once, never overwritten.
pub async fn mark_logs_archived(pool: &PgPool, job_id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "UPDATE pipeline_jobs SET logs_archived_at = now() WHERE id = $1 AND logs_archived_at IS NULL",
    )
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Archived jobs whose Postgres log chunks have outlived the hot window and
/// still exist — janitor batch.
pub async fn find_prunable_archived(
    pool: &PgPool,
    hot_days: i64,
    limit: i64,
) -> sqlx::Result<Vec<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT j.id FROM pipeline_jobs j
        WHERE j.logs_archived_at IS NOT NULL
          AND j.finished_at IS NOT NULL
          AND j.finished_at < now() - ($1 * interval '1 day')
          AND EXISTS (SELECT 1 FROM pipeline_log_chunks c WHERE c.job_id = j.id)
        LIMIT $2
        "#,
    )
    .bind(hot_days.max(1))
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// (status, conclusion) of every job — the pipeline finalizer's input.
pub async fn statuses_for_pipeline(
    pool: &PgPool,
    pipeline_id: Uuid,
) -> sqlx::Result<Vec<(String, Option<String>)>> {
    sqlx::query_as("SELECT status, conclusion FROM pipeline_jobs WHERE pipeline_id = $1")
        .bind(pipeline_id)
        .fetch_all(pool)
        .await
}
