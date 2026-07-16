use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::http::{HeaderName, HeaderValue, Method, Request, header};
use axum::routing::{delete, get, patch, post, put};
use axum::{Router, middleware as axum_middleware};
use tower_governor::GovernorLayer;
use tower_governor::errors::GovernorError;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::handlers::{
    activity, artifacts, auth, browser_ws, dashboard, dashboard_ws, environments,
    github_installations, github_webhooks, health, jobs, me, notification_ws, notifications,
    pipelines, repositories, runner_ws, runners, search, secrets, sessions, workflows,
    workspaces, ws_tickets,
};
use crate::middleware::{csrf, security_headers};
use crate::services::session;
use crate::state::AppState;

/// Browser-facing surfaces: JSON API calls and OAuth redirects are small.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Webhook payloads (push events especially) and editor validation content
/// legitimately exceed the browser budget.
const LARGE_BODY_BYTES: usize = 1024 * 1024;
/// Workspace logo uploads: raw image bytes through the backend (magic-byte
/// verification happens server-side). Slightly above the 2 MiB image cap so
/// the handler — not the transport layer — produces the friendly error.
const LOGO_BODY_BYTES: usize = 2 * 1024 * 1024 + 1024;
const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Rate-limit key: the client IP. Without TRUST_PROXY this is the socket
/// peer address — forwarded headers are client-controlled and deliberately
/// ignored, so limits cannot be bypassed by spoofing (behind a reverse proxy
/// this collapses every client into one bucket; set TRUST_PROXY=true there).
/// With TRUST_PROXY (deployment behind exactly one trusted reverse proxy)
/// the key is the RIGHTMOST parseable X-Forwarded-For hop — the entry the
/// trusted proxy itself appended; leftmost hops remain whatever the client
/// chose to send, so rotation/spoofing still buys nothing. Identical trust
/// model to the session display IP (services::session::client_info).
#[derive(Clone, Copy)]
struct ClientIpKeyExtractor {
    trust_proxy: bool,
}

impl KeyExtractor for ClientIpKeyExtractor {
    type Key = IpAddr;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        if self.trust_proxy
            && let Some(ip) = session::forwarded_client_ip(req.headers())
        {
            return Ok(ip);
        }
        req.extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|addr| addr.ip())
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

pub fn build_router(state: AppState) -> anyhow::Result<Router> {
    // Brute-force / abuse protection on the authentication surface. Every
    // governor below keys on ClientIpKeyExtractor: the socket peer address,
    // or — only under TRUST_PROXY — the rightmost X-Forwarded-For hop the
    // trusted proxy appended (see the extractor above for the trust model).
    let rate_key = ClientIpKeyExtractor { trust_proxy: state.config.trust_proxy };
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(2)
            .burst_size(10)
            .finish()
            .expect("valid governor configuration"),
    );

    let auth_routes = Router::new()
        .route("/github/login", get(auth::login))
        .route("/github/callback", get(auth::callback))
        // GitHub App Setup URL: post-install browser redirect. Session
        // cookie + server-side installation verification inside.
        .route("/github/app/setup", get(github_installations::setup))
        .route("/logout", post(auth::logout))
        .layer(GovernorLayer::new(governor_config))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES));

    // General abuse protection for the API surface, plus much stricter
    // budgets on expensive operations: workspace provisioning and manual
    // repository syncs (each fans out into GitHub API calls). The shared
    // budget is sized for an SPA: one page navigation legitimately fans out
    // 10-15 authenticated reads on top of background polling and WS-ticket
    // mints, all from one client IP.
    let api_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(20)
            .burst_size(60)
            .finish()
            .expect("valid governor configuration"),
    );
    let workspace_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    let sync_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    // Pipeline dispatch/rerun fan out into planning, token minting, and
    // runner traffic — same strict budget as manual syncs.
    let dispatch_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    // Hosted-runner provisioning pulls images and starts containers — the
    // strictest budget of all.
    let hosted_runner_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    // Secret value replacement is a security-sensitive write path; keep it
    // on the strict budget (creation shares the general api_governor like
    // other cheap single-row writes).
    let secrets_value_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    // Global Search is keystroke-driven (debounced client-side); its own
    // budget keeps a fast typist inside limits while stopping scrapers from
    // riding the general api_governor headroom.
    let search_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(5)
            .burst_size(20)
            .finish()
            .expect("valid governor configuration"),
    );
    // Account-sensitive mutations (profile edits, session revocation,
    // account deletion) get the strict write budget.
    let account_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    // Workspace settings mutations (rename, logo) share the same shape.
    let workspace_settings_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );

    // Standard API routes live under the browser body budget. Every handler
    // authenticates via CurrentUser and authorizes via workspace membership
    // permissions (services::authz).
    let api_standard = Router::new()
        .route(
            "/me",
            get(me::get_me).merge(
                patch(me::update_me)
                    .delete(me::delete_me)
                    .layer(GovernorLayer::new(account_governor.clone())),
            ),
        )
        .route("/me/sessions", get(sessions::list))
        .route(
            "/me/sessions/revoke-all",
            post(sessions::revoke_all).layer(GovernorLayer::new(account_governor.clone())),
        )
        .route(
            "/me/sessions/{session_id}",
            delete(sessions::revoke).layer(GovernorLayer::new(account_governor)),
        )
        .route(
            "/workspaces",
            post(workspaces::create_workspace).layer(GovernorLayer::new(workspace_governor)),
        )
        .route(
            "/workspaces/availability",
            get(workspaces::check_availability),
        )
        .route(
            "/workspaces/{workspace_id}",
            patch(workspaces::update_workspace)
                .layer(GovernorLayer::new(workspace_settings_governor.clone())),
        )
        .route(
            "/workspaces/{workspace_id}/members",
            get(workspaces::list_members),
        )
        .route(
            "/workspaces/{workspace_id}/logo-url",
            get(workspaces::logo_url),
        )
        .route(
            "/workspaces/{workspace_id}/installations",
            get(github_installations::list),
        )
        .route(
            "/workspaces/{workspace_id}/installations/{installation_id}",
            delete(github_installations::unlink),
        )
        .route(
            "/workspaces/{workspace_id}/repositories",
            get(repositories::list).post(repositories::import),
        )
        .route(
            "/workspaces/{workspace_id}/repositories/available",
            get(repositories::available),
        )
        .route(
            "/workspaces/{workspace_id}/repositories/{repository_id}",
            get(repositories::detail).delete(repositories::remove),
        )
        .route(
            "/workspaces/{workspace_id}/repositories/{repository_id}/sync",
            post(repositories::sync).layer(GovernorLayer::new(sync_governor)),
        )
        .route(
            "/workspaces/{workspace_id}/workflows",
            get(workflows::list),
        )
        .route(
            "/workspaces/{workspace_id}/workflows/{workflow_id}",
            get(workflows::detail),
        )
        .route(
            "/workspaces/{workspace_id}/workflows/{workflow_id}/dispatch",
            post(pipelines::dispatch).layer(GovernorLayer::new(dispatch_governor.clone())),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines",
            get(pipelines::list),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}",
            get(pipelines::detail),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/cancel",
            post(pipelines::cancel),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/rerun",
            post(pipelines::rerun).layer(GovernorLayer::new(dispatch_governor)),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/jobs/{job_id}",
            get(pipelines::job_detail),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/jobs/{job_id}/cancel",
            post(pipelines::job_cancel),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/jobs/{job_id}/logs",
            get(pipelines::job_logs),
        )
        .route("/workspaces/{workspace_id}/jobs", get(jobs::list))
        .route("/workspaces/{workspace_id}/jobs/summary", get(jobs::summary))
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/jobs/{job_id}/logs/raw",
            get(pipelines::job_logs_raw),
        )
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}/artifacts",
            get(pipelines::artifacts),
        )
        .route(
            "/workspaces/{workspace_id}/artifacts/{artifact_id}/download",
            get(pipelines::artifact_download),
        )
        .route("/workspaces/{workspace_id}/artifacts", get(artifacts::list))
        .route(
            "/workspaces/{workspace_id}/artifacts/summary",
            get(artifacts::summary),
        )
        .route(
            "/workspaces/{workspace_id}/artifacts/retention",
            get(artifacts::retention_get).put(artifacts::retention_put),
        )
        .route(
            "/workspaces/{workspace_id}/artifacts/{artifact_id}",
            get(artifacts::detail).delete(artifacts::remove),
        )
        .route(
            "/workspaces/{workspace_id}/secrets",
            get(secrets::list).post(secrets::create),
        )
        .route(
            "/workspaces/{workspace_id}/secrets/summary",
            get(secrets::summary),
        )
        .route(
            "/workspaces/{workspace_id}/secrets/requirements",
            get(secrets::requirements),
        )
        .route(
            "/workspaces/{workspace_id}/secrets/audit",
            get(secrets::audit),
        )
        .route(
            "/workspaces/{workspace_id}/secrets/{secret_id}",
            get(secrets::detail).patch(secrets::update).delete(secrets::remove),
        )
        .route(
            "/workspaces/{workspace_id}/secrets/{secret_id}/value",
            put(secrets::replace_value).layer(GovernorLayer::new(secrets_value_governor)),
        )
        .route(
            "/workspaces/{workspace_id}/environments",
            get(environments::list).post(environments::create),
        )
        .route(
            "/workspaces/{workspace_id}/environments/summary",
            get(environments::summary),
        )
        .route(
            "/workspaces/{workspace_id}/environments/requirements",
            get(environments::requirements),
        )
        .route(
            "/workspaces/{workspace_id}/environments/audit",
            get(environments::audit),
        )
        .route(
            "/workspaces/{workspace_id}/environments/{environment_id}",
            get(environments::detail)
                .patch(environments::update)
                .delete(environments::remove),
        )
        .route(
            "/workspaces/{workspace_id}/runners",
            get(runners::list).post(runners::create),
        )
        .route("/workspaces/{workspace_id}/runners/bootstrap", post(runners::bootstrap))
        .route(
            "/workspaces/{workspace_id}/runners/hosted",
            post(runners::create_hosted).layer(GovernorLayer::new(hosted_runner_governor)),
        )
        .route(
            "/workspaces/{workspace_id}/runners/{runner_id}",
            get(runners::detail).patch(runners::update).delete(runners::revoke),
        )
        .route(
            "/workspaces/{workspace_id}/runners/{runner_id}/regenerate-token",
            post(runners::regenerate_token),
        )
        .route("/workspaces/{workspace_id}/runners/{runner_id}/drain", post(runners::drain))
        .route("/workspaces/{workspace_id}/runners/{runner_id}/disable", post(runners::disable))
        .route("/workspaces/{workspace_id}/runners/{runner_id}/resume", post(runners::resume))
        .route(
            "/workspaces/{workspace_id}/dashboard/summary",
            get(dashboard::summary),
        )
        .route(
            "/workspaces/{workspace_id}/dashboard/activity",
            get(dashboard::activity),
        )
        // Global Search over the async-maintained search_documents index.
        .route(
            "/workspaces/{workspace_id}/search",
            get(search::query).layer(GovernorLayer::new(search_governor)),
        )
        // Workspace activity feed: the audit_logs ledger (distinct from the
        // dashboard's time-bucketed pipeline chart above).
        .route("/workspaces/{workspace_id}/activity", get(activity::list))
        .route(
            "/workspaces/{workspace_id}/activity/summary",
            get(activity::summary),
        )
        .route(
            "/workspaces/{workspace_id}/activity/export",
            get(activity::export),
        )
        // Notification Center: per-user projection of the ledger. Reads and
        // mutations are all self-scoped (user_id = caller in SQL).
        .route(
            "/workspaces/{workspace_id}/notifications",
            get(notifications::list),
        )
        .route(
            "/workspaces/{workspace_id}/notifications/unread-count",
            get(notifications::unread_count),
        )
        .route(
            "/workspaces/{workspace_id}/notifications/export",
            get(notifications::export),
        )
        .route(
            "/workspaces/{workspace_id}/notifications/read-all",
            post(notifications::read_all),
        )
        .route(
            "/workspaces/{workspace_id}/notifications/bulk",
            post(notifications::bulk),
        )
        .route(
            "/workspaces/{workspace_id}/notifications/{id}/read",
            post(notifications::mark_read),
        )
        .route(
            "/workspaces/{workspace_id}/notification-preferences",
            get(notifications::get_preferences).put(notifications::put_preferences),
        )
        // One-time WS auth tickets for deployments whose SPA proxy cannot
        // forward upgrades (see handlers/ws_tickets.rs).
        .route("/workspaces/{workspace_id}/ws-ticket", post(ws_tickets::create))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES));

    // Editor validation accepts whole workflow files — its own, larger cap.
    let api_validate = Router::new()
        .route(
            "/workspaces/{workspace_id}/workflows/validate",
            post(workflows::validate),
        )
        .layer(RequestBodyLimitLayer::new(LARGE_BODY_BYTES));

    // Workspace logo uploads carry raw image bytes, so they need their own
    // body budget — a sibling sub-router, NOT inside api_standard, whose
    // 64 KiB layer would cap them.
    let api_logo = Router::new()
        .route(
            "/workspaces/{workspace_id}/logo",
            put(workspaces::upload_logo).delete(workspaces::remove_logo),
        )
        .layer(GovernorLayer::new(workspace_settings_governor))
        .layer(RequestBodyLimitLayer::new(LOGO_BODY_BYTES));

    let api_routes = Router::new()
        .merge(api_standard)
        .merge(api_validate)
        .merge(api_logo)
        .layer(GovernorLayer::new(api_governor));

    // GitHub webhooks: server-to-server, authenticated by HMAC signature —
    // deliberately outside the CSRF layer and under the large body budget.
    let webhook_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(10)
            .burst_size(20)
            .finish()
            .expect("valid governor configuration"),
    );
    let webhook_routes = Router::new()
        .route("/github", post(github_webhooks::receive))
        .layer(GovernorLayer::new(webhook_governor))
        .layer(RequestBodyLimitLayer::new(LARGE_BODY_BYTES));

    // WebSocket surfaces live OUTSIDE the CSRF layer: native WebSockets
    // cannot send the X-Requested-With header. Each endpoint authenticates
    // itself before upgrading — runners with a bearer token, browsers with
    // a strict Origin check + session cookie + workspace RBAC.
    let runner_ws_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );
    let runner_ws_routes = Router::new()
        .route("/ws", get(runner_ws::connect))
        .layer(GovernorLayer::new(runner_ws_governor));

    let browser_ws_governor = Arc::new(
        GovernorConfigBuilder::default()
            .key_extractor(rate_key)
            .per_second(2)
            .burst_size(10)
            .finish()
            .expect("valid governor configuration"),
    );
    let browser_ws_routes = Router::new()
        .route(
            "/workspaces/{workspace_id}/pipelines/{pipeline_id}",
            get(browser_ws::connect),
        )
        .route(
            "/workspaces/{workspace_id}/dashboard",
            get(dashboard_ws::connect),
        )
        .route(
            "/workspaces/{workspace_id}/notifications",
            get(notification_ws::connect),
        )
        .layer(GovernorLayer::new(browser_ws_governor));

    let cors = CorsLayer::new()
        .allow_origin(
            state
                .config
                .frontend_url
                .parse::<HeaderValue>()
                .map_err(|_| AppError::Validation("FRONTEND_URL is not a valid origin".into()))
                .map_err(anyhow::Error::new)?,
        )
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("x-requested-with"),
        ]);

    // The CSRF XHR-header check guards the browser-facing surfaces only;
    // webhooks authenticate with HMAC instead. Body limits are per-scope
    // (see the sub-routers above) because an outer global limit would cap
    // webhook payloads at the browser budget.
    let browser_routes = Router::new()
        .nest("/auth", auth_routes)
        .nest("/api", api_routes)
        .layer(axum_middleware::from_fn(csrf::require_xhr_header));

    // Layers run top-down for requests: request-id -> tracing -> CORS ->
    // security headers -> (per-scope: CSRF, body limits, governors) -> route.
    let router = Router::new()
        .route("/healthz", get(health::healthz))
        .merge(browser_routes)
        .nest("/webhooks", webhook_routes)
        .nest("/runner", runner_ws_routes)
        .nest("/ws", browser_ws_routes)
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            security_headers::security_headers,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(X_REQUEST_ID))
        .layer(SetRequestIdLayer::new(X_REQUEST_ID, MakeRequestUuid))
        .with_state(state);

    Ok(router)
}
