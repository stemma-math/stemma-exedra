//! The JSON bodies of the hub's API, version 1 (`/api/v1`).
//!
//! Times are RFC 3339 strings, in UTC.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::address::{Address, AddressKind};

/// The prefix of every route of this version of the API.
pub const PREFIX: &str = "/api/v1";

/// An error, as every failed request returns it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorDetail {
    /// A stable code, such as `not_found` or `invalid`.
    pub code: String,
    /// What happened, for people.
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
    pub version: String,
}

// ── Accounts ──────────────────────────────────────────────────────────

/// A person with an account on the hub.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
}

/// The person signed in.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Me {
    #[serde(flatten)]
    pub user: User,
    /// Whether they administer the hub: create groups.
    pub hub_admin: bool,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignIn {
    pub username: String,
    pub password: String,
}

/// Whether the hub still needs its first account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetupState {
    pub needed: bool,
}

/// The first account, created with the setup token the hub printed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Setup {
    pub token: String,
    #[serde(flatten)]
    pub account: NewAccount,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewAccount {
    pub username: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateMe {
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangePassword {
    pub current: String,
    pub new: String,
}

/// A personal API token, for the command line and agents. Its secret is
/// shown once, when it is created.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: i64,
    pub name: String,
    /// The first characters of the secret, to recognise it.
    pub prefix: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateToken {
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedToken {
    #[serde(flatten)]
    pub token: ApiToken,
    pub secret: String,
}

// ── Groups ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "member" => Some(Role::Member),
            _ => None,
        }
    }
}

/// A group, as one of its members sees it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub slug: String,
    pub name: String,
    /// The role of the person asking.
    pub role: Role,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateGroup {
    pub slug: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    #[serde(flatten)]
    pub user: User,
    pub role: Role,
    #[serde(with = "time::serde::rfc3339")]
    pub joined_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateMember {
    pub role: Role,
}

/// A link that lets people join a group. Its secret is shown once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invitation {
    pub id: i64,
    pub role: Role,
    pub created_by: User,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    pub max_uses: Option<u32>,
    pub uses: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateInvitation {
    pub role: Option<Role>,
    /// Days until it expires; never, if absent.
    pub expires_in_days: Option<u32>,
    /// How many people may use it; any number, if absent.
    pub max_uses: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedInvitation {
    #[serde(flatten)]
    pub invitation: Invitation,
    /// The secret of the link: `/invite/<token>` on the hub.
    pub token: String,
}

/// What anyone with an invitation's link may know about it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvitationInfo {
    pub group_slug: String,
    pub group_name: String,
    pub role: Role,
}

/// Accepts an invitation: as the person signed in, or with a new account.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AcceptInvitation {
    pub account: Option<NewAccount>,
}

// ── The forum ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadKind {
    /// People talk; agents speak when summoned.
    Discussion,
    /// A resident agent answers every message.
    Chat,
}

impl ThreadKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadKind::Discussion => "discussion",
            ThreadKind::Chat => "chat",
        }
    }

    pub fn parse(s: &str) -> Option<ThreadKind> {
        match s {
            "discussion" => Some(ThreadKind::Discussion),
            "chat" => Some(ThreadKind::Chat),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Thread {
    /// Its number in the group: the `12` of `t12`.
    pub number: u32,
    pub title: String,
    pub kind: ThreadKind,
    /// The agent profile that answers every message of a chat.
    pub resident_agent: Option<String>,
    /// What the thread is about: a paper section or a declaration.
    pub anchor: Option<Address>,
    pub created_by: User,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub last_activity_at: OffsetDateTime,
    /// The number of the last message, deleted or not.
    pub last_message: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateThread {
    pub title: String,
    #[serde(default = "default_kind")]
    pub kind: ThreadKind,
    pub resident_agent: Option<String>,
    pub anchor: Option<Address>,
    /// The first message, if any.
    pub body: Option<String>,
    #[serde(default)]
    pub attachments: Vec<i64>,
}

fn default_kind() -> ThreadKind {
    ThreadKind::Discussion
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateThread {
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    /// Its number in the thread: the `45` of `t12#45`. Never reused.
    pub number: u32,
    pub author: User,
    /// The agent that wrote it for its author, if one did.
    pub agent: Option<String>,
    /// Markdown, with `$…$` math and `[[…]]` references. Empty once
    /// deleted.
    pub body: String,
    pub reply_to: Option<u32>,
    pub attachments: Vec<Attachment>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub edited_at: Option<OffsetDateTime>,
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PostMessage {
    pub body: String,
    pub reply_to: Option<u32>,
    #[serde(default)]
    pub attachments: Vec<i64>,
    /// The agent writing it for the person whose token posts it, such as
    /// `Claude Code`. Only with an API token; a job's token always posts
    /// as the job's agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditMessage {
    pub body: String,
}

/// A thread with its first page of messages.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThreadPage {
    pub thread: Thread,
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: i64,
    pub name: String,
    pub content_type: String,
    pub size: u64,
    /// Where to download it, relative to the hub.
    pub url: String,
}

// ── References ────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resolve {
    pub addresses: Vec<String>,
}

/// What an address names, as far as the hub knows.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resolved {
    /// The address as asked.
    pub address: String,
    /// The canonical address, if it is one.
    pub canonical: Option<String>,
    pub kind: Option<AddressKind>,
    /// Whether the object exists.
    pub exists: bool,
    /// Where it is in the web app.
    pub path: Option<String>,
    /// A title to show for it.
    pub title: Option<String>,
    /// Why it cannot be shown, when it cannot.
    pub note: Option<String>,
}

// ── Search ────────────────────────────────────────────────────────────

/// A message, or a thread's title, that matches a search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    /// The message (`t12#45`), or the thread when its title matched.
    pub address: Address,
    pub thread_title: String,
    pub author: Option<User>,
    pub agent: Option<String>,
    /// A piece of the text around what matched.
    pub snippet: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

// ── Agents ────────────────────────────────────────────────────────────

/// The runners a worker can run an agent with.
pub mod runners {
    /// Claude Code, headless, with the member's own login.
    pub const CLAUDE_CODE: &str = "claude-code";
    /// Codex, headless, with the member's own login.
    pub const CODEX: &str = "codex";
    /// Stemma's own harness, over an OpenAI-compatible chat API.
    pub const API: &str = "api";
    /// Every runner this version knows.
    pub const ALL: &[&str] = &[CLAUDE_CODE, CODEX, API];
}

/// Whose worker runs an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    /// The worker of whoever summons it, with their own login: `@claude`
    /// is each person's own Claude.
    Personal,
    /// The group's workers.
    Group,
}

impl AgentScope {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentScope::Personal => "personal",
            AgentScope::Group => "group",
        }
    }

    pub fn parse(s: &str) -> Option<AgentScope> {
        match s {
            "personal" => Some(AgentScope::Personal),
            "group" => Some(AgentScope::Group),
            _ => None,
        }
    }
}

/// An agent a group can summon by its handle: `@claude`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub handle: String,
    /// The name its messages are signed with: `Claude, for Ada`.
    pub display_name: String,
    /// One of [`runners::ALL`].
    pub runner: String,
    /// The model, if not the runner's default.
    #[serde(default)]
    pub model: Option<String>,
    /// For the `api` runner: which of the worker's endpoints to use.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// What to tell it beyond Stemma's own instructions.
    #[serde(default)]
    pub instructions: Option<String>,
    pub scope: AgentScope,
}

// ── Workers ───────────────────────────────────────────────────────────

/// Whose a worker is, which decides whose jobs it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerScope {
    /// A group's: it runs the group's agents.
    Group,
    /// A member's: it runs only the jobs that member asks for.
    Member,
}

/// A worker registered with the hub.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Worker {
    pub id: i64,
    pub name: String,
    pub scope: WorkerScope,
    /// The group of a group worker.
    pub group: Option<String>,
    /// The member of a member worker.
    pub owner: Option<User>,
    /// The first characters of its token, to recognise it.
    pub prefix: String,
    pub online: bool,
    /// What it announced when it last connected.
    pub runners: Vec<String>,
    pub lean: bool,
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_seen_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateWorker {
    pub name: String,
}

/// A new worker, with the secret of its token, shown once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedWorker {
    #[serde(flatten)]
    pub worker: Worker,
    pub secret: String,
}

// ── Jobs ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting for a worker that can run it.
    Queued,
    /// A worker is on it.
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<JobState> {
        match s {
            "queued" => Some(JobState::Queued),
            "running" => Some(JobState::Running),
            "done" => Some(JobState::Done),
            "failed" => Some(JobState::Failed),
            "cancelled" => Some(JobState::Cancelled),
            _ => None,
        }
    }

    /// Whether the job is over.
    pub fn is_final(self) -> bool {
        matches!(
            self,
            JobState::Done | JobState::Failed | JobState::Cancelled
        )
    }
}

/// An agent's answer on its way: what the hub asked a worker for.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub thread: u32,
    /// The message it answers.
    pub trigger: u32,
    /// The agent's handle and name.
    pub agent: String,
    pub agent_name: String,
    pub requester: User,
    pub state: JobState,
    /// What the agent is doing, while it runs.
    pub progress: Option<String>,
    /// Why it waits, while it waits.
    pub note: Option<String>,
    /// Why it failed, if it did.
    pub error: Option<String>,
    /// The message with the answer, once there is one.
    pub answer: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

// ── Live updates ──────────────────────────────────────────────────────

/// What the hub sends on a group's event stream.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ThreadCreated { thread: Thread },
    ThreadUpdated { thread: Thread },
    MessageCreated { thread: u32, message: Message },
    MessageUpdated { thread: u32, message: Message },
    JobUpdated { job: Job },
}
