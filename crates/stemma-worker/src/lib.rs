//! Running agents on people's machines, and how Stemma launches each agent
//! ([`launch`]).
//!
//! A machine ([`machine`]) connects outbound to a hub with its token, says
//! what it can run, and runs the jobs the hub hands it: its owner's own
//! agents' answers, with the runners it found installed (`claude-code`,
//! `codex`, `opencode`), with its owner's own sign-in to each; and if its
//! owner made it work for a group's workshop, and it has Lean, the
//! workshop's tasks ([`workshop`]), with the agents its owner offers it,
//! and the builds of the group's library ([`library`]). Credentials never
//! leave the machine they are on.

pub mod config;
pub mod connection;
pub mod detect;
pub mod job;
pub mod launch;
pub mod library;
pub mod machine;
pub mod runners;
pub mod session;
pub mod workshop;

use anyhow::Context as _;

pub use config::WorkerConfig;
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
