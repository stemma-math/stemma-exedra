//! `stemma`: the hub, the worker and the client, in one binary.

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use stemma_core::api::NewAccount;
use stemma_hub::Config;
use tracing_subscriber::EnvFilter;

/// Stemma: where a research group does mathematics by talking.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the hub: accounts, groups, the forum and the web app.
    Hub(HubArgs),
    /// Run a worker for a hub (not yet available).
    Worker,
    /// Sign in to a hub from this machine (not yet available).
    Login,
    /// Serve Stemma's tools to an agent over MCP (not yet available).
    Mcp,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info")),
        )
        .init();

    match Cli::parse().command {
        Command::Hub(args) => hub(args).await,
        Command::Worker | Command::Login | Command::Mcp => {
            anyhow::bail!("not yet: this stage of Stemma has only the hub")
        }
    }
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
