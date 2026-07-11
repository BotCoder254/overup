use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::runner::Runner;

/// Every column except token_hash — the hash never leaves the database
/// layer's lookup path.
const RUNNER_COLUMNS: &str = "id, workspace_id, name, labels, status, version, \
     last_seen_at, created_by, created_at, revoked_at, last_health, last_health_at, \
     draining_at, managed, container_id, provision_error, resource_profile";

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
    /// True for hosted runners the control plane provisions itself.
    pub managed: bool,
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
        managed,
    } = params;
    let mut tx = pool.begin().await?;

    let runner = match sqlx::query_as::<_, Runner>(&format!(
        r#"
        INSERT INTO runners
            (workspace_id, name, labels, token_hash, bootstrap_token_hash, bootstrap_expires_at, created_by, managed)
        VALUES ($1, $2, $3, NULL, $4, $5, $6, $7)
        RETURNING {RUNNER_COLUMNS}
        "#,
    ))
    .bind(workspace_id)
    .bind(name)
    .bind(labels)
    .bind(bootstrap_token_hash)
    .bind(bootstrap_expires_at)
    .bind(created_by)
    .bind(managed)
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

pub enum HostedCreateOutcome {
    Created(Vec<Runner>),
    NameTaken,
    QuotaExceeded,
}

/// Serializes hosted-runner creation so concurrent requests can't both pass
/// the quota check. Fixed constant — never derived from input.
const HOSTED_CREATE_LOCK_KEY: i64 = 4_859_234_701;

/// Parameters for [`create_hosted_pending`], bundled to keep the function
/// signature within clippy's argument-count lint.
pub struct CreateHostedPendingParams<'a> {
    pub workspace_id: Uuid,
    /// Used verbatim when `instances == 1`, suffixed `-1..-N` otherwise.
    pub base_name: &'a str,
    pub labels: &'a [String],
    /// Validated preset slug (`small` | `standard` | `large`).
    pub resource_profile: &'a str,
    /// How many runner rows/containers to create (>= 1, handler-validated).
    pub instances: u32,
    pub created_by: Uuid,
    pub request_id: Option<&'a str>,
}

/// Create hosted runner rows in *pending* state — no credential of any kind
/// exists yet (`token_hash` and `bootstrap_token_hash` both NULL). The
/// bootstrap credential is minted just-in-time by the provisioning task and
/// armed via [`arm_bootstrap`] right before the container starts, so no
/// plaintext token ever spans the (potentially minutes-long) image pull.
///
/// Per-workspace and global quota enforcement: the count + inserts run under
/// a transaction-scoped advisory lock, so the quota can never be
/// oversubscribed by a race; the lock only serializes hosted creations
/// (rare, already rate-limited). A name collision on any instance rolls the
/// whole batch back — creation is all-or-nothing.
pub async fn create_hosted_pending(
    pool: &PgPool,
    params: CreateHostedPendingParams<'_>,
    max_per_workspace: i64,
    max_global: i64,
) -> sqlx::Result<HostedCreateOutcome> {
    let CreateHostedPendingParams {
        workspace_id,
        base_name,
        labels,
        resource_profile,
        instances,
        created_by,
        request_id,
    } = params;
    let instances = instances.max(1);
    let mut tx = pool.begin().await?;

    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(HOSTED_CREATE_LOCK_KEY)
        .execute(&mut *tx)
        .await?;

    let (ws_count, global_count): (i64, i64) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FILTER (WHERE workspace_id = $1), COUNT(*)
        FROM runners WHERE managed AND revoked_at IS NULL
        "#,
    )
    .bind(workspace_id)
    .fetch_one(&mut *tx)
    .await?;
    if ws_count + i64::from(instances) > max_per_workspace
        || global_count + i64::from(instances) > max_global
    {
        return Ok(HostedCreateOutcome::QuotaExceeded);
    }

    let mut runners = Vec::with_capacity(instances as usize);
    for i in 1..=instances {
        let name = if instances == 1 {
            base_name.to_string()
        } else {
            format!("{base_name}-{i}")
        };

        let runner = match sqlx::query_as::<_, Runner>(&format!(
            r#"
            INSERT INTO runners
                (workspace_id, name, labels, token_hash, bootstrap_token_hash,
                 bootstrap_expires_at, created_by, managed, resource_profile)
            VALUES ($1, $2, $3, NULL, NULL, NULL, $4, true, $5)
            RETURNING {RUNNER_COLUMNS}
            "#,
        ))
        .bind(workspace_id)
        .bind(&name)
        .bind(labels)
        .bind(created_by)
        .bind(resource_profile)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(runner) => runner,
            Err(err) if is_unique_violation(&err, "runners_ws_name_key") => {
                return Ok(HostedCreateOutcome::NameTaken);
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
        .bind(serde_json::json!({
            "name": name,
            "labels": labels,
            "managed": true,
            "resourceProfile": resource_profile,
        }))
        .bind(request_id)
        .execute(&mut *tx)
        .await?;

        runners.push(runner);
    }

    tx.commit().await?;
    Ok(HostedCreateOutcome::Created(runners))
}

/// Arm a pending hosted runner's bootstrap credential just-in-time — called
/// by the provisioning task right before the container is created, never
/// earlier. Returns `false` when the row was revoked/purged/failed in the
/// meantime; the caller must NOT create a container in that case.
pub async fn arm_bootstrap(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    bootstrap_token_hash: &str,
    expires_at: DateTime<Utc>,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE runners
        SET bootstrap_token_hash = $3, bootstrap_expires_at = $4
        WHERE workspace_id = $1 AND id = $2 AND managed AND revoked_at IS NULL
          AND token_hash IS NULL AND bootstrap_token_hash IS NULL
          AND provision_error IS NULL
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .bind(bootstrap_token_hash)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Managed rows that never got armed (no credential of any kind) and are old
/// enough that no provisioning task can still be working on them. The
/// expiry-based purge can't catch these — `bootstrap_expires_at` stays NULL
/// until arm time. `container_id` is returned defensively (arm precedes
/// container creation, so it should always be NULL here).
pub async fn find_stale_pending_managed(
    pool: &PgPool,
    min_age_hours: i64,
) -> sqlx::Result<Vec<(Uuid, Option<String>)>> {
    sqlx::query_as(
        r#"
        SELECT id, container_id FROM runners
        WHERE managed AND token_hash IS NULL AND bootstrap_token_hash IS NULL
          AND created_at < now() - make_interval(hours => $1)
        "#,
    )
    .bind(min_age_hours as i32)
    .fetch_all(pool)
    .await
}

/// Sweep for [`find_stale_pending_managed`] rows (same predicate).
pub async fn purge_stale_pending_managed(pool: &PgPool, min_age_hours: i64) -> sqlx::Result<u64> {
    let result = sqlx::query(
        r#"
        DELETE FROM runners
        WHERE managed AND token_hash IS NULL AND bootstrap_token_hash IS NULL
          AND created_at < now() - make_interval(hours => $1)
        "#,
    )
    .bind(min_age_hours as i32)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Audit entry for a control-plane-initiated runner event (provisioning
/// outcomes). `actor_user_id` is NULL — the system, not a user, acted.
/// `action` and `metadata` values must be static/sanitized (never Docker
/// text).
pub async fn insert_system_runner_audit(
    pool: &PgPool,
    workspace_id: Uuid,
    runner_id: Uuid,
    action: &str,
    metadata: serde_json::Value,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
        VALUES ($1, NULL, $2, 'runner', $3, $4)
        "#,
    )
    .bind(workspace_id)
    .bind(action)
    .bind(runner_id)
    .bind(metadata)
    .execute(pool)
    .await?;
    Ok(())
}

/// (workspace, global) counts of live managed runners — the wizard's
/// remaining-quota display.
pub async fn count_hosted(pool: &PgPool, workspace_id: Uuid) -> sqlx::Result<(i64, i64)> {
    sqlx::query_as(
        r#"
        SELECT COUNT(*) FILTER (WHERE workspace_id = $1), COUNT(*)
        FROM runners WHERE managed AND revoked_at IS NULL
        "#,
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await
}

/// Every live managed runner: (id, container_id, status) — the janitor's
/// reconciliation input.
pub async fn list_managed_live(
    pool: &PgPool,
) -> sqlx::Result<Vec<(Uuid, Option<String>, String)>> {
    sqlx::query_as(
        "SELECT id, container_id, status FROM runners WHERE managed AND revoked_at IS NULL",
    )
    .fetch_all(pool)
    .await
}

/// Reconciler: a managed runner's container vanished from the Docker host.
/// Guarded to offline rows so a live, connected runner is never flagged.
/// (Deliberately not `set_provision_error` — that one only touches
/// pre-registration rows.)
pub async fn flag_container_missing(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE runners
        SET provision_error = 'container_missing', container_id = NULL
        WHERE id = $1 AND managed AND revoked_at IS NULL
          AND status = 'offline' AND container_id IS NOT NULL
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
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

/// Diagnostic lookup: does this hash match a bootstrap credential that has
/// already expired (but not yet been purged)? Used only to pick the static
/// 401 category the runner logs — never grants access.
pub async fn bootstrap_token_hash_expired(pool: &PgPool, token_hash: &str) -> sqlx::Result<bool> {
    let row: Option<(bool,)> = sqlx::query_as(
        r#"
        SELECT true FROM runners
        WHERE bootstrap_token_hash = $1 AND bootstrap_expires_at <= now()
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
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

/// Record the Docker container backing a hosted runner. Returns `false`
/// when the row is gone or was revoked while provisioning ran — the caller
/// must tear the now-ownerless container down.
pub async fn set_container_id(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    container_id: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        "UPDATE runners SET container_id = $3 \
         WHERE workspace_id = $1 AND id = $2 AND managed AND revoked_at IS NULL",
    )
    .bind(workspace_id)
    .bind(id)
    .bind(container_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Record why background provisioning of a hosted runner failed. `category`
/// must be a static string (never upstream Docker text). Guarded to
/// pending-registration rows only (`token_hash IS NULL`) — a runner that
/// already exchanged its bootstrap credential is alive and can't be flagged.
pub async fn set_provision_error(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    category: &str,
) -> sqlx::Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE runners SET provision_error = $3
        WHERE workspace_id = $1 AND id = $2 AND managed AND token_hash IS NULL
        "#,
    )
    .bind(workspace_id)
    .bind(id)
    .bind(category)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Managed runners whose bootstrap expired unexchanged — the janitor needs
/// their container ids to deprovision BEFORE the rows are purged.
pub async fn find_expired_bootstrap_managed(
    pool: &PgPool,
) -> sqlx::Result<Vec<(Uuid, Option<String>)>> {
    sqlx::query_as(
        r#"
        SELECT id, container_id FROM runners
        WHERE managed AND token_hash IS NULL AND bootstrap_expires_at IS NOT NULL
          AND bootstrap_expires_at < now()
        "#,
    )
    .fetch_all(pool)
    .await
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
