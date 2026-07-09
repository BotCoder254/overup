use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, EndpointNotSet, EndpointSet, RedirectUrl, TokenUrl};
use sqlx::PgPool;

use crate::config::Config;

/// GitHub OAuth client with the authorization and token endpoints configured.
pub type OAuthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

const GITHUB_AUTH_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub oauth: Arc<OAuthClient>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> anyhow::Result<Self> {
        let oauth = BasicClient::new(ClientId::new(config.github_client_id.clone()))
            .set_client_secret(ClientSecret::new(config.github_client_secret.clone()))
            .set_auth_uri(AuthUrl::new(GITHUB_AUTH_URL.to_string())?)
            .set_token_uri(TokenUrl::new(GITHUB_TOKEN_URL.to_string())?)
            .set_redirect_uri(RedirectUrl::new(config.oauth_redirect_url.clone())?);

        // Redirects are disabled so a compromised upstream can never bounce a
        // token exchange somewhere else (also required by the oauth2 crate).
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .user_agent("overup/0.1")
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            pool,
            config: Arc::new(config),
            oauth: Arc::new(oauth),
            http,
        })
    }
}
