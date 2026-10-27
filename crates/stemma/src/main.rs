//! `stemma`: the hub, the worker and the client, in one binary.

mod client;
mod launcher;
mod machine;
mod output;
mod settings;
mod tools;
mod workshop;

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use stemma_core::api::{NewAccount, runners};
use stemma_hub::Config;
use tracing_subscriber::EnvFilter;

const HELP: &str = "{about}

Usage: {usage}

Your account:
  login       Sign in on this computer
  logout      Sign out
  group       Show or choose the active group

Your agents:
  claude      Open Claude Code
  codex       Open Codex
  opencode    Open OpenCode
  mcp         Serve Stemma's tools

The group:
  tools       Read and write in the group, as your agents do

This computer:
  machine     Manage the agents' service and image
  config      Explain and edit settings
  doctor      Check this computer

Workshop:
  workshop    Approve work on this computer

Running a hub:
  hub         Run a hub

{options}";

/// Stemma: where a research group does mathematics by talking.
#[derive(Parser)]
#[command(version, about, disable_help_subcommand = true, help_template = HELP, styles = stemma_design_terminal::themes::clap::styles())]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the hub: accounts, groups, the forum and the web app.
    Hub(HubArgs),
    /// Manage this computer's agents and service.
    Machine(machine::MachineArgs),
    /// Approve and inspect this computer's workshop processes.
    Workshop(workshop::WorkshopArgs),
    /// Sign in to a hub from this machine.
    Login(client::LoginArgs),
    /// Sign out of the hub, revoking this machine's token.
    Logout,
    /// Check this computer, its runners, workspace and service.
    Doctor(DoctorArgs),
    /// Show or choose your active group.
    Group(settings::GroupArgs),
    /// Explain and edit this computer's settings.
    Config(settings::ConfigArgs),
    /// Read and write in the group, as your agents do.
    Tools,
    /// Open your Claude Code, with Stemma's tools and instructions for the
    /// session.
    Claude(launcher::LaunchArgs),
    /// Open your Codex, with Stemma's tools and instructions for the
    /// session.
    Codex(launcher::LaunchArgs),
    /// Open your OpenCode, with Stemma's tools and instructions for the
    /// session.
    Opencode(launcher::LaunchArgs),
    /// Serve Stemma's tools to an agent over MCP, on standard input and
    /// output, for agents you install Stemma into.
    Mcp(client::McpArgs),
}

#[derive(Args)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    group: Option<String>,
}

#[derive(Args)]
struct HubArgs {
    /// The configuration file (TOML).
    #[arg(long, short, env = "STEMMA_CONFIG", global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<HubCommand>,
}

#[derive(Subcommand)]
enum HubCommand {
    /// Serve the hub (what `stemma hub` does alone).
    Serve,
    /// Create an account that administers the hub.
    CreateAdmin {
        #[arg(long)]
        username: String,
        /// The name shown to others; the username if absent.
        #[arg(long)]
        display_name: Option<String>,
        /// Read the password from standard input instead of asking.
        #[arg(long)]
        password_stdin: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let policy = stemma_design_terminal::Output::detect(stemma_design_terminal::Stream::Stderr);
    policy.apply_to_console(stemma_design_terminal::Stream::Stderr);
    cliclack::set_theme(stemma_design_terminal::themes::cliclack::Theme::new(policy));
    let matches = Cli::command()
        .mut_subcommand("tools", |_| tools::command())
        .help_template(output::help(HELP))
        .get_matches();
    if let Some(("tools", args)) = matches.subcommand() {
        return tools::run(args).await;
    }
    let cli = Cli::from_arg_matches(&matches)?;
    // Standard output is for what commands print, and for MCP's messages:
    // logs go to standard error, and the client commands log only problems.
    let default = match cli.command {
        Command::Hub(_) => "info,sqlx=warn,tower_http=info",
        Command::Machine(_) => "info",
        _ => "warn",
    };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default)),
        )
        .init();

    match cli.command {
        Command::Hub(args) => hub(args).await,
        Command::Machine(args) => machine::machine(args).await,
        Command::Workshop(args) => workshop::run(args),
        Command::Login(args) => client::login(args).await,
        Command::Logout => client::logout().await,
        Command::Doctor(args) => {
            let checks = stemma_worker::doctor::run(args.group.as_deref()).await;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&checks)?);
            } else {
                use stemma_design_terminal::{Role, Symbol};
                let out = output::stdout();
                for check in checks {
                    let (role, symbol) = match check.status {
                        stemma_worker::doctor::Status::Success => (Role::Success, Symbol::Success),
                        stemma_worker::doctor::Status::Warning => (Role::Warning, Symbol::Warning),
                        stemma_worker::doctor::Status::Danger => (Role::Danger, Symbol::Failure),
                    };
                    println!(
                        "{} {} – {}",
                        out.paint(role, out.symbol(symbol)),
                        out.paint(Role::Strong, &check.name),
                        out.paint(Role::Muted, &check.detail)
                    );
                    if let Some(remedy) = check.remedy {
                        println!("  {remedy}");
                    }
                }
            }
            Ok(())
        }
        Command::Group(args) => settings::group(args).await,
        Command::Config(args) => settings::config(args),
        Command::Tools => unreachable!("tools are handled from the shared catalog"),
        Command::Mcp(args) => client::mcp(args).await,
        Command::Claude(args) => exit(launcher::launch(runners::CLAUDE_CODE, args).await?),
        Command::Codex(args) => exit(launcher::launch(runners::CODEX, args).await?),
        Command::Opencode(args) => exit(launcher::launch(runners::OPENCODE, args).await?),
    }
}

/// Exits as the agent did.
fn exit(code: i32) -> anyhow::Result<()> {
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

async fn hub(args: HubArgs) -> anyhow::Result<()> {
    let config = Config::load(args.config.as_deref())?;
    match args.command.unwrap_or(HubCommand::Serve) {
        HubCommand::Serve => stemma_hub::run(config).await,
        HubCommand::CreateAdmin {
            username,
            display_name,
            password_stdin,
        } => {
            let password = read_password(password_stdin)?;
            let account = NewAccount {
                display_name: display_name.unwrap_or_else(|| username.clone()),
                username,
                password,
            };
            stemma_hub::create_admin(config, account).await
        }
    }
}

fn read_password(from_stdin: bool) -> anyhow::Result<String> {
    if from_stdin || !std::io::stdin().is_terminal() {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_owned());
    }
    let password = rpassword::prompt_password("Password: ")?;
    let again = rpassword::prompt_password("Again: ")?;
    anyhow::ensure!(password == again, "the passwords differ");
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::Cli;

    #[test]
    fn every_command_is_listed_once() {
        use clap::CommandFactory;
        for command in Cli::command().get_subcommands() {
            assert_eq!(
                super::HELP
                    .lines()
                    .filter(|line| line.split_whitespace().next() == Some(command.get_name()))
                    .count(),
                1,
                "{}",
                command.get_name()
            );
        }
    }
}
