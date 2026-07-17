//! GitHub webhook receiver. Mounted outside /api and /auth: no cookies, no
//! CSRF header, no CORS — authentication is the HMAC-SHA256 signature over
//! the raw body, verified in constant time before anything is parsed.
//! Payload contents and signatures are never logged.
//!
//! The handler does verification, validation, persistence, acknowledgement,
//! and a wake-up poke ONLY (GitHub's 10-second ack budget): the delivery is
//! stored as a normalized, server-built, capped payload in the durable
//! `webhook_deliveries` queue and every side effect — sync scheduling,
//! trigger evaluation, pipeline creation — happens asynchronously in
//! `services/webhook_processor.rs`.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::db;
use crate::services::github_app;
use crate::services::webhook_stats::SignatureError;
use crate::state::AppState;

type HmacSha256 = Hmac<Sha256>;

const MAX_HEADER_LEN: usize = 256;
/// Caps applied while normalizing the payload: everything stored is
/// server-built and bounded, never the raw body.
const MAX_FIELD_LEN: usize = 512;
const MAX_TITLE_LEN: usize = 200;
const MAX_CHANGED_PATHS: usize = 300;
const MAX_REMOVED_REPOS: usize = 100;

/// Events the background processor handles; everything else is recorded as
/// 'ignored' for observability and dropped.
const PROCESSED_EVENTS: &[&str] = &[
    "push",
    "pull_request",
    "repository",
    "installation",
    "installation_repositories",
];

/// Loose envelope: only the fields the normalizer needs, everything else is
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
    /// Push events: per-commit changed files feed the path-filter evaluation.
    commits: Option<Vec<PushCommitRef>>,
    /// Push events: total commit count — when it exceeds `commits.len()`
    /// GitHub truncated the list and path filters must fail open.
    size: Option<i64>,
    pull_request: Option<PullRequestRef>,
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
struct PushCommitRef {
    added: Option<Vec<String>>,
    removed: Option<Vec<String>>,
    modified: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PullRequestRef {
    number: i64,
    title: Option<String>,
    merged: Option<bool>,
    draft: Option<bool>,
    head: Option<PrSideRef>,
    base: Option<PrSideRef>,
    user: Option<SenderRef>,
}

#[derive(Debug, Deserialize)]
struct PrSideRef {
    sha: Option<String>,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    repo: Option<RepositoryRef>,
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
    avatar_url: Option<String>,
}

/// POST /webhooks/github
pub async fn receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // Read the identifying headers first so a rejection can name the delivery.
    // Both are UNAUTHENTICATED at this point: `header_str` caps them at
    // MAX_HEADER_LEN and they are only ever emitted as tracing fields (which
    // escape their values), never interpolated into a message or a query. The
    // BAD_REQUEST gates for absent headers stay below verification so an
    // unauthenticated caller can't tell the two rejections apart.
    let event = header_str(&headers, "x-github-event");
    let delivery_id = header_str(&headers, "x-github-delivery");

    // 1. Authenticate the delivery before touching the payload.
    if let Err(cause) = verify_signature(&state.config.github_webhook_secret, &headers, &body) {
        // Rejections never persist (unauthenticated input must not reach
        // Postgres), but the static cause feeds the in-memory gauge so the
        // repository health panel can show "deliveries are being rejected"
        // instead of a silently empty timeline.
        state.webhook_auth.record(cause);
        // Static cause only — never the signature, the expected digest, the
        // secret, or any body bytes. `cause=mismatch` means the configured
        // secret differs from the App's; `cause=missing_header` means the App
        // has no secret set at all.
        tracing::warn!(
            cause = cause.as_str(),
            event = event.as_deref().unwrap_or("unknown"),
            delivery_id = delivery_id.as_deref().unwrap_or("unknown"),
            "webhook delivery failed signature verification"
        );
        return StatusCode::UNAUTHORIZED;
    }

    let Some(event) = event else {
        return StatusCode::BAD_REQUEST;
    };
    let Some(delivery_id) = delivery_id else {
        return StatusCode::BAD_REQUEST;
    };

    // 2. Typed parse of the minimal envelope.
    let Ok(envelope) = serde_json::from_slice::<Envelope>(&body) else {
        tracing::warn!(event, "webhook payload failed to parse");
        return StatusCode::BAD_REQUEST;
    };

    // 3. Durable persistence — the idempotency gate AND the work queue.
    // Replayed deliveries are acknowledged and dropped; events the processor
    // handles are stored 'pending' with a normalized, capped payload.
    let installation_id = envelope.installation.as_ref().map(|i| i.id);
    let github_repo_id = envelope.repository.as_ref().map(|r| r.id);
    let (status, payload) = if PROCESSED_EVENTS.contains(&event.as_str()) {
        ("pending", Some(normalize_payload(&envelope)))
    } else {
        ("ignored", None)
    };
    match db::webhook_deliveries::insert(
        &state.pool,
        &delivery_id,
        &event,
        envelope.action.as_deref(),
        installation_id,
        github_repo_id,
        status,
        payload.as_ref(),
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

    // 4. Acknowledge immediately; the background worker does the rest.
    if status == "pending" {
        state.webhook_processor.poke();
    }
    StatusCode::ACCEPTED
}

/// Build the normalized payload the processor consumes: server-built JSON
/// with every field capped, avatar URLs sanitized, and changed paths
/// aggregated/deduped. The raw body is never stored.
fn normalize_payload(envelope: &Envelope) -> serde_json::Value {
    let cap = |s: &str| s.chars().take(MAX_FIELD_LEN).collect::<String>();

    let mut payload = serde_json::Map::new();

    if let Some(git_ref) = envelope.git_ref.as_deref() {
        payload.insert("ref".into(), cap(git_ref).into());
    }
    if let Some(after) = envelope.after.as_deref() {
        payload.insert("after".into(), cap(after).into());
    }
    if let Some(head) = &envelope.head_commit {
        if let Some(id) = head.id.as_deref() {
            payload.insert("headCommitSha".into(), cap(id).into());
        }
        if let Some(message) = head.message.as_deref() {
            // First line is enough, and caps stored size.
            payload.insert(
                "commitMessage".into(),
                cap(message.lines().next().unwrap_or("")).into(),
            );
        }
        if let Some(name) = head.author.as_ref().and_then(|a| a.name.as_deref()) {
            payload.insert("commitAuthor".into(), cap(name).into());
        }
    }
    if let Some(name) = envelope.pusher.as_ref().and_then(|p| p.name.as_deref()) {
        payload.insert("pusherName".into(), cap(name).into());
    }
    if let Some(sender) = &envelope.sender {
        if !sender.login.is_empty() && sender.login.len() <= 200 {
            payload.insert("senderLogin".into(), sender.login.clone().into());
        }
        if let Some(avatar) = github_app::sanitize_avatar_url(sender.avatar_url.as_deref()) {
            payload.insert("senderAvatarUrl".into(), avatar.into());
        }
    }
    if let Some(account) = envelope
        .installation
        .as_ref()
        .and_then(|i| i.account.as_ref())
    {
        payload.insert("installationAccountLogin".into(), cap(&account.login).into());
    }
    if let Some(removed) = &envelope.repositories_removed {
        let ids: Vec<i64> = removed.iter().take(MAX_REMOVED_REPOS).map(|r| r.id).collect();
        payload.insert("repositoriesRemoved".into(), serde_json::json!(ids));
    }

    // Changed paths for push path-filter evaluation: aggregated across
    // commits, deduped, capped. `pathsTruncated` marks an incomplete set —
    // the trigger evaluator then fails open.
    if let Some(commits) = &envelope.commits {
        let mut paths: Vec<String> = Vec::new();
        let mut truncated = envelope.size.is_some_and(|s| s > commits.len() as i64);
        'outer: for commit in commits {
            for list in [&commit.added, &commit.removed, &commit.modified]
                .into_iter()
                .flatten()
            {
                for path in list {
                    if path.is_empty() || path.len() > MAX_FIELD_LEN {
                        continue;
                    }
                    if !paths.contains(path) {
                        if paths.len() >= MAX_CHANGED_PATHS {
                            truncated = true;
                            break 'outer;
                        }
                        paths.push(path.clone());
                    }
                }
            }
        }
        payload.insert("changedPaths".into(), serde_json::json!(paths));
        payload.insert("pathsTruncated".into(), truncated.into());
    }

    if let Some(pr) = &envelope.pull_request {
        let mut pr_json = serde_json::Map::new();
        pr_json.insert("number".into(), pr.number.into());
        if let Some(title) = pr.title.as_deref() {
            pr_json.insert(
                "title".into(),
                title
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(MAX_TITLE_LEN)
                    .collect::<String>()
                    .into(),
            );
        }
        if let Some(merged) = pr.merged {
            pr_json.insert("merged".into(), merged.into());
        }
        if let Some(draft) = pr.draft {
            pr_json.insert("draft".into(), draft.into());
        }
        if let Some(head) = &pr.head {
            if let Some(sha) = head.sha.as_deref() {
                pr_json.insert("headSha".into(), cap(sha).into());
            }
            if let Some(git_ref) = head.git_ref.as_deref() {
                pr_json.insert("headRef".into(), cap(git_ref).into());
            }
            if let Some(repo) = &head.repo {
                pr_json.insert("headRepoId".into(), repo.id.into());
            }
        }
        if let Some(base) = &pr.base {
            if let Some(git_ref) = base.git_ref.as_deref() {
                pr_json.insert("baseRef".into(), cap(git_ref).into());
            }
            if let Some(repo) = &base.repo {
                pr_json.insert("baseRepoId".into(), repo.id.into());
            }
        }
        if let Some(user) = &pr.user
            && !user.login.is_empty()
            && user.login.len() <= 200
        {
            pr_json.insert("userLogin".into(), user.login.clone().into());
        }
        payload.insert("pullRequest".into(), serde_json::Value::Object(pr_json));
    }

    serde_json::Value::Object(payload)
}

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    if value.is_empty() || value.len() > MAX_HEADER_LEN {
        return None;
    }
    Some(value.to_string())
}

// `SignatureError` (the static rejection-cause vocabulary) lives in
// `services/webhook_stats.rs` next to the rejection gauge it feeds.

/// HMAC-SHA256 over the raw body with the configured secret, compared in
/// constant time against `X-Hub-Signature-256: sha256=<hex>`.
fn verify_signature(
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), SignatureError> {
    let Some(raw) = headers.get("x-hub-signature-256") else {
        return Err(SignatureError::MissingHeader);
    };
    let Ok(signature) = raw.to_str() else {
        return Err(SignatureError::MalformedHeader);
    };
    let Some(hex_digest) = signature.strip_prefix("sha256=") else {
        return Err(SignatureError::BadPrefix);
    };
    let Ok(expected) = hex::decode(hex_digest) else {
        return Err(SignatureError::InvalidHex);
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        // Unreachable: HMAC accepts any key length. Treated as a mismatch
        // rather than unwrapped so a future key-type change can't panic the
        // receiver.
        return Err(SignatureError::Mismatch);
    };
    mac.update(body);
    // verify_slice is constant-time and length-checks, so a truncated digest
    // fails cleanly rather than matching a prefix.
    mac.verify_slice(&expected)
        .map_err(|_| SignatureError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector published in GitHub's "Validating webhook deliveries" docs.
    /// Pins the wire format (raw body bytes, secret as raw UTF-8, lowercase
    /// hex) rather than merely proving we agree with ourselves.
    const DOC_SECRET: &str = "It's a Secret to Everybody";
    const DOC_BODY: &[u8] = b"Hello, World!";
    const DOC_SIGNATURE: &str =
        "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";

    fn signed(signature: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-hub-signature-256", signature.parse().unwrap());
        headers
    }

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn accepts_githubs_documented_vector() {
        assert_eq!(
            verify_signature(DOC_SECRET, &signed(DOC_SIGNATURE), DOC_BODY),
            Ok(())
        );
    }

    #[test]
    fn rejects_tampered_body() {
        assert_eq!(
            verify_signature(DOC_SECRET, &signed(DOC_SIGNATURE), b"Hello, World?"),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn rejects_wrong_secret() {
        assert_eq!(
            verify_signature("some other secret entirely", &signed(DOC_SIGNATURE), DOC_BODY),
            Err(SignatureError::Mismatch)
        );
    }

    /// Regression test for the 401 storm: a secret carrying a trailing newline
    /// (pasted into an env panel, or read from a file) computes a different MAC
    /// from the same secret trimmed. Config trims at load; this proves why it
    /// must. See config.rs `github_webhook_secret`.
    #[test]
    fn untrimmed_secret_does_not_verify() {
        let signature = sign(DOC_SECRET, DOC_BODY);
        // GitHub signs with the real secret; we'd verify with the padded one.
        for padded in [
            format!("{DOC_SECRET}\n"),
            format!("{DOC_SECRET} "),
            format!(" {DOC_SECRET}"),
            format!("{DOC_SECRET}\r\n"),
        ] {
            assert_eq!(
                verify_signature(&padded, &signed(&signature), DOC_BODY),
                Err(SignatureError::Mismatch),
                "padded secret must not verify — trimming at load is what fixes this"
            );
            // ...and trimming is exactly what recovers it.
            assert_eq!(
                verify_signature(padded.trim(), &signed(&signature), DOC_BODY),
                Ok(())
            );
        }
    }

    /// Quotes surviving from an env panel are a distinct silent failure: they
    /// pass the length check and fail every MAC. config.rs warns about this.
    #[test]
    fn quote_wrapped_secret_does_not_verify() {
        let signature = sign(DOC_SECRET, DOC_BODY);
        assert_eq!(
            verify_signature(&format!("\"{DOC_SECRET}\""), &signed(&signature), DOC_BODY),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn distinguishes_header_failure_causes() {
        // Absent header — the App has no webhook secret configured.
        assert_eq!(
            verify_signature(DOC_SECRET, &HeaderMap::new(), DOC_BODY),
            Err(SignatureError::MissingHeader)
        );
        // A sha1-only sender.
        assert_eq!(
            verify_signature(DOC_SECRET, &signed("sha1=abcdef"), DOC_BODY),
            Err(SignatureError::BadPrefix)
        );
        // Prefix matching is case-sensitive by design; GitHub sends lowercase.
        assert_eq!(
            verify_signature(DOC_SECRET, &signed("SHA256=abcdef"), DOC_BODY),
            Err(SignatureError::BadPrefix)
        );
        assert_eq!(
            verify_signature(DOC_SECRET, &signed("sha256=nothexatall"), DOC_BODY),
            Err(SignatureError::InvalidHex)
        );
    }

    /// A truncated digest must fail cleanly, not match on a prefix and not
    /// panic — verify_slice length-checks.
    #[test]
    fn rejects_truncated_digest() {
        let full = sign(DOC_SECRET, DOC_BODY);
        let truncated = &full[..full.len() - 8];
        assert_eq!(
            verify_signature(DOC_SECRET, &signed(truncated), DOC_BODY),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            verify_signature(DOC_SECRET, &signed("sha256="), DOC_BODY),
            Err(SignatureError::Mismatch)
        );
    }

    /// The seam the outage actually lived at: a real env var carrying a
    /// trailing newline, loaded the way boot loads it, verified against a
    /// signature GitHub computed from the clean secret. Chains
    /// config::shared_secret → verify_signature so neither side can regress
    /// alone. Uses a uniquely-named var (env is process-global and tests run
    /// in parallel).
    #[test]
    fn env_secret_with_trailing_newline_verifies_after_config_load() {
        const VAR: &str = "OVERUP_TEST_WEBHOOK_SECRET_SEAM";
        let clean = "0123456789abcdef0123456789abcdef";
        // What GitHub signs with — the secret as configured on the App.
        let signature = sign(clean, DOC_BODY);

        // What the env panel actually hands the process.
        unsafe { std::env::set_var(VAR, format!("{clean}\n")) };
        let raw = std::env::var(VAR).unwrap();

        // Before the fix this raw value reached the MAC and 401'd everything.
        assert_eq!(
            verify_signature(&raw, &signed(&signature), DOC_BODY),
            Err(SignatureError::Mismatch)
        );

        // Through the real load path, it verifies.
        let loaded = crate::config::shared_secret(VAR, raw, 16).unwrap();
        assert_eq!(verify_signature(&loaded, &signed(&signature), DOC_BODY), Ok(()));

        unsafe { std::env::remove_var(VAR) };
    }

    /// Causes are static, log-safe labels — no secret or digest material.
    #[test]
    fn cause_labels_are_static_categories() {
        for (cause, label) in [
            (SignatureError::MissingHeader, "missing_header"),
            (SignatureError::MalformedHeader, "malformed_header"),
            (SignatureError::BadPrefix, "bad_prefix"),
            (SignatureError::InvalidHex, "invalid_hex"),
            (SignatureError::Mismatch, "mismatch"),
        ] {
            assert_eq!(cause.as_str(), label);
        }
    }

    #[test]
    fn event_routing_table() {
        for event in PROCESSED_EVENTS {
            assert!(PROCESSED_EVENTS.contains(event));
        }
        for event in ["create", "delete", "check_suite", "workflow_run", "star"] {
            assert!(!PROCESSED_EVENTS.contains(&event));
        }
    }

    #[test]
    fn normalize_caps_and_aggregates_changed_paths() {
        let body = serde_json::json!({
            "ref": "refs/heads/main",
            "after": "abc123",
            "size": 2,
            "head_commit": {
                "id": "abc123",
                "message": "first line\nsecond line",
                "author": { "name": "Dev" }
            },
            "sender": { "login": "octocat", "avatar_url": "https://avatars.githubusercontent.com/u/1" },
            "commits": [
                { "added": ["src/a.rs"], "modified": ["src/a.rs", "README.md"], "removed": [] },
                { "added": [], "modified": ["docs/x.md"], "removed": ["old.txt"] }
            ]
        });
        let envelope: Envelope = serde_json::from_value(body).unwrap();
        let payload = normalize_payload(&envelope);
        assert_eq!(payload["commitMessage"], "first line");
        assert_eq!(payload["senderLogin"], "octocat");
        let paths: Vec<&str> = payload["changedPaths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        // Aggregation order: per commit, added → removed → modified.
        assert_eq!(paths, vec!["src/a.rs", "README.md", "old.txt", "docs/x.md"]);
        assert_eq!(payload["pathsTruncated"], false);
    }

    #[test]
    fn normalize_flags_truncated_pushes() {
        // size > commits.len(): GitHub truncated the commit list.
        let body = serde_json::json!({
            "ref": "refs/heads/main",
            "size": 50,
            "commits": [ { "added": ["a.txt"] } ]
        });
        let envelope: Envelope = serde_json::from_value(body).unwrap();
        let payload = normalize_payload(&envelope);
        assert_eq!(payload["pathsTruncated"], true);

        // Path-cap overflow also flags truncation.
        let many: Vec<String> = (0..(MAX_CHANGED_PATHS + 10))
            .map(|i| format!("file-{i}.rs"))
            .collect();
        let body = serde_json::json!({
            "ref": "refs/heads/main",
            "size": 1,
            "commits": [ { "added": many } ]
        });
        let envelope: Envelope = serde_json::from_value(body).unwrap();
        let payload = normalize_payload(&envelope);
        assert_eq!(
            payload["changedPaths"].as_array().unwrap().len(),
            MAX_CHANGED_PATHS
        );
        assert_eq!(payload["pathsTruncated"], true);
    }

    #[test]
    fn normalize_extracts_pull_request_context() {
        let body = serde_json::json!({
            "action": "opened",
            "pull_request": {
                "number": 42,
                "title": "Add feature\nwith details",
                "merged": false,
                "draft": false,
                "head": { "sha": "headsha", "ref": "feature/x", "repo": { "id": 7 } },
                "base": { "sha": "basesha", "ref": "main", "repo": { "id": 7 } },
                "user": { "login": "contributor" }
            }
        });
        let envelope: Envelope = serde_json::from_value(body).unwrap();
        let payload = normalize_payload(&envelope);
        let pr = &payload["pullRequest"];
        assert_eq!(pr["number"], 42);
        assert_eq!(pr["title"], "Add feature");
        assert_eq!(pr["headSha"], "headsha");
        assert_eq!(pr["headRepoId"], 7);
        assert_eq!(pr["baseRef"], "main");
        assert_eq!(pr["userLogin"], "contributor");
    }
}
