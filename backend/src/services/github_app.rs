//! GitHub App authentication and API access.
//!
//! The backend authenticates as the GitHub App with a short-lived RS256 JWT
//! and exchanges it for repository-scoped installation access tokens (1-hour
//! lifetime). Installation tokens are cached in memory only — never logged,
//! never serialized into responses, never persisted to the database.

use std::collections::HashMap;

use anyhow::{Context, bail};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::Deserialize;
use tokio::sync::RwLock;

const GITHUB_API: &str = "https://api.github.com";

/// Hard cap on any workflow file we mirror; enforced again before parsing.
pub const MAX_WORKFLOW_FILE_BYTES: usize = 512 * 1024;

/// Pagination safety valve: never follow more than this many pages.
const MAX_PAGES: u32 = 10;

/// Refresh a cached token once it has less than this long to live.
const TOKEN_REFRESH_MARGIN_MINUTES: i64 = 5;

pub struct GitHubApp {
    client_id: String,
    encoding_key: EncodingKey,
    token_cache: RwLock<HashMap<i64, CachedToken>>,
    /// Separate cache for checks-scoped tokens: the sync token stays
    /// read-only (least privilege) and never grows write permissions.
    checks_token_cache: RwLock<HashMap<i64, CachedToken>>,
}

struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(serde::Serialize)]
struct AppJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct Installation {
    #[allow(dead_code)] // present in the API shape; callers key off the query id
    pub id: i64,
    pub account: InstallationAccount,
    pub suspended_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct InstallationAccount {
    pub login: String,
    #[serde(rename = "type")]
    pub account_type: String,
    pub avatar_url: Option<String>,
}

impl GitHubApp {
    /// Validates the private key eagerly so a bad PEM fails at startup, not
    /// on the first installation call.
    pub fn new(client_id: String, private_key_pem: &[u8]) -> anyhow::Result<Self> {
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem)
            .context("GitHub App private key is not a valid RSA PEM")?;
        Ok(Self {
            client_id,
            encoding_key,
            token_cache: RwLock::new(HashMap::new()),
            checks_token_cache: RwLock::new(HashMap::new()),
        })
    }

    /// Short-lived app JWT: iat backdated 60s for clock drift, exp well under
    /// GitHub's 10-minute ceiling, iss = the app's client ID.
    fn app_jwt(&self) -> anyhow::Result<String> {
        let now = Utc::now().timestamp();
        let claims = AppJwtClaims {
            iat: now - 60,
            exp: now + 540,
            iss: self.client_id.clone(),
        };
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &self.encoding_key)
            .context("failed to sign GitHub App JWT")
    }

    /// Installation access token, from cache when it still has comfortable
    /// slack, otherwise freshly minted and scoped to read-only permissions.
    pub async fn installation_token(
        &self,
        http: &reqwest::Client,
        installation_id: i64,
    ) -> anyhow::Result<String> {
        self.mint_scoped_token(
            http,
            installation_id,
            &self.token_cache,
            // Least privilege: the token can never do more than read.
            &serde_json::json!({
                "permissions": { "contents": "read", "metadata": "read" }
            }),
        )
        .await
    }

    /// Checks-scoped installation token for the Checks API reporter. Minted
    /// separately so the sync token stays read-only; requires the GitHub App
    /// to have the Checks (Read & write) permission — GitHub answers 422
    /// when it doesn't, which the reporter treats as "checks unavailable".
    pub async fn checks_token(
        &self,
        http: &reqwest::Client,
        installation_id: i64,
    ) -> anyhow::Result<String> {
        self.mint_scoped_token(
            http,
            installation_id,
            &self.checks_token_cache,
            &serde_json::json!({ "permissions": { "checks": "write" } }),
        )
        .await
    }

    async fn mint_scoped_token(
        &self,
        http: &reqwest::Client,
        installation_id: i64,
        cache: &RwLock<HashMap<i64, CachedToken>>,
        permissions: &serde_json::Value,
    ) -> anyhow::Result<String> {
        let refresh_after = Utc::now() + Duration::minutes(TOKEN_REFRESH_MARGIN_MINUTES);
        {
            let cache = cache.read().await;
            if let Some(cached) = cache.get(&installation_id)
                && cached.expires_at > refresh_after
            {
                return Ok(cached.token.clone());
            }
        }

        let jwt = self.app_jwt()?;
        let response = http
            .post(format!(
                "{GITHUB_API}/app/installations/{installation_id}/access_tokens"
            ))
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(permissions)
            .send()
            .await
            .context("installation token request failed")?;

        if !response.status().is_success() {
            // Status only — the body could echo request details.
            bail!(
                "installation token request returned {}",
                response.status()
            );
        }
        let minted: AccessTokenResponse = response
            .json()
            .await
            .context("installation token response was malformed")?;

        let token = minted.token.clone();
        cache.write().await.insert(
            installation_id,
            CachedToken {
                token: minted.token,
                expires_at: minted.expires_at,
            },
        );
        Ok(token)
    }

    /// Fetch an installation with app-JWT auth. Succeeding proves the
    /// installation belongs to *this* app — the setup handler relies on it
    /// to validate the untrusted installation_id query parameter.
    pub async fn get_installation(
        &self,
        http: &reqwest::Client,
        installation_id: i64,
    ) -> anyhow::Result<Installation> {
        let jwt = self.app_jwt()?;
        api_get(
            http,
            &jwt,
            &format!("{GITHUB_API}/app/installations/{installation_id}"),
        )
        .await
    }

    pub async fn evict_token(&self, installation_id: i64) {
        self.token_cache.write().await.remove(&installation_id);
        self.checks_token_cache
            .write()
            .await
            .remove(&installation_id);
    }
}

// ---------------------------------------------------------------------------
// Typed installation-token API calls
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GitHubRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    pub owner: RepoOwner,
    pub archived: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RepoOwner {
    pub login: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InstallationRepositories {
    repositories: Vec<GitHubRepo>,
}

#[derive(Debug, Deserialize)]
pub struct Branch {
    pub name: String,
    pub commit: BranchCommit,
}

#[derive(Debug, Deserialize)]
pub struct BranchCommit {
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct ContentEntry {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub size: i64,
    #[serde(rename = "type")]
    pub entry_type: String,
}

#[derive(Debug, Deserialize)]
struct FileContent {
    content: String,
    encoding: String,
}

#[derive(Debug, Deserialize)]
pub struct CommitListItem {
    pub sha: String,
    pub commit: CommitDetail,
}

#[derive(Debug, Deserialize)]
pub struct CommitDetail {
    pub message: String,
    pub committer: Option<CommitSignature>,
    pub author: Option<CommitSignature>,
}

#[derive(Debug, Deserialize)]
pub struct CommitSignature {
    pub date: Option<DateTime<Utc>>,
}

/// Owner/repo segments are interpolated into URLs; reject anything outside
/// GitHub's own naming rules so a poisoned value can never restructure a path.
pub fn is_safe_name_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !value.starts_with('.')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Upstream avatar URLs (GitHub API responses, webhook payloads) are stored
/// as opaque text, but only when they are https and sanely sized; anything
/// else becomes NULL — never an error, never logged.
pub fn sanitize_avatar_url(url: Option<&str>) -> Option<&str> {
    url.filter(|u| u.starts_with("https://") && u.len() <= 512)
}

fn require_safe_segments(owner: &str, repo: &str) -> anyhow::Result<()> {
    if !is_safe_name_segment(owner) || !is_safe_name_segment(repo) {
        bail!("repository owner or name contains disallowed characters");
    }
    Ok(())
}

/// Every repository accessible to the installation (private repos included),
/// paginated with a hard page cap.
pub async fn list_installation_repositories(
    http: &reqwest::Client,
    token: &str,
) -> anyhow::Result<Vec<GitHubRepo>> {
    let mut repos = Vec::new();
    for page in 1..=MAX_PAGES {
        let batch: InstallationRepositories = api_get(
            http,
            token,
            &format!("{GITHUB_API}/installation/repositories?per_page=100&page={page}"),
        )
        .await?;
        let len = batch.repositories.len();
        repos.extend(batch.repositories);
        if len < 100 {
            break;
        }
    }
    Ok(repos)
}

/// Repository by immutable numeric ID — stable across renames/transfers.
pub async fn get_repository(
    http: &reqwest::Client,
    token: &str,
    github_repo_id: i64,
) -> anyhow::Result<GitHubRepo> {
    api_get(
        http,
        token,
        &format!("{GITHUB_API}/repositories/{github_repo_id}"),
    )
    .await
}

pub async fn list_branches(
    http: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<Vec<Branch>> {
    require_safe_segments(owner, repo)?;
    api_get(
        http,
        token,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/branches?per_page=100"),
    )
    .await
}

/// Contents of `.github/workflows`. A missing directory simply means the
/// repository defines no workflows — not an error.
pub async fn list_workflow_dir(
    http: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
) -> anyhow::Result<Vec<ContentEntry>> {
    require_safe_segments(owner, repo)?;
    let response = http
        .get(format!(
            "{GITHUB_API}/repos/{owner}/{repo}/contents/.github/workflows?per_page=100"
        ))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let entries: Vec<ContentEntry> = response.error_for_status()?.json().await?;
    Ok(entries)
}

/// Fetch one workflow file via the contents API. Enforces the base64
/// encoding contract and the size cap before decoding.
pub async fn get_file_content(
    http: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    path: &str,
) -> anyhow::Result<String> {
    require_safe_segments(owner, repo)?;
    let file: FileContent = api_get(
        http,
        token,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/contents/{path}"),
    )
    .await?;
    if file.encoding != "base64" {
        bail!("unexpected content encoding {}", file.encoding);
    }
    // GitHub wraps base64 payloads in newlines.
    let compact: String = file.content.split_whitespace().collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(compact)
        .context("workflow file content was not valid base64")?;
    if bytes.len() > MAX_WORKFLOW_FILE_BYTES {
        bail!("workflow file exceeds the {MAX_WORKFLOW_FILE_BYTES} byte cap");
    }
    String::from_utf8(bytes).context("workflow file content was not valid UTF-8")
}

pub struct LastCommit {
    pub sha: String,
    pub message: String,
    pub date: Option<DateTime<Utc>>,
}

pub async fn last_commit_for_path(
    http: &reqwest::Client,
    token: &str,
    owner: &str,
    repo: &str,
    path: &str,
) -> anyhow::Result<Option<LastCommit>> {
    require_safe_segments(owner, repo)?;
    let commits: Vec<CommitListItem> = api_get(
        http,
        token,
        &format!("{GITHUB_API}/repos/{owner}/{repo}/commits?path={path}&per_page=1"),
    )
    .await?;
    Ok(commits.into_iter().next().map(|c| LastCommit {
        sha: c.sha,
        date: c
            .commit
            .committer
            .and_then(|s| s.date)
            .or(c.commit.author.and_then(|s| s.date)),
        // First line is enough for the UI; caps stored size.
        message: c.commit.message.lines().next().unwrap_or("").to_string(),
    }))
}

async fn api_get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    token: &str,
    url: &str,
) -> anyhow::Result<T> {
    let response = http
        .get(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json::<T>().await?)
}

#[cfg(test)]
mod tests {
    use super::sanitize_avatar_url;

    #[test]
    fn sanitize_avatar_url_accepts_https_within_cap() {
        assert_eq!(
            sanitize_avatar_url(Some("https://avatars.githubusercontent.com/u/1?v=4")),
            Some("https://avatars.githubusercontent.com/u/1?v=4"),
        );
    }

    #[test]
    fn sanitize_avatar_url_rejects_non_https_and_oversized() {
        assert_eq!(sanitize_avatar_url(None), None);
        assert_eq!(sanitize_avatar_url(Some("")), None);
        assert_eq!(sanitize_avatar_url(Some("http://example.com/a.png")), None);
        assert_eq!(sanitize_avatar_url(Some("javascript:alert(1)")), None);
        assert_eq!(sanitize_avatar_url(Some("data:image/png;base64,AAAA")), None);
        let oversized = format!("https://{}", "a".repeat(512));
        assert_eq!(sanitize_avatar_url(Some(&oversized)), None);
    }
}
