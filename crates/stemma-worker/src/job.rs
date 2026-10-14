//! Running one job: its folder, its prompt, and the runner that answers.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use stemma_client::format;
use stemma_core::api::{ThreadKind, runners as names};
use stemma_core::protocol::{ContextItem, JobSpec, Outcome};
use tokio::sync::mpsc;

use crate::config::WorkerConfig;
use crate::runners;

/// Stemma's instructions to an agent answering in a thread.
pub const REPLY_INSTRUCTIONS: &str = include_str!("../prompts/reply.md");

/// What every job of a worker shares.
pub struct Context {
    pub config: WorkerConfig,
    /// The hub's address.
    pub hub: String,
    /// The `stemma` binary, for the agents' MCP server.
    pub stemma: PathBuf,
    pub work_dir: PathBuf,
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
        "search" => match arg("query") {
            Some(query) => format!("Searching for “{query}”"),
            None => "Searching".into(),
        },
        "list_threads" => "Looking through the threads".into(),
        other => format!("Using {other}"),
    }
}

/// The two parts of an agent's prompt.
pub struct Prompt {
    /// How to behave: Stemma's instructions and the profile's.
    pub system: String,
    /// What to answer: the thread and the request.
    pub user: String,
}

/// The prompt for a job.
pub fn prompt(job: &JobSpec) -> Prompt {
    let requester = &job.requester.display_name;
    let mut system = REPLY_INSTRUCTIONS
        .replace("{agent}", &job.agent.display_name)
        .replace("{requester}", requester)
        .replace("{group}", &job.group.name);
    if let Some(more) = job.agent.instructions.as_deref() {
        system.push_str("\nFrom the group, for you:\n\n");
        system.push_str(more.trim());
        system.push('\n');
    }

    let thread = &job.thread;
    let kind = match thread.kind {
        ThreadKind::Discussion => "a discussion",
        ThreadKind::Chat => "a chat, where you answer every message",
    };
    let trimmed = job
        .context
        .iter()
        .any(|item| matches!(item, ContextItem::Gap { .. }));
    let mut user = format!(
        "Group: {} ({}). Thread t{}, “{}”: {kind}.\n\n",
        job.group.name, job.group.slug, thread.number, thread.title
    );
    user.push_str(if trimmed {
        "The thread, with the messages left out marked:\n\n"
    } else {
        "The thread:\n\n"
    });
    for item in &job.context {
        match item {
            ContextItem::Message { message } => {
                user.push_str(&format::message(thread.number, message));
            }
            ContextItem::Gap { from, to } => {
                user.push_str(&format::gap(thread.number, *from, *to));
            }
        }
        user.push('\n');
    }
    user.push_str(&format!(
        "---\n\nAnswer message t{}#{}, by {requester} (@{}). Your answer is posted in reply to it.\n",
        thread.number, job.trigger, job.requester.username
    ));
    Prompt { system, user }
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

/// Runs a job to its outcome.
pub async fn run(context: Arc<Context>, job: JobSpec, progress: Progress) -> Outcome {
    match answer(&context, &job, &progress).await {
        Ok(body) if body.trim().is_empty() => Outcome::Failed {
            error: format!("{} gave no answer", job.agent.display_name),
        },
        Ok(body) => Outcome::Answer {
            body: body.trim().to_owned(),
        },
        Err(error) => {
            tracing::warn!(job = job.id, error = format!("{error:#}"), "a job failed");
            Outcome::Failed {
                error: format!("{error:#}"),
            }
        }
    }
}

async fn answer(context: &Context, job: &JobSpec, progress: &Progress) -> anyhow::Result<String> {
    let dir = JobDir::create(&context.work_dir, job)?;
    let prompt = prompt(job);
    progress.say("Thinking");
    let configured = &context.config.runners;
    let name = &job.agent.display_name;
    match job.agent.runner.as_str() {
        names::CLAUDE_CODE => {
            let config = configured.claude_code.clone().unwrap_or_default();
            let work = runners::claude_code::run(context, &config, job, &dir, &prompt, progress);
            within(config.timeout_minutes.unwrap_or(15), name, work).await
        }
        names::CODEX => {
            let config = configured.codex.clone().unwrap_or_default();
            let work = runners::codex::run(context, &config, job, &dir, &prompt, progress);
            within(config.timeout_minutes.unwrap_or(15), name, work).await
        }
        names::API => {
            let config = configured.api.clone().unwrap_or_default();
            let work = runners::api::run(context, &config, job, &prompt, progress);
            within(config.timeout_minutes.unwrap_or(10), name, work).await
        }
        other => anyhow::bail!("this worker has no runner {other}"),
    }
}

/// The answer, if it comes within `minutes`.
async fn within(
    minutes: u64,
    agent: &str,
    work: impl Future<Output = anyhow::Result<String>>,
) -> anyhow::Result<String> {
    match tokio::time::timeout(Duration::from_secs(minutes * 60), work).await {
        Ok(answer) => answer,
        Err(_) => anyhow::bail!("{agent} took more than {minutes} minutes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        assert_eq!(describe_call("other", &json!({})), "Using other");
    }
}
