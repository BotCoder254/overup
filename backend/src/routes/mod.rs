use std::sync::Arc;

use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::routing::{get, post};
use axum::{Router, middleware as axum_middleware};
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

use crate::error::AppError;
use crate::handlers::{auth, health, me, workspaces};
use crate::middleware::{csrf, security_headers};
use crate::state::AppState;

const MAX_BODY_BYTES: usize = 64 * 1024;
const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

pub fn build_router(state: AppState) -> anyhow::Result<Router> {
    // Brute-force / abuse protection on the authentication surface,
    // keyed by peer IP.
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(2)
            .burst_size(10)
            .finish()
            .expect("valid governor configuration"),
    );

    let auth_routes = Router::new()
        .route("/github/login", get(auth::login))
        .route("/github/callback", get(auth::callback))
        .route("/logout", post(auth::logout))
        .layer(GovernorLayer::new(governor_config));

    // General abuse protection for the API surface, plus a much stricter
    // budget on workspace provisioning — it is an expensive, once-per-user
    // operation and a natural target for automated creation attempts.
    let api_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(5)
            .burst_size(30)
            .finish()
            .expect("valid governor configuration"),
    );
    let workspace_governor = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("valid governor configuration"),
    );

    let api_routes = Router::new()
        .route("/me", get(me::get_me))
        .route(
            "/workspaces",
            post(workspaces::create_workspace).layer(GovernorLayer::new(workspace_governor)),
        )
        .layer(GovernorLayer::new(api_governor));

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
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([
            header::CONTENT_TYPE,
            HeaderName::from_static("x-requested-with"),
        ]);

    // Layers run top-down for requests: request-id -> tracing -> body limit
    // -> CORS -> security headers -> CSRF -> route handler.
    let router = Router::new()
        .route("/healthz", get(health::healthz))
        .nest("/auth", auth_routes)
        .nest("/api", api_routes)
        .layer(axum_middleware::from_fn(csrf::require_xhr_header))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            security_headers::security_headers,
        ))
        .layer(cors)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(X_REQUEST_ID))
        .layer(SetRequestIdLayer::new(X_REQUEST_ID, MakeRequestUuid))
        .with_state(state);

    Ok(router)
}
