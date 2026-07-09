use anyhow::Context;
use serde::Deserialize;

const GITHUB_API: &str = "https://api.github.com";

/// The subset of the GitHub user we mirror locally. The access token itself
/// is used once here and never persisted or returned to the frontend.
#[derive(Debug, Deserialize)]
pub struct GitHubProfile {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

pub async fn fetch_profile(
    http: &reqwest::Client,
    access_token: &str,
) -> anyhow::Result<GitHubProfile> {
    let mut profile: GitHubProfile = get(http, access_token, "/user")
        .await
        .context("failed to fetch GitHub user profile")?;

    // The public profile email is often unset; fall back to the primary
    // verified address exposed by the `user:email` scope.
    if profile.email.is_none()
        && let Ok(emails) = get::<Vec<GitHubEmail>>(http, access_token, "/user/emails").await
    {
        profile.email = emails
            .into_iter()
            .find(|e| e.primary && e.verified)
            .map(|e| e.email);
    }

    Ok(profile)
}

async fn get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    access_token: &str,
    path: &str,
) -> anyhow::Result<T> {
    let response = http
        .get(format!("{GITHUB_API}{path}"))
        .bearer_auth(access_token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json::<T>().await?)
}
