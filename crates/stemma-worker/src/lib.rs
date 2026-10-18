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
pub mod image;
pub mod isolation;
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
            isolator: None,
        })
    }

    /// The context of a machine that starts: with how it isolates the
    /// workshop's jobs, as its configuration says and what it finds.
    pub async fn start(
        config: WorkerConfig,
        stemma: std::path::PathBuf,
    ) -> anyhow::Result<Context> {
        let mut context = Context::new(config, stemma)?;
        let search = std::env::var_os("PATH");
        match isolation::resolve(&context.config.isolation, &context.hub, search.as_deref()).await {
            Ok(isolator) => {
                match &isolator {
                    isolation::Isolator::Host => tracing::warn!(
                        "the workshop's jobs run on this machine itself, UNISOLATED, as its \
                         owner chose: agents asked by others run commands as this user. Run it \
                         as a dedicated user, with nothing else of yours"
                    ),
                    isolation::Isolator::Containers(containers) => {
                        tracing::info!(
                            "the workshop's jobs run in containers, with {} and {}",
                            containers.engine.program().display(),
                            containers.image
                        );
                        warn_of_sign_ins(&context.config, containers);
                    }
                }
                context.isolator = Some(isolator);
            }
            Err(reason) => {
                tracing::info!("this machine runs none of the workshop's jobs: {reason}")
            }
        }
        Ok(context)
    }
}

/// Says which of the machine's runners would not sign in inside the
/// workshop's containers: those with no sign-in configured, and the
/// variables named that the machine does not have.
fn warn_of_sign_ins(config: &WorkerConfig, containers: &isolation::Containers) {
    for runner in config.runner_names() {
        let name = stemma_core::api::runners::name(&runner);
        match containers.agents.get(&runner) {
            None => tracing::warn!(
                "{name} has no sign-in for the workshop's containers: configure one in \
                 [isolation.agents.{runner}], or it cannot work for the workshop"
            ),
            Some(sign_in) => {
                for variable in &sign_in.env {
                    if std::env::var_os(variable).is_none() {
                        tracing::warn!(
                            "{name} signs in with {variable} in the workshop's containers, \
                             which this machine's environment does not have"
                        );
                    }
                }
            }
        }
    }
}
