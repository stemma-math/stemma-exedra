//! `stemma machine`: setting up this machine, where the person's own
//! agents run, and the person's other machines; and making one work for a
//! group's workshop, with the agents it offers it.

use anyhow::Context as _;
use clap::{Args, Subcommand};
use stemma_client::config::ClientConfig;
use stemma_core::api::{Isolation, Machine, WorkshopAgent, runners};
use stemma_worker::isolation::{self, Isolator, Runtime};
use stemma_worker::machine::{self, MachineConfig};
use stemma_worker::{image, service};

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
    /// Replace a registration the hub no longer knows, keeping local settings.
    RegisterAgain,
    /// Serve this machine: run your agents' jobs here until stopped.
    Run,
    /// Manage this machine's per-user system service.
    #[command(subcommand)]
    Service(ServiceCommand),
    /// Detected agents, their sign-in state, Lean and LaTeX on this machine.
    Status,
    /// Set the repository folder or enable the personal and workshop services.
    Configure {
        #[arg(long)]
        work_dir: Option<std::path::PathBuf>,
        #[arg(long, action = clap::ArgAction::Set)]
        personal: Option<bool>,
        #[arg(long, action = clap::ArgAction::Set)]
        workshop: Option<bool>,
    },
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
enum ServiceCommand {
    /// Install for login and start now.
    Install,
    /// Stop and remove the service.
    Uninstall,
    Start,
    Stop,
    /// Installation, process, modes, hub connection and logs.
    Status,
}

async fn service(command: ServiceCommand) -> anyhow::Result<()> {
    match command {
        ServiceCommand::Install => service::install(&std::env::current_exe()?),
        ServiceCommand::Uninstall => service::uninstall(),
        ServiceCommand::Start => service::start(),
        ServiceCommand::Stop => service::stop(),
        ServiceCommand::Status => {
            let status = service::status().await?;
            println!(
                "Installed: {}\nRunning: {}\nPersonal: {}\nWorkshop: {}\nConnected to hub: {}\nLogs: {}",
                status.installed,
                status.running,
                status.personal,
                status.workshop,
                status
                    .connected
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "unknown".into()),
                status.logs
            );
            if let Some(stopped) = status.stopped {
                println!("{stopped}");
                if stopped == machine::Stopped::NotKnown {
                    println!("Register again: `stemma machine register-again`");
                }
            }
            if !status.stale_agents.is_empty() {
                let names = status
                    .stale_agents
                    .iter()
                    .map(|agent| stemma_core::api::runners::name(agent))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "Warning: {names} found on the current PATH, but not with the service's stored PATH. Reinstall the service with `stemma machine service install` to fix it."
                );
            }
            Ok(())
        }
    }
}

#[derive(Subcommand)]
enum WorkshopCommand {
    /// Select sign-in material passed only to workshop containers, never to the hub.
    Credentials {
        runner: String,
        /// Environment variable names (not values).
        #[arg(long)]
        env: Vec<String>,
        /// Read-only files: ~/path, or /host/path=/container/path.
        #[arg(long)]
        file: Vec<String>,
    },
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
    /// How this machine isolates the workshop's jobs: each in a container
    /// of its own, with Docker or Podman (the default), or, where
    /// containers are not available, on the machine itself, unisolated.
    /// Without arguments, says how it would now.
    Isolation {
        /// The workshop image to use.
        #[arg(long)]
        image: Option<String>,
        /// The runtime: auto (Docker, or else Podman), docker or podman.
        #[arg(conflicts_with = "unisolated")]
        runtime: Option<String>,
        /// Run the workshop's jobs on this machine itself, as its user,
        /// with no container: agents that others ask for then run
        /// commands here. Only on a machine, or as a user, kept for it.
        #[arg(long)]
        unisolated: bool,
    },
    /// Build the image the workshop's containers run, with Lean, the
    /// agents' programs and this version of stemma, with Docker or Podman.
    Image {
        /// Its tag; the one this machine's configuration names, or
        /// stemma-workshop:<version>.
        #[arg(long)]
        tag: Option<String>,
        /// The Lean toolchain it has; a new library's if absent.
        #[arg(long)]
        toolchain: Option<String>,
        /// Where its stemma comes from: release (the release's Linux
        /// binary, for x86_64) or git (built from the release's tag); the
        /// release on x86_64, else git.
        #[arg(long)]
        source: Option<String>,
    },
}

pub async fn machine(args: MachineArgs) -> anyhow::Result<()> {
    match args.command {
        MachineCommand::Add { name, preferred } => add(name, preferred).await,
        MachineCommand::RegisterAgain => {
            let hub = ClientConfig::load()?.with_env().hub()?;
            let config = stemma_worker::control::register_again(&hub).await?;
            service::install(&std::env::current_exe()?)?;
            stemma_worker::control::wait_online(&config).await?;
            println!("Registered again. This machine is online.");
            Ok(())
        }
        MachineCommand::Service(command) => service(command).await,
        MachineCommand::Run => machine::run(&machine::path()?).await,
        MachineCommand::Status => {
            let config = MachineConfig::load_from(&machine::path()?)?;
            let runners = config.map(|c| c.runners).unwrap_or_default();
            for agent in stemma_worker::detect::agents(&runners).await {
                println!("{}: {}", agent.name, agent.sign_in);
            }
            for program in ["elan", "latexmk"] {
                println!(
                    "{program}: {}",
                    stemma_worker::detect::version(program)
                        .await
                        .unwrap_or_else(|| "Not installed".into())
                );
            }
            Ok(())
        }
        MachineCommand::Configure {
            work_dir,
            personal,
            workshop,
        } => stemma_worker::control::configure(work_dir, personal, workshop),
        MachineCommand::List => list().await,
        MachineCommand::Prefer { name } => prefer(name).await,
        MachineCommand::Remove { name } => remove(name).await,
        MachineCommand::Workshop(command) => workshop(command).await,
    }
}

async fn workshop(command: WorkshopCommand) -> anyhow::Result<()> {
    // Local settings and image builds ask the hub nothing.
    match command {
        WorkshopCommand::Credentials { runner, env, file } => {
            let files = file
                .into_iter()
                .map(|file| match file.split_once('=') {
                    Some((from, to)) => isolation::SignInFile {
                        from: from.into(),
                        to: Some(to.into()),
                    },
                    None => isolation::SignInFile {
                        from: file,
                        to: None,
                    },
                })
                .collect();
            return stemma_worker::control::credentials(runner, env, files);
        }
        WorkshopCommand::Isolation {
            runtime,
            image,
            unisolated,
        } => return isolation(runtime, image, unisolated).await,
        WorkshopCommand::Image {
            tag,
            toolchain,
            source,
        } => return build_image(tag, toolchain, source).await,
        _ => {}
    }
    let hub = ClientConfig::load()?.with_env().hub()?;
    match command {
        WorkshopCommand::Credentials { .. }
        | WorkshopCommand::Isolation { .. }
        | WorkshopCommand::Image { .. } => {}
        WorkshopCommand::Add { group, machine } => {
            let mut found = find(&hub, machine.as_deref()).await?;
            if machine.is_none() {
                stemma_worker::control::lend(&hub, &group, &std::env::current_exe()?).await?;
                found = find(&hub, None).await?;
            } else {
                anyhow::ensure!(
                    found.online,
                    "This machine is offline. Start its service before lending it."
                );
                hub.add_workshop_machine(&group, found.id).await?;
            }
            println!(
                "“{}” works for the workshop of {group}. Offer it agents with \
                 `stemma machine workshop offer`.",
                found.name
            );
            match found.isolation {
                Some(Isolation::Host) => println!(
                    "WARNING: it runs the workshop's jobs on itself, unisolated, as its owner \
                     chose: the group sees so."
                ),
                Some(isolation) => {
                    println!("It runs the workshop's jobs {}.", isolation.describe())
                }
                None => {}
            }
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
            let agents = stemma_worker::control::offers(&hub, &found, &agents).await?;
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
            let agents = stemma_worker::control::offers(&hub, &found, &agents).await?;
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
    match machine.isolation {
        Some(isolation) => println!("It runs the workshop's jobs {}.", isolation.describe()),
        None if machine.last_seen_at.is_some() => println!(
            "It cannot isolate the workshop's jobs, and runs none: see `stemma machine workshop \
             isolation` on it."
        ),
        None => {}
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
        println!("{line} – today: {jobs}, {minutes}");
    }
}

/// The warning of a machine that runs the workshop's jobs unisolated.
const UNISOLATED: &str = "WARNING: the workshop's jobs will run on this machine itself, with no \
container, as the user that serves it. Their agents, asked by others in the group, run commands \
here: they can read and change whatever that user can. Serve the machine as a dedicated system \
user, with nothing else of yours, and only where containers are not available. The group's \
workshop panel shows the machine as unisolated.";

/// Shows, or sets, how this machine isolates the workshop's jobs.
async fn isolation(
    runtime: Option<String>,
    image: Option<String>,
    unisolated: bool,
) -> anyhow::Result<()> {
    let path = machine::path()?;
    let mut config = MachineConfig::load_from(&path)?
        .context("this machine is not set up: `stemma machine add` first")?;
    let mut settings = config.isolation.clone().unwrap_or_default();
    let chosen = match (runtime, unisolated) {
        (_, true) => Some(Runtime::Host),
        (Some(text), false) => Some(Runtime::parse(&text).with_context(|| {
            format!("there is no runtime {text}: auto, docker or podman (or --unisolated)")
        })?),
        (None, false) => None,
    };
    if let Some(image) = &image {
        settings.image = Some(image.clone());
    }
    if chosen.is_some() || image.is_some() {
        let chosen = chosen.unwrap_or(settings.runtime());
        settings.runtime = Some(chosen);
        if chosen == Runtime::Host {
            config.isolation = Some(settings.clone());
            stemma_worker::control::save(&config)?;
        } else {
            stemma_worker::control::containers(chosen, settings.image())?;
        }
        if chosen == Runtime::Host {
            eprintln!("{UNISOLATED}");
        }
        println!(
            "Saved in {}. The active system service reloads these settings.",
            path.display()
        );
    }
    let search = std::env::var_os("PATH");
    match isolation::resolve(&settings, &config.hub, search.as_deref()).await {
        Ok(Isolator::Host) => {
            if chosen.is_none() {
                eprintln!("{UNISOLATED}");
            }
            println!("This machine runs the workshop's jobs on itself, unisolated.");
        }
        Ok(Isolator::Containers(containers)) => println!(
            "This machine runs the workshop's jobs {}, from the image {}.",
            containers.engine.isolation().describe(),
            containers.image
        ),
        Err(reason) => println!(
            "This machine runs none of the workshop's jobs: {reason}. Install Docker or Podman \
             and build the image (`stemma machine workshop image`), or, only where containers \
             are not available, `stemma machine workshop isolation --unisolated`."
        ),
    }
    Ok(())
}

/// Builds the workshop's image with this machine's runtime.
async fn build_image(
    tag: Option<String>,
    toolchain: Option<String>,
    source: Option<String>,
) -> anyhow::Result<()> {
    let settings = MachineConfig::load_from(&machine::path()?)?
        .and_then(|config| config.isolation)
        .unwrap_or_default();
    let source = match source {
        Some(text) => image::Source::parse(&text)
            .with_context(|| format!("there is no source {text}: release or git"))?,
        None => image::Source::for_this_machine(),
    };
    let build = image::Build {
        tag: tag.unwrap_or_else(|| settings.image()),
        source,
        toolchain: toolchain.unwrap_or_else(|| image::TOOLCHAIN.trim().to_owned()),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    println!(
        "Building {} with {} and stemma {} ({}).",
        build.tag,
        build.toolchain,
        build.version,
        if source == image::Source::Release {
            "its release's Linux binary"
        } else {
            "built from its release's tag"
        }
    );
    stemma_worker::control::build_image(&settings, &build).await?;
    println!(
        "Built {}. Serve the machine again (`stemma machine run`) for it to use it.",
        build.tag
    );
    if settings.runtime() == Runtime::Host {
        println!(
            "This machine runs the workshop's jobs unisolated, as chosen: \
             `stemma machine workshop isolation auto` to run them in containers."
        );
    }
    Ok(())
}

async fn add(name: Option<String>, preferred: bool) -> anyhow::Result<()> {
    let path = machine::path()?;
    let hub = ClientConfig::load()?.with_env().hub()?;
    let config =
        stemma_worker::control::register(&hub, &name.unwrap_or_else(hostname), preferred).await?;
    service::install(&std::env::current_exe()?)?;
    stemma_worker::control::wait_online(&config).await?;
    let found = machine::installed(&config.runners).await;
    println!(
        "Registered this machine as “{}” on {}. It is online.",
        config.name, config.hub
    );
    println!("Its token is in {}, readable only by you.", path.display());
    println!("Runners found: {}.", machine::names(&found));
    if found.is_empty() {
        println!(
            "Install Claude Code, Codex or OpenCode and sign in to it, for this machine to run your agents."
        );
    }
    println!(
        "The login service keeps it online. Enable the workshop with `stemma machine configure --workshop true`."
    );
    Ok(())
}

/// The machine named `name`, or this one.
async fn find(hub: &stemma_client::Hub, name: Option<&str>) -> anyhow::Result<Machine> {
    if name.is_none() {
        return stemma_worker::control::this(hub).await;
    }
    hub.machines()
        .await?
        .into_iter()
        .find(|m| Some(m.name.as_str()) == name)
        .with_context(|| format!("you have no machine “{}”", name.unwrap_or_default()))
}

async fn list() -> anyhow::Result<()> {
    let hub = ClientConfig::load()?.with_env().hub()?;
    let this = stemma_worker::control::this(&hub).await.ok().map(|m| m.id);
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
        service::remove_machine()?;
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        println!("This machine forgot its token.");
    }
    Ok(())
}
