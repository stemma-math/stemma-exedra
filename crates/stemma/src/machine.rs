//! This computer's service and agents' image.
use clap::{Args, Subcommand};
use stemma_worker::{image, service, sign_in};

#[derive(Args)]
pub struct MachineArgs {
    #[command(subcommand)]
    command: Option<MachineCommand>,
}
#[derive(Subcommand)]
enum MachineCommand {
    /// Install and start the service at login.
    Start,
    /// Stop and remove the service.
    Stop,
    /// Run the service in this terminal.
    Run,
    /// Build the agents' image for Docker or Podman.
    Image,
    /// Sign an agent in for its containers, apart from your own sign-in.
    SignIn {
        /// claude, codex or opencode.
        agent: String,
    },
    /// Forget an agent's sign-in for its containers.
    SignOut {
        /// claude, codex or opencode.
        agent: String,
    },
    /// Ask an agent, in a container, for one word: whether it signs in.
    Test {
        /// claude, codex or opencode.
        agent: String,
    },
}
pub async fn machine(args: MachineArgs) -> anyhow::Result<()> {
    match args.command {
        Some(MachineCommand::Start) => {
            let _ = stemma_client::config::Credentials::load()?
                .with_env()
                .hub()?
                .me()
                .await?;
            let progress = indicatif::ProgressBar::new_spinner();
            progress.set_style(stemma_design_terminal::themes::indicatif::spinner(
                &crate::output::stdout(),
            ));
            progress.set_message("Starting your agents");
            progress.enable_steady_tick(std::time::Duration::from_millis(100));
            service::install(&std::env::current_exe()?)?;
            progress.finish_and_clear();
            println!("Service started.");
        }
        Some(MachineCommand::Stop) => {
            service::uninstall()?;
            println!("Service stopped and removed.");
        }
        Some(MachineCommand::Run) => stemma_worker::machine::run(std::env::current_exe()?).await?,
        Some(MachineCommand::Image) => {
            image::build_for(None).await?;
        }
        Some(MachineCommand::SignIn { agent }) => sign_in(&agent)?,
        Some(MachineCommand::SignOut { agent }) => {
            sign_in::runner_of(&agent)?;
            sign_in::forget(&stemma_client::config::Paths::discover()?, &agent)?;
            println!("{agent} is signed out of its containers.");
        }
        Some(MachineCommand::Test { agent }) => {
            sign_in::runner_of(&agent)?;
            let said = sign_in::test(&agent, None).await?;
            println!("{agent} answered in its container: {said}");
        }
        None => {
            let status = service::status().await?;
            println!(
                "{} – {}",
                if status.signed_out {
                    "Signed out"
                } else if status.connected == Some(true) {
                    "Connected"
                } else {
                    "Offline"
                },
                if status.running {
                    "Service running"
                } else {
                    "Service stopped"
                }
            );
            println!("Logs: {}", status.logs);
        }
    }
    Ok(())
}

/// Signs `agent` in for its containers, with its own program here.
fn sign_in(agent: &str) -> anyhow::Result<()> {
    use stemma_client::config::{Credentials, Paths, Settings, path};
    sign_in::runner_of(agent)?;
    let paths = Paths::discover()?;
    let program = Settings::load()?
        .get(&format!("agents.{agent}.command"), None)?
        .value
        .as_str()
        .unwrap_or(agent)
        .to_owned();
    let status = sign_in::command(&paths, agent, &program)?.status()?;
    anyhow::ensure!(status.success(), "{program} did not sign in");
    if agent == "claude" {
        // setup-token prints the token: keep it, as the containers' sign-in.
        let token = cliclack::password("Paste the token it printed").interact()?;
        let token = token.trim();
        anyhow::ensure!(!token.is_empty(), "no token to keep");
        let file = path()?;
        let mut credentials = Credentials::load_from(&file)?;
        credentials
            .secrets
            .insert(sign_in::CLAUDE_TOKEN.into(), token.into());
        credentials.save_to(&file)?;
    }
    println!("{agent} is signed in for its containers. Try it with: stemma machine test {agent}");
    Ok(())
}
