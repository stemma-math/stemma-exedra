//! The worker protocol: what a worker and the hub say to each other.
//!
//! A worker connects outbound to the hub's WebSocket at
//! `/api/v1/worker`, with its token in an `Authorization: Bearer` header,
//! and sends [`WorkerMessage::Hello`]. The hub answers
//! [`HubMessage::Welcome`], or [`HubMessage::Refused`] and closes. Then the
//! hub sends jobs, and the worker sends their progress and their outcome.
//! Each message is one JSON text frame.
//!
//! When the connection drops, the worker stops what it was running and
//! the hub queues those jobs again, so they run from the start, on this
//! worker when it is back or on another.

use serde::{Deserialize, Serialize};

pub use crate::api::JobKind;
use crate::api::{AgentProfile, LibraryReport, Message, ThreadKind, User};

/// The version of this protocol. A hub refuses workers that speak another.
pub const PROTOCOL_VERSION: u32 = 5;

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
    /// How a job ended.
    Finished { job: i64, outcome: Outcome },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// The worker's version of Stemma.
    pub version: String,
    /// The runners it can run, such as `claude-code` or `codex`: those
    /// configured, or for a person's machine, those found installed and
    /// usable.
    pub runners: Vec<String>,
    /// Whether it has Lean (elan or lake), for library builds.
    pub lean: bool,
    /// Whether it works on the workshop's tasks, with Lean and the runners
    /// among its own that edit files.
    pub workshop: bool,
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

/// How a task's run ended.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub end: TaskEnd,
    /// Why it failed or is blocked, in a line.
    pub reason: Option<String>,
    /// The runner's report, in Markdown: what was done, what is pending.
    pub report: String,
    /// The commits pushed to `main`, in order; none if nothing changed.
    pub commits: Vec<String>,
    /// What the run did, trimmed to its end when long.
    pub log: String,
    /// The build and inspection of the last commit pushed, which is the
    /// library's new state.
    pub built: Option<Built>,
    /// The runner that worked on it, and its model, if known.
    pub runner: String,
    pub model: Option<String>,
}

/// How a task's run can end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEnd {
    Done,
    Failed,
    Blocked,
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
    /// The worker's name, as registered.
    pub name: String,
    pub owner: WorkerOwner,
}

/// Whose jobs a worker runs, as the hub knows from its token.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerOwner {
    /// A group's worker: the group's own agents, its builds and tasks.
    Group { slug: String, name: String },
    /// A person's machine: their own agents, only for them.
    Person { user: User },
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
    pub work: Work,
}

/// What a job is to do.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    /// Answer a message, with [`Outcome::Answer`].
    Reply(Conversation),
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

/// A task of the workshop, as its worker gets it.
///
/// The worker works on a fresh worktree of the library at `main`, with the
/// task's runner; checks that the build passes and that no definition's
/// data newly depends on `sorry`; commits, with the task's provenance in
/// trailers; and pushes to `main` with the job's token, which may push,
/// rebasing once if `main` moved meanwhile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRun {
    /// Its number: the `7` of `w7`.
    pub task: u32,
    pub title: String,
    pub instructions: String,
    /// The workshop's agent, with the runner this worker is to use; the
    /// thread of the message that asked, up to it; and who asked.
    pub conversation: Conversation,
    /// Where to fetch and push the library, relative to the hub:
    /// `/git/geometry/library.git`, with the job's token.
    pub repository: String,
    /// The definitions whose data already depends on `sorry`, in the
    /// library's state: only new ones fail the task.
    pub data_sorry: Vec<String>,
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
