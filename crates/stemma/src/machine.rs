//! `stemma machine`: setting up this machine, where the person's own
//! agents run, and the person's other machines; and making one work for a
//! group's workshop, with the agents it offers it.

use anyhow::Context as _;
use clap::{Args, Subcommand};
use stemma_client::config::ClientConfig;
use stemma_core::api::{Machine, WorkshopAgent, runners};
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
    /// Make a machine work for a group's workshop, and choose the agents
    /// it offers it.
    #[command(subcommand)]
    Workshop(WorkshopCommand),
}

#[derive(Subcommand)]
enum WorkshopCommand {
    /// Make one of your machines work for the workshop of a group you
    /// administer: its tasks, with the agents you offer it, and the
    /// library's builds, with Lean.
    Add {
        /// The group.
        group: String,
        /// The machine; this one if absent.
        #[arg(long)]
        machine: Option<String>,
    },
    /// Take a machine from a group's workshop.
    Remove {
        group: String,
        #[arg(long)]
        machine: Option<String>,
    },
    /// The groups a machine works for, and its agents for the workshop,
    /// in order, with what each did today.
    Show {
        #[arg(long)]
        machine: Option<String>,
    },
    /// Offer the workshop an agent, last in order unless told otherwise.
    Offer {
        /// The runner: claude-code, codex or opencode.
        runner: String,
        /// The model, if not the runner's default.
        #[arg(long)]
        model: Option<String>,
        /// How many jobs it may run in a day (UTC).
        #[arg(long)]
        jobs: Option<u32>,
        /// How many minutes it may work in a day (UTC).
        #[arg(long)]
        minutes: Option<u32>,
        /// Where it goes in the order, from 1; last if absent.
        #[arg(long)]
        position: Option<usize>,
        #[arg(long)]
        machine: Option<String>,
        /// More arguments for the runner, after --.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Stop offering the workshop an agent, by its place in the order.
    Unoffer {
        /// Its place in the order, from 1, as `show` lists it.
        position: usize,
        #[arg(long)]
        machine: Option<String>,
    },
}

pub async fn machine(args: MachineArgs) -> anyhow::Result<()> {
    match args.command {
        MachineCommand::Add { name, preferred } => add(name, preferred).await,
        MachineCommand::Run => machine::run(&machine::path()?).await,
        MachineCommand::List => list().await,
        MachineCommand::Prefer { name } => prefer(name).await,
        MachineCommand::Remove { name } => remove(name).await,
        MachineCommand::Workshop(command) => workshop(command).await,
    }
}

async fn workshop(command: WorkshopCommand) -> anyhow::Result<()> {
    let hub = ClientConfig::load()?.with_env().hub()?;
    match command {
        WorkshopCommand::Add { group, machine } => {
            let found = find(&hub, machine.as_deref()).await?;
            hub.add_workshop_machine(&group, found.id).await?;
            println!(
                "“{}” works for the workshop of {group}. Offer it agents with \
                 `stemma machine workshop offer`.",
                found.name
            );
            if !found.lean && found.last_seen_at.is_some() {
                println!("It has no Lean: install elan for it to run the workshop's tasks.");
            }
        }
        WorkshopCommand::Remove { group, machine } => {
            let found = find(&hub, machine.as_deref()).await?;
            hub.remove_workshop_machine(&group, found.id).await?;
            println!(
                "“{}” no longer works for the workshop of {group}.",
                found.name
            );
        }
        WorkshopCommand::Show { machine } => {
            let found = find(&hub, machine.as_deref()).await?;
            show(&found);
        }
        WorkshopCommand::Offer {
            runner,
            model,
            jobs,
            minutes,
            position,
            machine,
            args,
        } => {
            let found = find(&hub, machine.as_deref()).await?;
            let mut agents = found.workshop_agents.clone();
            let agent = WorkshopAgent {
                id: None,
                runner,
                model,
                args,
                daily_jobs: jobs,
                daily_minutes: minutes,
                today: None,
            };
            let at = position
                .map(|p| p.saturating_sub(1).min(agents.len()))
                .unwrap_or(agents.len());
            agents.insert(at, agent);
            let agents = hub.set_workshop_agents(found.id, &agents).await?;
            show(&Machine {
                workshop_agents: agents,
                ..found
            });
        }
        WorkshopCommand::Unoffer { position, machine } => {
            let found = find(&hub, machine.as_deref()).await?;
            let mut agents = found.workshop_agents.clone();
            anyhow::ensure!(
                (1..=agents.len()).contains(&position),
                "“{}” offers the workshop {} agents",
                found.name,
                agents.len()
            );
            agents.remove(position - 1);
            let agents = hub.set_workshop_agents(found.id, &agents).await?;
            show(&Machine {
                workshop_agents: agents,
                ..found
            });
        }
    }
    Ok(())
}

/// What a machine does for the workshop.
fn show(machine: &Machine) {
    if machine.workshop.is_empty() {
        println!("“{}” works for no workshop.", machine.name);
    } else {
        println!(
            "“{}” works for the workshop of {}.",
            machine.name,
            machine.workshop.join(", ")
        );
    }
    if machine.workshop_agents.is_empty() {
        println!("It offers the workshop no agents.");
        return;
    }
    println!("Its agents for the workshop, in order:");
    for (i, agent) in machine.workshop_agents.iter().enumerate() {
        let mut line = format!("{}. {}", i + 1, runners::name(&agent.runner));
        if let Some(model) = &agent.model {
            line.push_str(&format!(" ({model})"));
        }
        if !agent.args.is_empty() {
            line.push_str(&format!(" {}", agent.args.join(" ")));
        }
        let today = agent.today.clone().unwrap_or_default();
        let jobs = match agent.daily_jobs {
            Some(n) => format!("{}/{n} jobs", today.jobs),
            None => format!("{} jobs", today.jobs),
        };
        let minutes = match agent.daily_minutes {
            Some(n) => format!("{}/{n} minutes", today.minutes),
            None => format!("{} minutes", today.minutes),
        };
        println!("{line} · today: {jobs}, {minutes}");
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
        lean: None,
        workshop: None,
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
