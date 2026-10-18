//! `stemma`: the hub, the worker and the client, in one binary.

mod client;
mod launcher;
mod machine;

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use stemma_core::api::{NewAccount, runners};
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
    /// Set up and serve this machine, where your own agents run, and the
    /// workshops it works for.
    Machine(machine::MachineArgs),
    /// Sign in to a hub from this machine.
    Login(client::LoginArgs),
    /// Sign out of the hub, revoking this machine's token.
    Logout,
    /// List a group's threads.
    Threads(client::ThreadsArgs),
    /// Read a thread, some of its messages, a declaration or a task: t12,
    /// t12#45-50, lean:Foo.bar, w7.
    Read(client::ReadArgs),
    /// Post a message in a thread, or in reply to a message.
    Post(client::PostArgs),
    /// Search a group's threads and messages.
    Search(client::SearchArgs),
    /// How the group's library stands, or search it.
    Library(client::LibraryArgs),
    /// List the workshop's tasks; read one with `stemma read w7`.
    Tasks(client::TasksArgs),
    /// Hand a task to the workshop, with its instructions on standard input.
    Task(client::TaskArgs),
    /// Approve a task's changes (w7): as the one who asked, or as an admin.
    Approve(client::ReviewArgs),
    /// Reject a task's changes (w7): as the one who asked, or as an admin.
    Reject(client::ReviewArgs),
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
        Command::Login(args) => client::login(args).await,
        Command::Logout => client::logout().await,
        Command::Threads(args) => client::threads(args).await,
        Command::Read(args) => client::read(args).await,
        Command::Post(args) => client::post(args).await,
        Command::Search(args) => client::search(args).await,
        Command::Library(args) => client::library(args).await,
        Command::Tasks(args) => client::tasks(args).await,
        Command::Task(args) => client::task(args).await,
        Command::Approve(args) => client::approve(args).await,
        Command::Reject(args) => client::reject(args).await,
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
