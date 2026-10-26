//! The personal-token worker WebSocket protocol. Connections and capabilities are live state only.

use serde::{Deserialize, Serialize};

pub use crate::api::JobKind;
use crate::api::{AgentProfile, Message, ThreadKind, User};

/// The version of this protocol. A hub refuses workers that speak another.
pub const PROTOCOL_VERSION: u32 = 14;

/// The path of the workers' WebSocket, under the API's prefix.
pub const WORKER_PATH: &str = "/worker";

/// From a worker to the hub.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    /// The first message: what the worker can do.
    Hello(Hello),
    ProcessDecision {
        process: i64,
        accept: bool,
        pending: bool,
    },
    ProcessHeartbeat {
        lease: LeaseRef,
    },
    ProcessFinished {
        lease: LeaseRef,
        result: Box<ProcessResult>,
    },
    BuildHeartbeat {
        lease: LeaseRef,
    },
    BuildFinished {
        lease: LeaseRef,
        result: Box<BuildResult>,
    },
    /// What a job is doing now, for people watching: "Reading t12#3-5".
    Progress {
        job: i64,
        text: String,
    },
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
    pub version: String,
    pub runners: Vec<String>,
    pub concurrency: u32,
    pub agents: GroupAgents,
    pub workshop: GroupWorkshop,
    pub workshop_groups: std::collections::BTreeMap<String, GroupWorkshop>,
    /// Only groups whose effective capabilities differ from general settings.
    pub groups: std::collections::BTreeMap<String, GroupAgents>,
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

/// From the hub to a worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HubMessage {
    Welcome(Welcome),
    Identity(crate::api::Me),
    /// The hub will not work with this worker; the connection closes.
    Refused {
        reason: String,
    },
    Job(Box<JobSpec>),
    Build(Box<BuildSpec>),
    ProcessOffer(Box<ProcessOffer>),
    Process(Box<ProcessSpec>),
    ProcessTaken {
        process: i64,
    },
    CancelProcess {
        lease: LeaseRef,
    },
    CancelBuild {
        lease: LeaseRef,
    },
    /// Stop a job: nobody wants its outcome any more.
    Cancel {
        job: i64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Welcome {
    pub groups: Vec<crate::api::Group>,
    pub protocol: u32,
    /// The machine's name, as registered.
    pub name: String,
    /// Whose it is.
    pub owner: User,
}

/// A job, with what its runner needs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    #[serde(default)]
    pub process: bool,
    pub id: i64,
    pub group: GroupRef,
    /// A token for the hub's API while the job lasts, scoped to its group;
    /// it reads, and does not post.
    pub token: String,
    pub work: Work,
}

/// What a job is to do.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    /// Answer a message, with [`Outcome::Answer`].
    Reply(Box<Conversation>),
}

impl JobSpec {
    pub fn kind(&self) -> JobKind {
        match self.work {
            Work::Reply(_) => JobKind::Reply,
        }
    }

    /// The conversation an agent's job comes from.
    pub fn conversation(&self) -> Option<&Conversation> {
        match &self.work {
            Work::Reply(conversation) => Some(conversation),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupRef {
    pub id: i64,
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

/// Whether a computer runs agents in a group, and its scheduling priority there.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupAgents {
    pub attachments: crate::api::AttachmentRule,
    pub enabled: bool,
    pub priority: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runners: Option<Vec<String>>,
}

/// Effective workshop capabilities; live only while this connection lasts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupWorkshop {
    pub enabled: bool,
    pub build: bool,
    pub build_concurrency: u32,
    pub concurrency: u32,
    /// Offered runner handles and their configured models, empty means default.
    #[serde(default)]
    pub runners: std::collections::BTreeMap<String, String>,
    /// Whether requests from other members reach this machine.
    #[serde(default)]
    pub others: Others,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Others {
    #[default]
    Ask,
    Never,
}

/// A lease generation. A retry can never report for a previous holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRef {
    pub id: i64,
    pub attempt: u32,
}

/// One immutable repository commit to build, in an isolated workshop container.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildSpec {
    pub lease: LeaseRef,
    pub group: GroupRef,
    /// A hub-relative git path. The build token fetches only this repository.
    pub repository: String,
    pub commit: String,
    pub kind: BuildKind,
    /// Fetches and reports this build only, until its lease ends.
    pub token: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildKind {
    Library,
    Paper,
}

/// The log is trimmed and secrets hidden by the hub before storage.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BuildResult {
    pub succeeded: bool,
    pub error: Option<String>,
    pub log: String,
    /// Required on a passing library build; papers upload a PDF first.
    pub report: Option<crate::api::LibraryReport>,
}

/// An offer contains no execution credential. A machine decides locally before
/// accepting; accepting this offer does not promise that it won the lease.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessOffer {
    /// Durable process number, unique on this hub.
    pub process: i64,
    /// Stable group identity and its current name.
    pub group: GroupRef,
    /// The request's numbered thread, title and requester.
    pub request: ThreadRef,
    pub requester: User,
    /// The committed recipe at main: blob version, recipes commit, front matter
    /// and source prompt. Approvals must match the blob version exactly.
    pub recipe: crate::workshop::RecipeFile,
    /// Validated inputs, also rendered into prompt without recursive expansion.
    pub inputs: std::collections::BTreeMap<String, String>,
    pub prompt: String,
    /// Recipe runner handle and optional exact model requested.
    pub runner: String,
    pub model: Option<String>,
    /// Who chose this machine assignment, independent of who owns the machine.
    pub assigned_by: User,
    /// Stable request identity for a series approval, scoped to this group.
    pub recurring: bool,
    /// No acceptance is valid after this time, in Unix milliseconds.
    pub expires_at: i64,
}
/// Execution contract supplied only to the first machine that accepts an offer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessSpec {
    /// Generation fencing heartbeat, result and token access.
    pub lease: LeaseRef,
    pub offer: ProcessOffer,
    /// The winning machine's owner; all process messages are attributed to them.
    pub owner: User,
    /// Fetches the group; posts and attaches only in the request's thread;
    /// pushes only the listed branches while this lease is alive.
    pub token: String,
    /// Every writable repository, immutable base and exclusive push branch.
    /// A read-only recipe receives an empty list.
    pub repositories: Vec<ProcessRepository>,
    /// Hard end of this run, in Unix milliseconds; heartbeat never extends it.
    pub deadline: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessRepository {
    /// Hub-relative git path, never an upstream credential.
    pub path: String,
    /// Immutable commit of main at launch.
    pub base: String,
    /// Exactly workshop/<request number>-<process number>.
    pub branch: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProcessResult {
    /// Markdown summary posted by the hub, with secrets hidden.
    pub summary: String,
    pub error: Option<String>,
    /// What was pushed; the hub verifies these against Forgejo before opening PRs.
    pub pushed: Vec<crate::workshop::ProcessBranch>,
    pub usage: Usage,
}
