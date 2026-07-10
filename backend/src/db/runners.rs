use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::runner::Runner;

/// Every column except token_hash — the hash never leaves the database
/// layer's lookup path.
const RUNNER_COLUMNS: &str = "id, workspace_id, name, labels, status, version, \
     last_seen_at, created_by, created_at, revoked_at, last_health, last_health_at, draining_at";

pub enum CreateOutcome {
    Created(Box<Runner>),
    NameTaken,
}

pub enum UpdateOutcome {
    Updated(Box<Runner>),
    NameTaken,
    NotFound,
}

fn is_unique_violation(err: &sqlx::Error, constraint: &str) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some(constraint)
    )
}

pub async fn create(
    pool: &PgPool,
    workspace_id: Uuid,
    name: &str,
    labels: &[String],
    token_hash: &str,
    created_by: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<CreateOutcome> {
    let mut tx = pool.begin().await?;

    let runner = match sqlx::query_as::<_, Runner>(&format!(
        r#"
        INSERT INTO runners (workspace_id, name, labels, token_hash, created_by)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING {RUNNER_COLUMNS}
        "#,
    ))
    .bind(workspace_id)
    .bind(name)
    .bind(labels)
    .bind(token_hash)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(runner) => runner,
        Err(err) if is_unique_violation(&err, "runners_ws_name_key") => {
            return Ok(CreateOutcome::NameTaken);
        }
        Err(err) => return Err(err),
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.created', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(created_by)
    .bind(runner.id)
    .bind(serde_json::json!({ "name": name, "labels": labels }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(CreateOutcome::Created(Box::new(runner)))
}

pub async fn list_for_workspace(pool: &PgPool, workspace_id: Uuid) -> sqlx::Result<Vec<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        "SELECT {RUNNER_COLUMNS} FROM runners WHERE workspace_id = $1 ORDER BY created_at",
    ))
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// Parameters for [`create_bootstrap`], bundled to keep the function
/// signature within clippy's argument-count lint.
pub struct CreateBootstrapParams<'a> {
    pub workspace_id: Uuid,
    pub name: &'a str,
    pub labels: &'a [String],
    pub bootstrap_token_hash: &'a str,
    pub bootstrap_expires_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub request_id: Option<&'a str>,
}

/// Create a runner in pending-registration state: no permanent token yet,
/// only a short-lived bootstrap credential the wizard shows once.
pub async fn create_bootstrap(
    pool: &PgPool,
    params: CreateBootstrapParams<'_>,
) -> sqlx::Result<CreateOutcome> {
    let CreateBootstrapParams {
        workspace_id,
        name,
        labels,
        bootstrap_token_hash,
        bootstrap_expires_at,
        created_by,
        request_id,
    } = params;
    let mut tx = pool.begin().await?;

    let runner = match sqlx::query_as::<_, Runner>(&format!(
        r#"
        INSERT INTO runners
            (workspace_id, name, labels, token_hash, bootstrap_token_hash, bootstrap_expires_at, created_by)
        VALUES ($1, $2, $3, NULL, $4, $5, $6)
        RETURNING {RUNNER_COLUMNS}
        "#,
    ))
    .bind(workspace_id)
    .bind(name)
    .bind(labels)
    .bind(bootstrap_token_hash)
    .bind(bootstrap_expires_at)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(runner) => runner,
        Err(err) if is_unique_violation(&err, "runners_ws_name_key") => {
            return Ok(CreateOutcome::NameTaken);
        }
        Err(err) => return Err(err),
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.created', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(created_by)
    .bind(runner.id)
    .bind(serde_json::json!({ "name": name, "labels": labels, "bootstrap": true }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(CreateOutcome::Created(Box::new(runner)))
}

/// Bearer-token lookup for a still-pending bootstrap credential.
pub async fn find_by_bootstrap_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> sqlx::Result<Option<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        r#"
        SELECT {RUNNER_COLUMNS} FROM runners
        WHERE bootstrap_token_hash = $1 AND revoked_at IS NULL
          AND bootstrap_expires_at > now()
        "#,
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

/// One-shot exchange: a bootstrap credential becomes the permanent one.
/// The `WHERE bootstrap_token_hash IS NOT NULL` guard makes this atomic
/// against a racing second connection with the same bootstrap token — only
/// one wins.
pub async fn exchange_bootstrap_token(
    pool: &PgPool,
    id: Uuid,
    new_token_hash: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE runners
        SET token_hash = $2, bootstrap_token_hash = NULL, bootstrap_expires_at = NULL
        WHERE id = $1 AND bootstrap_token_hash IS NOT NULL
        "#,
    )
    .bind(id)
    .bind(new_token_hash)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Sweep: a bootstrap credential that was never exchanged is a dead runner
/// row (no permanent identity was ever established) — delete it outright
/// rather than leaving an inert placeholder behind.
pub async fn purge_expired_bootstrap(pool: &PgPool) -> sqlx::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM runners
        WHERE token_hash IS NULL AND bootstrap_expires_at IS NOT NULL
          AND bootstrap_expires_at < now()
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub async fn find_by_id(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        "SELECT {RUNNER_COLUMNS} FROM runners WHERE workspace_id = $1 AND id = $2",
    ))
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Authenticate a runner connection: the token hash must match a live,
/// non-revoked runner. Lookup by hash of a 32-byte OS-RNG token — the same
/// non-guessable design as browser sessions.
pub async fn find_by_token_hash(pool: &PgPool, token_hash: &str) -> sqlx::Result<Option<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        "SELECT {RUNNER_COLUMNS} FROM runners WHERE token_hash = $1 AND revoked_at IS NULL",
    ))
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

/// Revoke a runner: it can never authenticate again. Its in-flight jobs are
/// orphaned by the caller.
pub async fn revoke_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let revoked: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE runners
        SET revoked_at = now(), status = 'offline'
        WHERE workspace_id = $1 AND id = $2 AND revoked_at IS NULL
        RETURNING name
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((name,)) = revoked else {
        return Ok(false);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.revoked', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "name": name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Rename/relabel a runner. Either field may be omitted (left unchanged).
pub async fn update(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    name: Option<&str>,
    labels: Option<&[String]>,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<UpdateOutcome> {
    let mut tx = pool.begin().await?;

    let runner = match sqlx::query_as::<_, Runner>(&format!(
        r#"
        UPDATE runners
        SET name = COALESCE($3, name), labels = COALESCE($4, labels)
        WHERE workspace_id = $1 AND id = $2 AND revoked_at IS NULL
        RETURNING {RUNNER_COLUMNS}
        "#,
    ))
    .bind(workspace_id)
    .bind(id)
    .bind(name)
    .bind(labels)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(Some(runner)) => runner,
        Ok(None) => return Ok(UpdateOutcome::NotFound),
        Err(err) if is_unique_violation(&err, "runners_ws_name_key") => {
            return Ok(UpdateOutcome::NameTaken);
        }
        Err(err) => return Err(err),
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.updated', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "name": runner.name, "labels": runner.labels }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(UpdateOutcome::Updated(Box::new(runner)))
}

/// Issue a new registration token for an existing runner, invalidating the
/// old one. The caller is responsible for severing any live connection.
pub async fn regenerate_token(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    new_token_hash: &str,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let updated: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE runners
        SET token_hash = $3
        WHERE workspace_id = $1 AND id = $2 AND revoked_at IS NULL
        RETURNING name
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .bind(new_token_hash)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((name,)) = updated else {
        return Ok(false);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.token_regenerated', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "name": name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// A runner connected and said hello: it is idle and reachable.
pub async fn mark_connected(pool: &PgPool, id: Uuid, version: &str) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE runners
        SET status = 'idle', version = $2, last_seen_at = now()
        WHERE id = $1 AND revoked_at IS NULL
        "#,
    )
    .bind(id)
    .bind(version)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn touch_last_seen(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE runners SET last_seen_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Same as [`touch_last_seen`] but also records freshly-sanitized health
/// telemetry from a heartbeat, in one round-trip.
pub async fn touch_last_seen_with_health(
    pool: &PgPool,
    id: Uuid,
    health: &serde_json::Value,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE runners SET last_seen_at = now(), last_health = $2, last_health_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(health)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_offline(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE runners SET status = 'offline' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Startup recovery: nothing can be connected before the hub exists.
pub async fn mark_all_offline(pool: &PgPool) -> sqlx::Result<()> {
    sqlx::query("UPDATE runners SET status = 'offline' WHERE status <> 'offline'")
        .execute(pool)
        .await?;
    Ok(())
}

/// Free a busy runner (job finished / assignment reverted). A runner that
/// was draining goes offline instead of idle — it must not pick up new
/// work — and the one-shot drain flag clears.
pub async fn release(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE runners
        SET status = CASE WHEN draining_at IS NOT NULL THEN 'offline' ELSE 'idle' END,
            draining_at = NULL
        WHERE id = $1 AND status = 'busy'
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Idle runners among the currently connected set, for the scheduler.
/// Draining runners are excluded even if technically idle (no current job
/// to wait for, but the operator asked them to stop taking new work).
pub async fn find_idle_by_ids(pool: &PgPool, ids: &[Uuid]) -> sqlx::Result<Vec<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        r#"
        SELECT {RUNNER_COLUMNS} FROM runners
        WHERE id = ANY($1::uuid[]) AND status = 'idle' AND draining_at IS NULL
          AND revoked_at IS NULL
        "#,
    ))
    .bind(ids)
    .fetch_all(pool)
    .await
}

/// Stop scheduling new work onto a runner immediately: `idle` goes straight
/// to `offline` (nothing to finish); `busy` stays busy but is flagged so
/// [`release`] sends it `offline` once its current job concludes. A no-op
/// (returns `false`) for runners that are already offline/disabled/draining.
pub async fn drain_start(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let drained: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE runners
        SET draining_at = now(),
            status = CASE WHEN status = 'idle' THEN 'offline' ELSE status END
        WHERE workspace_id = $1 AND id = $2 AND revoked_at IS NULL
          AND draining_at IS NULL AND status IN ('idle', 'busy')
        RETURNING name
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((name,)) = drained else {
        return Ok(false);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.drained', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "name": name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Stop scheduling new work onto a runner and prevent it from being matched
/// again until resumed. Unlike revoke, this is fully reversible and does not
/// disturb a job already in flight (it simply won't be handed another).
pub async fn disable(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let disabled: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE runners
        SET status = 'disabled'
        WHERE workspace_id = $1 AND id = $2 AND revoked_at IS NULL AND status <> 'disabled'
        RETURNING name
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((name,)) = disabled else {
        return Ok(false);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.disabled', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "name": name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Undo [`disable`]. `connected` (the runner's live WS presence) decides
/// whether it comes back as `idle` (ready for work) or `offline` (disabled
/// while disconnected — nothing to resume onto until it reconnects).
pub async fn resume(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    connected: bool,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let resumed: Option<(String,)> = sqlx::query_as(
        r#"
        UPDATE runners
        SET status = CASE WHEN $3 THEN 'idle' ELSE 'offline' END
        WHERE workspace_id = $1 AND id = $2 AND revoked_at IS NULL AND status = 'disabled'
        RETURNING name
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .bind(connected)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((name,)) = resumed else {
        return Ok(false);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'runner.resumed', 'runner', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "name": name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Runners that stopped heartbeating but were never marked offline (e.g. a
/// TCP half-open). The sweep turns them offline and orphans their jobs.
pub async fn find_stale(pool: &PgPool, cutoff_secs: i64) -> sqlx::Result<Vec<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        r#"
        SELECT {RUNNER_COLUMNS} FROM runners
        WHERE status <> 'offline'
          AND (last_seen_at IS NULL OR last_seen_at < now() - make_interval(secs => $1))
        "#,
    ))
    .bind(cutoff_secs as f64)
    .fetch_all(pool)
    .await
}
