use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, ClientSecret, EndpointNotSet, EndpointSet, RedirectUrl, TokenUrl};
use sqlx::PgPool;

use crate::config::Config;
use crate::services::github_app::GitHubApp;
use crate::services::github_checks::GithubChecks;
use crate::services::log_hub::LogHub;
use crate::services::notification_hub::NotificationHub;
use crate::services::notification_projector::NotificationProjector;
use crate::services::object_store::Storage;
use crate::services::runner_hub::RunnerHub;
use crate::services::runner_provisioner::RunnerProvisioner;
use crate::services::scheduler::Scheduler;
use crate::services::search_indexer::SearchIndexer;
use crate::services::secrets_crypto::SecretsCrypto;
use crate::services::webhook_processor::WebhookProcessor;
use crate::services::webhook_stats::WebhookAuthStats;
use crate::services::workspace_hub::WorkspaceHub;
use crate::services::ws_ticket::WsTicketStore;

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
    /// Dirty-workspace queue for the Global Search indexer loop.
    pub search_indexer: Arc<SearchIndexer>,
    /// Per-user live fan-out for the Notification Center bell.
    pub notification_hub: Arc<NotificationHub>,
    /// Wake handle for the audit-tail notification projector loop.
    pub notification_projector: Arc<NotificationProjector>,
    /// Wake handle for the async webhook-delivery processor loop.
    pub webhook_processor: Arc<WebhookProcessor>,
    /// Deployment-global webhook signature-rejection gauge (in-memory;
    /// surfaced on repository health so a bad GITHUB_WEBHOOK_SECRET is
    /// visible in the UI instead of only in GitHub's Recent Deliveries).
    pub webhook_auth: Arc<WebhookAuthStats>,
    /// Per-installation availability cache for GitHub Checks reporting.
    pub github_checks: Arc<GithubChecks>,
    /// Object storage router (MinIO primary / R2 fallback when both are
    /// configured); None disables artifact grants cleanly.
    pub storage: Option<Arc<Storage>>,
    /// One-time tickets for cross-origin browser WebSocket auth
    /// (deployments whose proxy cannot forward upgrades, e.g. Netlify).
    pub ws_tickets: Arc<WsTicketStore>,
    /// Hosted-runner container spawner; None means the feature is
    /// unavailable (not configured, or Docker unreachable at startup).
    /// Initialized asynchronously in main after construction.
    pub runner_provisioner: Option<Arc<RunnerProvisioner>>,
    /// Envelope-encryption engine for secret values; None disables the
    /// Secrets feature cleanly (mutations denied, dispatch fails closed
    /// for repositories that already have stored secrets).
    pub secrets_crypto: Option<Arc<SecretsCrypto>>,
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

        // Validates the master key shape at startup so a bad key cannot
        // surface later as a mid-request failure (the GitHub App PEM
        // pattern).
        let secrets_crypto = config
            .secrets_master_key
            .as_ref()
            .map(|key| SecretsCrypto::new(key).map(Arc::new))
            .transpose()?;

        // MinIO (when configured) is the default/primary object store and
        // R2 the fallback; R2 alone keeps its historical primary role.
        let storage = match (config.minio.as_ref(), config.r2.as_ref()) {
            (Some(minio), r2) => Some(Arc::new(Storage::new(
                crate::services::minio::store(minio),
                r2.map(crate::services::r2::store),
            ))),
            (None, Some(r2)) => Some(Arc::new(Storage::new(
                crate::services::r2::store(r2),
                None,
            ))),
            (None, None) => None,
        };

        // Every WorkspaceHub::publish marks its workspace dirty on this
        // indexer and pokes the notification projector, so the hub is
        // constructed around both.
        let search_indexer = Arc::new(SearchIndexer::default());
        let notification_projector = Arc::new(NotificationProjector::default());

        Ok(Self {
            pool,
            config: Arc::new(config),
            oauth: Arc::new(oauth),
            http,
            github_app: Arc::new(github_app),
            log_hub: Arc::new(LogHub::default()),
            runner_hub: Arc::new(RunnerHub::default()),
            workspace_hub: Arc::new(WorkspaceHub::new(
                search_indexer.clone(),
                notification_projector.clone(),
            )),
            scheduler: Arc::new(Scheduler::default()),
            search_indexer,
            notification_hub: Arc::new(NotificationHub::default()),
            notification_projector,
            webhook_processor: Arc::new(WebhookProcessor::default()),
            webhook_auth: Arc::new(WebhookAuthStats::default()),
            github_checks: Arc::new(GithubChecks::default()),
            storage,
            ws_tickets: Arc::new(WsTicketStore::default()),
            // Requires async Docker probing; main fills it in right after.
            runner_provisioner: None,
            secrets_crypto,
        })
    }
}
