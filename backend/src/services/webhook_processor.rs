//! Asynchronous webhook processor.
//!
//! The HTTP receiver (`handlers/github_webhooks.rs`) only verifies,
//! validates, persists, and acks — every side effect happens here, off the
//! request path (GitHub's 10-second ack budget). The loop mirrors the
//! notification projector (Notify poke + tick backstop) but claims queue
//! rows individually with guarded status UPDATEs: a single sequential
//! consumer per process, so two pushes to the same repository can never be
//! processed out of order. NOTE: that per-repo ordering guarantee holds only
//! under a SINGLE consumer process — deployment docs mandate `replicas: 1`.
//! `FOR UPDATE SKIP LOCKED` in the claim keeps accidental concurrent
//! instances from blocking each other, but it does NOT preserve ordering
//! across instances. Failures retry up to `MAX_RETRIES`, stuck 'processing'
//! rows revert on the tick (crash recovery), and every completion commits
//! the repository-event timeline row(s) and the delivery status flip in ONE
//! transaction. Pipeline creation is idempotent per (delivery, workflow), so
//! a retried delivery re-running its side effects never duplicates pipelines.
//!
//! Outcomes, ignored reasons, and skip reasons are static category strings.
//! Upstream-derived text (commit-message first line, PR title, sender login)
//! DOES flow into pipeline metadata and event summaries as data — always
//! length-capped by the receiver's normalizer, never raw payload bodies, and
//! rendered client-side as text (never markup or URLs).

use std::collections::HashSet;
use std::time::Duration;

use tokio::sync::Notify;
use uuid::Uuid;

use crate::db;
use crate::db::webhook_deliveries::ClaimedDelivery;
use crate::models::repository::Repository;
use crate::services::{github_checks, pipeline_run, repo_sync, trigger_eval};
use crate::state::AppState;

/// Coalesce webhook bursts before draining.
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Safety tick: drains pending rows even if every poke was lost.
const TICK: Duration = Duration::from_secs(2);
/// A delivery whose processing fails re-queues this many times before the
/// row parks as terminally 'failed'.
const MAX_RETRIES: i32 = 3;
/// 'processing' rows older than this revert to 'pending' (crash recovery).
const STUCK_SECS: i64 = 300;
/// Cap on per-workflow skip entries recorded into the event summary.
const MAX_SKIPPED_SUMMARY: usize = 50;

// Outcome vocabulary (CHECK-constrained in SQL).
const OUTCOME_PIPELINES: &str = "pipelines_created";
const OUTCOME_SYNC: &str = "sync_scheduled";
const OUTCOME_BOTH: &str = "pipelines_and_sync";
const OUTCOME_IGNORED: &str = "ignored";
const OUTCOME_FAILED: &str = "failed";

// Static ignored_reason categories (never upstream text).
const REASON_NO_MATCHING_WORKFLOWS: &str = "no_matching_workflows";
const REASON_FILTERS_NOT_MATCHED: &str = "filters_not_matched";
const REASON_BRANCH_DELETED: &str = "branch_deleted";
const REASON_TAG_DELETED: &str = "tag_deleted";
const REASON_NO_HEAD_COMMIT: &str = "no_head_commit";
const REASON_FORK_PR_SKIPPED: &str = "fork_pr_skipped";
const REASON_PR_ACTION_IGNORED: &str = "pr_action_ignored";
const REASON_PR_CLOSED: &str = "pr_closed";
const REASON_EVENT_NOT_SUPPORTED: &str = "event_not_supported";
const REASON_REPOSITORY_DELETED: &str = "repository_deleted";
const REASON_ACCESS_REVOKED: &str = "access_revoked";

/// Wake handle held in `AppState`; pure signal, no payload — the queue in
/// Postgres is the only source of truth for what needs processing.
#[derive(Default)]
pub struct WebhookProcessor {
    notify: Notify,
}

impl WebhookProcessor {
    /// Queue a drain; cheap and callable from anywhere.
    pub fn poke(&self) {
        self.notify.notify_one();
    }
}

/// The processor loop, spawned once from main.
pub async fn run(state: AppState) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = state.webhook_processor.notify.notified() => {
                tokio::time::sleep(DEBOUNCE).await;
            }
            _ = tick.tick() => {
                // Crash recovery rides the tick, never the hot poke path.
                if let Err(error) =
                    db::webhook_deliveries::revert_stuck(&state.pool, STUCK_SECS).await
                {
                    tracing::warn!(error = ?error, "failed to revert stuck webhook deliveries");
                }
            }
        }
        // Drain sequentially until the queue is empty. Sequential processing
        // is the per-repo ordering guarantee.
        loop {
            match db::webhook_deliveries::claim_next(&state.pool).await {
                Ok(Some(delivery)) => process(&state, delivery).await,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(error = ?error, "webhook delivery claim failed; will retry");
                    break;
                }
            }
        }
    }
}

/// One repository-event timeline row to be committed with the delivery.
struct EventRow {
    repository_id: Uuid,
    git_ref: Option<String>,
    head_sha: Option<String>,
    outcome: &'static str,
    ignored_reason: Option<&'static str>,
    pipeline_ids: Vec<Uuid>,
    sync_run_id: Option<Uuid>,
    summary: serde_json::Value,
}

/// What processing one delivery concluded: the terminal delivery status and
/// any timeline rows for connected repositories.
struct Completion {
    delivery_status: &'static str, // 'processed' | 'ignored'
    events: Vec<EventRow>,
}

impl Completion {
    fn processed(events: Vec<EventRow>) -> Self {
        Self {
            delivery_status: "processed",
            events,
        }
    }

    fn ignored() -> Self {
        Self {
            delivery_status: "ignored",
            events: Vec::new(),
        }
    }
}

/// Process one claimed delivery end to end, then commit its completion.
async fn process(state: &AppState, delivery: ClaimedDelivery) {
    match process_delivery(state, &delivery).await {
        Ok(completion) => {
            if let Err(error) = commit_completion(state, &delivery, &completion).await {
                tracing::warn!(
                    error = ?error,
                    event = %delivery.event,
                    "failed to commit webhook completion; delivery will retry"
                );
                let _ = db::webhook_deliveries::record_failure(
                    &state.pool,
                    &delivery.delivery_id,
                    MAX_RETRIES,
                )
                .await;
                if delivery.retry_count + 1 >= MAX_RETRIES {
                    record_failed_event(state, &delivery).await;
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                error = ?error,
                event = %delivery.event,
                retry = delivery.retry_count,
                "webhook processing failed"
            );
            let _ = db::webhook_deliveries::record_failure(
                &state.pool,
                &delivery.delivery_id,
                MAX_RETRIES,
            )
            .await;
            // Terminal failure: surface it on the repository timeline
            // (best-effort — the delivery row is the authority).
            if delivery.retry_count + 1 >= MAX_RETRIES {
                record_failed_event(state, &delivery).await;
            }
        }
    }
}

/// Timeline rows + delivery status flip in one transaction, then post-commit
/// check-run creation for any new pipelines.
async fn commit_completion(
    state: &AppState,
    delivery: &ClaimedDelivery,
    completion: &Completion,
) -> anyhow::Result<()> {
    let actor_login = payload_str(delivery, "senderLogin");
    let actor_avatar = payload_str(delivery, "senderAvatarUrl");

    let mut tx = state.pool.begin().await?;
    for event in &completion.events {
        db::repository_events::insert(
            &mut tx,
            &db::repository_events::NewRepositoryEvent {
                repository_id: event.repository_id,
                delivery_id: &delivery.delivery_id,
                event: &delivery.event,
                action: delivery.action.as_deref(),
                git_ref: event.git_ref.as_deref(),
                head_sha: event.head_sha.as_deref(),
                actor_login,
                actor_avatar_url: actor_avatar,
                outcome: event.outcome,
                ignored_reason: event.ignored_reason,
                pipeline_ids: &event.pipeline_ids,
                sync_run_id: event.sync_run_id,
                summary: &event.summary,
                received_at: delivery.received_at,
            },
        )
        .await?;
    }
    db::webhook_deliveries::finish(&mut tx, &delivery.delivery_id, completion.delivery_status)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Best-effort 'failed' timeline row(s) for a terminally failed delivery —
/// one per workspace connection of the repository.
async fn record_failed_event(state: &AppState, delivery: &ClaimedDelivery) {
    let Some(github_repo_id) = delivery.github_repo_id else {
        return;
    };
    let Ok(repos) = db::repositories::find_all_by_github_id(&state.pool, github_repo_id).await
    else {
        return;
    };
    if repos.is_empty() {
        return;
    }
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => return,
    };
    for repo in &repos {
        let inserted = db::repository_events::insert(
            &mut tx,
            &db::repository_events::NewRepositoryEvent {
                repository_id: repo.id,
                delivery_id: &delivery.delivery_id,
                event: &delivery.event,
                action: delivery.action.as_deref(),
                git_ref: payload_str(delivery, "ref"),
                head_sha: payload_str(delivery, "after"),
                actor_login: payload_str(delivery, "senderLogin"),
                actor_avatar_url: payload_str(delivery, "senderAvatarUrl"),
                outcome: OUTCOME_FAILED,
                ignored_reason: None,
                pipeline_ids: &[],
                sync_run_id: None,
                summary: &serde_json::json!({}),
                received_at: delivery.received_at,
            },
        )
        .await;
        if inserted.is_err() {
            return;
        }
    }
    let _ = tx.commit().await;
}

fn payload_str<'a>(delivery: &'a ClaimedDelivery, key: &str) -> Option<&'a str> {
    delivery.payload.as_ref()?.get(key)?.as_str()
}

/// Route one delivery. Unconnected repositories are dropped before any
/// further processing (the delivery records as 'ignored', no timeline row).
async fn process_delivery(
    state: &AppState,
    delivery: &ClaimedDelivery,
) -> anyhow::Result<Completion> {
    let action = delivery.action.as_deref().unwrap_or("");
    match delivery.event.as_str() {
        "push" => process_push(state, delivery).await,
        "pull_request" => process_pull_request(state, delivery, action).await,
        "repository" => process_repository(state, delivery, action).await,
        "installation" => process_installation(state, delivery, action).await,
        "installation_repositories" => {
            process_installation_repositories(state, delivery, action).await
        }
        _ => Ok(Completion::ignored()),
    }
}

/// An all-zero SHA marks a ref deletion push.
fn is_zero_sha(sha: &str) -> bool {
    sha.is_empty() || sha.chars().all(|c| c == '0')
}

async fn process_push(
    state: &AppState,
    delivery: &ClaimedDelivery,
) -> anyhow::Result<Completion> {
    let Some(github_repo_id) = delivery.github_repo_id else {
        return Ok(Completion::ignored());
    };
    // The same GitHub repository can be connected in several workspaces —
    // fan out so every connection gets its timeline row and pipelines.
    let repos = db::repositories::find_all_by_github_id(&state.pool, github_repo_id).await?;
    if repos.is_empty() {
        // Not connected to any workspace: discard before further processing.
        return Ok(Completion::ignored());
    }
    let Some(git_ref) = payload_str(delivery, "ref").map(str::to_string) else {
        return Ok(Completion::ignored());
    };

    let commit_sha = payload_str(delivery, "after")
        .or(payload_str(delivery, "headCommitSha"))
        .unwrap_or("")
        .to_string();

    // Changed paths for filter evaluation (shared across repo rows); a
    // truncated set fails open.
    let truncated = delivery
        .payload
        .as_ref()
        .and_then(|p| p.get("pathsTruncated"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let changed_paths: Option<Vec<String>> = if truncated {
        None
    } else {
        delivery
            .payload
            .as_ref()
            .and_then(|p| p.get("changedPaths"))
            .and_then(serde_json::Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
    };

    let mut events = Vec::with_capacity(repos.len());
    for repo in &repos {
        events.push(
            push_event_for_repo(
                state,
                delivery,
                repo,
                &git_ref,
                &commit_sha,
                changed_paths.as_deref(),
            )
            .await?,
        );
    }
    Ok(Completion::processed(events))
}

/// One workspace connection's share of a push delivery: sync scheduling,
/// trigger evaluation, pipeline creation, and its timeline row.
async fn push_event_for_repo(
    state: &AppState,
    delivery: &ClaimedDelivery,
    repo: &Repository,
    git_ref: &str,
    commit_sha: &str,
    changed_paths: Option<&[String]>,
) -> anyhow::Result<EventRow> {
    if let Some(branch) = git_ref.strip_prefix("refs/heads/") {
        // A push to the default branch re-syncs the repository (the sync
        // diffs blob shas, so this stays cheap when nothing changed).
        let mut sync_run_id = None;
        let mut sync_collapsed = false;
        if branch == repo.default_branch {
            match repo_sync::schedule(state, repo.id, "webhook").await? {
                Some(run_id) => sync_run_id = Some(run_id),
                // A sync was already running; the intent still counts.
                None => sync_collapsed = true,
            }
        }
        let synced = sync_run_id.is_some() || sync_collapsed;

        // Branch deletions push an all-zero SHA; nothing to build.
        if is_zero_sha(commit_sha) {
            return Ok(EventRow {
                repository_id: repo.id,
                git_ref: Some(git_ref.to_string()),
                head_sha: None,
                outcome: if synced { OUTCOME_SYNC } else { OUTCOME_IGNORED },
                ignored_reason: (!synced).then_some(REASON_BRANCH_DELETED),
                pipeline_ids: Vec::new(),
                sync_run_id,
                summary: serde_json::json!({ "branchDeleted": true }),
            });
        }

        let ctx = trigger_eval::EventContext {
            kind: trigger_eval::EventKind::Push { branch },
            changed_paths,
        };
        let (pipeline_ids, skipped, considered) =
            run_matching_workflows(state, delivery, repo, "push", commit_sha, git_ref, None, &ctx)
                .await?;

        let mut summary = serde_json::json!({ "skipped": skipped });
        if sync_collapsed {
            summary["syncCollapsed"] = serde_json::json!(true);
        }
        let (outcome, reason) = push_outcome(&pipeline_ids, synced, considered);
        return Ok(EventRow {
            repository_id: repo.id,
            git_ref: Some(git_ref.to_string()),
            head_sha: Some(commit_sha.to_string()),
            outcome,
            ignored_reason: reason,
            pipeline_ids,
            sync_run_id,
            summary,
        });
    }

    if let Some(tag) = git_ref.strip_prefix("refs/tags/") {
        if is_zero_sha(commit_sha) {
            return Ok(EventRow {
                repository_id: repo.id,
                git_ref: Some(git_ref.to_string()),
                head_sha: None,
                outcome: OUTCOME_IGNORED,
                ignored_reason: Some(REASON_TAG_DELETED),
                pipeline_ids: Vec::new(),
                sync_run_id: None,
                summary: serde_json::json!({ "tagDeleted": true }),
            });
        }

        // Path filters deliberately never apply to tag pushes.
        let ctx = trigger_eval::EventContext {
            kind: trigger_eval::EventKind::TagPush { tag },
            changed_paths: None,
        };
        let (pipeline_ids, skipped, considered) =
            run_matching_workflows(state, delivery, repo, "tag", commit_sha, git_ref, None, &ctx)
                .await?;

        let (outcome, reason) = push_outcome(&pipeline_ids, false, considered);
        return Ok(EventRow {
            repository_id: repo.id,
            git_ref: Some(git_ref.to_string()),
            head_sha: Some(commit_sha.to_string()),
            outcome,
            ignored_reason: reason,
            pipeline_ids,
            sync_run_id: None,
            summary: serde_json::json!({ "skipped": skipped }),
        });
    }

    // Neither a branch nor a tag ref — record it, run nothing.
    Ok(EventRow {
        repository_id: repo.id,
        git_ref: Some(git_ref.to_string()),
        head_sha: None,
        outcome: OUTCOME_IGNORED,
        ignored_reason: Some(REASON_EVENT_NOT_SUPPORTED),
        pipeline_ids: Vec::new(),
        sync_run_id: None,
        summary: serde_json::json!({}),
    })
}

fn push_outcome(
    pipeline_ids: &[Uuid],
    synced: bool,
    considered: usize,
) -> (&'static str, Option<&'static str>) {
    match (pipeline_ids.is_empty(), synced) {
        (false, true) => (OUTCOME_BOTH, None),
        (false, false) => (OUTCOME_PIPELINES, None),
        (true, true) => (OUTCOME_SYNC, None),
        (true, false) => (
            OUTCOME_IGNORED,
            Some(if considered == 0 {
                REASON_NO_MATCHING_WORKFLOWS
            } else {
                REASON_FILTERS_NOT_MATCHED
            }),
        ),
    }
}

/// Evaluate every runnable workflow of the repo against the event and create
/// pipelines for the matches. Returns (created ids, skip summary entries,
/// how many workflows were considered). Per-workflow creation failures are
/// expected (uses:-only workflows) and never block the others.
#[allow(clippy::too_many_arguments)]
async fn run_matching_workflows(
    state: &AppState,
    delivery: &ClaimedDelivery,
    repo: &Repository,
    trigger: &'static str,
    commit_sha: &str,
    git_ref: &str,
    pr: Option<&PrContext>,
    ctx: &trigger_eval::EventContext<'_>,
) -> anyhow::Result<(Vec<Uuid>, Vec<serde_json::Value>, usize)> {
    // Both "push" and "tag" pipelines run workflows declaring `on: push`.
    let event = match trigger {
        "pull_request" => "pull_request",
        _ => "push",
    };
    let workflows = db::workflows::runnable_for_repo(&state.pool, repo.id, event).await?;
    let considered = workflows.len();

    let commit_message = match pr {
        Some(pr) => pr.title.clone(),
        None => payload_str(delivery, "commitMessage").map(str::to_string),
    };
    let commit_author = match pr {
        Some(pr) => pr.user_login.clone(),
        None => payload_str(delivery, "commitAuthor")
            .or(payload_str(delivery, "pusherName"))
            .map(str::to_string),
    };
    let actor_login = payload_str(delivery, "senderLogin");
    let actor_avatar_url = payload_str(delivery, "senderAvatarUrl");

    let mut pipeline_ids = Vec::new();
    let mut skipped = Vec::new();
    for workflow in workflows {
        match trigger_eval::evaluate(&workflow.triggers, &workflow.metadata, ctx) {
            trigger_eval::Decision::Skip(reason) => {
                if skipped.len() < MAX_SKIPPED_SUMMARY {
                    skipped.push(serde_json::json!({ "path": workflow.path, "reason": reason }));
                }
            }
            trigger_eval::Decision::Run => {
                let trigger_ctx = pipeline_run::TriggerContext {
                    trigger,
                    triggered_by: None,
                    commit_sha,
                    commit_message: commit_message.as_deref(),
                    commit_author: commit_author.as_deref(),
                    actor_login,
                    actor_avatar_url,
                    git_ref,
                    inputs: None,
                    pr_number: pr.map(|pr| pr.number),
                    request_id: None,
                    // Idempotency key: a retried delivery re-running this
                    // side effect gets the existing pipeline back.
                    webhook_delivery_id: Some(&delivery.delivery_id),
                };
                match pipeline_run::create_pipeline(
                    state,
                    repo,
                    workflow.id,
                    &workflow.name,
                    &workflow.path,
                    &workflow.raw_content,
                    &trigger_ctx,
                )
                .await
                {
                    Ok(created) => {
                        // Report a queued check run to GitHub (best-effort) —
                        // only for genuinely new pipelines, so a retried
                        // delivery never posts a duplicate check run.
                        if created.newly_created {
                            github_checks::spawn_create(state, created.pipeline.id);
                        }
                        pipeline_ids.push(created.pipeline.id);
                    }
                    Err(error) => {
                        // Validation failures (e.g. uses:-only workflows) are
                        // expected; they must not fail the whole delivery.
                        tracing::debug!(
                            workflow = %workflow.path,
                            error = ?error,
                            "event did not trigger a pipeline for this workflow"
                        );
                    }
                }
            }
        }
    }
    Ok((pipeline_ids, skipped, considered))
}

/// PR context lifted out of the normalized payload.
struct PrContext {
    number: i32,
    title: Option<String>,
    user_login: Option<String>,
}

async fn process_pull_request(
    state: &AppState,
    delivery: &ClaimedDelivery,
    action: &str,
) -> anyhow::Result<Completion> {
    let Some(github_repo_id) = delivery.github_repo_id else {
        return Ok(Completion::ignored());
    };
    // Fan out to every workspace connection of this repository.
    let repos = db::repositories::find_all_by_github_id(&state.pool, github_repo_id).await?;
    if repos.is_empty() {
        return Ok(Completion::ignored());
    }
    let Some(pr) = delivery.payload.as_ref().and_then(|p| p.get("pullRequest")) else {
        return Ok(Completion::ignored());
    };

    let number = pr.get("number").and_then(serde_json::Value::as_i64);
    let head_sha = pr.get("headSha").and_then(serde_json::Value::as_str);
    let base_ref = pr.get("baseRef").and_then(serde_json::Value::as_str);
    let head_repo = pr.get("headRepoId").and_then(serde_json::Value::as_i64);
    let base_repo = pr.get("baseRepoId").and_then(serde_json::Value::as_i64);
    let merged = pr
        .get("merged")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let event_row = |repository_id: Uuid,
                     outcome: &'static str,
                     reason: Option<&'static str>,
                     pipeline_ids: Vec<Uuid>,
                     skipped: Vec<serde_json::Value>| {
        let mut summary = serde_json::json!({ "skipped": skipped });
        if let Some(number) = number {
            summary["prNumber"] = serde_json::json!(number);
        }
        if merged {
            summary["merged"] = serde_json::json!(true);
        }
        EventRow {
            repository_id,
            git_ref: number.map(|n| format!("refs/pull/{n}/head")),
            head_sha: head_sha.map(str::to_string),
            outcome,
            ignored_reason: reason,
            pipeline_ids,
            sync_run_id: None,
            summary,
        }
    };
    // The same terminal outcome, stamped onto every workspace connection.
    let rows_for_all = |outcome: &'static str, reason: Option<&'static str>| {
        repos
            .iter()
            .map(|repo| event_row(repo.id, outcome, reason, Vec::new(), Vec::new()))
            .collect::<Vec<_>>()
    };

    // A closed PR never creates a pipeline here: a merge produces a push
    // event for the merge commit, which flows through push triggers.
    if action == "closed" {
        return Ok(Completion::processed(rows_for_all(
            OUTCOME_IGNORED,
            Some(REASON_PR_CLOSED),
        )));
    }
    if action.is_empty() {
        return Ok(Completion::processed(rows_for_all(
            OUTCOME_IGNORED,
            Some(REASON_PR_ACTION_IGNORED),
        )));
    }

    // Fork PRs never execute: workspace secrets must not flow to fork code,
    // and the checkout token could not fetch the fork anyway. Missing repo
    // ids fail safe (treated as a fork).
    let same_repo = matches!((head_repo, base_repo), (Some(h), Some(b)) if h == b);
    if !same_repo {
        return Ok(Completion::processed(rows_for_all(
            OUTCOME_IGNORED,
            Some(REASON_FORK_PR_SKIPPED),
        )));
    }

    let (Some(number), Some(head_sha), Some(base_ref)) = (number, head_sha, base_ref) else {
        return Ok(Completion::processed(rows_for_all(
            OUTCOME_IGNORED,
            Some(REASON_NO_HEAD_COMMIT),
        )));
    };
    let Ok(pr_number) = i32::try_from(number) else {
        return Ok(Completion::processed(rows_for_all(
            OUTCOME_IGNORED,
            Some(REASON_PR_ACTION_IGNORED),
        )));
    };

    let pr_ctx = PrContext {
        number: pr_number,
        title: pr
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        user_login: pr
            .get("userLogin")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    };
    let git_ref = format!("refs/pull/{pr_number}/head");
    // PR changed files are not in the webhook payload; path filters fail
    // open (deliberate — no List-PR-files API call this phase).
    let ctx = trigger_eval::EventContext {
        kind: trigger_eval::EventKind::PullRequest {
            action,
            base_branch: base_ref,
        },
        changed_paths: None,
    };
    let mut events = Vec::with_capacity(repos.len());
    for repo in &repos {
        let (pipeline_ids, skipped, considered) = run_matching_workflows(
            state,
            delivery,
            repo,
            "pull_request",
            head_sha,
            &git_ref,
            Some(&pr_ctx),
            &ctx,
        )
        .await?;

        let (outcome, reason) = if pipeline_ids.is_empty() {
            (
                OUTCOME_IGNORED,
                Some(if considered == 0 {
                    REASON_NO_MATCHING_WORKFLOWS
                } else {
                    REASON_FILTERS_NOT_MATCHED
                }),
            )
        } else {
            (OUTCOME_PIPELINES, None)
        };
        events.push(event_row(repo.id, outcome, reason, pipeline_ids, skipped));
    }
    Ok(Completion::processed(events))
}

async fn process_repository(
    state: &AppState,
    delivery: &ClaimedDelivery,
    action: &str,
) -> anyhow::Result<Completion> {
    let Some(github_repo_id) = delivery.github_repo_id else {
        return Ok(Completion::ignored());
    };
    // Fan out to every workspace connection of this repository.
    let repos = db::repositories::find_all_by_github_id(&state.pool, github_repo_id).await?;
    if repos.is_empty() {
        return Ok(Completion::ignored());
    }
    match action {
        "deleted" => {
            // mark_failed keys on the GitHub id, so one call covers every
            // workspace connection.
            db::repositories::mark_failed(
                &state.pool,
                &[github_repo_id],
                "repository deleted on github",
            )
            .await?;
            Ok(Completion::processed(
                repos
                    .iter()
                    .map(|repo| EventRow {
                        repository_id: repo.id,
                        git_ref: None,
                        head_sha: None,
                        outcome: OUTCOME_IGNORED,
                        ignored_reason: Some(REASON_REPOSITORY_DELETED),
                        pipeline_ids: Vec::new(),
                        sync_run_id: None,
                        summary: serde_json::json!({}),
                    })
                    .collect(),
            ))
        }
        "renamed" | "edited" | "privatized" | "publicized" | "transferred" => {
            let mut events = Vec::with_capacity(repos.len());
            for repo in &repos {
                let sync_run_id = repo_sync::schedule(state, repo.id, "webhook").await?;
                events.push(EventRow {
                    repository_id: repo.id,
                    git_ref: None,
                    head_sha: None,
                    outcome: OUTCOME_SYNC,
                    ignored_reason: None,
                    pipeline_ids: Vec::new(),
                    sync_run_id,
                    summary: if sync_run_id.is_none() {
                        serde_json::json!({ "syncCollapsed": true })
                    } else {
                        serde_json::json!({})
                    },
                });
            }
            Ok(Completion::processed(events))
        }
        _ => Ok(Completion::processed(
            repos
                .iter()
                .map(|repo| EventRow {
                    repository_id: repo.id,
                    git_ref: None,
                    head_sha: None,
                    outcome: OUTCOME_IGNORED,
                    ignored_reason: Some(REASON_EVENT_NOT_SUPPORTED),
                    pipeline_ids: Vec::new(),
                    sync_run_id: None,
                    summary: serde_json::json!({}),
                })
                .collect(),
        )),
    }
}

async fn process_installation(
    state: &AppState,
    delivery: &ClaimedDelivery,
    action: &str,
) -> anyhow::Result<Completion> {
    let Some(installation_id) = delivery.installation_id else {
        return Ok(Completion::ignored());
    };
    match action {
        // Recorded so an org install can be claimed via the setup redirect
        // by the member who performed it.
        "created" => {
            let account = payload_str(delivery, "installationAccountLogin").unwrap_or("");
            let sender = payload_str(delivery, "senderLogin").unwrap_or("");
            if !account.is_empty() && !sender.is_empty() {
                db::github_installations::record_created_event(
                    &state.pool,
                    installation_id,
                    account,
                    sender,
                )
                .await?;
            }
        }
        "deleted" => {
            state.github_app.evict_token(installation_id).await;
            db::github_installations::delete_by_installation_id(&state.pool, installation_id)
                .await?;
        }
        "suspend" => {
            state.github_app.evict_token(installation_id).await;
            db::github_installations::set_suspended(&state.pool, installation_id, true).await?;
        }
        "unsuspend" => {
            db::github_installations::set_suspended(&state.pool, installation_id, false).await?;
        }
        _ => {}
    }
    // Installation lifecycle targets the installation, not a repository —
    // no timeline row.
    Ok(Completion::processed(Vec::new()))
}

async fn process_installation_repositories(
    state: &AppState,
    delivery: &ClaimedDelivery,
    action: &str,
) -> anyhow::Result<Completion> {
    if action != "removed" {
        return Ok(Completion::processed(Vec::new()));
    }
    let ids: Vec<i64> = delivery
        .payload
        .as_ref()
        .and_then(|p| p.get("repositoriesRemoved"))
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(serde_json::Value::as_i64)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Ok(Completion::processed(Vec::new()));
    }

    // Timeline rows only for repositories actually connected here — one per
    // workspace connection.
    let mut events = Vec::new();
    let mut seen = HashSet::new();
    for github_repo_id in &ids {
        if !seen.insert(*github_repo_id) {
            continue;
        }
        for repo in db::repositories::find_all_by_github_id(&state.pool, *github_repo_id).await? {
            events.push(EventRow {
                repository_id: repo.id,
                git_ref: None,
                head_sha: None,
                outcome: OUTCOME_IGNORED,
                ignored_reason: Some(REASON_ACCESS_REVOKED),
                pipeline_ids: Vec::new(),
                sync_run_id: None,
                summary: serde_json::json!({}),
            });
        }
    }
    db::repositories::mark_failed(&state.pool, &ids, "access revoked").await?;
    Ok(Completion::processed(events))
}
