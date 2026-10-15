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
    /// For a declaration, its state in the library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<DeclarationState>,
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

// ── Working papers ────────────────────────────────────────────────────

/// A working paper: the readable account of a line of work.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paper {
    /// The `compactification` of `p/compactification`.
    pub slug: String,
    pub title: String,
    /// Its current revision, from 1.
    pub revision: u32,
    pub created_by: User,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Who wrote the current revision, and the agent that wrote it for them.
    pub updated_by: User,
    pub updated_agent: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// The revision the person asking read last; none if never.
    pub last_read: Option<u32>,
}

/// A revision of a paper: who wrote it, and why.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Revision {
    pub number: u32,
    pub title: String,
    pub author: User,
    /// The agent that wrote it for its author, if one did.
    pub agent: Option<String>,
    pub reason: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// A revision with its text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevisionText {
    #[serde(flatten)]
    pub revision: Revision,
    pub body: String,
}

/// A section of a paper: a heading, addressable as `p/slug#section`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SectionInfo {
    pub slug: String,
    pub title: String,
    pub level: u8,
}

/// A paper as someone opens it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperPage {
    pub paper: Paper,
    /// Markdown, with `$…$` math, `[[…]]` references and callouts.
    pub body: String,
    pub sections: Vec<SectionInfo>,
    /// The revisions after the one the person read last, newest first:
    /// empty if they are up to date, or never read it.
    pub since_read: Vec<Revision>,
    /// The text as they read it last, if it changed since.
    pub read_body: Option<String>,
    /// The threads anchored to the paper or one of its sections.
    pub threads: Vec<Thread>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatePaper {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Why; "Started the paper" if absent.
    pub reason: Option<String>,
    /// The agent writing it for the person whose token asks, as for
    /// messages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// A new revision of a paper, written over `base_revision`: if the paper
/// has moved past it, the edit is refused, and should be made again over
/// the newer text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditPaper {
    /// A new title, if it changes.
    pub title: Option<String>,
    pub body: String,
    pub reason: String,
    pub base_revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MarkRead {
    pub revision: u32,
}

// ── The library ───────────────────────────────────────────────────────

/// Where a declaration is in its module's file, from line 1 and column 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// A declaration of the library, as the inspection reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Declaration {
    pub name: String,
    pub module: String,
    /// `theorem`, `definition`, `instance`, `axiom`, `opaque`,
    /// `inductive`, `structure`, `class`, `constructor`…
    pub kind: String,
    /// Its type, pretty-printed.
    #[serde(rename = "type")]
    pub type_: String,
    pub doc: Option<String>,
    pub range: Option<SourceRange>,
    /// Whether it depends on `sorry`, itself or through what it uses.
    pub sorry: bool,
    /// For a definition, whether its data (not only its proofs) does.
    #[serde(default)]
    pub data_sorry: bool,
    /// The axioms it depends on, `sorryAx` included.
    pub axioms: Vec<String>,
    /// The declarations of the library it uses.
    pub uses: Vec<String>,
}

/// What the inspection of a commit of the library reports.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LibraryReport {
    /// Lean's version, as it ran.
    pub lean: String,
    /// The library's modules.
    pub modules: Vec<String>,
    pub declarations: Vec<Declaration>,
}

/// The state of a declaration, as shown where it is referenced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationState {
    /// Checked, without `sorry`: proved, for a theorem; defined, otherwise.
    Proved,
    /// It depends on `sorry`.
    Sorry,
    /// The library has no such declaration.
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildState {
    Queued,
    Running,
    Succeeded,
    Failed,
    /// A newer commit came before it ran.
    Cancelled,
}

impl BuildState {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildState::Queued => "queued",
            BuildState::Running => "running",
            BuildState::Succeeded => "succeeded",
            BuildState::Failed => "failed",
            BuildState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<BuildState> {
        match s {
            "queued" => Some(BuildState::Queued),
            "running" => Some(BuildState::Running),
            "succeeded" => Some(BuildState::Succeeded),
            "failed" => Some(BuildState::Failed),
            "cancelled" => Some(BuildState::Cancelled),
            _ => None,
        }
    }
}

/// A build of a commit of the library, with its inspection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Build {
    pub id: i64,
    /// The commit, in full.
    pub commit: String,
    pub state: BuildState,
    /// What it is doing, while it runs; why it waits, while it waits.
    pub progress: Option<String>,
    /// Why it failed, in a line.
    pub error: Option<String>,
    /// Lean's version, as it ran.
    pub lean: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

/// A build with what it wrote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildLog {
    #[serde(flatten)]
    pub build: Build,
    pub log: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub name: String,
    pub declarations: u32,
    pub sorry: u32,
}

/// The library as it stands: its last successful build.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryState {
    /// Whether the hub keeps a library for the group: it needs `git`.
    pub available: bool,
    /// Where to clone it from, relative to the hub: `/git/geometry.git`.
    pub clone_path: String,
    /// The commit `main` is at.
    pub head: Option<String>,
    /// The newest build, whatever its state.
    pub latest: Option<Build>,
    /// The build whose report is the library's state.
    pub current: Option<Build>,
    pub modules: Vec<ModuleSummary>,
    pub declarations: u32,
    /// How many depend on `sorry`.
    pub sorry: u32,
}

/// A declaration in a list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeclarationSummary {
    pub name: String,
    pub module: String,
    pub kind: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub state: DeclarationState,
    pub doc: Option<String>,
}

/// Where something is referenced: a message or a paper.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferenceSite {
    pub address: Address,
    pub title: String,
}

/// A declaration, with what uses it and where it is referenced.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeclarationPage {
    #[serde(flatten)]
    pub declaration: Declaration,
    pub state: DeclarationState,
    /// The commit of the report it comes from.
    pub commit: String,
    /// The declarations of the library that use it.
    pub used_by: Vec<String>,
    /// The messages and papers that reference it.
    pub references: Vec<ReferenceSite>,
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
    /// OpenCode, headless, with a model of any provider it supports.
    pub const OPENCODE: &str = "opencode";
    /// Every runner this version knows.
    pub const ALL: &[&str] = &[CLAUDE_CODE, CODEX, API, OPENCODE];
    /// The runners that can work on the library for the workshop: those
    /// that edit files and run commands.
    pub const WORKSHOP: &[&str] = &[OPENCODE, CLAUDE_CODE, CODEX];
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
    /// What it does when summoned.
    #[serde(default)]
    pub role: AgentRole,
}

/// What an agent does when summoned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// It answers, with a message.
    #[default]
    Answer,
    /// It updates the working paper the message names, from the threads.
    Editor,
    /// It hands what the message asks to the workshop, as a task.
    Workshop,
}

impl AgentRole {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentRole::Answer => "answer",
            AgentRole::Editor => "editor",
            AgentRole::Workshop => "workshop",
        }
    }

    pub fn parse(s: &str) -> Option<AgentRole> {
        match s {
            "answer" => Some(AgentRole::Answer),
            "editor" => Some(AgentRole::Editor),
            "workshop" => Some(AgentRole::Workshop),
            _ => None,
        }
    }
}

// ── The workshop ──────────────────────────────────────────────────────

/// Where a task of the workshop is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting for its turn, a worker, or the night.
    Open,
    /// A worker is on it.
    Running,
    /// Done: what it did is in the library, if it did anything.
    Done,
    /// It could not be done: the build did not pass, or the runner failed.
    Failed,
    /// It was done, but could not be put in the library: `main` moved
    /// where it worked, or kept moving.
    Blocked,
    /// Someone cancelled it.
    Cancelled,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Open => "open",
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::Blocked => "blocked",
            TaskState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<TaskState> {
        match s {
            "open" => Some(TaskState::Open),
            "running" => Some(TaskState::Running),
            "done" => Some(TaskState::Done),
            "failed" => Some(TaskState::Failed),
            "blocked" => Some(TaskState::Blocked),
            "cancelled" => Some(TaskState::Cancelled),
            _ => None,
        }
    }

    /// Whether the task is over.
    pub fn is_final(self) -> bool {
        !matches!(self, TaskState::Open | TaskState::Running)
    }
}

/// When a task runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSchedule {
    /// As soon as a worker is free.
    #[default]
    Now,
    /// In the group's night window.
    Night,
}

impl TaskSchedule {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskSchedule::Now => "now",
            TaskSchedule::Night => "night",
        }
    }

    pub fn parse(s: &str) -> Option<TaskSchedule> {
        match s {
            "now" => Some(TaskSchedule::Now),
            "night" => Some(TaskSchedule::Night),
            _ => None,
        }
    }
}

/// A task of the workshop: work on the library, asked for in a
/// conversation, done in the background.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    /// Its number in the group: the `7` of `w7`.
    pub number: u32,
    pub title: String,
    /// What to do, as asked.
    pub instructions: String,
    /// The message that asked for it: `t12#48`. The report goes to its
    /// thread.
    pub origin: Address,
    /// Who asked: the work is theirs, done by the workshop.
    pub requester: User,
    /// The agent that handed it to the workshop for them, if one did.
    pub agent: Option<String>,
    pub state: TaskState,
    pub schedule: TaskSchedule,
    /// The runner and model that work on it, once one does.
    pub runner: Option<String>,
    pub model: Option<String>,
    /// What it is doing, while it runs; why it waits, while it is open.
    pub progress: Option<String>,
    /// Why it failed or is blocked.
    pub reason: Option<String>,
    /// The commits it put in the library, in order.
    pub commits: Vec<String>,
    /// The message in the origin's thread that reports on it.
    pub report_message: Option<Address>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

/// A task with what its run wrote and reported.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskPage {
    #[serde(flatten)]
    pub task: Task,
    /// The run's report, in Markdown: what was done, what is pending.
    pub report: Option<String>,
    /// What the run did, step by step, trimmed to its end when long.
    pub log: String,
}

/// Hands a task to the workshop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateTask {
    pub title: String,
    pub instructions: String,
    /// The message that asks for it: `t12#48`. With a job's token, the
    /// message the job answers if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default)]
    pub schedule: TaskSchedule,
    /// The agent handing it over for the person whose token asks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// How a group's workshop works.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkshopSettings {
    /// The night window, in the group's time zone: `01:00` to `07:00`.
    pub night_from: String,
    pub night_to: String,
    /// An IANA time zone, such as `Europe/Madrid`.
    pub time_zone: String,
    /// Whether it is night now.
    pub night_now: bool,
    /// Whether the person asking lends their own worker to the workshop
    /// during the night window.
    pub lend: bool,
    /// The members who lend theirs.
    pub lenders: Vec<User>,
}

/// Changes to the workshop's settings: the window and time zone, by an
/// admin; whether one lends their worker, by anyone, for themselves.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateWorkshop {
    pub night_from: Option<String>,
    pub night_to: Option<String>,
    pub time_zone: Option<String>,
    pub lend: Option<bool>,
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
    /// Whether it works on the workshop's tasks.
    #[serde(default)]
    pub workshop: bool,
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

/// What a job is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Answer a message that summoned the agent, or a message in its chat.
    Reply,
    /// Update a working paper, as a message asked an editor.
    PaperUpdate,
    /// Build a commit of the library, and inspect it.
    LibraryBuild,
    /// Work on a task of the workshop.
    TaskRun,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Reply => "reply",
            JobKind::PaperUpdate => "paper_update",
            JobKind::LibraryBuild => "library_build",
            JobKind::TaskRun => "task_run",
        }
    }

    pub fn parse(s: &str) -> Option<JobKind> {
        match s {
            "reply" => Some(JobKind::Reply),
            "paper_update" => Some(JobKind::PaperUpdate),
            "library_build" => Some(JobKind::LibraryBuild),
            "task_run" => Some(JobKind::TaskRun),
            _ => None,
        }
    }
}

/// Work on its way: what the hub asked a worker for.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub kind: JobKind,
    /// The thread of the message that asked for it, if one did.
    pub thread: Option<u32>,
    /// The message that asked for it.
    pub trigger: Option<u32>,
    /// For a paper's update, the paper's slug.
    pub paper: Option<String>,
    /// For a task's run, the task's number: the `7` of `w7`.
    #[serde(default)]
    pub task: Option<u32>,
    /// The agent's handle and name; for a build, `lean` and `Lean`.
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
    ThreadCreated {
        thread: Thread,
    },
    ThreadUpdated {
        thread: Thread,
    },
    MessageCreated {
        thread: u32,
        message: Message,
    },
    MessageUpdated {
        thread: u32,
        message: Message,
    },
    JobUpdated {
        job: Job,
    },
    /// A paper has a new revision, or is new.
    PaperUpdated {
        paper: Paper,
    },
    /// A build of the library started, ended or waits.
    BuildUpdated {
        build: Build,
    },
    /// A task of the workshop is new, or changed.
    TaskUpdated {
        task: Task,
    },
}
