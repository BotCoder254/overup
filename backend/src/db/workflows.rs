use std::collections::HashMap;

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::models::workflow::{WorkflowDetailRow, WorkflowJobRow, WorkflowSummaryRow};
use crate::services::workflow_parse::ParsedJob;

/// Per-path sync state of the workflows currently stored for a repository:
/// blob sha (fetch only changed files) and the stamped parser version
/// (re-parse stored files after a parser upgrade even when the sha is
/// unchanged — otherwise new metadata never materializes). The `jsonb_typeof`
/// guard reads legacy/hand-edited metadata as version 0, never an error.
pub async fn sync_states_for_repo(
    pool: &PgPool,
    repository_id: Uuid,
) -> sqlx::Result<HashMap<String, (String, i64)>> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT path, blob_sha,
               CASE WHEN jsonb_typeof(metadata->'parserVersion') = 'number'
                    THEN floor((metadata->>'parserVersion')::numeric)::bigint
                    ELSE 0 END
        FROM workflows
        WHERE repository_id = $1
        "#,
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(path, sha, version)| (path, (sha, version)))
        .collect())
}

/// Upsert one workflow and replace its job rows. Runs inside the sync
/// transaction so a failed sync leaves the previous snapshot intact.
#[allow(clippy::too_many_arguments)]
pub async fn upsert(
    tx: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    path: &str,
    name: &str,
    blob_sha: &str,
    file_size: i32,
    raw_content: &str,
    triggers: &[String],
    metadata: &serde_json::Value,
    validation_status: &str,
    validation_errors: &serde_json::Value,
    jobs: &[ParsedJob],
    last_commit: Option<(&str, &str, Option<chrono::DateTime<chrono::Utc>>)>,
) -> sqlx::Result<Uuid> {
    let (commit_sha, commit_message, commit_at) = match last_commit {
        Some((sha, message, at)) => (Some(sha), Some(message), at),
        None => (None, None, None),
    };

    let (workflow_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO workflows
            (repository_id, path, name, blob_sha, file_size, raw_content, triggers,
             metadata, job_count, validation_status, validation_errors,
             last_commit_sha, last_commit_message, last_commit_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
        ON CONFLICT ON CONSTRAINT workflows_repo_path_key
        DO UPDATE SET
            name = EXCLUDED.name,
            blob_sha = EXCLUDED.blob_sha,
            file_size = EXCLUDED.file_size,
            raw_content = EXCLUDED.raw_content,
            triggers = EXCLUDED.triggers,
            metadata = EXCLUDED.metadata,
            job_count = EXCLUDED.job_count,
            validation_status = EXCLUDED.validation_status,
            validation_errors = EXCLUDED.validation_errors,
            last_commit_sha = EXCLUDED.last_commit_sha,
            last_commit_message = EXCLUDED.last_commit_message,
            last_commit_at = EXCLUDED.last_commit_at,
            updated_at = now()
        RETURNING id
        "#,
    )
    .bind(repository_id)
    .bind(path)
    .bind(name)
    .bind(blob_sha)
    .bind(file_size)
    .bind(raw_content)
    .bind(triggers)
    .bind(metadata)
    .bind(jobs.len() as i32)
    .bind(validation_status)
    .bind(validation_errors)
    .bind(commit_sha)
    .bind(commit_message)
    .bind(commit_at)
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM workflow_jobs WHERE workflow_id = $1")
        .bind(workflow_id)
        .execute(&mut **tx)
        .await?;

    for job in jobs {
        sqlx::query(
            r#"
            INSERT INTO workflow_jobs
                (workflow_id, job_key, name, runs_on, needs, uses, strategy, step_count, position)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(workflow_id)
        .bind(&job.key)
        .bind(&job.name)
        .bind(&job.runs_on)
        .bind(&job.needs)
        .bind(&job.uses)
        .bind(&job.strategy)
        .bind(job.step_count)
        .bind(job.position)
        .execute(&mut **tx)
        .await?;
    }

    Ok(workflow_id)
}

/// Drop workflows whose files vanished from `.github/workflows`.
pub async fn delete_missing(
    tx: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    keep_paths: &[String],
) -> sqlx::Result<()> {
    sqlx::query(
        "DELETE FROM workflows WHERE repository_id = $1 AND NOT (path = ANY($2::text[]))",
    )
    .bind(repository_id)
    .bind(keep_paths)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Workspace-wide catalog. The workspace_id filter on the join is the
/// authorization boundary — a workflow id from another workspace matches
/// no row.
pub async fn list_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<WorkflowSummaryRow>> {
    sqlx::query_as::<_, WorkflowSummaryRow>(
        r#"
        SELECT w.id, w.repository_id, r.full_name AS repo_full_name,
               r.owner AS repo_owner, r.owner_avatar_url AS repo_owner_avatar_url,
               w.path, w.name, w.triggers, w.validation_status, w.job_count,
               w.last_commit_sha, w.last_commit_at, w.updated_at
        FROM workflows w
        JOIN repositories r ON r.id = w.repository_id
        WHERE r.workspace_id = $1
        ORDER BY r.full_name, w.path
        "#,
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn summaries_for_repo(
    pool: &PgPool,
    repository_id: Uuid,
) -> sqlx::Result<Vec<WorkflowSummaryRow>> {
    sqlx::query_as::<_, WorkflowSummaryRow>(
        r#"
        SELECT w.id, w.repository_id, r.full_name AS repo_full_name,
               r.owner AS repo_owner, r.owner_avatar_url AS repo_owner_avatar_url,
               w.path, w.name, w.triggers, w.validation_status, w.job_count,
               w.last_commit_sha, w.last_commit_at, w.updated_at
        FROM workflows w
        JOIN repositories r ON r.id = w.repository_id
        WHERE w.repository_id = $1
        ORDER BY w.path
        "#,
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await
}

pub async fn find_detail_for_workspace(
    pool: &PgPool,
    workspace_id: Uuid,
    workflow_id: Uuid,
) -> sqlx::Result<Option<WorkflowDetailRow>> {
    sqlx::query_as::<_, WorkflowDetailRow>(
        r#"
        SELECT w.id, w.repository_id, r.full_name AS repo_full_name,
               r.owner AS repo_owner, r.owner_avatar_url AS repo_owner_avatar_url,
               r.default_branch,
               w.path, w.name, w.file_size, w.raw_content, w.triggers, w.metadata,
               w.job_count, w.validation_status, w.validation_errors,
               w.last_commit_sha, w.last_commit_message, w.last_commit_at, w.updated_at
        FROM workflows w
        JOIN repositories r ON r.id = w.repository_id
        WHERE r.workspace_id = $1 AND w.id = $2
        "#,
    )
    .bind(workspace_id)
    .bind(workflow_id)
    .fetch_optional(pool)
    .await
}

/// Workflows of one repository that an event of the given kind can trigger:
/// they declare the event in `triggers` and are not in an error state.
/// Callers pass static event names only ("push", "pull_request"); the
/// per-workflow filter conditions in `metadata.triggerFilters` are evaluated
/// afterwards by `services/trigger_eval.rs`.
pub async fn runnable_for_repo(
    pool: &PgPool,
    repository_id: Uuid,
    event: &str,
) -> sqlx::Result<Vec<RunnableWorkflow>> {
    sqlx::query_as::<_, RunnableWorkflow>(
        r#"
        SELECT id, name, path, raw_content, triggers, metadata
        FROM workflows
        WHERE repository_id = $1
          AND $2 = ANY(triggers)
          AND validation_status <> 'errors'
        ORDER BY path
        "#,
    )
    .bind(repository_id)
    .bind(event)
    .fetch_all(pool)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct RunnableWorkflow {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub raw_content: String,
    pub triggers: Vec<String>,
    pub metadata: serde_json::Value,
}

/// One workflow-YAML reference to a secret/var/environment name, with the
/// repository and workflow it came from and — when something satisfies it —
/// the id of the configured row. Names and ids only, never values.
#[derive(Debug, sqlx::FromRow)]
pub struct RequirementRefRow {
    pub name: String,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub workflow_id: Uuid,
    pub workflow_path: String,
    pub configured_id: Option<Uuid>,
}

/// CTE expanding one metadata ref array across a workspace's workflows.
/// `key` is always a compile-time literal (`secretRefs`/`varRefs`/
/// `environments`) — never user input; every user-facing value stays a bind.
/// The `jsonb_typeof` guard makes workflows synced before a key existed (and
/// failed parses, whose metadata is `{}`) read as empty rather than erroring.
fn refs_cte(key: &str) -> String {
    format!(
        r#"
        WITH refs AS (
            SELECT ref.name AS name,
                   r.id AS repository_id,
                   r.full_name AS repository_name,
                   w.id AS workflow_id,
                   w.path AS workflow_path
            FROM workflows w
            JOIN repositories r ON r.id = w.repository_id
            CROSS JOIN LATERAL jsonb_array_elements_text(
                CASE WHEN jsonb_typeof(w.metadata->'{key}') = 'array'
                     THEN w.metadata->'{key}' ELSE '[]'::jsonb END
            ) AS ref(name)
            WHERE r.workspace_id = $1
        )
        "#
    )
}

/// Every secret name referenced by workflow YAML, with the id of the secret
/// that would satisfy it when one exists: a name counts as configured for a
/// referencing repo when a workspace-scoped secret, a repository-scoped
/// secret on that repo, or an environment-scoped secret in any environment
/// carries it (which environment applies is a dispatch-time question). The
/// LATERAL picks the highest-precedence match so the UI can link straight
/// to it.
pub async fn secret_ref_states(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<RequirementRefRow>> {
    let sql = refs_cte("secretRefs")
        + r#"
        SELECT refs.name, refs.repository_id, refs.repository_name,
               refs.workflow_id, refs.workflow_path, s.id AS configured_id
        FROM refs
        LEFT JOIN LATERAL (
            SELECT s.id
            FROM secrets s
            WHERE s.workspace_id = $1
              AND s.name = refs.name
              AND (
                   (s.repository_id IS NULL AND s.environment_id IS NULL)
                OR s.repository_id = refs.repository_id
                OR s.environment_id IS NOT NULL
              )
            ORDER BY (s.environment_id IS NOT NULL) DESC,
                     (s.repository_id IS NOT NULL) DESC
            LIMIT 1
        ) s ON TRUE
        ORDER BY refs.name, refs.repository_name, refs.workflow_path
        LIMIT 1000
        "#;
    sqlx::query_as::<_, RequirementRefRow>(&sql)
        .bind(workspace_id)
        .fetch_all(pool)
        .await
}

/// Every `${{ vars.NAME }}` reference across the workspace — informational
/// only (the platform doesn't manage plain variables), never configured.
pub async fn workspace_var_refs(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<RequirementRefRow>> {
    let sql = refs_cte("varRefs")
        + r#"
        SELECT name, repository_id, repository_name, workflow_id, workflow_path,
               NULL::uuid AS configured_id
        FROM refs
        ORDER BY name, repository_name, workflow_path
        LIMIT 1000
        "#;
    sqlx::query_as::<_, RequirementRefRow>(&sql)
        .bind(workspace_id)
        .fetch_all(pool)
        .await
}

/// Every environment name bound by workflow YAML, with the id of the
/// matching environment row when it exists (case-insensitive, like dispatch
/// resolution).
pub async fn environment_ref_states(
    pool: &PgPool,
    workspace_id: Uuid,
) -> sqlx::Result<Vec<RequirementRefRow>> {
    let sql = refs_cte("environments")
        + r#"
        SELECT refs.name, refs.repository_id, refs.repository_name,
               refs.workflow_id, refs.workflow_path, e.id AS configured_id
        FROM refs
        LEFT JOIN environments e
          ON e.workspace_id = $1 AND lower(e.name) = lower(refs.name)
        ORDER BY refs.name, refs.repository_name, refs.workflow_path
        LIMIT 1000
        "#;
    sqlx::query_as::<_, RequirementRefRow>(&sql)
        .bind(workspace_id)
        .fetch_all(pool)
        .await
}

/// Workflows whose YAML binds the given environment name (case-insensitive)
/// — feeds the environment detail page's "bound workflows" list.
pub async fn list_binding_environment(
    pool: &PgPool,
    workspace_id: Uuid,
    name: &str,
) -> sqlx::Result<Vec<RequirementRefRow>> {
    let sql = refs_cte("environments")
        + r#"
        SELECT name, repository_id, repository_name, workflow_id, workflow_path,
               NULL::uuid AS configured_id
        FROM refs
        WHERE lower(refs.name) = lower($2)
        ORDER BY repository_name, workflow_path
        LIMIT 50
        "#;
    sqlx::query_as::<_, RequirementRefRow>(&sql)
        .bind(workspace_id)
        .bind(name)
        .fetch_all(pool)
        .await
}

pub async fn list_jobs(pool: &PgPool, workflow_id: Uuid) -> sqlx::Result<Vec<WorkflowJobRow>> {
    sqlx::query_as::<_, WorkflowJobRow>(
        r#"
        SELECT job_key, name, runs_on, needs, uses, strategy, step_count
        FROM workflow_jobs
        WHERE workflow_id = $1
        ORDER BY position
        "#,
    )
    .bind(workflow_id)
    .fetch_all(pool)
    .await
}
