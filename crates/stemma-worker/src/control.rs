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
        offers: Vec::new(),
        restore_offers: false,
    };
    if let Err(error) = config.save_to(&path) {
        let _ = hub.revoke_machine(config.id).await;
        return Err(error);
    }
    Ok(config)
}

/// Use the full worker token; ids can be reused by a fresh hub.
pub async fn identity(config: &MachineConfig) -> anyhow::Result<Machine> {
    let hub = Hub::new(&config.hub, Some(config.token.clone()))?;
    match hub.machine_identity().await {
        Ok(machine) => Ok(machine),
        Err(error) if error.status().is_some_and(|s| s.as_u16() == 401) => {
            Err(machine::Stopped::NotKnown.into())
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn this(hub: &Hub) -> anyhow::Result<Machine> {
    let mut config = load()?;
    check_hub(&config, hub)?;
    let machine = identity(&config).await?;
    if config.restore_offers {
        restore_offers(hub, &mut config, &machine::path()?).await?;
        return identity(&config).await;
    }
    if config.offers != machine.workshop_agents {
        config.offers = machine.workshop_agents.clone();
        config.save_to(&machine::path()?)?;
    }
    Ok(machine)
}

/// Replace only a registration the hub has explicitly rejected, preserving settings.
pub async fn register_again(hub: &Hub) -> anyhow::Result<MachineConfig> {
    register_again_at(hub, &machine::path()?).await
}

pub async fn register_again_at(hub: &Hub, path: &std::path::Path) -> anyhow::Result<MachineConfig> {
    let mut config = MachineConfig::load_from(path)?.context("Register this machine first")?;
    check_hub(&config, hub)?;
    match identity(&config).await {
        Err(error)
            if error.downcast_ref::<machine::Stopped>() == Some(&machine::Stopped::NotKnown) => {}
        Err(error) => return Err(error),
        Ok(_) if config.restore_offers => {
            restore_offers(hub, &mut config, path).await?;
            return Ok(config);
        }
        Ok(_) => anyhow::bail!("The hub still knows this machine"),
    }
    let created = hub.create_machine(&config.name, false).await?;
    config.token = created.secret;
    config.id = created.machine.id;
    config.restore_offers = true;
    // Save the new token before further requests, so a network failure is recoverable.
    if let Err(error) = config.save_to(path) {
        let _ = hub.revoke_machine(config.id).await;
        return Err(error);
    }
    restore_offers(hub, &mut config, path).await?;
    Ok(config)
}

async fn restore_offers(
    hub: &Hub,
    config: &mut MachineConfig,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    let mut offers = config.offers.clone();
    for offer in &mut offers {
        offer.id = None;
        offer.today = None;
    }
    config.offers = hub.set_workshop_agents(config.id, &offers).await?;
    config.restore_offers = false;
    config.save_to(path)
}

pub async fn offers(
    hub: &Hub,
    machine: &Machine,
    agents: &[stemma_core::api::WorkshopAgent],
) -> anyhow::Result<Vec<stemma_core::api::WorkshopAgent>> {
    let saved = hub.set_workshop_agents(machine.id, agents).await?;
    if let Some(mut config) = MachineConfig::load_from(&machine::path()?)?
        && config.hub == hub.base()
        && identity(&config).await.is_ok_and(|m| m.id == machine.id)
    {
        config.offers = saved.clone();
        config.save_to(&machine::path()?)?;
    }
    Ok(saved)
}

/// Success is the hub observing the authenticated worker's live connection.
pub async fn wait_online(config: &MachineConfig) -> anyhow::Result<()> {
    let wait = async {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(2), identity(config)).await {
                Ok(Ok(machine)) if machine.online => return Ok(()),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(error),
                Err(_) => anyhow::bail!("The hub is unreachable."),
            }
            if let Some(reason) = machine::stopped(&machine::path()?, config)? {
                return Err(reason.into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    };
    match tokio::time::timeout(std::time::Duration::from_secs(10), wait).await {
        Ok(result) => result,
        Err(_) => {
            let status = crate::service::status().await?;
            anyhow::bail!(if status.running {
                "The service is running, but this machine has not connected to the hub. Check its logs."
            } else {
                "The machine service is not running."
            })
        }
    }
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
