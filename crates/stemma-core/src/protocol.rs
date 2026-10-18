//! The worker protocol: what a person's machine and the hub say to each
//! other.
//!
//! A machine connects outbound to the hub's WebSocket at
//! `/api/v1/worker`, with its token in an `Authorization: Bearer` header,
//! and sends [`WorkerMessage::Hello`]. The hub answers
//! [`HubMessage::Welcome`], or [`HubMessage::Refused`] and closes. Then the
//! hub sends jobs: its owner's agents' answers and, if it is a workshop
//! machine, the workshop's tasks and builds; and the machine sends their
//! progress, their outcome and what they consumed. Each message is one
//! JSON text frame.
//!
//! When the connection drops, the machine stops what it was running and
//! the hub queues those jobs again, so they run from the start, on this
//! machine when it is back or on another.

use serde::{Deserialize, Serialize};

pub use crate::api::JobKind;
use crate::api::{AgentProfile, LibraryReport, Message, ThreadKind, User};

/// The version of this protocol. A hub refuses workers that speak another.
pub const PROTOCOL_VERSION: u32 = 7;

/// The path of the workers' WebSocket, under the API's prefix.
pub const WORKER_PATH: &str = "/worker";

/// From a worker to the hub.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// The first message: what the worker can do.
    Hello(Hello),
    /// What a job is doing now, for people watching: "Reading t12#3-5".
    Progress { job: i64, text: String },
    /// How a job ended, and what its runner said it consumed.
    Finished {
        job: i64,
        outcome: Outcome,
        #[serde(default)]
        usage: Usage,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// The worker's version of Stemma.
    pub version: String,
    /// The runners it can run, such as `claude-code` or `codex`: those
    /// found installed and usable.
    pub runners: Vec<String>,
    /// Whether it has Lean (elan or lake), for the workshop: its tasks and
    /// the library's builds.
    pub lean: bool,
    /// How it isolates the workshop's jobs: in containers, or on the
    /// machine itself if its owner chose so; none if it cannot, and then
    /// it runs none of them.
    #[serde(default)]
    pub isolation: Option<crate::api::Isolation>,
    /// How many jobs it runs at once.
    pub concurrency: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// An agent's answer, in Markdown, for the hub to post.
    Answer {
        body: String,
    },
    /// A build of the library, whether it passed or not.
    Built(Box<Built>),
    /// How a task's run ended, whatever the end.
    Task(Box<TaskOutcome>),
    Failed {
        error: String,
    },
}

/// How a build of the library went.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Built {
    /// Whether `lake build` passed and the library could be inspected.
    pub succeeded: bool,
    /// Why not, in a line, if it did not.
    pub error: Option<String>,
    /// What the build wrote, trimmed to its end when long.
    pub log: String,
    /// What the inspection found, if it ran.
    pub report: Option<LibraryReport>,
}

/// What a runner said a job consumed, as far as it says: Claude Code
/// reports tokens and cost, Codex tokens, OpenCode tokens and cost.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Input read from the provider's cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    /// Input written to the provider's cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// In US dollars, as the runner priced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl Usage {
    /// Adds what another run consumed.
    pub fn add(&mut self, other: &Usage) {
        fn sum<T: Copy + std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a + b),
                (a, b) => a.or(b),
            }
        }
        self.input_tokens = sum(self.input_tokens, other.input_tokens);
        self.output_tokens = sum(self.output_tokens, other.output_tokens);
        self.cache_read_tokens = sum(self.cache_read_tokens, other.cache_read_tokens);
        self.cache_write_tokens = sum(self.cache_write_tokens, other.cache_write_tokens);
        self.cost_usd = sum(self.cost_usd, other.cost_usd);
    }
}

/// How a task's run ended.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub end: TaskEnd,
    /// Why it failed, in a line.
    pub reason: Option<String>,
    /// The runner's report, in Markdown: what was done, what is pending.
    pub report: String,
    /// The commit pushed to the task's branch; none if nothing changed.
    pub commit: Option<String>,
    /// What the run did, trimmed to its end when long.
    pub log: String,
    /// The build and inspection of the commit pushed.
    pub built: Option<Built>,
    /// The runner that worked on it, and its model, if known.
    pub runner: String,
    pub model: Option<String>,
}

/// How a task's run can end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEnd {
    /// The work passed its checks, and is on the task's branch if it
    /// changed anything.
    Done,
    Failed,
}

/// From the hub to a worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HubMessage {
    Welcome(Welcome),
    /// The hub will not work with this worker; the connection closes.
    Refused {
        reason: String,
    },
    Job(Box<JobSpec>),
    /// Stop a job: nobody wants its outcome any more.
    Cancel {
        job: i64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol: u32,
    /// The machine's name, as registered.
    pub name: String,
    /// Whose it is.
    pub owner: User,
}

/// A job, with what its runner needs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub id: i64,
    pub group: GroupRef,
    /// A token for the hub's API while the job lasts, scoped to its group;
    /// it reads, and does not post.
    pub token: String,
    /// Whether the job's agent may hand tasks to the workshop: one
    /// answering in a thread, for someone who may ask for work.
    #[serde(default)]
    pub hands_over: bool,
    /// Whether the job's agent may open threads of its own in the group,
    /// to ask it something: the workshop's.
    #[serde(default)]
    pub opens_threads: bool,
    pub work: Work,
}

/// What a job is to do.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    /// Answer a message, with [`Outcome::Answer`].
    Reply(Box<Conversation>),
    /// Build a commit of the library and inspect it, with
    /// [`Outcome::Built`].
    LibraryBuild(LibraryBuild),
    /// Work on a task of the workshop in the library, with
    /// [`Outcome::Task`].
    TaskRun(Box<TaskRun>),
}

impl JobSpec {
    pub fn kind(&self) -> JobKind {
        match self.work {
            Work::Reply(_) => JobKind::Reply,
            Work::LibraryBuild(_) => JobKind::LibraryBuild,
            Work::TaskRun(_) => JobKind::TaskRun,
        }
    }

    /// The conversation an agent's job comes from.
    pub fn conversation(&self) -> Option<&Conversation> {
        match &self.work {
            Work::Reply(conversation) => Some(conversation),
            Work::TaskRun(run) => Some(&run.conversation),
            Work::LibraryBuild(_) => None,
        }
    }
}

/// A message that asks an agent for something, in its thread.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub agent: AgentProfile,
    pub thread: ThreadRef,
    /// The number of the message that asks.
    pub trigger: u32,
    /// Who asked: what the agent does is theirs, written by the agent.
    pub requester: User,
    /// The thread up to the trigger, trimmed when long.
    pub context: Vec<ContextItem>,
}

/// A commit of the library to build.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryBuild {
    /// The build's number on the hub.
    pub build: i64,
    /// Where to fetch the library from, relative to the hub:
    /// `/git/geometry/library.git`, with the job's token.
    pub repository: String,
    /// The commit, in full.
    pub commit: String,
}

/// A task of the workshop, as its workshop machine gets it.
///
/// The machine runs the steps of the task's architecture: for
/// `formalize`, it works on a fresh worktree of the library at `main`
/// with the chosen agent (`work`); checks that the build passes and that
/// no definition's data newly depends on `sorry` (`check`); and commits,
/// with the task's provenance in trailers, and pushes to the task's
/// branch with the job's token, which pushes only there (`propose`). The
/// hub then opens a pull request, which people approve.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRun {
    /// Its number: the `7` of `w7`.
    pub task: u32,
    pub title: String,
    pub instructions: String,
    /// The architecture it runs with.
    pub architecture: ArchitectureRun,
    /// The workshop's agent, with the runner, model and arguments this
    /// machine is to use; the thread of the message that asked, up to it;
    /// and who asked.
    pub conversation: Conversation,
    /// Where to fetch the repository and push the branch, relative to the
    /// hub: `/git/geometry/library.git`, with the job's token.
    pub repository: String,
    /// The branch to push to: `workshop/w7`.
    pub branch: String,
    /// The definitions whose data already depends on `sorry`, in the
    /// library's state: only new ones fail the task.
    pub data_sorry: Vec<String>,
}

/// An architecture, as a workshop machine runs it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchitectureRun {
    pub name: String,
    /// Its steps, by name, in order: the machine refuses a task with a
    /// step it does not know.
    pub steps: Vec<String>,
    /// The instructions its agent is given, in Markdown, with `{agent}`,
    /// `{requester}`, `{group}`, `{task}` and `{report}` to fill in.
    pub instructions: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupRef {
    pub slug: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadRef {
    pub number: u32,
    pub title: String,
    pub kind: ThreadKind,
}

/// A message of the context, or the messages left out of it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextItem {
    Message {
        message: Box<Message>,
    },
    /// Messages `from` to `to`, both included, are not in the context.
    Gap {
        from: u32,
        to: u32,
    },
}
