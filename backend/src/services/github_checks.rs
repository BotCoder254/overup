//! GitHub Checks API reporting: pipelines triggered by repository events
//! (push / pull_request / tag) surface as check runs on their commit —
//! queued at creation, in_progress on first job start, completed with a
//! mapped conclusion and a details link back to the pipeline page.
//!
//! Everything here is best-effort and spawned off the caller's path: a
//! reporting failure can never fail, delay, or retry a pipeline. The check
//! payloads are server-built from static templates and job counts — never
//! runner or upstream text. Manual dispatches deliberately do not report
//! (only event-triggered pipelines represent repository state on GitHub).
//!
//! Requires the GitHub App's Checks (Read & write) permission. When the app
//! lacks it (403/422 from GitHub) the installation is marked unavailable
//! for an hour with a single edge-triggered warning — the operator action
//! (grant the permission, approve on installations) is logged once, not
//! sprayed per pipeline.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::db;
use crate::services::github_app;
use crate::state::AppState;

const GITHUB_API: &str = "https://api.github.com";
/// A permission-denied installation is retried after this long.
const UNAVAILABLE_RETRY: Duration = Duration::from_secs(3600);
/// Triggers that report to GitHub. Manual dispatch/rerun never does.
const REPORTED_TRIGGERS: &[&str] = &["push", "pull_request", "tag"];

/// Per-installation availability cache; held in AppState.
#[derive(Default)]
pub struct GithubChecks {
    unavailable: RwLock<HashMap<i64, Instant>>,
}

impl GithubChecks {
    async fn is_unavailable(&self, installation_id: i64) -> bool {
        let mut map = self.unavailable.write().await;
        match map.get(&installation_id) {
            Some(marked) if marked.elapsed() < UNAVAILABLE_RETRY => true,
            Some(_) => {
                map.remove(&installation_id);
                false
            }
            None => false,
        }
    }

    /// Returns true when this installation was newly marked — the caller
    /// warns exactly once per outage edge.
    async fn mark_unavailable(&self, installation_id: i64) -> bool {
        self.unavailable
            .write()
            .await
            .insert(installation_id, Instant::now())
            .is_none()
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Create,
    Started,
    Completed,
}

/// Report a queued check run for a freshly created pipeline.
pub fn spawn_create(state: &AppState, pipeline_id: Uuid) {
    spawn(state, pipeline_id, Phase::Create);
}

/// Flip the check run to in_progress (first job started).
pub fn spawn_started(state: &AppState, pipeline_id: Uuid) {
    spawn(state, pipeline_id, Phase::Started);
}

/// Complete the check run with the pipeline's conclusion.
pub fn spawn_completed(state: &AppState, pipeline_id: Uuid) {
    spawn(state, pipeline_id, Phase::Completed);
}

fn spawn(state: &AppState, pipeline_id: Uuid, phase: Phase) {
    if !state.config.github_checks_enabled {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        report(&state, pipeline_id, phase).await;
    });
}

/// Map an overup pipeline conclusion onto GitHub's check-run vocabulary.
/// `partial` (some jobs succeeded, some failed) reads as failure on GitHub,
/// with the summary carrying the nuance.
pub fn map_conclusion(conclusion: &str) -> &'static str {
    match conclusion {
        "success" => "success",
        "cancelled" => "cancelled",
        "timed_out" => "timed_out",
        // failure, partial, and anything unexpected fail closed.
        _ => "failure",
    }
}

async fn report(state: &AppState, pipeline_id: Uuid, phase: Phase) {
    let ctx = match db::pipelines::checks_context(&state.pool, pipeline_id).await {
        Ok(Some(ctx)) => ctx,
        Ok(None) => return, // pipeline/repository gone — nothing to report
        Err(error) => {
            tracing::debug!(error = ?error, %pipeline_id, "checks context lookup failed");
            return;
        }
    };
    if !REPORTED_TRIGGERS.contains(&ctx.trigger.as_str()) {
        return;
    }
    // Defense-in-depth before URL interpolation: owner/name are our own
    // synced metadata (and the workspace slug our own generated value), but
    // all three still must pass the segment allow-list — owner/name land in
    // the GitHub API path, the slug in the details_url sent to GitHub.
    if !github_app::is_safe_name_segment(&ctx.repo_owner)
        || !github_app::is_safe_name_segment(&ctx.repo_name)
        || !github_app::is_safe_name_segment(&ctx.workspace_slug)
    {
        return;
    }
    // The in_progress flip needs the created check run; completion instead
    // falls back to creating the run directly in `completed` status — a fast
    // pipeline can finish before the async create round-trip persists the id,
    // and skipping would leave the GitHub check stuck at `queued` forever.
    if matches!(phase, Phase::Started) && ctx.check_run_id.is_none() {
        return;
    }
    if state.github_checks.is_unavailable(ctx.installation_id).await {
        return;
    }

    let token = match state
        .github_app
        .checks_token(&state.http, ctx.installation_id)
        .await
    {
        Ok(token) => token,
        Err(error) => {
            // Minting fails with 422 when the app lacks the Checks
            // permission; the error text carries the status only.
            let newly = state
                .github_checks
                .mark_unavailable(ctx.installation_id)
                .await;
            if newly {
                tracing::warn!(
                    installation_id = ctx.installation_id,
                    error = ?error,
                    "GitHub checks reporting unavailable — grant the App the \
                     Checks (Read & write) permission and approve it on the \
                     installation; retrying hourly"
                );
            }
            return;
        }
    };

    let details_url = format!(
        "{}/w/{}/pipelines/{}",
        state.config.frontend_url, ctx.workspace_slug, pipeline_id
    );
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let (method_url, body) = match phase {
        Phase::Create => (
            (
                reqwest::Method::POST,
                format!(
                    "{GITHUB_API}/repos/{}/{}/check-runs",
                    ctx.repo_owner, ctx.repo_name
                ),
            ),
            serde_json::json!({
                "name": format!("overup / {}", ctx.workflow_name),
                "head_sha": ctx.commit_sha,
                "status": "queued",
                "external_id": pipeline_id.to_string(),
                "details_url": details_url,
                "output": {
                    "title": "Pipeline queued",
                    "summary": "The pipeline is queued on overup. Follow the details link for live logs.",
                },
            }),
        ),
        Phase::Started => (
            (
                reqwest::Method::PATCH,
                format!(
                    "{GITHUB_API}/repos/{}/{}/check-runs/{}",
                    ctx.repo_owner,
                    ctx.repo_name,
                    ctx.check_run_id.unwrap_or_default()
                ),
            ),
            serde_json::json!({
                "status": "in_progress",
                "started_at": now,
                "details_url": details_url,
                "output": {
                    "title": "Pipeline running",
                    "summary": "The pipeline is executing on overup. Follow the details link for live logs.",
                },
            }),
        ),
        Phase::Completed => {
            let conclusion = ctx.conclusion.as_deref().unwrap_or("failure");
            let (title, summary) = match completion_output(state, pipeline_id, conclusion).await {
                Some(output) => output,
                None => return,
            };
            let mut body = serde_json::json!({
                "status": "completed",
                "conclusion": map_conclusion(conclusion),
                "completed_at": now,
                "details_url": details_url,
                "output": { "title": title, "summary": summary },
            });
            match ctx.check_run_id {
                Some(check_run_id) => (
                    (
                        reqwest::Method::PATCH,
                        format!(
                            "{GITHUB_API}/repos/{}/{}/check-runs/{}",
                            ctx.repo_owner, ctx.repo_name, check_run_id
                        ),
                    ),
                    body,
                ),
                // Fallback create: same name/head_sha/external_id as the
                // Create phase, so a late-landing queued run is superseded.
                None => {
                    body["name"] =
                        serde_json::Value::from(format!("overup / {}", ctx.workflow_name));
                    body["head_sha"] = serde_json::Value::from(ctx.commit_sha.clone());
                    body["external_id"] = serde_json::Value::from(pipeline_id.to_string());
                    (
                        (
                            reqwest::Method::POST,
                            format!(
                                "{GITHUB_API}/repos/{}/{}/check-runs",
                                ctx.repo_owner, ctx.repo_name
                            ),
                        ),
                        body,
                    )
                }
            }
        }
    };

    let response = state
        .http
        .request(method_url.0, &method_url.1)
        .bearer_auth(&token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .json(&body)
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => {
            // Both create paths (queued create and the completed fallback)
            // persist the returned id; the UPDATE is guarded on NULL so a
            // racing late create can never clobber it.
            if ctx.check_run_id.is_none() {
                #[derive(serde::Deserialize)]
                struct CreatedCheckRun {
                    id: i64,
                }
                if let Ok(created) = response.json::<CreatedCheckRun>().await
                    && let Err(error) =
                        db::pipelines::set_check_run_id(&state.pool, pipeline_id, created.id)
                            .await
                {
                    tracing::debug!(error = ?error, %pipeline_id, "failed to store check run id");
                }
            }
        }
        Ok(response)
            if response.status() == reqwest::StatusCode::FORBIDDEN
                || response.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY =>
        {
            let newly = state
                .github_checks
                .mark_unavailable(ctx.installation_id)
                .await;
            if newly {
                tracing::warn!(
                    installation_id = ctx.installation_id,
                    status = %response.status(),
                    "GitHub checks reporting unavailable — grant the App the \
                     Checks (Read & write) permission and approve it on the \
                     installation; retrying hourly"
                );
            }
        }
        Ok(response) => {
            // Status only — the body could echo request details.
            tracing::debug!(status = %response.status(), %pipeline_id, "check run report rejected");
        }
        Err(error) => {
            tracing::debug!(error = ?error, %pipeline_id, "check run report failed");
        }
    }
}

/// Static title + summary from server-side job counts only.
async fn completion_output(
    state: &AppState,
    pipeline_id: Uuid,
    conclusion: &str,
) -> Option<(String, String)> {
    let statuses = db::pipeline_jobs::statuses_for_pipeline(&state.pool, pipeline_id)
        .await
        .ok()?;
    let total = statuses.len();
    let count = |c: &str| {
        statuses
            .iter()
            .filter(|(_, conclusion)| conclusion.as_deref() == Some(c))
            .count()
    };
    let succeeded = count("success");
    let failed = count("failure") + count("timed_out");
    let title = match conclusion {
        "success" => "Pipeline succeeded".to_string(),
        "cancelled" => "Pipeline cancelled".to_string(),
        "timed_out" => "Pipeline timed out".to_string(),
        "partial" => "Pipeline partially succeeded".to_string(),
        _ => "Pipeline failed".to_string(),
    };
    let mut summary = format!("{succeeded} of {total} jobs succeeded.");
    if failed > 0 {
        summary.push_str(&format!(" {failed} failed."));
    }
    if conclusion == "partial" {
        summary.push_str(" Some jobs succeeded while others failed (reported as failure).");
    }
    summary.push_str(" Full logs and artifacts are on overup.");
    Some((title, summary))
}

#[cfg(test)]
mod tests {
    use super::map_conclusion;

    #[test]
    fn conclusion_mapping() {
        assert_eq!(map_conclusion("success"), "success");
        assert_eq!(map_conclusion("failure"), "failure");
        assert_eq!(map_conclusion("cancelled"), "cancelled");
        assert_eq!(map_conclusion("timed_out"), "timed_out");
        // partial reads as failure on GitHub; the summary carries the nuance.
        assert_eq!(map_conclusion("partial"), "failure");
        // Anything unexpected fails closed.
        assert_eq!(map_conclusion("bogus"), "failure");
    }
}
