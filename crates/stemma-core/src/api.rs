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
    /// When it stops working by itself, if it does.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateToken {
    pub name: String,
    /// How long it lasts, in minutes; until revoked if absent. The
    /// launcher's tokens last a session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_minutes: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedToken {
    #[serde(flatten)]
    pub token: ApiToken,
    pub secret: String,
}

// ── Groups ────────────────────────────────────────────────────────────

/// What someone may do in a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Manages the group (its settings, members, invitations and workshop
    /// machines), also approves big changes, and does everything a member
    /// does.
    Admin,
    /// Reads, converses, summons agents and asks the workshop for work.
    Member,
    /// Reads and converses, summoning their own agents, but asks the
    /// workshop for no work and changes nothing of the group's.
    Guest,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Guest => "guest",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "admin" => Some(Role::Admin),
            "member" => Some(Role::Member),
            "guest" => Some(Role::Guest),
            _ => None,
        }
    }

    /// Whether it manages the group.
    pub fn is_admin(self) -> bool {
        self == Role::Admin
    }

    /// Whether it may ask the workshop for work: create tasks, directly or
    /// through an agent that hands one over.
    pub fn can_ask_for_work(self) -> bool {
        matches!(self, Role::Admin | Role::Member)
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
    /// The model it answers with, if chosen when the chat started.
    #[serde(default)]
    pub resident_model: Option<String>,
    /// How hard it thinks, if chosen when the chat started.
    #[serde(default)]
    pub resident_effort: Option<Effort>,
    /// What the thread is about: a declaration of the library.
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
    /// For a chat, the model its resident agent answers with, instead of
    /// the one in each writer's profile.
    #[serde(default)]
    pub model: Option<String>,
    /// For a chat, how hard its resident agent thinks.
    #[serde(default)]
    pub effort: Option<Effort>,
    pub anchor: Option<Address>,
    /// The first message, if any.
    pub body: Option<String>,
    #[serde(default)]
    pub attachments: Vec<i64>,
}

/// How hard an agent thinks before it answers, as its runner takes it:
/// Claude Code's `--effort`, Codex's `model_reasoning_effort`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Effort> {
        match s {
            "low" => Some(Effort::Low),
            "medium" => Some(Effort::Medium),
            "high" => Some(Effort::High),
            _ => None,
        }
    }

    /// The arguments that ask a runner for this effort, or none if the
    /// runner takes no effort from its command line.
    pub fn args(self, runner: &str) -> Option<Vec<String>> {
        match runner {
            runners::CLAUDE_CODE => Some(vec!["--effort".into(), self.as_str().into()]),
            runners::CODEX => Some(vec![
                "-c".into(),
                format!("model_reasoning_effort={}", self.as_str()),
            ]),
            _ => None,
        }
    }
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
    /// The kind of the thread it is in.
    pub thread_kind: ThreadKind,
    pub author: Option<User>,
    pub agent: Option<String>,
    /// A piece of the text around what matched.
    pub snippet: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
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
    /// Whether the group has a library: the hub needs a forge to keep it.
    pub available: bool,
    /// Where to clone it from with a personal token, relative to the hub:
    /// `/git/geometry/library.git`.
    pub clone_path: String,
    /// Where it is in the forge, for people signed in there.
    pub forge: Option<ForgeLinks>,
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

/// Where a group's library is in the forge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForgeLinks {
    /// The group's organization.
    pub organization: String,
    /// The library's repository.
    pub repository: String,
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

/// Where something is referenced: a message; or a thread about it.
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
    /// The threads about it, and the messages that reference it.
    pub references: Vec<ReferenceSite>,
}

// ── Working papers ────────────────────────────────────────────────────

/// The file a paper's build compiles, at the root of its repository.
pub const PAPER_MAIN: &str = "main.tex";

/// A build of a commit of a paper: compiled into a PDF with `latexmk`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperBuild {
    pub id: i64,
    /// The commit, in full.
    pub commit: String,
    pub state: BuildState,
    /// What it is doing, while it runs; why it waits, while it waits.
    pub progress: Option<String>,
    /// Why it failed, in a line.
    pub error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
}

/// A paper's PDF: that of its last build that made one.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperPdf {
    /// The build that made it.
    pub build: i64,
    pub commit: String,
    /// In bytes.
    pub size: u64,
    /// Where to read it, relative to the hub.
    pub url: String,
    #[serde(with = "time::serde::rfc3339")]
    pub built_at: OffsetDateTime,
}

/// A working paper: a LaTeX repository of the group, its authors, and its
/// PDF, compiled by the workshop after each push to its `main`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paper {
    /// Its address is `p/<slug>`.
    pub slug: String,
    pub title: String,
    /// Its repository in the group's organization: `paper-<slug>`.
    pub repository: String,
    /// Where to clone it from with a personal token, relative to the hub.
    pub clone_path: String,
    /// Where it is in the forge, for people signed in there.
    pub forge_url: Option<String>,
    /// Who writes it: its creator first.
    pub authors: Vec<User>,
    pub created_by: User,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// The newest build, whatever its state.
    pub latest: Option<PaperBuild>,
    /// The last PDF made, which a failed build leaves as it is.
    pub pdf: Option<PaperPdf>,
}

/// A paper, with what its newest build wrote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperPage {
    #[serde(flatten)]
    pub paper: Paper,
    /// The newest build's log, trimmed to its end when long.
    pub log: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatePaper {
    /// Lowercase letters, digits and inner dashes.
    pub slug: String,
    pub title: String,
}

/// A file of a paper's source, at its `main`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaperFile {
    pub path: String,
    pub size: u64,
}

/// A paper's source files, at the commit its `main` is at.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperSource {
    pub commit: String,
    pub files: Vec<PaperFile>,
}

/// One file of a paper's source, as text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaperFileText {
    pub path: String,
    pub commit: String,
    pub text: String,
}

/// Asks the workshop to edit a paper: a thread about it is opened with the
/// request, and the task is handed over from its first message.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditPaper {
    /// What to do, in a line; the thread's and the task's title.
    pub title: String,
    pub instructions: String,
}

// ── Agents ────────────────────────────────────────────────────────────

/// The runners an agent runs with: the harnesses Stemma launches.
pub mod runners {
    /// Claude Code, with the person's own sign-in.
    pub const CLAUDE_CODE: &str = "claude-code";
    /// Codex, with the person's own sign-in.
    pub const CODEX: &str = "codex";
    /// OpenCode, with a model of any provider it supports.
    pub const OPENCODE: &str = "opencode";
    /// Every runner this version knows.
    pub const ALL: &[&str] = &[CLAUDE_CODE, CODEX, OPENCODE];
    /// The runners that can work on the library for the workshop: those
    /// that edit files and run commands.
    pub const WORKSHOP: &[&str] = &[OPENCODE, CLAUDE_CODE, CODEX];

    /// How a runner is called in what people read.
    pub fn name(runner: &str) -> &str {
        match runner {
            CLAUDE_CODE => "Claude Code",
            CODEX => "Codex",
            OPENCODE => "OpenCode",
            other => other,
        }
    }

    /// The program a runner runs, unless told otherwise.
    pub fn program(runner: &str) -> Option<&'static str> {
        match runner {
            CLAUDE_CODE => Some("claude"),
            CODEX => Some("codex"),
            OPENCODE => Some("opencode"),
            _ => None,
        }
    }
}

/// How a machine isolates the workshop's jobs: each in a container of its
/// own, with Docker or Podman, or, if its owner chose so explicitly, on
/// the machine itself, unisolated. A machine that can do neither runs no
/// job of the workshop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
    Docker,
    Podman,
    /// On the machine itself, as its user: its owner's explicit choice.
    Host,
}

impl Isolation {
    /// Whether the jobs run in containers.
    pub fn contained(self) -> bool {
        !matches!(self, Isolation::Host)
    }

    /// As people read it.
    pub fn describe(self) -> &'static str {
        match self {
            Isolation::Docker => "in containers, with Docker",
            Isolation::Podman => "in containers, with Podman",
            Isolation::Host => "on the machine itself, unisolated",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Isolation::Docker => "docker",
            Isolation::Podman => "podman",
            Isolation::Host => "host",
        }
    }

    pub fn parse(text: &str) -> Option<Isolation> {
        match text {
            "docker" => Some(Isolation::Docker),
            "podman" => Some(Isolation::Podman),
            "host" => Some(Isolation::Host),
            _ => None,
        }
    }
}

/// The handle of the group's workshop, which every group has: a message
/// that mentions it hands the workshop a task. Nobody's own agent has it.
pub const WORKSHOP: &str = "workshop";

/// An agent: who it is and how it runs. A person's own agents answer for
/// them; the workshop's run on the group's workshop machines.
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
    /// More arguments for the runner.
    #[serde(default)]
    pub args: Vec<String>,
}

/// One of a person's own agents, which answer for them in every group
/// they are in, run on their machines with their own sign-in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAgent {
    pub handle: String,
    /// The name its messages are signed with: `Claude Sonnet, for Ada`.
    pub display_name: String,
    /// One of [`runners::ALL`].
    pub runner: String,
    /// The model, if not the runner's default.
    #[serde(default)]
    pub model: Option<String>,
    /// More arguments for the runner, as given on its command line.
    #[serde(default)]
    pub args: Vec<String>,
    /// Whether it is one every person has, one per runner: `@claude`,
    /// `@codex`, `@opencode`. Those are changed, not removed; removing
    /// one puts it back as it was.
    #[serde(default)]
    pub default: bool,
}

impl PersonalAgent {
    /// The agents everyone has: one per runner.
    pub fn defaults() -> Vec<PersonalAgent> {
        [
            ("claude", "Claude", runners::CLAUDE_CODE),
            ("codex", "Codex", runners::CODEX),
            ("opencode", "OpenCode", runners::OPENCODE),
        ]
        .into_iter()
        .map(|(handle, name, runner)| PersonalAgent {
            handle: handle.into(),
            display_name: name.into(),
            runner: runner.into(),
            model: None,
            args: Vec::new(),
            default: true,
        })
        .collect()
    }

    /// The default agent with `handle`, if there is one.
    pub fn default_for(handle: &str) -> Option<PersonalAgent> {
        Self::defaults().into_iter().find(|a| a.handle == handle)
    }

    /// The default agent of `runner`.
    pub fn default_of_runner(runner: &str) -> Option<PersonalAgent> {
        Self::defaults().into_iter().find(|a| a.runner == runner)
    }

    pub fn profile(&self) -> AgentProfile {
        AgentProfile {
            handle: self.handle.clone(),
            display_name: self.display_name.clone(),
            runner: self.runner.clone(),
            model: self.model.clone(),
            args: self.args.clone(),
        }
    }
}

// ── The workshop ──────────────────────────────────────────────────────

/// Where a task of the workshop is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting for its turn, or a workshop machine.
    Open,
    /// A workshop machine is on it.
    Running,
    /// Its changes wait for approval, in a pull request.
    Review,
    /// Done: its changes were approved and merged, or it changed nothing.
    Done,
    /// It could not be done: the check did not pass, or the agent failed.
    Failed,
    /// Its changes were rejected.
    Rejected,
    /// Someone cancelled it.
    Cancelled,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Open => "open",
            TaskState::Running => "running",
            TaskState::Review => "review",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::Rejected => "rejected",
            TaskState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<TaskState> {
        match s {
            "open" => Some(TaskState::Open),
            "running" => Some(TaskState::Running),
            "review" => Some(TaskState::Review),
            "done" => Some(TaskState::Done),
            "failed" => Some(TaskState::Failed),
            "rejected" => Some(TaskState::Rejected),
            "cancelled" => Some(TaskState::Cancelled),
            _ => None,
        }
    }

    /// Whether the task is over.
    pub fn is_final(self) -> bool {
        !matches!(
            self,
            TaskState::Open | TaskState::Running | TaskState::Review
        )
    }
}

/// Whose approval a change needs: the rules, each a kind of approver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approver {
    /// The person who asked for the task.
    Asker,
    /// One of the group's admins: for big changes.
    Admin,
    /// One of the authors of the paper it changes, when the asker is not.
    Author,
}

impl Approver {
    pub fn as_str(self) -> &'static str {
        match self {
            Approver::Asker => "asker",
            Approver::Admin => "admin",
            Approver::Author => "author",
        }
    }

    pub fn parse(s: &str) -> Option<Approver> {
        match s {
            "asker" => Some(Approver::Asker),
            "admin" => Some(Approver::Admin),
            "author" => Some(Approver::Author),
            _ => None,
        }
    }
}

/// An approval a change was given.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub user: User,
    /// What it approves as.
    pub approver: Approver,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
}

/// The changes a task proposes: a branch of a repository, in a pull
/// request of the forge, which approval merges.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Changes {
    /// The repository: `library`, `paper-notes`.
    pub repository: String,
    /// The branch: `workshop/w12`.
    pub branch: String,
    /// The commit the branch is at.
    pub commit: String,
    /// The pull request's number in the forge.
    pub pull: u32,
    /// Where people see its changes, in the forge.
    pub url: String,
    /// Whether it is big, and so also needs an admin's approval.
    pub big: bool,
    /// Why it is big: "deletes Geometry/Old.lean".
    #[serde(default)]
    pub why_big: Vec<String>,
    /// The approvals it still needs, while it waits for any.
    #[serde(default)]
    pub needs: Vec<Approver>,
    #[serde(default)]
    pub approvals: Vec<Approval>,
    /// Who rejected it, if anyone did.
    #[serde(default)]
    pub rejected_by: Option<User>,
    /// The commit that merged it into `main`, once merged.
    #[serde(default)]
    pub merged: Option<String>,
}

/// A task of the workshop: work on a repository of the group, asked for
/// in a conversation, done in the background, and put in only once
/// approved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    /// Its number in the group: the `7` of `w7`.
    pub number: u32,
    pub title: String,
    /// What to do, as asked.
    pub instructions: String,
    /// The architecture it runs with: `formalize`, `write`.
    pub architecture: String,
    /// The paper it changes, by slug, if it changes one.
    #[serde(default)]
    pub paper: Option<String>,
    /// The message that asked for it: `t12#48`. The report goes to its
    /// thread.
    pub origin: Address,
    /// Who asked: the work is theirs, done by the workshop.
    pub requester: User,
    /// The agent that handed it to the workshop for them, if one did.
    pub agent: Option<String>,
    pub state: TaskState,
    /// The runner and model that work on it, once one does.
    pub runner: Option<String>,
    pub model: Option<String>,
    /// What it is doing, while it runs; why it waits, while it is open.
    pub progress: Option<String>,
    /// Why it failed.
    pub reason: Option<String>,
    /// What it proposes, once it does.
    pub changes: Option<Changes>,
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
    /// The agent handing it over for the person whose token asks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The architecture to run it with, if absent: `write` for a paper,
    /// `formalize` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// The paper it is to change, by slug (`notes`, or `p/notes`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<String>,
}

/// A kind of work the workshop knows: a folder of the repository, with a
/// manifest and the instructions its agent is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Architecture {
    /// Its name: `formalize`.
    pub name: String,
    /// What it does, in a line.
    pub description: String,
    /// What of the group it changes: `library`, or `paper`, the paper the
    /// task names.
    pub works_on: String,
    /// What it does, in order.
    pub steps: Vec<ArchitectureStep>,
    /// The files whose change makes a change big, besides deleting or
    /// renaming any file.
    pub big: Vec<String>,
    /// The files of its instructions, in order.
    pub instructions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureStep {
    /// The step, as workshop machines know it: `work`, `check`, `propose`.
    pub name: String,
    pub description: String,
}

/// One of the agents a workshop machine offers the workshop, in the order
/// its owner prefers them, with how much it may do in a day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkshopAgent {
    /// Its id, once kept; absent for a new one.
    #[serde(default)]
    pub id: Option<i64>,
    /// One of [`runners::WORKSHOP`].
    pub runner: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// How many jobs it may run in a day (UTC); no limit if absent.
    #[serde(default)]
    pub daily_jobs: Option<u32>,
    /// How many minutes it may work in a day (UTC); no limit if absent.
    #[serde(default)]
    pub daily_minutes: Option<u32>,
    /// What it did today, as the hub counts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub today: Option<AgentToday>,
}

/// How much a workshop agent did today.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToday {
    pub jobs: u32,
    pub minutes: u32,
}

/// A machine working for a group's workshop.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkshopMachine {
    pub id: i64,
    pub name: String,
    /// Whose it is: only they configure it.
    pub owner: User,
    pub online: bool,
    pub lean: bool,
    /// Whether it has LaTeX, for papers.
    #[serde(default)]
    pub latex: bool,
    /// How it isolates the workshop's jobs, as it said when it last
    /// connected; none if it cannot, and then it runs none.
    #[serde(default)]
    pub isolation: Option<Isolation>,
    pub runners: Vec<String>,
    /// Its agents for the workshop, in order of preference.
    pub agents: Vec<WorkshopAgent>,
    #[serde(with = "time::serde::rfc3339")]
    pub added_at: OffsetDateTime,
}

/// Makes one of the admin's own machines a workshop machine of the group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddWorkshopMachine {
    pub machine: i64,
}

/// What the workshop's jobs consumed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Consumption {
    /// The jobs: the runs of tasks and the builds of the library.
    pub jobs: u32,
    /// How long they took, in seconds.
    pub seconds: u64,
    /// The tokens and cost the runners reported; absent if none did.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// What the workshop's jobs consumed in a day, on a machine, with a runner
/// and model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsumptionRow {
    pub machine: String,
    /// The runner, or `lean` for builds.
    pub runner: String,
    pub model: Option<String>,
    #[serde(flatten)]
    pub consumption: Consumption,
}

/// A day of the workshop's consumption.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsumptionDay {
    /// The day, in UTC: `2026-10-18`.
    pub day: String,
    #[serde(flatten)]
    pub total: Consumption,
    pub rows: Vec<ConsumptionRow>,
}

// ── Machines ──────────────────────────────────────────────────────────

/// One of a person's machines: where their own agents run, with their
/// own sign-in to each runner, and only for them; and, if its owner
/// adds it to a group they administer, where the group's workshop works
/// and builds its library.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Machine {
    pub id: i64,
    pub name: String,
    /// The first characters of its token, to recognise it.
    pub prefix: String,
    pub online: bool,
    /// The runners it found installed and usable when it last connected.
    pub runners: Vec<String>,
    /// Whether it has Lean, for the workshop.
    #[serde(default)]
    pub lean: bool,
    /// Whether it has LaTeX (`latexmk`), for the workshop's papers.
    #[serde(default)]
    pub latex: bool,
    /// How it isolates the workshop's jobs, as it said when it last
    /// connected; none if it cannot, and then it may not work for a
    /// workshop.
    #[serde(default)]
    pub isolation: Option<Isolation>,
    /// Whether their jobs go to it first, when it has the runner.
    pub preferred: bool,
    /// The groups whose workshop it works for, by slug.
    #[serde(default)]
    pub workshop: Vec<String>,
    /// Its agents for the workshop, in order of preference.
    #[serde(default)]
    pub workshop_agents: Vec<WorkshopAgent>,
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_seen_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateMachine {
    pub name: String,
    /// Whether it is to be the preferred one.
    #[serde(default)]
    pub preferred: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateMachine {
    pub preferred: bool,
}

/// A new machine, with the secret of its token, shown once.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedMachine {
    #[serde(flatten)]
    pub machine: Machine,
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
    /// Build a commit of the library, and inspect it.
    LibraryBuild,
    /// Compile a commit of a paper into its PDF.
    PaperBuild,
    /// Work on a task of the workshop.
    TaskRun,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Reply => "reply",
            JobKind::LibraryBuild => "library_build",
            JobKind::PaperBuild => "paper_build",
            JobKind::TaskRun => "task_run",
        }
    }

    pub fn parse(s: &str) -> Option<JobKind> {
        match s {
            "reply" => Some(JobKind::Reply),
            "library_build" => Some(JobKind::LibraryBuild),
            "paper_build" => Some(JobKind::PaperBuild),
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
    /// For a task's run, the task's number: the `7` of `w7`.
    #[serde(default)]
    pub task: Option<u32>,
    /// The agent's handle and name; for a build, `lean` and `Lean`, or
    /// `latex` and `LaTeX`.
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
    /// A build of the library started, ended or waits.
    BuildUpdated {
        build: Build,
    },
    /// A task of the workshop is new, or changed.
    TaskUpdated {
        task: Box<Task>,
    },
    /// A paper is new, or changed: its authors, or a build of it.
    PaperUpdated {
        paper: Box<Paper>,
    },
}
