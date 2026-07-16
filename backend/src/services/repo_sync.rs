//! Repository synchronization. GitHub stays the source of truth; a sync
//! snapshots repository identity, branches, and `.github/workflows` metadata
//! into PostgreSQL. Syncs run as background tasks (the janitor pattern from
//! main.rs) claimed atomically through the repository's sync_status row so
//! concurrent triggers — webhook bursts, impatient clicks — collapse into
//! one running sync per repository.
//!
//! Error strings persisted to sync rows are static categories only; upstream
//! detail stays in tracing.

use uuid::Uuid;

use crate::db;
use crate::services::{github_app, workflow_parse};
use crate::state::AppState;

/// Never parse more workflow files than this per repository.
const MAX_WORKFLOW_FILES: usize = 50;

/// Claim the repository and spawn a background sync. Returns false when a
/// sync is already running (callers map that to 409).
pub async fn schedule(state: &AppState, repository_id: Uuid, trigger: &str) -> sqlx::Result<bool> {
    let Some(repository) = db::repositories::claim_for_sync(&state.pool, repository_id).await?
    else {
        return Ok(false);
    };
    let run_id = db::repositories::insert_sync_run(&state.pool, repository_id, trigger).await?;

    let state = state.clone();
    tokio::spawn(async move {
        let (error, stats) = match run_sync(&state, &repository).await {
            Ok(stats) => (None, stats),
            Err(SyncFailure { category, source }) => {
                tracing::warn!(
                    repository_id = %repository.id,
                    category,
                    error = ?source,
                    "repository sync failed"
                );
                // Ledger entry (Activity Feed + Notification Center) with
                // the static category only — upstream detail stays in
                // tracing. Best-effort: the sync row is the authority.
                if let Err(audit_err) = sqlx::query(
                    r#"
                    INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
                    VALUES ($1, NULL, 'repository.sync_failed', 'repository', $2, $3)
                    "#,
                )
                .bind(repository.workspace_id)
                .bind(repository.id)
                .bind(serde_json::json!({ "name": repository.name, "syncError": category }))
                .execute(&state.pool)
                .await
                {
                    tracing::warn!(repository_id = %repository.id, error = ?audit_err, "failed to record sync_failed audit entry");
                }
                state.notification_projector.poke();
                (Some(category), serde_json::json!({}))
            }
        };
        if let Err(err) =
            db::repositories::finish_sync(&state.pool, repository.id, run_id, error, stats).await
        {
            tracing::error!(repository_id = %repository.id, error = ?err, "failed to finalize sync run");
        }
    });

    Ok(true)
}

/// Static failure category + full server-side detail.
struct SyncFailure {
    category: &'static str,
    source: anyhow::Error,
}

fn fail(category: &'static str) -> impl FnOnce(anyhow::Error) -> SyncFailure {
    move |source| SyncFailure { category, source }
}

async fn run_sync(
    state: &AppState,
    repository: &crate::models::repository::Repository,
) -> Result<serde_json::Value, SyncFailure> {
    // Resolve the installation and mint a scoped, short-lived token.
    let installation = db::github_installations::find_for_workspace(
        &state.pool,
        repository.workspace_id,
        repository.installation_id,
    )
    .await
    .map_err(|e| fail("database error")(e.into()))?
    .ok_or_else(|| fail("installation missing")(anyhow::anyhow!("installation row gone")))?;

    if installation.suspended_at.is_some() {
        return Err(fail("installation suspended")(anyhow::anyhow!(
            "installation {} is suspended",
            installation.installation_id
        )));
    }

    let token = state
        .github_app
        .installation_token(&state.http, installation.installation_id)
        .await
        .map_err(fail("github authentication failed"))?;

    // Fresh identity by immutable numeric id — survives renames/transfers.
    let remote = github_app::get_repository(&state.http, &token, repository.github_repo_id)
        .await
        .map_err(fail("repository fetch failed"))?;
    let default_branch = remote.default_branch.clone().unwrap_or_else(|| "main".into());

    let branches = github_app::list_branches(&state.http, &token, &remote.owner.login, &remote.name)
        .await
        .map_err(fail("branch listing failed"))?;

    let entries =
        github_app::list_workflow_dir(&state.http, &token, &remote.owner.login, &remote.name)
            .await
            .map_err(fail("workflow discovery failed"))?;

    // Only YAML files that really live inside .github/workflows are eligible;
    // everything else (subdirectories, stray files, hostile paths) is skipped.
    let eligible: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            e.entry_type == "file"
                && e.path.starts_with(".github/workflows/")
                && !e.path.contains("..")
                && e.path.matches('/').count() == 2
                && (e.name.ends_with(".yml") || e.name.ends_with(".yaml"))
                && github_app::is_safe_name_segment(e.name.trim_end_matches(".yaml").trim_end_matches(".yml"))
        })
        .take(MAX_WORKFLOW_FILES)
        .collect();

    let known = db::workflows::sync_states_for_repo(&state.pool, repository.id)
        .await
        .map_err(|e| fail("database error")(e.into()))?;

    // Fetch + parse changed files outside the transaction; persist inside it.
    let mut parsed_files = Vec::new();
    let mut fetched = 0usize;
    for entry in &eligible {
        // Skip only when the content is unchanged AND it was parsed by the
        // current parser — a parser upgrade re-parses stored files once so
        // new metadata (secretRefs/varRefs/environments) materializes for
        // workflows that never change on GitHub.
        if known.get(&entry.path).is_some_and(|(sha, version)| {
            sha == &entry.sha && *version == workflow_parse::PARSER_VERSION
        }) {
            continue;
        }
        if entry.size as usize > github_app::MAX_WORKFLOW_FILE_BYTES {
            // Oversized files become an errored catalog entry, not a fetch.
            parsed_files.push((entry, None, None));
            continue;
        }
        let content = github_app::get_file_content(
            &state.http,
            &token,
            &remote.owner.login,
            &remote.name,
            &entry.path,
        )
        .await
        .map_err(fail("workflow fetch failed"))?;
        let commit = github_app::last_commit_for_path(
            &state.http,
            &token,
            &remote.owner.login,
            &remote.name,
            &entry.path,
        )
        .await
        .unwrap_or_default();
        fetched += 1;
        parsed_files.push((entry, Some(content), commit));
    }

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| fail("database error")(e.into()))?;

    db::repositories::update_metadata(
        &mut tx,
        repository.id,
        &remote.owner.login,
        github_app::sanitize_avatar_url(remote.owner.avatar_url.as_deref()),
        &remote.name,
        &remote.full_name,
        remote.private,
        &default_branch,
        remote.language.as_deref(),
        remote.description.as_deref(),
    )
    .await
    .map_err(|e| fail("database error")(e.into()))?;

    db::repositories::replace_branches(&mut tx, repository.id, &branches, &default_branch)
        .await
        .map_err(|e| fail("database error")(e.into()))?;

    for (entry, content, commit) in &parsed_files {
        // Oversized files were never fetched: store an errored catalog entry
        // with empty content instead of parsing.
        let (parsed, raw) = match content {
            Some(raw) => (workflow_parse::parse_and_validate(raw), raw.as_str()),
            None => (oversize_placeholder(), ""),
        };
        let status = parsed.status();
        let diagnostics = &parsed.diagnostics;
        let name = parsed
            .name
            .clone()
            .unwrap_or_else(|| entry.name.clone());
        let commit_ref = commit
            .as_ref()
            .map(|c| (c.sha.as_str(), c.message.as_str(), c.date));

        // Edge detection for the validation ledger entry: only a
        // valid/warnings -> errors transition records workflow.invalid, so
        // re-syncing a persistently broken file stays quiet.
        let previous_status: Option<(String,)> = sqlx::query_as(
            "SELECT validation_status FROM workflows WHERE repository_id = $1 AND path = $2",
        )
        .bind(repository.id)
        .bind(&entry.path)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| fail("database error")(e.into()))?;

        let workflow_id = db::workflows::upsert(
            &mut tx,
            repository.id,
            &entry.path,
            &name,
            &entry.sha,
            entry.size.min(i32::MAX as i64) as i32,
            raw,
            &parsed.triggers,
            &parsed.metadata,
            status,
            &serde_json::to_value(diagnostics).unwrap_or_else(|_| serde_json::json!([])),
            &parsed.jobs,
            commit_ref,
        )
        .await
        .map_err(|e| fail("database error")(e.into()))?;

        if status == "errors" && previous_status.map(|(s,)| s).as_deref() != Some("errors") {
            sqlx::query(
                r#"
                INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
                VALUES ($1, NULL, 'workflow.invalid', 'workflow', $2, $3)
                "#,
            )
            .bind(repository.workspace_id)
            .bind(workflow_id)
            .bind(serde_json::json!({
                "path": entry.path,
                "name": repository.name,
                "diagnosticCount": diagnostics.len(),
            }))
            .execute(&mut *tx)
            .await
            .map_err(|e| fail("database error")(e.into()))?;
        }
    }

    let keep: Vec<String> = eligible.iter().map(|e| e.path.clone()).collect();
    db::workflows::delete_missing(&mut tx, repository.id, &keep)
        .await
        .map_err(|e| fail("database error")(e.into()))?;

    // Audit inside the same transaction — the sync either fully happened
    // (and is recorded) or didn't.
    sqlx::query(
        r#"
        INSERT INTO audit_logs (workspace_id, actor_user_id, action, subject_type, subject_id, metadata)
        VALUES ($1, NULL, 'repository.synced', 'repository', $2, $3)
        "#,
    )
    .bind(repository.workspace_id)
    .bind(repository.id)
    .bind(serde_json::json!({
        "workflows": keep.len(),
        "fetched": fetched,
        "branches": branches.len(),
    }))
    .execute(&mut *tx)
    .await
    .map_err(|e| fail("database error")(e.into()))?;

    tx.commit()
        .await
        .map_err(|e| fail("database error")(e.into()))?;

    // The ledger gained a repository.synced entry — nudge live feeds.
    state.workspace_hub.publish(
        repository.workspace_id,
        crate::services::workspace_hub::WorkspaceEvent::ActivityUpdate {
            category: "repository".into(),
        },
    );

    Ok(serde_json::json!({
        "workflows": keep.len(),
        "fetched": fetched,
        "branches": branches.len(),
    }))
}

/// Catalog entry for a file too large to fetch: no content, one error.
fn oversize_placeholder() -> workflow_parse::ParsedWorkflow {
    workflow_parse::ParsedWorkflow {
        name: None,
        triggers: Vec::new(),
        jobs: Vec::new(),
        // Version-stamped so sync doesn't re-process the oversized file on
        // every run.
        metadata: serde_json::json!({ "parserVersion": workflow_parse::PARSER_VERSION }),
        diagnostics: vec![workflow_parse::Diagnostic {
            severity: workflow_parse::Severity::Error,
            message: "workflow file exceeds the size limit and was not parsed".into(),
            path: None,
            line: None,
        }],
    }
}
