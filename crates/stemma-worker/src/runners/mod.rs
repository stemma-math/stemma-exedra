//! The runners: how a worker gets an answer from an agent.
//!
//! - `claude-code` and `codex` run those harnesses headless, with the
//!   person's own login, and give them Stemma's tools through `stemma mcp`
//!   with the job's token, read-only.
//! - `api` is Stemma's own harness: a loop of tool calls over an
//!   OpenAI-compatible chat API, with the same tools.

pub mod api;
pub mod claude_code;
pub mod codex;

use std::process::{ExitStatus, Stdio};

use anyhow::Context as _;
use stemma_core::api::AgentProfile;
use stemma_core::protocol::JobSpec;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::job::{Context, JobDir};

/// The Stemma tools an agent may use, by name: those that read.
pub fn allowed_tools() -> Vec<&'static str> {
    stemma_client::tools::read_only_names()
}

/// The arguments of `stemma mcp` for a job: its hub, its group, its token
/// and its agent, offering only the tools that read.
pub fn mcp_args(
    context: &Context,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
) -> Vec<String> {
    vec![
        "mcp".into(),
        "--hub".into(),
        context.hub.clone(),
        "--group".into(),
        job.group.slug.clone(),
        "--token-file".into(),
        dir.token_file.display().to_string(),
        "--agent".into(),
        agent.display_name.clone(),
        "--read-only".into(),
    ]
}

/// How a command ended.
pub struct Ran {
    pub status: ExitStatus,
    /// The end of what it wrote to standard error.
    pub stderr: String,
}

/// Runs a command with `stdin` as its input, handing each line of its
/// output to `line`. The command is killed if this is dropped.
pub async fn run_command(
    mut command: Command,
    stdin: &str,
    mut line: impl FnMut(&str),
) -> anyhow::Result<Ran> {
    let program = command
        .as_std()
        .get_program()
        .to_string_lossy()
        .into_owned();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("running {program}"))?;
    let mut input = child.stdin.take().expect("piped");
    let stdin = stdin.to_owned();
    let writing = tokio::spawn(async move {
        let _ = input.write_all(stdin.as_bytes()).await;
        let _ = input.shutdown().await;
    });
    let mut stderr = child.stderr.take().expect("piped");
    let reading_errors = tokio::spawn(async move {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text).await;
        text
    });
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    while let Some(text) = lines.next_line().await? {
        line(&text);
    }
    let status = child.wait().await?;
    let _ = writing.await;
    let stderr = reading_errors.await.unwrap_or_default();
    let tail: String = {
        let chars: Vec<char> = stderr.trim().chars().collect();
        chars[chars.len().saturating_sub(1_000)..].iter().collect()
    };
    Ok(Ran {
        status,
        stderr: tail,
    })
}

/// The error of a command that failed.
pub fn failed(program: &str, ran: &Ran) -> anyhow::Error {
    if ran.stderr.is_empty() {
        anyhow::anyhow!("{program} failed ({})", ran.status)
    } else {
        anyhow::anyhow!("{program} failed ({}): {}", ran.status, ran.stderr)
    }
}
