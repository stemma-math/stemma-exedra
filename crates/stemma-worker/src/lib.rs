//! Running personal replies and interactive sessions on a computer.
pub mod config;
pub mod connection;
pub mod detect;
pub mod doctor;
pub mod image;
pub mod isolation;
pub mod job;
pub mod launch;
pub mod machine;
pub mod runners;
pub mod service;
pub mod session;
pub mod sign_in;
pub mod workspace;

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
            isolator: None,
        })
    }
    pub async fn start(
        config: WorkerConfig,
        stemma: std::path::PathBuf,
    ) -> anyhow::Result<Context> {
        let mut context = Context::new(config, stemma)?;
        let search = std::env::var_os("PATH");
        context.isolator = Some(
            isolation::resolve(&context.config.isolation, &context.hub, search.as_deref())
                .await
                .map_err(anyhow::Error::msg)?,
        );
        Ok(context)
    }
}
