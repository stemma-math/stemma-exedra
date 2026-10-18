//! The runners: how a worker gets an answer from an agent, or has one work
//! on the library for the workshop.
//!
//! - `claude-code`, `codex` and `opencode` run those harnesses headless,
//!   launched as [`crate::launch`] says, with the person's own sign-in (or,
//!   for OpenCode, the keys of the providers it is configured with), and
//!   give them Stemma's tools through `stemma mcp` with the job's token,
//!   read-only: an agent answering in a thread, for someone who may ask for
//!   work, may also hand a task to the workshop. For a task they work in a
//!   worktree of the library, where they edit files and run commands,
//!   Lean's among them.

pub mod claude_code;
pub mod codex;
pub mod opencode;

use std::path::Path;
use std::process::{ExitStatus, Stdio};

use anyhow::Context as _;
use stemma_core::api::AgentProfile;
use stemma_core::protocol::{JobSpec, Usage};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::config::CliRunner;
use crate::isolation::Sandbox;
use crate::job::{Context, JobDir, Prompt};
use crate::launch::{self, Launch, Mcp, Mode, Prepared};

/// Where and how a runner works.
pub struct Work<'a> {
    /// The folder it works in, on the machine: the job's own, or for a
    /// task, the worktree of the library.
    pub dir: &'a Path,
    /// Where it runs: on the machine, or in the job's container.
    pub sandbox: &'a dyn Sandbox,
    /// Its program in the container, if not its usual name.
    pub contained_program: Option<String>,
    /// Whether it edits files and runs commands there, as for a task.
    pub edits: bool,
    /// The most steps it may take, for runners that count them.
    pub max_turns: Option<u32>,
}

/// What an agent said, and what it consumed, as its runner reports it.
pub struct Answer {
    pub text: String,
    pub usage: Usage,
}

/// A count of tokens, from a runner's JSON.
pub(crate) fn tokens(value: &serde_json::Value) -> Option<u64> {
    value.as_u64()
}

/// Whether a job's agent may hand tasks to the workshop, as the hub says:
/// one answering in a thread, for someone who may ask for work.
pub fn hands_over(job: &JobSpec) -> bool {
    job.hands_over
}

/// The Stemma tools a job's agent may use, by name: those that read, and
/// if it may, what hands a task over, or what opens a thread.
pub fn allowed_tools(job: &JobSpec) -> Vec<&'static str> {
    let mut names = stemma_client::tools::read_only_names();
    if hands_over(job) {
        names.extend(stemma_client::tools::HANDS_OVER);
    }
    if job.opens_threads {
        names.extend(stemma_client::tools::OPENS_THREADS);
    }
    names
}

/// The arguments of `stemma mcp` for a job: its hub, its group, its token
/// and its agent, offering only the tools that read, and what hands a
/// task over if the job's agent may; as the job's commands see them, in
/// `sandbox`.
pub fn mcp_args(
    sandbox: &dyn Sandbox,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "mcp".into(),
        "--hub".into(),
        sandbox.hub().to_owned(),
        "--group".into(),
        job.group.slug.clone(),
        "--token-file".into(),
        sandbox.path(&dir.token_file).display().to_string(),
        "--agent".into(),
        agent.display_name.clone(),
        "--read-only".into(),
    ];
    if hands_over(job) {
        args.push("--hand-over".into());
    }
    if job.opens_threads {
        args.push("--open-threads".into());
    }
    args
}

/// Prepares a runner's launch for a job: its own and the agent's
/// arguments, the agent's model or else its own, Stemma's instructions,
/// and Stemma's tools with the job's token; where it runs, as it sees its
/// files there.
pub fn prepared(
    _context: &Context,
    config: &CliRunner,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
    work: &Work<'_>,
    prompt: &Prompt,
) -> anyhow::Result<Prepared> {
    let sandbox = work.sandbox;
    let contained = sandbox.isolation().contained();
    let mcp = Mcp {
        command: sandbox.stemma(),
        args: mcp_args(sandbox, job, agent, dir),
    };
    let mut args = config.args.clone();
    args.extend(agent.args.iter().cloned());
    // The machine's own program for it is not the container's.
    let program = if contained {
        work.contained_program.as_deref()
    } else {
        config.command.as_deref()
    };
    let files_seen = sandbox.path(&dir.path);
    let dir_seen = sandbox.path(work.dir);
    launch::prepare(&Launch {
        runner: &agent.runner,
        program,
        model: agent.model.as_deref().or(config.model.as_deref()),
        args: &args,
        instructions: &prompt.system,
        mcp: &mcp,
        files: &dir.path,
        files_seen: &files_seen,
        mode: Mode::Job {
            dir: &dir_seen,
            edits: work.edits,
            contained,
            max_turns: work.max_turns,
            allowed: allowed_tools(job).into_iter().map(str::to_owned).collect(),
        },
    })
}

/// How a command ended.
pub struct Ran {
    pub status: ExitStatus,
    /// The end of what it wrote to standard error.
    pub stderr: String,
}

/// The command that runs a prepared launch, where `work` says.
pub fn command(prepared: &Prepared, work: &Work<'_>) -> Command {
    work.sandbox
        .command(&prepared.program, &prepared.args, &prepared.env, work.dir)
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
