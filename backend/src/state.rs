use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, EndpointNotSet, EndpointSet, RedirectUrl, TokenUrl};
use sqlx::PgPool;

use crate::config::Config;
use crate::services::github_app::GitHubApp;
use crate::services::log_hub::LogHub;
use crate::services::r2::R2;
use crate::services::runner_hub::RunnerHub;
use crate::services::scheduler::Scheduler;
use crate::services::workspace_hub::WorkspaceHub;

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
    pub github_app: Arc<GitHubApp>,
    /// Live browser fan-out + log ingest (masking, caps).
    pub log_hub: Arc<LogHub>,
    /// Outbound channels to connected runners.
    pub runner_hub: Arc<RunnerHub>,
    /// Live workspace-wide fan-out for the Dashboard and Runner Management pages.
    pub workspace_hub: Arc<WorkspaceHub>,
    /// Wake handle for the scheduling loop.
    pub scheduler: Arc<Scheduler>,
    /// Artifact storage; None disables artifact grants cleanly.
    pub r2: Option<Arc<R2>>,
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

        // Validates the App private key at startup so a bad PEM cannot
        // surface later as a mid-request failure.
        let github_app = GitHubApp::new(
            config.github_app_client_id.clone(),
            &config.github_app_private_key_pem,
        )?;

        let r2 = config.r2.as_ref().map(|r2| {
            Arc::new(R2::new(
                &r2.account_id,
                &r2.access_key_id,
                &r2.secret_access_key,
                &r2.bucket,
            ))
        });

        Ok(Self {
            pool,
            config: Arc::new(config),
            oauth: Arc::new(oauth),
            http,
            github_app: Arc::new(github_app),
            log_hub: Arc::new(LogHub::default()),
            runner_hub: Arc::new(RunnerHub::default()),
            workspace_hub: Arc::new(WorkspaceHub::default()),
            scheduler: Arc::new(Scheduler::default()),
            r2,
        })
    }
}
