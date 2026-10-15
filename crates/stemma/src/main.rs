//! `stemma`: the hub, the worker and the client, in one binary.

mod client;

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
    /// Run a worker: connect to a hub and run the jobs of its agents.
    Worker(WorkerArgs),
    /// Sign in to a hub from this machine.
    Login(client::LoginArgs),
    /// Sign out of the hub, revoking this machine's token.
    Logout,
    /// List a group's threads.
    Threads(client::ThreadsArgs),
    /// Read a thread, some of its messages, a paper or a declaration: t12,
    /// t12#45-50, p/notes#gluing, lean:Foo.bar.
    Read(client::ReadArgs),
    /// Post a message in a thread, or in reply to a message.
    Post(client::PostArgs),
    /// Search a group's threads and messages.
    Search(client::SearchArgs),
    /// List a group's working papers.
    Papers(client::PapersArgs),
    /// What changed in a paper since you read it, or in every paper.
    Changes(client::ChangesArgs),
    /// Write a new revision of a paper, from standard input.
    Edit(client::EditArgs),
    /// How the group's library stands, or search it.
    Library(client::LibraryArgs),
    /// Serve Stemma's tools to an agent over MCP, on standard input and output.
    Mcp(client::McpArgs),
}

#[derive(Args)]
struct WorkerArgs {
    /// The worker's configuration file (TOML).
    #[arg(long, short, env = "STEMMA_WORKER_CONFIG")]
    config: PathBuf,
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
    let cli = Cli::parse();
    // Standard output is for what commands print, and for MCP's messages:
    // logs go to standard error, and the client commands log only problems.
    let default = match cli.command {
        Command::Hub(_) | Command::Worker(_) => "info,sqlx=warn,tower_http=info",
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
        Command::Worker(args) => stemma_worker::run(&args.config).await,
        Command::Login(args) => client::login(args).await,
        Command::Logout => client::logout().await,
        Command::Threads(args) => client::threads(args).await,
        Command::Read(args) => client::read(args).await,
        Command::Post(args) => client::post(args).await,
        Command::Search(args) => client::search(args).await,
        Command::Papers(args) => client::papers(args).await,
        Command::Changes(args) => client::changes(args).await,
        Command::Edit(args) => client::edit(args).await,
        Command::Library(args) => client::library(args).await,
        Command::Mcp(args) => client::mcp(args).await,
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
