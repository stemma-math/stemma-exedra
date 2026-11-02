//! What a workshop process's `run.sh` and its agents use: `stemma agent run`
//! and `stemma output`. They work only inside a process.
use std::path::PathBuf;

use clap::{Args, Subcommand};
use stemma_core::workshop::Delivered;

#[derive(Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    command: AgentCommand,
}

#[derive(Subcommand)]
enum AgentCommand {
    /// Have the request's agent work on the prompt in FILE, in this
    /// process's folder, and print its answer.
    Run {
        file: PathBuf,
        /// Another model than the request's.
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Args)]
pub struct OutputArgs {
    #[command(subcommand)]
    command: OutputCommand,
}

#[derive(Subcommand)]
enum OutputCommand {
    /// Post a message in the request's thread when the process ends.
    Message {
        /// The message, in Markdown.
        text: Option<String>,
        /// Read the message from this file instead.
        #[arg(long, conflicts_with = "text")]
        file: Option<PathBuf>,
    },
    /// Open a pull request from this process's branch, which it pushed.
    PullRequest {
        /// `library`, or `paper-<slug>`.
        repository: String,
        #[arg(long)]
        title: String,
        /// What changed, and how it was checked.
        #[arg(long)]
        body: Option<String>,
        /// Read the body from this file instead.
        #[arg(long, conflicts_with = "body")]
        body_file: Option<PathBuf>,
    },
}

fn text_of(text: Option<String>, file: Option<PathBuf>) -> anyhow::Result<String> {
    Ok(match file {
        Some(file) => std::fs::read_to_string(&file)
            .map_err(|e| anyhow::anyhow!("Reading {}: {e}", file.display()))?,
        None => text.unwrap_or_default(),
    })
}

pub async fn agent(args: AgentArgs) -> anyhow::Result<()> {
    match args.command {
        AgentCommand::Run { file, model } => {
            let answer = stemma_worker::processes::agent_run(&file, model).await?;
            println!("{}", answer.trim());
        }
    }
    Ok(())
}

pub fn output(args: OutputArgs) -> anyhow::Result<()> {
    let output = match args.command {
        OutputCommand::Message { text, file } => Delivered::Message {
            text: text_of(text, file)?,
        },
        OutputCommand::PullRequest {
            repository,
            title,
            body,
            body_file,
        } => Delivered::PullRequest {
            repository,
            title,
            body: text_of(body, body_file)?,
        },
    };
    stemma_worker::processes::deliver(output)?;
    eprintln!("Delivered when the process ends.");
    Ok(())
}
