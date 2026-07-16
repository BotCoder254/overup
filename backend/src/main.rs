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

    let mut state = AppState::new(pool.clone(), config.clone())?;
    // Object storage bootstrap: create missing MinIO buckets (fresh local
    // deployments start empty). Warn-only — storage stays enabled and any
    // real outage surfaces on the first write.
    if let Some(storage) = &state.storage {
        tracing::info!(primary = %storage.primary_backend(), "object storage configured");
        storage.ensure_buckets().await;
    }
    // Optional hosted-runner provisioner. Constructed whenever configured;
    // its reconnect loop owns the Docker connection, so a daemon outage (at
    // boot or later) degrades to a clean 409 on the hosted-runner endpoint
    // and recovers automatically — never a permanently disabled feature.
    state.runner_provisioner = config
        .runner_provisioner
        .clone()
        .map(services::runner_provisioner::RunnerProvisioner::new);
    if let Some(provisioner) = state.runner_provisioner.clone() {
        tokio::spawn(provisioner.run_reconnect_loop());
    }

    // Boot-time execution recovery: no runner can be connected yet, so
    // anything marked online or in progress is a leftover from the previous
    // process. Requeue first attempts, fail repeat offenders.
    db::runners::mark_all_offline(&pool)
        .await
        .context("failed to reset runner statuses")?;
    let (requeued, failed) = db::pipeline_jobs::requeue_all_orphans(&pool)
        .await
        .context("failed to recover orphaned jobs")?;
    if !requeued.is_empty() || !failed.is_empty() {
        tracing::info!(
            requeued = requeued.len(),
            failed = failed.len(),
            "recovered orphaned pipeline jobs from previous run"
        );
        for job in &failed {
            if let Err(error) = services::pipeline_run::maybe_finalize(&state, job.pipeline_id).await
            {
                tracing::warn!(error = ?error, "failed to finalize recovered pipeline");
            }
        }
    }

    // The scheduler loop: assignment, ack/timeout/stale sweeps.
    tokio::spawn(services::scheduler::run(state.clone()));

    // Hourly janitor: expired sessions/oauth states, artifact retention +
    // R2 cleanup, archived log-chunk pruning.
    tokio::spawn(services::janitor::run(state.clone()));

    // Global Search indexer: boot backfill, dirty-workspace drains, and the
    // periodic reconcile pass over the search_documents projection.
    tokio::spawn(services::search_indexer::run(state.clone()));

    // Notification projector: tails the audit ledger past its persisted
    // cursor and materializes per-user Notification Center rows.
    tokio::spawn(services::notification_projector::run(state.clone()));

    let router = routes::build_router(state)?;

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
