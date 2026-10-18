//! `stemma machine`: setting up this machine, where the person's own
//! agents run, and the person's other machines.

use anyhow::Context as _;
use clap::{Args, Subcommand};
use stemma_client::config::ClientConfig;
use stemma_core::api::Machine;
use stemma_worker::machine::{self, MachineConfig};

use crate::client::hostname;

#[derive(Args)]
pub struct MachineArgs {
    #[command(subcommand)]
    command: MachineCommand,
}

#[derive(Subcommand)]
enum MachineCommand {
    /// Register this machine with the hub you signed in to, and keep its
    /// token here. It runs the agents it finds installed: claude, codex,
    /// opencode.
    Add {
        /// What to call it; this machine's name if absent.
        name: Option<String>,
        /// Send your jobs here first, when it has the runner.
        #[arg(long)]
        preferred: bool,
    },
    /// Serve this machine: run your agents' jobs here until stopped.
    Run,
    /// List your machines, online or not, and what they run.
    List,
    /// Send your jobs to a machine first, when it has the runner.
    Prefer {
        /// The machine; this one if absent.
        name: Option<String>,
    },
    /// Revoke a machine; this one, which forgets its token, if none is
    /// named.
    Remove {
        /// The machine; this one if absent.
        name: Option<String>,
    },
}

pub async fn machine(args: MachineArgs) -> anyhow::Result<()> {
    match args.command {
        MachineCommand::Add { name, preferred } => add(name, preferred).await,
        MachineCommand::Run => machine::run(&machine::path()?).await,
        MachineCommand::List => list().await,
        MachineCommand::Prefer { name } => prefer(name).await,
        MachineCommand::Remove { name } => remove(name).await,
    }
}

async fn add(name: Option<String>, preferred: bool) -> anyhow::Result<()> {
    let path = machine::path()?;
    if let Some(existing) = MachineConfig::load_from(&path)? {
        anyhow::bail!(
            "this machine is already set up, as “{}” on {}: `stemma machine remove` first",
            existing.name,
            existing.hub
        );
    }
    let hub = ClientConfig::load()?.with_env().hub()?;
    let name = name.unwrap_or_else(hostname);
    let created = hub
        .create_machine(&name, preferred)
        .await
        .context("registering this machine")?;
    let config = MachineConfig {
        hub: hub.base().to_owned(),
        token: created.secret,
        id: created.machine.id,
        name: created.machine.name.clone(),
        concurrency: None,
        work_dir: None,
        runners: Default::default(),
    };
    config.save_to(&path)?;
    let found = machine::installed(&config.runners).await;
    println!(
        "Registered this machine as “{}” on {}.",
        config.name, config.hub
    );
    println!("Its token is in {}, readable only by you.", path.display());
    println!("Runners found: {}.", machine::names(&found));
    if found.is_empty() {
        println!(
            "Install Claude Code, Codex or OpenCode and sign in to it, for this machine to run your agents."
        );
    }
    println!("Serve it with `stemma machine run`.");
    Ok(())
}

/// The machine named `name`, or this one.
async fn find(hub: &stemma_client::Hub, name: Option<&str>) -> anyhow::Result<Machine> {
    let machines = hub.machines().await?;
    let found = match name {
        Some(name) => machines.into_iter().find(|m| m.name == name),
        None => {
            let this = MachineConfig::load_from(&machine::path()?)?
                .context("this machine is not set up: name one, or `stemma machine add`")?;
            machines.into_iter().find(|m| m.id == this.id)
        }
    };
    found.with_context(|| match name {
        Some(name) => format!("you have no machine “{name}”"),
        None => "this machine is no longer registered: `stemma machine remove`, then add it".into(),
    })
}

async fn list() -> anyhow::Result<()> {
    let hub = ClientConfig::load()?.with_env().hub()?;
    let this = MachineConfig::load_from(&machine::path()?)?.map(|m| m.id);
    let machines = hub.machines().await?;
    if machines.is_empty() {
        println!("You have no machines: `stemma machine add` sets this one up.");
    }
    for m in machines {
        let mut notes = vec![if m.online { "online" } else { "offline" }.to_owned()];
        if m.preferred {
            notes.push("preferred".into());
        }
        if Some(m.id) == this {
            notes.push("this machine".into());
        }
        let runners = if m.last_seen_at.is_some() {
            machine::names(&m.runners)
        } else {
            "not connected yet".into()
        };
        println!("{} ({}): {}", m.name, notes.join(", "), runners);
    }
    Ok(())
}

async fn prefer(name: Option<String>) -> anyhow::Result<()> {
    let hub = ClientConfig::load()?.with_env().hub()?;
    let found = find(&hub, name.as_deref()).await?;
    hub.prefer_machine(found.id, true).await?;
    println!("Your jobs go to “{}” first.", found.name);
    Ok(())
}

async fn remove(name: Option<String>) -> anyhow::Result<()> {
    let path = machine::path()?;
    let this = MachineConfig::load_from(&path)?;
    let hub = ClientConfig::load()?.with_env().hub()?;
    match find(&hub, name.as_deref()).await {
        Ok(found) => {
            hub.revoke_machine(found.id).await?;
            println!("Revoked “{}”.", found.name);
        }
        // This machine, revoked already elsewhere: it only forgets.
        Err(error) if name.is_some() || this.is_none() => return Err(error),
        Err(_) => {}
    }
    if let Some(this) = this
        && name.as_deref().is_none_or(|n| n == this.name)
    {
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        println!("This machine forgot its token.");
    }
    Ok(())
}
