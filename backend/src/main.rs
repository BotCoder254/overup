mod config;
mod db;
mod error;
mod handlers;
mod middleware;
mod models;
mod routes;
mod services;
mod state;

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env in development; real deployments use actual env vars.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env()?;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("failed to run database migrations")?;

    let state = AppState::new(pool.clone(), config.clone())?;
    let router = routes::build_router(state)?;

    // Periodic janitor: expired sessions and abandoned login transactions.
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            if let Err(error) = db::sessions::delete_expired(&pool).await {
                tracing::warn!(error = ?error, "failed to purge expired sessions");
            }
            if let Err(error) = db::oauth_states::delete_expired(&pool).await {
                tracing::warn!(error = ?error, "failed to purge expired oauth states");
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;

    tracing::info!(addr = %config.bind_addr, "overup control plane listening");

    axum::serve(
        listener,
        // Connect info is required by the per-IP rate limiter.
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("server error")?;

    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install ctrl-c handler");
    tracing::info!("shutdown signal received");
}
