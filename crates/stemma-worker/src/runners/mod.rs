//! The runners: how a worker gets an answer from an agent, or has one work
//! on the library for the workshop.
//!
//! - `claude-code`, `codex` and `opencode` run those harnesses headless,
//!   with the person's own login (or, for OpenCode, the keys of the
//!   providers it is configured with), and give them Stemma's tools
//!   through `stemma mcp` with the job's token, read-only: an agent
//!   answering in a thread may also hand a task to the workshop. For a
//!   task they work in a worktree of the library, where they edit files
//!   and run commands, Lean's among them.

pub mod claude_code;
pub mod codex;
pub mod opencode;

use std::path::Path;
use std::process::{ExitStatus, Stdio};

use anyhow::Context as _;
use stemma_core::api::{AgentProfile, JobKind};
use stemma_core::protocol::JobSpec;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::job::{Context, JobDir};

/// Where and how a runner works.
pub struct Work<'a> {
    /// The folder it works in: the job's own, or for a task, the worktree
    /// of the library.
    pub dir: &'a Path,
    /// Whether it edits files and runs commands there, as for a task.
    pub edits: bool,
    /// The most steps it may take, for runners that count them.
    pub max_turns: Option<u32>,
}

/// Whether a job's agent may hand tasks to the workshop: one answering in
/// a thread.
pub fn hands_over(job: &JobSpec) -> bool {
    job.kind() == JobKind::Reply
}

/// The Stemma tools a job's agent may use, by name: those that read, and
/// for an agent answering in a thread, what hands a task over.
pub fn allowed_tools(job: &JobSpec) -> Vec<&'static str> {
    let mut names = stemma_client::tools::read_only_names();
    if hands_over(job) {
        names.extend(stemma_client::tools::HANDS_OVER);
    }
    names
}

/// The arguments of `stemma mcp` for a job: its hub, its group, its token
/// and its agent, offering only the tools that read, and what hands a
/// task over if the job's agent may.
pub fn mcp_args(
    context: &Context,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
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
    ];
    if hands_over(job) {
        args.push("--hand-over".into());
    }
    args
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
