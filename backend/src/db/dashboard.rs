use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Aggregate counts for the Dashboard's KPI strip, computed over pipelines
/// created since `since` plus the current runner fleet (runner status is a
/// present-tense snapshot, not windowed by time).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummary {
    pub pipelines_total: i64,
    pub pipelines_succeeded: i64,
    pub pipelines_failed: i64,
    pub pipelines_cancelled: i64,
    pub pipelines_in_progress: i64,
    pub pipelines_queued: i64,
    pub success_rate: f64,
    pub avg_duration_secs: Option<f64>,
    pub runners_total: i64,
    pub runners_idle: i64,
    pub runners_busy: i64,
    pub runners_offline: i64,
    pub runners_disabled: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct PipelineCounts {
    succeeded: i64,
    failed: i64,
    cancelled: i64,
    in_progress: i64,
    queued: i64,
    total: i64,
    avg_duration_secs: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct RunnerCount {
    status: String,
    count: i64,
}

pub async fn summary(pool: &PgPool, workspace_id: Uuid, since: DateTime<Utc>) -> sqlx::Result<DashboardSummary> {
    let pipeline_counts = sqlx::query_as::<_, PipelineCounts>(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE conclusion = 'success') AS succeeded,
            COUNT(*) FILTER (WHERE conclusion IN ('failure', 'timed_out')) AS failed,
            COUNT(*) FILTER (WHERE conclusion = 'cancelled') AS cancelled,
            COUNT(*) FILTER (WHERE status = 'in_progress') AS in_progress,
            COUNT(*) FILTER (WHERE status = 'queued') AS queued,
            COUNT(*) AS total,
            AVG(EXTRACT(EPOCH FROM (finished_at - started_at))::double precision)
                FILTER (WHERE finished_at IS NOT NULL AND started_at IS NOT NULL) AS avg_duration_secs
        FROM pipelines
        WHERE workspace_id = $1 AND created_at >= $2
        "#,
    )
    .bind(workspace_id)
    .bind(since)
    .fetch_one(pool)
    .await?;

    let runner_counts: Vec<RunnerCount> = sqlx::query_as(
        r#"
        SELECT status, COUNT(*) AS count
        FROM runners
        WHERE workspace_id = $1 AND revoked_at IS NULL
        GROUP BY status
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    let runner_count = |status: &str| {
        runner_counts
            .iter()
            .find(|row| row.status == status)
            .map(|row| row.count)
            .unwrap_or(0)
    };

    let closed = pipeline_counts.succeeded + pipeline_counts.failed + pipeline_counts.cancelled;
    let success_rate = if closed > 0 {
        pipeline_counts.succeeded as f64 / closed as f64
    } else {
        0.0
    };

    Ok(DashboardSummary {
        pipelines_total: pipeline_counts.total,
        pipelines_succeeded: pipeline_counts.succeeded,
        pipelines_failed: pipeline_counts.failed,
        pipelines_cancelled: pipeline_counts.cancelled,
        pipelines_in_progress: pipeline_counts.in_progress,
        pipelines_queued: pipeline_counts.queued,
        success_rate,
        avg_duration_secs: pipeline_counts.avg_duration_secs,
        runners_total: runner_counts.iter().map(|row| row.count).sum(),
        runners_idle: runner_count("idle"),
        runners_busy: runner_count("busy"),
        runners_offline: runner_count("offline"),
        runners_disabled: runner_count("disabled"),
    })
}

/// Pre-bucketed pipeline activity for the Dashboard's charts. `trunc` must be
/// one of `"hour"`/`"day"` — it is selected server-side from an allow-listed
/// range enum by the caller, never taken from user text, since it lands in
/// the SQL identifier position of `date_trunc`.
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ActivityBucket {
    pub bucket: DateTime<Utc>,
    pub succeeded: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub queued: i64,
}

pub async fn activity_buckets(
    pool: &PgPool,
    workspace_id: Uuid,
    since: DateTime<Utc>,
    trunc: &'static str,
) -> sqlx::Result<Vec<ActivityBucket>> {
    let sql = format!(
        r#"
        SELECT
            date_trunc('{trunc}', created_at) AS bucket,
            COUNT(*) FILTER (WHERE conclusion = 'success') AS succeeded,
            COUNT(*) FILTER (WHERE conclusion IN ('failure', 'timed_out')) AS failed,
            COUNT(*) FILTER (WHERE conclusion = 'cancelled') AS cancelled,
            COUNT(*) FILTER (WHERE status <> 'completed') AS queued
        FROM pipelines
        WHERE workspace_id = $1 AND created_at >= $2
        GROUP BY 1
        ORDER BY 1
        "#
    );
    sqlx::query_as(&sql)
        .bind(workspace_id)
        .bind(since)
        .fetch_all(pool)
        .await
}
