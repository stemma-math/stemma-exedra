//! The Stemma hub: accounts, groups, the forum and the agents in it, and
//! the libraries and their workshop, behind a JSON API, with the web app
//! and the workers' WebSocket. The groups' repositories are kept by a
//! forge beside it, which people sign in to through the hub, and whose git
//! the hub passes on to workers and personal tokens.

pub mod agents;
pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod forge;
pub mod git;
pub mod jobs;
pub mod library;
pub mod oidc;
pub mod secret;
pub mod state;
pub mod web;
pub mod workshop;

use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use axum::routing::post;
use sqlx::SqlitePool;
use stemma_core::api::NewAccount;
use tower_http::trace::TraceLayer;

pub use config::Config;
pub use state::{AppState, Hub};

/// The hub's HTTP application.
pub fn app(state: AppState) -> Router {
    let hook = Router::new()
        .route(forge::HOOK_PATH, post(forge::hook::receive))
        .with_state(state.clone());
    api::router(state.clone())
        .merge(git::router(state.clone()))
        .merge(oidc::router(state))
        .merge(hook)
        .fallback(web::serve)
        .layer(TraceLayer::new_for_http())
}

/// Opens the hub's data directory and database.
pub async fn open(config: Config) -> anyhow::Result<AppState> {
    tokio::fs::create_dir_all(&config.data_dir)
        .await
        .with_context(|| format!("creating {}", config.data_dir.display()))?;
    let db = db::open(&config.database_path()).await?;
    Ok(Arc::new(Hub::new(db, config)?))
}

/// Runs the hub until it is interrupted.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let state = open(config).await?;
    remove_unclaimed_uploads(&state).await?;
    oidc::forget_expired(&state.db).await?;
    jobs::recover(&state).await?;
    {
        let state = state.clone();
        tokio::spawn(async move { forge::reconcile(&state).await });
    }

    if user_count(&state.db).await? == 0 {
        let token = secret::token("");
        tracing::warn!(
            "The hub has no accounts yet. Create the first, an admin, at {}/setup?token={token} \
             or with `stemma hub create-admin`.",
            state.config.base_url(),
        );
        *state.setup_token.lock().expect("not poisoned") = Some(token);
    }

    let listener = tokio::net::TcpListener::bind(state.config.listen)
        .await
        .with_context(|| format!("listening on {}", state.config.listen))?;
    tracing::info!("Stemma hub listening on http://{}", state.config.listen);
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let interrupt = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("can listen for SIGTERM");
        tokio::select! {
            _ = interrupt => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = interrupt.await;
    tracing::info!("Shutting down");
}

pub async fn user_count(db: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(db)
        .await
}

/// Creates a hub admin from the command line.
pub async fn create_admin(config: Config, account: NewAccount) -> anyhow::Result<()> {
    let state = open(config).await?;
    let id = api::create_user(&state, account, true)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?;
    tracing::info!(id, "Created the admin account");
    Ok(())
}

/// Removes files uploaded more than a day ago that no message carries.
async fn remove_unclaimed_uploads(state: &AppState) -> anyhow::Result<()> {
    let keys: Vec<String> = sqlx::query_scalar(
        "DELETE FROM attachments WHERE message_id IS NULL AND created_at < ? RETURNING storage_key",
    )
    .bind(db::now() - 24 * 60 * 60 * 1000)
    .fetch_all(&state.db)
    .await?;
    for key in keys {
        let _ = tokio::fs::remove_file(state.config.attachments_dir().join(key)).await;
    }
    Ok(())
}
