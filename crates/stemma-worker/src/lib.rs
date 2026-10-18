//! Running agents: a group's workers and people's machines, and how
//! Stemma launches each agent ([`launch`]).
//!
//! A worker or a machine connects outbound to a hub with its token, says
//! what it can run, and runs the jobs the hub hands it: agents' answers,
//! with the runners it offers (`claude-code`, `codex`, `opencode`); if it
//! is a group's and has Lean, builds of the group's library; and with Lean
//! and a runner that edits files, the group's workshop's tasks.
//!
//! A group's worker runs the group's own agents and its workshop, with the
//! sign-ins and keys of its configuration. A person's machine
//! ([`machine`]) runs only that person's own agents, with their own
//! sign-in to each runner. Credentials never leave the machine they are
//! on.

pub mod config;
pub mod connection;
pub mod detect;
pub mod job;
pub mod launch;
pub mod library;
pub mod machine;
pub mod runners;
pub mod workshop;

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
            checkouts: Default::default(),
        })
    }
}
