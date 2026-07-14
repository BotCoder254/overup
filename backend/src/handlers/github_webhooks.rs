//! GitHub webhook receiver. Mounted outside /api and /auth: no cookies, no
//! CSRF header, no CORS — authentication is the HMAC-SHA256 signature over
//! the raw body, verified in constant time before anything is parsed.
//! Payload contents and signatures are never logged.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::db;
use crate::services::{pipeline_run, repo_sync};
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

const MAX_HEADER_LEN: usize = 256;

/// Loose envelope: only the fields the dispatcher needs, everything else is
/// ignored by serde. Strong typing per event keeps parsing unambiguous.
#[derive(Debug, Deserialize)]
struct Envelope {
    action: Option<String>,
    installation: Option<InstallationRef>,
    repository: Option<RepositoryRef>,
    repositories_removed: Option<Vec<RepositoryIdRef>>,
    sender: Option<SenderRef>,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    /// Push events: the post-push head SHA and commit context — everything a
    /// pipeline needs to identify what to build.
    after: Option<String>,
    head_commit: Option<HeadCommitRef>,
    pusher: Option<PusherRef>,
}

#[derive(Debug, Deserialize)]
struct HeadCommitRef {
    id: Option<String>,
    message: Option<String>,
    author: Option<CommitAuthorRef>,
}

#[derive(Debug, Deserialize)]
struct CommitAuthorRef {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PusherRef {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InstallationRef {
    id: i64,
    account: Option<AccountRef>,
}

#[derive(Debug, Deserialize)]
struct AccountRef {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryRef {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct RepositoryIdRef {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct SenderRef {
    login: String,
}

/// POST /webhooks/github
pub async fn receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // 1. Authenticate the delivery before touching the payload.
    if !verify_signature(&state.config.github_webhook_secret, &headers, &body) {
        // Generic 401; nothing about the payload or signature is logged.
        tracing::warn!("webhook delivery failed signature verification");
        return StatusCode::UNAUTHORIZED;
    }

    let Some(event) = header_str(&headers, "x-github-event") else {
        return StatusCode::BAD_REQUEST;
    };
    let Some(delivery_id) = header_str(&headers, "x-github-delivery") else {
        return StatusCode::BAD_REQUEST;
    };

    // 2. Typed parse of the minimal envelope.
    let Ok(envelope) = serde_json::from_slice::<Envelope>(&body) else {
        tracing::warn!(event, "webhook payload failed to parse");
        return StatusCode::BAD_REQUEST;
    };

    // 3. Idempotency gate: replays are acknowledged and dropped.
    let installation_id = envelope.installation.as_ref().map(|i| i.id);
    match db::webhook_deliveries::insert(
        &state.pool,
        &delivery_id,
        &event,
        envelope.action.as_deref(),
        installation_id,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => return StatusCode::OK, // duplicate delivery
        Err(error) => {
            tracing::error!(error = ?error, "failed to record webhook delivery");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }

    // 4. Dispatch. Failures are recorded but still acknowledged with 2xx so
    // GitHub does not retry a delivery we have already claimed.
    if let Err(error) = dispatch(&state, &event, &envelope).await {
        tracing::error!(error = ?error, event, "webhook processing failed");
        let _ = db::webhook_deliveries::set_status(&state.pool, &delivery_id, "failed").await;
    }

    StatusCode::ACCEPTED
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    if value.is_empty() || value.len() > MAX_HEADER_LEN {
        return None;
    }
    Some(value.to_string())
}

/// HMAC-SHA256 over the raw body with the configured secret, compared in
/// constant time against `X-Hub-Signature-256: sha256=<hex>`.
fn verify_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(signature) = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(hex_digest) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_digest) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    // verify_slice is constant-time.
    mac.verify_slice(&expected).is_ok()
}

async fn dispatch(state: &AppState, event: &str, envelope: &Envelope) -> anyhow::Result<()> {
    let action = envelope.action.as_deref().unwrap_or("");
    match event {
        // A push to a connected repository's default branch re-syncs it.
        // The sync itself diffs blob shas, so this stays cheap even when
        // no workflow file changed.
        "push" => {
            let (Some(repository), Some(git_ref)) = (&envelope.repository, &envelope.git_ref)
            else {
                return Ok(());
            };
            let Some(repo) =
                db::repositories::find_by_github_id(&state.pool, repository.id).await?
            else {
                return Ok(());
            };
            if *git_ref == format!("refs/heads/{}", repo.default_branch) {
                repo_sync::schedule(state, repo.id, "webhook").await?;
            }
            // Pipelines trigger for any branch push carrying a real head
            // commit (branch deletions send after = 0000...).
            if git_ref.starts_with("refs/heads/") {
                trigger_push_pipelines(state, &repo, envelope, git_ref).await?;
            }
        }
        "repository" => {
            let Some(repository) = &envelope.repository else {
                return Ok(());
            };
            match action {
                "deleted" => {
                    db::repositories::mark_failed(
                        &state.pool,
                        &[repository.id],
                        "repository deleted on github",
                    )
                    .await?;
                }
                "renamed" | "edited" | "privatized" | "publicized" | "transferred" => {
                    if let Some(repo) =
                        db::repositories::find_by_github_id(&state.pool, repository.id).await?
                    {
                        repo_sync::schedule(state, repo.id, "webhook").await?;
                    }
                }
                _ => {}
            }
        }
        "installation" => {
            let Some(installation) = &envelope.installation else {
                return Ok(());
            };
            match action {
                // Recorded so an org install can be claimed via the setup
                // redirect by the member who performed it.
                "created" => {
                    let account = installation
                        .account
                        .as_ref()
                        .map(|a| a.login.as_str())
                        .unwrap_or("");
                    let sender = envelope
                        .sender
                        .as_ref()
                        .map(|s| s.login.as_str())
                        .unwrap_or("");
                    if !account.is_empty() && !sender.is_empty() {
                        db::github_installations::record_created_event(
                            &state.pool,
                            installation.id,
                            account,
                            sender,
                        )
                        .await?;
                    }
                }
                "deleted" => {
                    state.github_app.evict_token(installation.id).await;
                    db::github_installations::delete_by_installation_id(
                        &state.pool,
                        installation.id,
                    )
                    .await?;
                }
                "suspend" => {
                    state.github_app.evict_token(installation.id).await;
                    db::github_installations::set_suspended(&state.pool, installation.id, true)
                        .await?;
                }
                "unsuspend" => {
                    db::github_installations::set_suspended(&state.pool, installation.id, false)
                        .await?;
                }
                _ => {}
            }
        }
        "installation_repositories" => {
            if action == "removed"
                && let Some(removed) = &envelope.repositories_removed
            {
                let ids: Vec<i64> = removed.iter().map(|r| r.id).collect();
                if !ids.is_empty() {
                    db::repositories::mark_failed(&state.pool, &ids, "access revoked").await?;
                }
            }
        }
        // Recorded (step 3) for observability. Extension points: pipeline
        // triggers for pull_request / tag events land here.
        "pull_request" | "create" | "delete" => {}
        _ => {}
    }
    Ok(())
}

/// Create one pipeline per push-triggered workflow of this repository.
/// Failures are per-workflow: one bad workflow never blocks the others.
async fn trigger_push_pipelines(
    state: &AppState,
    repo: &crate::models::repository::Repository,
    envelope: &Envelope,
    git_ref: &str,
) -> anyhow::Result<()> {
    let commit_sha = envelope
        .after
        .as_deref()
        .or(envelope
            .head_commit
            .as_ref()
            .and_then(|c| c.id.as_deref()))
        .unwrap_or("");
    // Branch deletions push an all-zero SHA; nothing to build.
    if commit_sha.is_empty() || commit_sha.chars().all(|c| c == '0') {
        return Ok(());
    }

    let commit_message = envelope
        .head_commit
        .as_ref()
        .and_then(|c| c.message.as_deref())
        // First line is enough, and caps stored size.
        .map(|m| m.lines().next().unwrap_or("").to_string());
    let commit_author = envelope
        .head_commit
        .as_ref()
        .and_then(|c| c.author.as_ref())
        .and_then(|a| a.name.as_deref())
        .or(envelope.pusher.as_ref().and_then(|p| p.name.as_deref()));

    let workflows = db::workflows::push_runnable_for_repo(&state.pool, repo.id).await?;
    for workflow in workflows {
        let ctx = pipeline_run::TriggerContext {
            trigger: "push",
            triggered_by: None,
            commit_sha,
            commit_message: commit_message.as_deref(),
            commit_author,
            git_ref,
            inputs: None,
            request_id: None,
        };
        if let Err(error) = pipeline_run::create_pipeline(
            state,
            repo,
            workflow.id,
            &workflow.name,
            &workflow.path,
            &workflow.raw_content,
            &ctx,
        )
        .await
        {
            // Validation failures (e.g. uses:-only workflows) are expected;
            // they must not fail the whole delivery.
            tracing::debug!(
                workflow = %workflow.path,
                error = ?error,
                "push did not trigger a pipeline for this workflow"
            );
        }
    }
    Ok(())
}
