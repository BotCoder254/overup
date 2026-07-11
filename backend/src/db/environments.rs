//! Environments table access. Environments are workspace-level metadata
//! plus a secrets scope: a workflow job's YAML `environment:` name resolves
//! (case-insensitively) to a row here at dispatch, and its secrets take the
//! highest precedence (environment > repository > workspace). Every
//! mutation writes its immutable audit entry inside the same transaction —
//! including the per-secret `secret.deleted` entries when a delete cascades.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::environment::EnvironmentMeta;

fn is_unique_violation(err: &sqlx::Error, constraint: &str) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some(constraint)
    )
}

/// Meta columns + joins shared by the catalog and detail selects.
const META_SELECT: &str = r#"
    SELECT e.id, e.workspace_id, e.name, e.description,
           cu.username AS creator_login, uu.username AS updater_login,
           (SELECT COUNT(*) FROM secrets s WHERE s.environment_id = e.id)::bigint
               AS secret_count,
           e.created_at, e.updated_at
    FROM environments e
    LEFT JOIN users cu ON cu.id = e.created_by
    LEFT JOIN users uu ON uu.id = e.updated_by
"#;

pub enum InsertOutcome {
    Created(Box<EnvironmentMeta>),
    DuplicateName,
}

pub enum UpdateOutcome {
    Updated(Box<EnvironmentMeta>),
    DuplicateName,
    NotFound,
}

/// Insert with the audit entry in the same transaction.
pub async fn insert(
    pool: &PgPool,
    workspace_id: Uuid,
    name: &str,
    description: Option<&str>,
    created_by: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<InsertOutcome> {
    let mut tx = pool.begin().await?;

    let inserted: Result<(Uuid,), sqlx::Error> = sqlx::query_as(
        r#"
        INSERT INTO environments (workspace_id, name, description, created_by, updated_by)
        VALUES ($1, $2, $3, $4, $4)
        RETURNING id
        "#,
    )
    .bind(workspace_id)
    .bind(name)
    .bind(description)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await;

    let (id,) = match inserted {
        Ok(row) => row,
        Err(err) if is_unique_violation(&err, "environments_ws_name_key") => {
            return Ok(InsertOutcome::DuplicateName);
        }
        Err(err) => return Err(err),
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'environment.created', 'environment', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(created_by)
    .bind(id)
    .bind(serde_json::json!({ "name": name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let meta = sqlx::query_as::<_, EnvironmentMeta>(&format!("{META_SELECT} WHERE e.id = $1"))
        .bind(id)
        .fetch_one(pool)
        .await?;
    Ok(InsertOutcome::Created(Box::new(meta)))
}

pub async fn find_meta(
    pool: &PgPool,
    workspace_id: Uuid,
    environment_id: Uuid,
) -> sqlx::Result<Option<EnvironmentMeta>> {
    sqlx::query_as::<_, EnvironmentMeta>(&format!(
        "{META_SELECT} WHERE e.workspace_id = $1 AND e.id = $2"
    ))
    .bind(workspace_id)
    .bind(environment_id)
    .fetch_optional(pool)
    .await
}

/// Case-insensitive name lookup — the dispatch path (rides the functional
/// unique index on lower(name)).
pub async fn find_by_name(
    pool: &PgPool,
    workspace_id: Uuid,
    name: &str,
) -> sqlx::Result<Option<Uuid>> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM environments WHERE workspace_id = $1 AND lower(name) = lower($2)",
    )
    .bind(workspace_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Validated filters for the catalog list; the search pattern is
/// pre-escaped by the handler — never raw input.
pub struct CatalogFilter {
    pub search_pattern: Option<String>,
    pub cursor: Option<(DateTime<Utc>, Uuid)>,
    pub limit: i64,
}

/// Keyset-paginated catalog, newest first (secrets pattern).
pub async fn list_catalog(
    pool: &PgPool,
    workspace_id: Uuid,
    filter: &CatalogFilter,
) -> sqlx::Result<Vec<EnvironmentMeta>> {
    let (cursor_at, cursor_id) = match filter.cursor {
        Some((at, id)) => (Some(at), Some(id)),
        None => (None, None),
    };
    sqlx::query_as::<_, EnvironmentMeta>(&format!(
        r#"
        {META_SELECT}
        WHERE e.workspace_id = $1
          AND ($2::text IS NULL OR e.name ILIKE $2 ESCAPE '\'
                                OR e.description ILIKE $2 ESCAPE '\')
          AND ($3::timestamptz IS NULL OR (e.created_at, e.id) < ($3, $4))
        ORDER BY e.created_at DESC, e.id DESC
        LIMIT $5
        "#,
    ))
    .bind(workspace_id)
    .bind(&filter.search_pattern)
    .bind(cursor_at)
    .bind(cursor_id)
    .bind(filter.limit.clamp(1, 50))
    .fetch_all(pool)
    .await
}

/// Rename and/or re-describe. `name = None` keeps the current name;
/// `description` is always written (the PATCH clears it when absent, the
/// secrets `update_description` semantics).
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &PgPool,
    workspace_id: Uuid,
    environment_id: Uuid,
    name: Option<&str>,
    description: Option<&str>,
    updated_by: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<UpdateOutcome> {
    let mut tx = pool.begin().await?;

    let updated: Result<Option<(String,)>, sqlx::Error> = sqlx::query_as(
        r#"
        UPDATE environments
        SET name = COALESCE($3, name), description = $4,
            updated_by = $5, updated_at = now()
        WHERE workspace_id = $1 AND id = $2
        RETURNING name
        "#,
    )
    .bind(workspace_id)
    .bind(environment_id)
    .bind(name)
    .bind(description)
    .bind(updated_by)
    .fetch_optional(&mut *tx)
    .await;

    let new_name = match updated {
        Ok(Some((new_name,))) => new_name,
        Ok(None) => return Ok(UpdateOutcome::NotFound),
        Err(err) if is_unique_violation(&err, "environments_ws_name_key") => {
            return Ok(UpdateOutcome::DuplicateName);
        }
        Err(err) => return Err(err),
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'environment.updated', 'environment', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(updated_by)
    .bind(environment_id)
    .bind(serde_json::json!({ "name": new_name, "renamed": name.is_some() }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let meta = find_meta(pool, workspace_id, environment_id).await?;
    Ok(match meta {
        Some(meta) => UpdateOutcome::Updated(Box::new(meta)),
        None => UpdateOutcome::NotFound,
    })
}

/// Hard-delete with the full audit trail in one transaction: one
/// `secret.deleted` entry per cascaded secret (metadata notes the cascade),
/// then the `environment.deleted` entry with the count. Returns the number
/// of cascaded secrets, or None if the environment didn't exist.
pub async fn remove(
    pool: &PgPool,
    workspace_id: Uuid,
    environment_id: Uuid,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<Option<i64>> {
    let mut tx = pool.begin().await?;

    // Collected BEFORE the delete — the FK cascade removes these rows.
    let secrets: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, name FROM secrets WHERE workspace_id = $1 AND environment_id = $2",
    )
    .bind(workspace_id)
    .bind(environment_id)
    .fetch_all(&mut *tx)
    .await?;

    let deleted: Option<(String,)> = sqlx::query_as(
        "DELETE FROM environments WHERE workspace_id = $1 AND id = $2 RETURNING name",
    )
    .bind(workspace_id)
    .bind(environment_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((name,)) = deleted else {
        return Ok(None);
    };

    for (secret_id, secret_name) in &secrets {
        sqlx::query(
            r#"
            INSERT INTO audit_logs
                (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
            VALUES ($1, $2, 'secret.deleted', 'secret', $3, $4, $5)
            "#,
        )
        .bind(workspace_id)
        .bind(actor)
        .bind(secret_id)
        .bind(serde_json::json!({
            "name": secret_name,
            "scope": "environment",
            "environmentId": environment_id,
            "cascade": "environment",
        }))
        .bind(request_id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'environment.deleted', 'environment', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(environment_id)
    .bind(serde_json::json!({ "name": name, "deletedSecrets": secrets.len() }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(secrets.len() as i64))
}

/// Workspace summary in one aggregate pass.
#[derive(Debug, sqlx::FromRow)]
pub struct EnvironmentsSummary {
    pub total: i64,
    pub with_secrets: i64,
    pub created_last_30d: i64,
    pub scoped_secrets: i64,
}

pub async fn summary(pool: &PgPool, workspace_id: Uuid) -> sqlx::Result<EnvironmentsSummary> {
    sqlx::query_as::<_, EnvironmentsSummary>(
        r#"
        SELECT COUNT(*)                                                     AS total,
               COUNT(*) FILTER (WHERE EXISTS (
                   SELECT 1 FROM secrets s WHERE s.environment_id = e.id))  AS with_secrets,
               COUNT(*) FILTER (WHERE e.created_at > now() - interval '30 days')
                                                                            AS created_last_30d,
               COALESCE((SELECT COUNT(*) FROM secrets s
                         WHERE s.workspace_id = $1
                           AND s.environment_id IS NOT NULL), 0)::bigint    AS scoped_secrets
        FROM environments e
        WHERE e.workspace_id = $1
        "#,
    )
    .bind(workspace_id)
    .fetch_one(pool)
    .await
}

/// Recent environment-subject audit events, optionally narrowed to one
/// environment (same shape as the secrets audit feed).
#[derive(Debug, sqlx::FromRow)]
pub struct EnvironmentAuditRow {
    pub action: String,
    pub actor_login: Option<String>,
    pub subject_id: Option<Uuid>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub async fn list_audit(
    pool: &PgPool,
    workspace_id: Uuid,
    subject_id: Option<Uuid>,
    limit: i64,
) -> sqlx::Result<Vec<EnvironmentAuditRow>> {
    sqlx::query_as::<_, EnvironmentAuditRow>(
        r#"
        SELECT a.action, u.username AS actor_login, a.subject_id, a.metadata, a.created_at
        FROM audit_logs a
        LEFT JOIN users u ON u.id = a.actor_user_id
        WHERE a.workspace_id = $1 AND a.subject_type = 'environment'
          AND ($2::uuid IS NULL OR a.subject_id = $2)
        ORDER BY a.created_at DESC
        LIMIT $3
        "#,
    )
    .bind(workspace_id)
    .bind(subject_id)
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await
}
