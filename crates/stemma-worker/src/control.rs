//! Local machine operations shared by the CLI and desktop.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use stemma_client::Hub;
use stemma_core::api::Machine;
use tokio::task::JoinHandle;

use crate::{
    Context, connection, detect, image, isolation,
    machine::{self, MachineConfig},
};

pub fn load() -> anyhow::Result<MachineConfig> {
    MachineConfig::load_from(&machine::path()?)?.context("Register this machine first")
}

/// Refuse to send a machine's id or settings to a different hub.
pub fn check_hub(config: &MachineConfig, hub: &Hub) -> anyhow::Result<()> {
    anyhow::ensure!(
        config.hub == hub.base(),
        "This machine is registered with {}. Remove it there before changing hubs.",
        config.hub
    );
    Ok(())
}

pub async fn register(hub: &Hub, name: &str, preferred: bool) -> anyhow::Result<MachineConfig> {
    let path = machine::path()?;
    anyhow::ensure!(
        MachineConfig::load_from(&path)?.is_none(),
        "This machine is already registered"
    );
    let created = hub.create_machine(name, preferred).await?;
    let config = MachineConfig {
        hub: hub.base().into(),
        token: created.secret,
        id: created.machine.id,
        name: created.machine.name,
        personal: true,
        workshop_enabled: false,
        concurrency: None,
        work_dir: None,
        runners: Default::default(),
        lean: None,
        latex: None,
        workshop: None,
        isolation: None,
    };
    if let Err(error) = config.save_to(&path) {
        let _ = hub.revoke_machine(config.id).await;
        return Err(error);
    }
    Ok(config)
}

pub async fn this(hub: &Hub) -> anyhow::Result<Machine> {
    let config = load()?;
    check_hub(&config, hub)?;
    hub.machines()
        .await?
        .into_iter()
        .find(|m| m.id == config.id)
        .context("This machine is no longer registered")
}

pub fn configure(
    work_dir: Option<PathBuf>,
    personal: Option<bool>,
    workshop: Option<bool>,
) -> anyhow::Result<()> {
    let mut config = load()?;
    if let Some(dir) = work_dir {
        anyhow::ensure!(
            dir.is_absolute(),
            "The repository folder must be an absolute path"
        );
        std::fs::create_dir_all(&dir)?;
        config.work_dir = Some(dir);
    }
    if let Some(enabled) = personal {
        config.personal = enabled;
    }
    if let Some(enabled) = workshop {
        config.workshop_enabled = enabled;
    }
    config.save_to(&machine::path()?)
}

/// Save container settings without exposing an implicit unisolated option.
pub fn containers(runtime: isolation::Runtime, image: String) -> anyhow::Result<()> {
    anyhow::ensure!(
        runtime != isolation::Runtime::Host,
        "Use --unisolated explicitly to run on the host"
    );
    anyhow::ensure!(!image.trim().is_empty(), "Name the workshop image");
    let mut config = load()?;
    let mut settings = config.isolation.unwrap_or_default();
    settings.runtime = Some(runtime);
    settings.image = Some(image);
    settings.validate()?;
    config.isolation = Some(settings);
    config.save_to(&machine::path()?)
}

pub async fn build_image(
    settings: &isolation::IsolationConfig,
    build: &image::Build,
) -> anyhow::Result<()> {
    let mut runtime = settings.clone();
    if runtime.runtime() == isolation::Runtime::Host {
        runtime.runtime = Some(isolation::Runtime::Auto);
    }
    let engine = isolation::find_engine(&runtime, std::env::var_os("PATH").as_deref())
        .await
        .context("Docker or Podman must be running")?;
    image::build(engine.as_ref(), build).await
}

/// A worker owned by its caller. Dropping it disconnects and cancels its jobs.
#[derive(Default)]
pub struct Service {
    task: Option<JoinHandle<anyhow::Result<()>>>,
    context: Option<Arc<Context>>,
}

impl Service {
    pub async fn start(&mut self, config: MachineConfig, stemma: PathBuf) -> anyhow::Result<()> {
        if !config.personal && !config.workshop_enabled {
            self.stop().await;
            return Ok(());
        }
        let found = machine::installed(&config.runners).await;
        tracing::info!("found {}", machine::names(&found));
        let context = Context::start(config.worker_config(&found), stemma).await?;
        anyhow::ensure!(
            !config.workshop_enabled || context.isolator.is_some(),
            "The workshop has no usable isolation. Choose Docker or Podman and build the configured image."
        );
        self.stop().await;
        let context = Arc::new(context);
        self.task = Some(tokio::spawn(connection::serve(
            context.clone(),
            config.token,
        )));
        self.context = Some(context);
        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        // Jobs hold the context until cancellation drops their worktrees and
        // containers. Wait before opening the same checkouts again.
        if let Some(context) = self.context.take() {
            while Arc::strong_count(&context) > 1 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
    }

    pub async fn wait(&mut self) -> anyhow::Result<()> {
        if let Some(task) = self.task.as_mut() {
            task.await??;
        }
        Ok(())
    }

    pub fn personal(&self) -> bool {
        self.running() && self.context.as_ref().is_some_and(|c| c.config.personal)
    }

    pub fn workshop(&self) -> bool {
        self.running()
            && self
                .context
                .as_ref()
                .is_some_and(|c| c.config.workshop_enabled)
    }

    pub fn running(&self) -> bool {
        self.task.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

/// The installed CLI provides MCP to personal agents; the container has its own.
pub fn stemma() -> anyhow::Result<PathBuf> {
    detect::find_program("stemma", std::env::var_os("PATH").as_deref())
        .context("Install the stemma CLI so personal agents can use its tools")
}

/// Give only the selected sign-in material to workshop containers.
pub fn credentials(
    runner: String,
    env: Vec<String>,
    files: Vec<isolation::SignInFile>,
) -> anyhow::Result<()> {
    let mut config = load()?;
    let mut settings = config.isolation.unwrap_or_default();
    settings.agents.insert(
        runner,
        isolation::AgentSignIn {
            env,
            files,
            command: None,
        },
    );
    settings.validate()?;
    for agent in settings.agents.values() {
        for file in &agent.files {
            file.source(dirs::home_dir().as_deref())?;
        }
    }
    config.isolation = Some(settings);
    config.save_to(&machine::path()?)
}
