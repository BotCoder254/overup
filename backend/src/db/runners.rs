use sqlx::PgPool;
use uuid::Uuid;

use crate::models::runner::Runner;

/// Every column except token_hash — the hash never leaves the database
/// layer's lookup path.
const RUNNER_COLUMNS: &str = "id, workspace_id, name, labels, status, version, \
     last_seen_at, created_by, created_at, revoked_at";

pub enum CreateOutcome {
    Created(Box<Runner>),
    NameTaken,
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

/// Free a busy runner (job finished / assignment reverted).
pub async fn release(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE runners SET status = 'idle' WHERE id = $1 AND status = 'busy'")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Idle runners among the currently connected set, for the scheduler.
pub async fn find_idle_by_ids(pool: &PgPool, ids: &[Uuid]) -> sqlx::Result<Vec<Runner>> {
    sqlx::query_as::<_, Runner>(&format!(
        r#"
        SELECT {RUNNER_COLUMNS} FROM runners
        WHERE id = ANY($1::uuid[]) AND status = 'idle' AND revoked_at IS NULL
        "#,
    ))
    .bind(ids)
    .fetch_all(pool)
    .await
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
