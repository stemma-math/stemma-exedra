//! Local machine operations shared by the CLI and desktop.

use std::path::PathBuf;

use anyhow::Context as _;
use stemma_client::Hub;
use stemma_core::api::Machine;

use crate::{
    detect, image, isolation,
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
    save(&config)
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
    save(&config)
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
    save(&config)
}

/// Apply saved settings to the service managed by either face.
pub fn save(config: &MachineConfig) -> anyhow::Result<()> {
    config.save_to(&machine::path()?)?;
    crate::service::reload()
}
