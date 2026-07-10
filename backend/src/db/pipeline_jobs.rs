use sqlx::PgPool;
use uuid::Uuid;

use crate::models::pipeline::PipelineJob;

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
