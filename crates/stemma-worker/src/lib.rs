//! The Stemma worker: it connects outbound to a hub with a worker token,
//! says what it can run, and runs the jobs the hub hands it: agents'
//! answers, with the runners it offers (`claude-code`, `codex`, `api`).
//!
//! A group's worker runs the group's agents, with keys in its
//! configuration; a member's worker runs only that member's jobs, with
//! their own logins. Credentials never leave the worker's machine.

pub mod config;
pub mod connection;
pub mod job;
pub mod runners;

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;

pub use config::WorkerConfig;

/// Runs a worker with the configuration at `path` until it is stopped.
pub async fn run(path: &Path) -> anyhow::Result<()> {
    let config = WorkerConfig::load(path)?;
    let token = config.token()?;
    let stemma = match &config.stemma {
        Some(path) => path.clone(),
        None => std::env::current_exe().context("finding the stemma binary")?,
    };
    let context = Context::new(config, stemma)?;
    tokio::select! {
        served = connection::serve(Arc::new(context), token) => served,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Stopping");
            Ok(())
        }
    }
}

pub use job::Context;

impl Context {
    pub fn new(config: WorkerConfig, stemma: std::path::PathBuf) -> anyhow::Result<Context> {
        let hub = stemma_client::hub::normalize_url(&config.hub)?;
        let work_dir = config.work_dir();
        std::fs::create_dir_all(&work_dir)
            .with_context(|| format!("creating {}", work_dir.display()))?;
        Ok(Context {
            config,
            hub,
            stemma,
            work_dir,
        })
    }
}
