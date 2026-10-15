//! Running one job: its folder, its prompt, and the runner that does it;
//! or, for a library's build, Lean.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use stemma_client::format;
use stemma_core::api::{AgentProfile, ThreadKind, runners as names};
use stemma_core::protocol::{
    ContextItem, Conversation, GroupRef, JobSpec, Outcome, PaperUpdate, ThreadRef, Work,
};
use tokio::sync::mpsc;

use crate::config::WorkerConfig;
use crate::{library, runners};

/// Stemma's instructions to an agent answering in a thread.
pub const REPLY_INSTRUCTIONS: &str = include_str!("../prompts/reply.md");

/// Stemma's instructions to an editor updating a paper.
pub const PAPER_INSTRUCTIONS: &str = include_str!("../prompts/paper.md");

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
        "read_thread" | "read_paper" => match arg("address") {
            Some(address) => format!("Reading {address}"),
            None => "Reading".into(),
        },
        "search" | "library_search" => match arg("query") {
            Some(query) => format!("Searching for “{query}”"),
            None => "Searching".into(),
        },
        "list_threads" => "Looking through the threads".into(),
        "list_papers" => "Looking through the papers".into(),
        "paper_changes" => "Looking at what changed".into(),
        "library_lookup" => match arg("name") {
            Some(name) => format!("Looking up lean:{}", name.trim_start_matches("lean:")),
            None => "Looking in the library".into(),
        },
        "library_state" => "Looking at the library".into(),
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

/// Stemma's instructions, filled in, and the profile's own.
fn instructions(template: &str, group: &GroupRef, conversation: &Conversation) -> String {
    let mut system = template
        .replace("{agent}", &conversation.agent.display_name)
        .replace("{requester}", &conversation.requester.display_name)
        .replace("{group}", &group.name);
    if let Some(more) = conversation.agent.instructions.as_deref() {
        system.push_str("\nFrom the group, for you:\n\n");
        system.push_str(more.trim());
        system.push('\n');
    }
    system
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
fn conversation_text(group: &GroupRef, conversation: &Conversation) -> String {
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
pub fn reply_prompt(group: &GroupRef, conversation: &Conversation) -> Prompt {
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
        system: instructions(REPLY_INSTRUCTIONS, group, conversation),
        user,
    }
}

/// The prompt for an editor's update of a paper.
pub fn paper_prompt(group: &GroupRef, update: &PaperUpdate) -> Prompt {
    let conversation = &update.conversation;
    let paper = &update.paper;
    let system =
        instructions(PAPER_INSTRUCTIONS, group, conversation).replace("{slug}", &paper.slug);
    let mut user = conversation_text(group, conversation);

    user.push_str("---\n\n");
    match &paper.title {
        Some(title) => user.push_str(&format!(
            "The paper p/{}, “{title}”, at revision {}:\n\n<current>\n{}</current>\n\n",
            paper.slug,
            paper.revision,
            if paper.body.is_empty() {
                "(empty)\n"
            } else {
                &paper.body
            }
        )),
        None => user.push_str(&format!(
            "The paper p/{} does not exist yet: you start it.\n\n",
            paper.slug
        )),
    }
    if !update.related.is_empty() {
        user.push_str("Other threads about the paper:\n\n");
        for related in &update.related {
            let thread = &related.thread;
            let about = match &related.anchor {
                Some(anchor) => format!(", anchored to {anchor}"),
                None => String::new(),
            };
            user.push_str(&format!(
                "#### Thread t{}, “{}”{about}\n\n",
                thread.number, thread.title
            ));
            user.push_str(&thread_text(thread, &related.context));
        }
        user.push_str("---\n\n");
    }
    user.push_str(&format!(
        "Update p/{} as message t{}#{}, by {} (@{}), asks.\n",
        paper.slug,
        conversation.thread.number,
        conversation.trigger,
        conversation.requester.display_name,
        conversation.requester.username
    ));
    Prompt { system, user }
}

/// The text between the last `<tag>` of `text` and its closing tag.
fn tagged<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.rfind(&open)? + open.len();
    let end = text[start..].find(&close).map(|n| start + n)?;
    Some(&text[start..end])
}

/// Reads an editor's answer: the reason, the title and the new text.
pub fn read_paper(answer: &str, update: &PaperUpdate) -> anyhow::Result<Outcome> {
    let Some(body) = tagged(answer, "paper") else {
        anyhow::bail!(
            "{} did not end with the paper's text, between <paper> and </paper>",
            update.conversation.agent.display_name
        );
    };
    let mut body = body.trim_matches('\n').to_owned();
    // A text wrapped whole in a fence is the text.
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() >= 2
        && lines[0].trim_start().starts_with("```")
        && lines[lines.len() - 1].trim() == "```"
    {
        body = lines[1..lines.len() - 1].join("\n");
    }
    anyhow::ensure!(
        !body.trim().is_empty() || update.paper.body.trim().is_empty(),
        "{} returned an empty paper",
        update.conversation.agent.display_name
    );
    let title = tagged(answer, "title")
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty() && t.chars().count() <= 200)
        .or_else(|| update.paper.title.clone())
        .unwrap_or_else(|| update.paper.slug.replace('-', " "));
    let reason = tagged(answer, "reason")
        .map(|r| r.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| {
            format!(
                "Updated as t{}#{} asked",
                update.conversation.thread.number, update.conversation.trigger
            )
        });
    Ok(Outcome::Paper {
        title,
        body: format!("{}\n", body.trim_end()),
        reason: reason.chars().take(1_000).collect(),
    })
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
    let outcome = match &job.work {
        Work::Reply(conversation) => {
            let prompt = reply_prompt(&job.group, conversation);
            ask(&context, &job, &conversation.agent, &prompt, &progress)
                .await
                .and_then(|body| {
                    anyhow::ensure!(
                        !body.trim().is_empty(),
                        "{} gave no answer",
                        conversation.agent.display_name
                    );
                    Ok(Outcome::Answer {
                        body: body.trim().to_owned(),
                    })
                })
        }
        Work::PaperUpdate(update) => {
            let prompt = paper_prompt(&job.group, update);
            let agent = &update.conversation.agent;
            ask(&context, &job, agent, &prompt, &progress)
                .await
                .and_then(|answer| read_paper(&answer, update))
        }
        Work::LibraryBuild(build) => library::build(&context, &job, build, &progress).await,
    };
    outcome.unwrap_or_else(|error| {
        tracing::warn!(job = job.id, error = format!("{error:#}"), "a job failed");
        Outcome::Failed {
            error: format!("{error:#}"),
        }
    })
}

/// What an agent says to a prompt, through its runner.
async fn ask(
    context: &Context,
    job: &JobSpec,
    agent: &AgentProfile,
    prompt: &Prompt,
    progress: &Progress,
) -> anyhow::Result<String> {
    let dir = JobDir::create(&context.work_dir, job)?;
    progress.say("Thinking");
    let configured = &context.config.runners;
    let name = &agent.display_name;
    match agent.runner.as_str() {
        names::CLAUDE_CODE => {
            let config = configured.claude_code.clone().unwrap_or_default();
            let work =
                runners::claude_code::run(context, &config, job, agent, &dir, prompt, progress);
            within(config.timeout_minutes.unwrap_or(15), name, work).await
        }
        names::CODEX => {
            let config = configured.codex.clone().unwrap_or_default();
            let work = runners::codex::run(context, &config, job, agent, &dir, prompt, progress);
            within(config.timeout_minutes.unwrap_or(15), name, work).await
        }
        names::API => {
            let config = configured.api.clone().unwrap_or_default();
            let work = runners::api::run(context, &config, job, agent, prompt, progress);
            within(config.timeout_minutes.unwrap_or(10), name, work).await
        }
        other => anyhow::bail!("this worker has no runner {other}"),
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
    use stemma_core::api::{AgentScope, User};
    use stemma_core::protocol::PaperSpec;

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
    }

    fn update(title: Option<&str>) -> PaperUpdate {
        PaperUpdate {
            conversation: Conversation {
                agent: AgentProfile {
                    handle: "editor".into(),
                    display_name: "Editor".into(),
                    runner: "api".into(),
                    model: None,
                    endpoint: None,
                    instructions: None,
                    scope: AgentScope::Group,
                    role: Default::default(),
                },
                thread: ThreadRef {
                    number: 4,
                    title: "Decisions".into(),
                    kind: ThreadKind::Discussion,
                },
                trigger: 7,
                requester: User {
                    id: 1,
                    username: "ada".into(),
                    display_name: "Ada".into(),
                },
                context: Vec::new(),
            },
            paper: PaperSpec {
                slug: "gluing-maps".into(),
                title: title.map(str::to_owned),
                revision: if title.is_some() { 3 } else { 0 },
                body: if title.is_some() {
                    "Old.\n".into()
                } else {
                    String::new()
                },
            },
            related: Vec::new(),
        }
    }

    #[test]
    fn reads_an_editors_answer() {
        let answer = "I read the thread.\n\n<reason>Added the\n decision of t4#7.</reason>\n<title>Gluing maps</title>\n<paper>\n# Setting\n\nNew.\n</paper>\n";
        let Outcome::Paper {
            title,
            body,
            reason,
        } = read_paper(answer, &update(Some("Old title"))).unwrap()
        else {
            panic!("expected a paper");
        };
        assert_eq!(title, "Gluing maps");
        assert_eq!(body, "# Setting\n\nNew.\n");
        assert_eq!(reason, "Added the decision of t4#7.");

        // A fenced text, with no reason nor title.
        let answer = "<paper>\n```markdown\nNew.\n```\n</paper>";
        let Outcome::Paper {
            title,
            body,
            reason,
        } = read_paper(answer, &update(None)).unwrap()
        else {
            panic!("expected a paper");
        };
        assert_eq!(title, "gluing maps");
        assert_eq!(body, "New.\n");
        assert_eq!(reason, "Updated as t4#7 asked");

        assert!(read_paper("No paper here.", &update(None)).is_err());
        assert!(read_paper("<paper>\n</paper>", &update(Some("T"))).is_err());
    }

    #[test]
    fn an_editors_prompt_has_the_paper() {
        let prompt = paper_prompt(
            &GroupRef {
                slug: "geometry".into(),
                name: "Geometry".into(),
            },
            &update(Some("Gluing maps")),
        );
        assert!(
            prompt
                .system
                .contains("update the\nworking paper p/gluing-maps")
        );
        assert!(
            prompt
                .user
                .contains("at revision 3:\n\n<current>\nOld.\n</current>")
        );
        assert!(
            prompt
                .user
                .contains("Update p/gluing-maps as message t4#7, by Ada (@ada), asks.")
        );
    }
}
