use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::models::repository::{RepoBranch, RepoSyncRun, Repository, RepositoryWithCount};
use crate::services::github_app::Branch;

pub enum ImportOutcome {
    Created(Box<Repository>),
    AlreadyImported,
}

fn is_unique_violation(err: &sqlx::Error, constraint: &str) -> bool {
    matches!(
        err,
        sqlx::Error::Database(db)
            if db.is_unique_violation() && db.constraint() == Some(constraint)
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn import(
    pool: &PgPool,
    workspace_id: Uuid,
    installation_id: Uuid,
    github_repo_id: i64,
    owner: &str,
    owner_avatar_url: Option<&str>,
    name: &str,
    full_name: &str,
    private: bool,
    default_branch: &str,
    language: Option<&str>,
    description: Option<&str>,
    imported_by: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<ImportOutcome> {
    let mut tx = pool.begin().await?;

    let repository = match sqlx::query_as::<_, Repository>(
        r#"
        INSERT INTO repositories
            (workspace_id, installation_id, github_repo_id, owner, owner_avatar_url,
             name, full_name, private, default_branch, language, description, imported_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        RETURNING *
        "#,
    )
    .bind(workspace_id)
    .bind(installation_id)
    .bind(github_repo_id)
    .bind(owner)
    .bind(owner_avatar_url)
    .bind(name)
    .bind(full_name)
    .bind(private)
    .bind(default_branch)
    .bind(language)
    .bind(description)
    .bind(imported_by)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(repository) => repository,
        // Import races lose cleanly on the named unique constraint.
        Err(err) if is_unique_violation(&err, "repositories_workspace_repo_key") => {
            return Ok(ImportOutcome::AlreadyImported);
        }
        Err(err) => return Err(err),
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'repository.imported', 'repository', $3, $4, $5)
        "#, // audit inside the txn: import either fully happened or didn't
    )
    .bind(workspace_id)
    .bind(imported_by)
    .bind(repository.id)
    .bind(serde_json::json!({ "fullName": full_name, "private": private }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ImportOutcome::Created(Box::new(repository)))
}

pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<RepositoryWithCount>> {
    sqlx::query_as::<_, RepositoryWithCount>(
        r#"
        SELECT r.*, COUNT(w.id) AS workflow_count
        FROM repositories r
        LEFT JOIN workflows w ON w.repository_id = r.id
        WHERE r.workspace_id = $1
        GROUP BY r.id
        ORDER BY r.full_name
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn find_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
) -> sqlx::Result<Option<Repository>> {
    sqlx::query_as::<_, Repository>(
        "SELECT * FROM repositories WHERE workspace_id = $1 AND id = $2",
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Every workspace connection of a GitHub repository. `github_repo_id` is
/// only unique per (workspace_id, github_repo_id) — the same repo can be
/// connected in several workspaces, and webhook processing must fan out to
/// ALL of them (a single-row lookup would nondeterministically feed one
/// workspace's timeline/pipelines and starve the others). Deterministic
/// order for stable processing.
pub async fn find_all_by_github_id(
    pool: &PgPool,
    github_repo_id: i64,
) -> sqlx::Result<Vec<Repository>> {
    sqlx::query_as::<_, Repository>(
        "SELECT * FROM repositories WHERE github_repo_id = $1 ORDER BY created_at, id",
    )
    .bind(github_repo_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    id: Uuid,
    actor: Uuid,
    request_id: Option<&str>,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let deleted: Option<(String,)> = sqlx::query_as(
        "DELETE FROM repositories WHERE workspace_id = $1 AND id = $2 RETURNING full_name",
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((full_name,)) = deleted else {
        return Ok(false);
    };

    sqlx::query(
        r#"
        INSERT INTO audit_logs
            (workspace_id, actor_user_id, action, subject_type, subject_id, metadata, request_id)
        VALUES ($1, $2, 'repository.removed', 'repository', $3, $4, $5)
        "#,
    )
    .bind(workspace_id)
    .bind(actor)
    .bind(id)
    .bind(serde_json::json!({ "fullName": full_name }))
    .bind(request_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

/// Atomically claim a repository for syncing. Returns the row only when no
/// other sync currently holds it — the row-level status is the dedup guard.
pub async fn claim_for_sync(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<Repository>> {
    sqlx::query_as::<_, Repository>(
        r#"
        UPDATE repositories
        SET sync_status = 'syncing', sync_error = NULL, updated_at = now()
        WHERE id = $1 AND sync_status <> 'syncing'
        RETURNING *
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn insert_sync_run(pool: &PgPool, repository_id: Uuid, trigger: &str) -> sqlx::Result<Uuid> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO repo_sync_runs (repository_id, trigger) VALUES ($1, $2) RETURNING id",
    )
    .bind(repository_id)
    .bind(trigger)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Finalize a sync attempt: run row and repository status together.
/// `error` must be a static category string — never upstream detail.
pub async fn finish_sync(
    pool: &PgPool,
    repository_id: Uuid,
    run_id: Uuid,
    error: Option<&str>,
    stats: serde_json::Value,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE repo_sync_runs
        SET status = CASE WHEN $2::text IS NULL THEN 'success' ELSE 'failed' END,
            error = $2, stats = $3, finished_at = now()
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(error)
    .bind(&stats)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE repositories
        SET sync_status = CASE WHEN $2::text IS NULL THEN 'synced' ELSE 'failed' END,
            sync_error = $2,
            last_synced_at = CASE WHEN $2::text IS NULL THEN now() ELSE last_synced_at END,
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(repository_id)
    .bind(error)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Refresh the repository identity snapshot from GitHub during sync.
#[allow(clippy::too_many_arguments)]
pub async fn update_metadata(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    owner: &str,
    owner_avatar_url: Option<&str>,
    name: &str,
    full_name: &str,
    private: bool,
    default_branch: &str,
    language: Option<&str>,
    description: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE repositories
        SET owner = $2, owner_avatar_url = $3, name = $4, full_name = $5, private = $6,
            default_branch = $7, language = $8, description = $9, updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(owner)
    .bind(owner_avatar_url)
    .bind(name)
    .bind(full_name)
    .bind(private)
    .bind(default_branch)
    .bind(language)
    .bind(description)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replace the branch snapshot: upsert current branches, drop vanished ones.
pub async fn replace_branches(
    tx: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    branches: &[Branch],
    default_branch: &str,
) -> sqlx::Result<()> {
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    sqlx::query(
        "DELETE FROM repo_branches WHERE repository_id = $1 AND NOT (name = ANY($2::text[]))",
    )
    .bind(repository_id)
    .bind(&names)
    .execute(&mut **tx)
    .await?;

    for branch in branches {
        sqlx::query(
            r#"
            INSERT INTO repo_branches (repository_id, name, commit_sha, is_default)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT ON CONSTRAINT repo_branches_repo_name_key
            DO UPDATE SET commit_sha = EXCLUDED.commit_sha,
                          is_default = EXCLUDED.is_default,
                          updated_at = now()
            "#,
        )
        .bind(repository_id)
        .bind(&branch.name)
        .bind(&branch.commit.sha)
        .bind(branch.name == default_branch)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn list_branches(pool: &PgPool, repository_id: Uuid) -> sqlx::Result<Vec<RepoBranch>> {
    sqlx::query_as::<_, RepoBranch>(
        r#"
        SELECT name, commit_sha, is_default, updated_at
        FROM repo_branches
        WHERE repository_id = $1
        ORDER BY is_default DESC, name
        "#,
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await
}

pub async fn list_sync_runs(pool: &PgPool, repository_id: Uuid) -> sqlx::Result<Vec<RepoSyncRun>> {
    sqlx::query_as::<_, RepoSyncRun>(
        r#"
        SELECT id, trigger, status, error, stats, started_at, finished_at
        FROM repo_sync_runs
        WHERE repository_id = $1
        ORDER BY started_at DESC
        LIMIT 10
        "#,
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await
}

/// Webhook: repositories became unusable (access revoked, deleted upstream).
/// `category` is a static description, never upstream text.
pub async fn mark_failed(
    pool: &PgPool,
    github_repo_ids: &[i64],
    category: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE repositories
        SET sync_status = 'failed', sync_error = $2, updated_at = now()
        WHERE github_repo_id = ANY($1::bigint[])
        "#,
    )
    .bind(github_repo_ids)
    .bind(category)
    .execute(pool)
    .await?;
    Ok(())
}
