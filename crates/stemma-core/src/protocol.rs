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

use crate::api::{AgentProfile, Message, ThreadKind, User};

/// The version of this protocol. A hub refuses workers that speak another.
pub const PROTOCOL_VERSION: u32 = 1;

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
    /// The runners it can run, such as `claude-code` or `api`.
    pub runners: Vec<String>,
    /// Whether it has Lean (elan), for library builds.
    pub lean: bool,
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
    Failed {
        error: String,
    },
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
    /// A group's worker: the group's agents.
    Group { slug: String, name: String },
    /// A member's worker: only the jobs they ask for.
    Member { user: User },
}

/// What a job is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Answer a message that summoned the agent, or a message in its chat.
    Reply,
}

/// A job, with what its runner needs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub id: i64,
    pub kind: JobKind,
    pub agent: AgentProfile,
    pub group: GroupRef,
    pub thread: ThreadRef,
    /// The number of the message to answer.
    pub trigger: u32,
    /// Who asked: the answer is theirs, written by the agent.
    pub requester: User,
    /// The thread up to the trigger, trimmed when long.
    pub context: Vec<ContextItem>,
    /// A token for the hub's API while the job lasts, scoped to its group;
    /// it reads, and does not post.
    pub token: String,
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
