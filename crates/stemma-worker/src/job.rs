//! Running a personal reply in group scratch, with private launch files in state.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use stemma_client::format;
use stemma_core::api::{AgentProfile, ThreadKind, runners as names};
use stemma_core::protocol::{
    ContextItem, Conversation, GroupRef, JobSpec, Outcome, ThreadRef, Usage, Work,
};
use tokio::sync::mpsc;

use crate::config::WorkerConfig;
use crate::isolation::{HostSandbox, Isolator};
use crate::runners;
use crate::runners::{Answer, Work as RunnerWork};

/// Stemma's instructions to an agent answering in a thread.
pub const REPLY_INSTRUCTIONS: &str = include_str!("../prompts/reply.md");

/// What an answer in a container of its own is told of its workspace.
const CONTAINED: &str = "\
- You work in the group's workspace, in a container of your own:
  `library/` and `papers/<slug>/` are clones of its repositories at `main`,
  and you start in a scratch folder. You may run anything there, such as
  `lake build` or `latexmk`, to check your answer. Nothing you write there
  is kept.";

/// What an answer on its person's computer is told.
const ON_THE_COMPUTER: &str = "\
- You can read the group's workspace (`library/` and `papers/<slug>/`,
  clones of its repositories at `main`), but you cannot run commands or
  edit files: answer from what you read and reason.";

/// What every job of a machine shares.
pub struct Context {
    pub config: WorkerConfig,
    /// The hub's address.
    pub hub: String,
    /// The `stemma` binary, for the agents' MCP server.
    pub stemma: PathBuf,
    pub work_dir: PathBuf,
    /// How it isolates agent jobs; none if it cannot, and then it
    /// runs none.
    pub isolator: Option<Isolator>,
}

impl Context {
    /// The machine itself, where people's own jobs run.
    pub fn host(&self) -> HostSandbox {
        HostSandbox {
            hub: self.hub.clone(),
            stemma: self.stemma.clone(),
        }
    }
}

/// Where a job says what it is doing.
#[derive(Clone)]
pub struct Progress {
    job: i64,
    sender: mpsc::UnboundedSender<(i64, String)>,
}

impl Progress {
    pub fn new(job: i64, sender: mpsc::UnboundedSender<(i64, String)>) -> Progress {
        Progress { job, sender }
    }

    pub fn say(&self, text: impl Into<String>) {
        let _ = self.sender.send((self.job, text.into()));
    }

    /// Says what a call to a Stemma tool is doing.
    pub fn tool(&self, name: &str, args: &serde_json::Value) {
        self.say(describe_call(name, args));
    }
}

/// What a call to a Stemma tool is doing, for people watching.
pub fn describe_call(name: &str, args: &serde_json::Value) -> String {
    let arg = |key: &str| args.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    let name = name.rsplit("__").next().unwrap_or(name);
    match name {
        "read_thread" => match arg("address") {
            Some(address) => format!("Reading {address}"),
            None => "Reading".into(),
        },
        "search" | "library_search" => match arg("query") {
            Some(query) => format!("Searching for “{query}”"),
            None => "Searching".into(),
        },
        "list_threads" => "Looking through the threads".into(),
        "library_lookup" => match arg("name") {
            Some(name) => format!("Looking up lean:{}", name.trim_start_matches("lean:")),
            None => "Looking in the library".into(),
        },
        "library_state" => "Looking at the library".into(),
        "list_papers" => "Looking at the papers".into(),
        "read_paper" => match arg("address") {
            Some(address) => format!("Reading {address}"),
            None => "Reading a paper".into(),
        },
        "Bash" | "bash" | "shell" => match arg("command") {
            Some(command) => {
                let command: String = command.chars().take(80).collect();
                format!("Running {command}")
            }
            None => "Running a command".into(),
        },
        "Edit" | "MultiEdit" | "Write" | "edit" | "write" | "patch" => {
            match arg("file_path").or_else(|| arg("filePath")) {
                Some(path) => format!("Editing {path}"),
                None => "Editing".into(),
            }
        }
        "Read" | "read" => match arg("file_path").or_else(|| arg("filePath")) {
            Some(path) => format!("Reading {path}"),
            None => "Reading".into(),
        },
        other => format!("Using {other}"),
    }
}

/// The two parts of an agent's prompt.
pub struct Prompt {
    /// How to behave: Stemma's instructions and the profile's.
    pub system: String,
    /// What to do: the thread and the request.
    pub user: String,
}

/// Stemma's instructions, filled in.
fn instructions(template: &str, group: &GroupRef, conversation: &Conversation) -> String {
    template
        .replace("{agent}", &conversation.agent.display_name)
        .replace("{requester}", &conversation.requester.display_name)
        .replace("{group}", &group.name)
}

/// A thread's messages, with the messages left out marked.
fn thread_text(thread: &ThreadRef, context: &[ContextItem]) -> String {
    let mut text = String::new();
    for item in context {
        match item {
            ContextItem::Message { message } => {
                text.push_str(&format::message(thread.number, message));
            }
            ContextItem::Gap { from, to } => {
                text.push_str(&format::gap(thread.number, *from, *to));
            }
        }
        text.push('\n');
    }
    text
}

fn trimmed(context: &[ContextItem]) -> bool {
    context
        .iter()
        .any(|item| matches!(item, ContextItem::Gap { .. }))
}

/// The thread a request comes from, introduced.
pub fn conversation_text(group: &GroupRef, conversation: &Conversation) -> String {
    let thread = &conversation.thread;
    let kind = match thread.kind {
        ThreadKind::Discussion => "a discussion",
        ThreadKind::Chat => "a chat, where you answer every message",
    };
    let mut user = format!(
        "Group: {} ({}). Thread t{}, “{}”: {kind}.\n\n",
        group.name, group.slug, thread.number, thread.title
    );
    user.push_str(if trimmed(&conversation.context) {
        "The thread, with the messages left out marked:\n\n"
    } else {
        "The thread:\n\n"
    });
    user.push_str(&thread_text(thread, &conversation.context));
    user
}

/// The prompt for an answer.
pub fn reply_prompt(group: &GroupRef, conversation: &Conversation, contained: bool) -> Prompt {
    let thread = &conversation.thread;
    let mut user = conversation_text(group, conversation);
    user.push_str(&format!(
        "---\n\nAnswer message t{}#{}, by {} (@{}). Your answer is posted in reply to it.\n",
        thread.number,
        conversation.trigger,
        conversation.requester.display_name,
        conversation.requester.username
    ));
    Prompt {
        system: instructions(
            &REPLY_INSTRUCTIONS.replace(
                "{workspace}",
                if contained {
                    CONTAINED
                } else {
                    ON_THE_COMPUTER
                },
            ),
            group,
            conversation,
        ),
        user,
    }
}

/// A job's folder, removed when the job is over, with its token in a file
/// only its owner reads.
pub struct JobDir {
    pub path: PathBuf,
    pub token_file: PathBuf,
}

impl JobDir {
    pub fn create(work_dir: &Path, job: &JobSpec) -> anyhow::Result<JobDir> {
        let path = work_dir
            .join("jobs")
            .join(format!("{}-{:08x}", job.id, rand::random::<u32>()));
        std::fs::create_dir_all(&path)?;
        let token_file = path.join("token");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        std::io::Write::write_all(&mut options.open(&token_file)?, job.token.as_bytes())?;
        Ok(JobDir { path, token_file })
    }
}

impl Drop for JobDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Runs a job to its outcome, with what its runner said it consumed.
pub async fn run(context: Arc<Context>, job: JobSpec, progress: Progress) -> (Outcome, Usage) {
    let mut usage = Usage::default();
    let outcome = match &job.work {
        Work::Reply(conversation) => {
            let contained = matches!(context.isolator, Some(Isolator::Containers(_)));
            let prompt = reply_prompt(&job.group, conversation, contained);
            ask(&context, &job, &conversation.agent, &prompt, &progress)
                .await
                .and_then(|answer| {
                    usage = answer.usage;
                    anyhow::ensure!(
                        !answer.text.trim().is_empty(),
                        "{} gave no answer",
                        conversation.agent.display_name
                    );
                    Ok(Outcome::Answer {
                        body: answer.text.trim().to_owned(),
                    })
                })
        }
    };
    let outcome = outcome.unwrap_or_else(|error| {
        tracing::warn!(job = job.id, error = format!("{error:#}"), "a job failed");
        Outcome::Failed {
            error: format!("{error:#}"),
        }
    });
    (outcome, usage)
}

/// What an agent says to a prompt, through its runner.
async fn ask(
    context: &Context,
    job: &JobSpec,
    agent: &AgentProfile,
    prompt: &Prompt,
    progress: &Progress,
) -> anyhow::Result<Answer> {
    let dir = JobDir::create(&context.work_dir, job)?;
    progress.say("Thinking");
    let paths = &context.config.paths;
    let settings = &context.config.settings;
    let hub = stemma_client::Hub::new(&context.hub, Some(job.token.clone()))?;
    let workspace =
        crate::workspace::Workspace::open(settings, paths, &hub, &job.group.slug, &job.token)
            .await?;
    let scratch = crate::workspace::Scratch::create(&workspace.path, job.id)?;
    let host = context.host();
    let mut container = None;
    let mut contained_program = None;
    if let Some(Isolator::Containers(containers)) = &context.isolator {
        use crate::isolation::owner_of;
        let (env, files) = containers.sign_in(&agent.runner, dirs::home_dir().as_deref())?;
        let mounts = answer_mounts(&workspace.path, &dir.path, files);
        let mut spec = containers.spec(
            job.id,
            owner_of(&workspace.path),
            mounts,
            env,
            Duration::from_secs(
                60 * cli_runner(context, &agent.runner)?
                    .timeout_minutes
                    .unwrap_or(15)
                    + 60,
            ),
        );
        // The sign-in's values, from the person's credentials, or else the
        // computer's environment.
        let secrets = stemma_client::config::Credentials::load()
            .map(|c| c.secrets)
            .unwrap_or_default();
        spec.pass_values = spec
            .pass_env
            .iter()
            .filter_map(|name| {
                secrets
                    .get(name)
                    .cloned()
                    .or_else(|| std::env::var(name).ok())
                    .map(|value| (name.clone(), value))
            })
            .collect();
        container = Some(containers.start(&spec, &workspace.path).await?);
        contained_program = containers.program(&agent.runner);
    }
    let sandbox: &dyn crate::isolation::Sandbox = match &container {
        Some(container) => container,
        None => &host,
    };
    let work = RunnerWork {
        dir: &scratch.0,
        root: &workspace.path,
        sandbox,
        contained_program,
    };
    let name = &agent.display_name;
    let minutes = cli_runner(context, &agent.runner)?
        .timeout_minutes
        .unwrap_or(15);
    within(
        minutes,
        name,
        run_cli(context, job, agent, &dir, &work, prompt, progress),
    )
    .await
}

/// What an answer's container is given of the computer, and nothing else:
/// the group's workspace, read-write, where it works; the job's folder, with
/// its token and the agent's launch files, read-only; and the agent's
/// sign-in files.
pub fn answer_mounts(
    workspace: &Path,
    job_dir: &Path,
    sign_in: Vec<crate::isolation::Mount>,
) -> Vec<crate::isolation::Mount> {
    use crate::isolation::{Mount, ROOT};
    let mut mounts = vec![
        Mount::Bind {
            source: workspace.to_owned(),
            target: ROOT.into(),
            read_only: false,
        },
        Mount::Bind {
            source: job_dir.to_owned(),
            target: job_dir.to_owned(),
            read_only: true,
        },
    ];
    mounts.extend(sign_in);
    mounts
}

/// The configuration of one of the worker's command-line runners.
pub fn cli_runner(context: &Context, runner: &str) -> anyhow::Result<crate::config::CliRunner> {
    context
        .config
        .runners
        .get(runner)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("this machine has no runner {runner}"))
}

/// Runs one of the command-line runners: Claude Code, Codex or OpenCode.
pub async fn run_cli(
    context: &Context,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
    work: &RunnerWork<'_>,
    prompt: &Prompt,
    progress: &Progress,
) -> anyhow::Result<Answer> {
    let config = cli_runner(context, &agent.runner)?;
    match agent.runner.as_str() {
        names::CLAUDE_CODE => {
            runners::claude_code::run(context, &config, job, agent, dir, work, prompt, progress)
                .await
        }
        names::CODEX => {
            runners::codex::run(context, &config, job, agent, dir, work, prompt, progress).await
        }
        names::OPENCODE => {
            runners::opencode::run(context, &config, job, agent, dir, work, prompt, progress).await
        }
        other => anyhow::bail!("this machine has no runner {other}"),
    }
}

/// What `work` gives, if it comes within `minutes`.
pub async fn within<T>(
    minutes: u64,
    what: &str,
    work: impl Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    match tokio::time::timeout(Duration::from_secs(minutes * 60), work).await {
        Ok(done) => done,
        Err(_) => anyhow::bail!("{what} took more than {minutes} minutes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_answers_container_gets_only_the_workspace_its_job_and_its_sign_in() {
        use crate::isolation::{Mount, ROOT};
        let sign_in = vec![Mount::Bind {
            source: "/home/ada/.codex/auth.json".into(),
            target: "/home/stemma/.codex/auth.json".into(),
            read_only: true,
        }];
        let mounts = answer_mounts(
            Path::new("/data/groups/geometry"),
            Path::new("/state/jobs/7-0000abcd"),
            sign_in,
        );
        let binds: Vec<_> = mounts
            .iter()
            .map(|mount| match mount {
                Mount::Bind {
                    source,
                    target,
                    read_only,
                } => (
                    source.display().to_string(),
                    target.display().to_string(),
                    *read_only,
                ),
                Mount::Hidden { .. } => panic!("nothing hidden"),
            })
            .collect();
        assert_eq!(
            binds,
            [
                ("/data/groups/geometry".into(), ROOT.into(), false),
                (
                    "/state/jobs/7-0000abcd".into(),
                    "/state/jobs/7-0000abcd".into(),
                    true
                ),
                (
                    "/home/ada/.codex/auth.json".into(),
                    "/home/stemma/.codex/auth.json".into(),
                    true
                ),
            ]
        );
    }

    #[test]
    fn describes_tool_calls() {
        assert_eq!(
            describe_call("mcp__stemma__read_thread", &json!({"address": "t3#4-6"})),
            "Reading t3#4-6"
        );
        assert_eq!(
            describe_call("search", &json!({"query": "gluing"})),
            "Searching for “gluing”"
        );
        assert_eq!(
            describe_call("library_lookup", &json!({"name": "Foo.bar"})),
            "Looking up lean:Foo.bar"
        );
        assert_eq!(describe_call("other", &json!({})), "Using other");
        assert_eq!(
            describe_call("Bash", &json!({"command": "lake build"})),
            "Running lake build"
        );
        assert_eq!(
            describe_call("edit", &json!({"filePath": "Geometry/Basic.lean"})),
            "Editing Geometry/Basic.lean"
        );
    }
}
