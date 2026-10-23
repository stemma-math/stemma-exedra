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
    /// How many of their notifications are unread.
    #[serde(default)]
    pub unread: u32,
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
    pub username: Option<String>,
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
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub runners: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateToken {
    pub name: String,
    /// How long it lasts, in minutes; until revoked if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_minutes: Option<u32>,
}

/// Whose attachments an agent may download, fixed when its token is made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentRule {
    None,
    #[default]
    Mine,
    Group,
}
impl AttachmentRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Mine => "mine",
            Self::Group => "group",
        }
    }
    pub fn allows(self, owner: i64, author: i64) -> bool {
        match self {
            Self::None => false,
            Self::Mine => owner == author,
            Self::Group => true,
        }
    }
    pub fn refusal(self) -> &'static str {
        match self {
            Self::None => {
                "Not allowed to read this file: your settings do not let agents read anyone's attachments (`agents.attachments`)."
            }
            _ => {
                "Not allowed to read this file: your settings let agents read only your own attachments (`agents.attachments`)."
            }
        }
    }
}
impl std::str::FromStr for AttachmentRule {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "mine" => Ok(Self::Mine),
            "group" => Ok(Self::Group),
            _ => Err("attachments must be none, mine or group"),
        }
    }
}

/// A token for a session with one of the person's agents, in one group:
/// it reads the group, and posts and opens threads there as the agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateAgentToken {
    pub attachments: AttachmentRule,
    pub group: String,
    /// One of [`runners::ALL`]: whose agent the session is with.
    pub runner: String,
    /// The computer the session runs on, for the token's name.
    pub host: String,
}

/// What to do with a held answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishHeld {
    /// Whether to replace each secret with `[hidden]` first.
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedToken {
    #[serde(flatten)]
    pub token: ApiToken,
    pub secret: String,
}

// ── Groups ────────────────────────────────────────────────────────────

/// A person's proposal for a new group.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateGroupRequest {
    pub slug: String,
    pub name: String,
    pub purpose: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupRequest {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub purpose: String,
    pub requester: User,
    pub status: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubGroup {
    pub slug: String,
    pub name: String,
    pub members: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubPerson {
    #[serde(flatten)]
    pub user: User,
    pub hub_admin: bool,
}

/// What the hub's page asks for: those whose names hold `q`, a page at a
/// time.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HubSearch {
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub offset: u32,
}

/// A page of what the hub's page searched, and whether there is more.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HubPage<T> {
    pub items: Vec<T>,
    pub more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateHubPerson {
    pub hub_admin: bool,
}

/// What someone may do in a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
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
    pub fn can_write(self) -> bool {
        matches!(self, Role::Admin | Role::Member)
    }
}

/// A group, as one of its members sees it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: i64,
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

/// A link that lets people in. Its secret is shown once.
///
/// A group's link adds an existing account to the group, with its role. The
/// hub's makes an account, and nothing else: it has no role.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invitation {
    pub id: i64,
    pub role: Option<Role>,
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
    /// A group's link: the role it gives, a member's if absent. The hub's
    /// links take none.
    pub role: Option<Role>,
    /// When it stops working; never, if absent.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// How many people may use it; any number, if absent.
    pub max_uses: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatedInvitation {
    #[serde(flatten)]
    pub invitation: Invitation,
    /// The secret of the link: `/invite/<token>` for a group's,
    /// `/signup?token=<token>` for the hub's.
    pub token: String,
}

/// What anyone with a group's link may know about it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvitationInfo {
    pub group_slug: String,
    pub group_name: String,
    pub role: Role,
}

/// What anyone with the hub's link may know about it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignUpInfo {
    pub invited_by: User,
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
    pub resident_runner: Option<String>,
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

/// What goes with a thread or a paper when it is deleted, to say so before.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Contents {
    /// Messages, not counting deleted ones.
    pub messages: u32,
    /// Files attached to them.
    pub files: u32,
    /// A paper's own threads, anchored to it.
    pub threads: u32,
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
    #[serde(default)]
    pub missing_references: Vec<String>,
    /// Its number in the thread: the `45` of `t12#45`. Never reused.
    pub number: u32,
    pub author: User,
    /// The agent that wrote it for its author, if one did.
    pub agent: Option<String>,
    /// The runner that wrote it, kept independently of the profile name.
    pub agent_runner: Option<String>,
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
    pub author: User,
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

/// One of Stemma's three fixed agents; runtime settings belong to the computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfile {
    pub handle: String,
    pub display_name: String,
    pub runner: String,
}
impl AgentProfile {
    pub fn defaults() -> Vec<Self> {
        [
            ("claude", "Claude", runners::CLAUDE_CODE),
            ("codex", "Codex", runners::CODEX),
            ("opencode", "OpenCode", runners::OPENCODE),
        ]
        .into_iter()
        .map(|(handle, name, runner)| Self {
            handle: handle.into(),
            display_name: name.into(),
            runner: runner.into(),
        })
        .collect()
    }
    pub fn default_for(handle: &str) -> Option<Self> {
        Self::defaults().into_iter().find(|a| a.handle == handle)
    }
    pub fn default_of_runner(runner: &str) -> Option<Self> {
        Self::defaults().into_iter().find(|a| a.runner == runner)
    }
    pub fn profile(&self) -> Self {
        self.clone()
    }
}

// ── Jobs ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting for a worker that can run it.
    Queued,
    /// A worker is on it.
    Running,
    /// Its answer seems to hold a secret, and waits for its requester to
    /// publish it, hidden or not, or to discard it.
    Held,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Held => "held",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<JobState> {
        match s {
            "queued" => Some(JobState::Queued),
            "running" => Some(JobState::Running),
            "held" => Some(JobState::Held),
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
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Reply => "reply",
        }
    }

    pub fn parse(s: &str) -> Option<JobKind> {
        match s {
            "reply" => Some(JobKind::Reply),
            _ => None,
        }
    }
}

/// Work on its way: what the hub asked a worker for.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    /// The running computer's personal-token name.
    pub machine: Option<String>,
    pub kind: JobKind,
    /// The thread of the message that asked for it, if one did.
    pub thread: Option<u32>,
    /// The message that asked for it.
    pub trigger: Option<u32>,
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
    /// A held answer, with its secrets marked: only ever to its requester.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Vec<crate::secrets::Part>>,
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
    NamesChanged,
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
    /// A paper is new, or changed: its authors, or a build of it.
    PaperUpdated {
        paper: Box<Paper>,
    },
    /// A thread or a chat was deleted, with its messages.
    ThreadDeleted {
        thread: u32,
    },
    /// A paper was deleted, with its repository.
    PaperDeleted {
        paper: String,
    },
}

// ── Notifications ─────────────────────────────────────────────────────

/// Why a person is told something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// Someone mentioned them in a message.
    Mention,
    /// An agent answered a message of theirs.
    Answer,
    /// An agent's answer seems to hold a secret, and waits for them.
    Held,
    /// An agent could not answer them.
    Failed,
    /// A person or an agent left them a note.
    Note,
    /// Their role in a group changed.
    Role,
    /// They were removed from a group.
    Removed,
    /// Their request for a group was approved: the group is theirs.
    Approved,
    /// Their request for a group was declined.
    Declined,
    /// Someone asks for a group: to the hub's admins.
    Requested,
}

impl NotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NotificationKind::Mention => "mention",
            NotificationKind::Answer => "answer",
            NotificationKind::Held => "held",
            NotificationKind::Failed => "failed",
            NotificationKind::Note => "note",
            NotificationKind::Role => "role",
            NotificationKind::Removed => "removed",
            NotificationKind::Approved => "approved",
            NotificationKind::Declined => "declined",
            NotificationKind::Requested => "requested",
        }
    }

    pub fn parse(s: &str) -> Option<NotificationKind> {
        Some(match s {
            "mention" => NotificationKind::Mention,
            "answer" => NotificationKind::Answer,
            "held" => NotificationKind::Held,
            "failed" => NotificationKind::Failed,
            "note" => NotificationKind::Note,
            "role" => NotificationKind::Role,
            "removed" => NotificationKind::Removed,
            "approved" => NotificationKind::Approved,
            "declined" => NotificationKind::Declined,
            "requested" => NotificationKind::Requested,
            _ => return None,
        })
    }
}

/// The group a notification is about.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotifiedGroup {
    pub slug: String,
    pub name: String,
}

/// The thread a notification is about, and the message in it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotifiedThread {
    pub number: u32,
    pub kind: ThreadKind,
    pub title: String,
    /// The latest message it is about.
    pub message: Option<u32>,
    /// The start of that message, as it reads now.
    pub excerpt: Option<String>,
}

/// Something a person is told.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: i64,
    pub kind: NotificationKind,
    /// How many of the same, in the same thread, it gathers.
    pub count: u32,
    /// Who did it, if someone did.
    pub actor: Option<User>,
    /// The agent that did it for them, if one did.
    pub agent: Option<String>,
    pub group: Option<NotifiedGroup>,
    pub thread: Option<NotifiedThread>,
    /// A note's text, in Markdown, or what it is about: a role, a group's
    /// name.
    pub text: Option<String>,
    /// Whether what it is about is gone: a deleted thread or message.
    pub gone: bool,
    pub read: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// A page of a person's notifications, newest first.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationPage {
    pub items: Vec<Notification>,
    pub unread: u32,
    pub more: bool,
}

/// What one asks for: the notifications before one, a page at a time;
/// with `q`, only those that say it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NotificationQuery {
    pub before: Option<i64>,
    pub q: Option<String>,
}

/// What to do with some of one's notifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChange {
    Read,
    Unread,
    Delete,
}

/// Some of one's notifications, and what to do with them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeNotifications {
    pub ids: Vec<i64>,
    pub change: NotificationChange,
}

/// What a person accepts as notes, and how long they keep what they read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// How many notes they accept an hour; none for no limit.
    pub notes_per_hour: Option<u32>,
    /// Whether they accept notes from people's agents, not only people.
    pub notes_from_agents: bool,
    /// How many days they keep read notifications; none for always.
    pub keep_read_days: Option<u32>,
}

/// A note for some of a group's members, from a person or their agent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SendNote {
    /// Their usernames.
    pub to: Vec<String>,
    /// Markdown, with `$…$` math and `[[…]]` references.
    pub body: String,
    /// The agent that writes it for the person, with a personal token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// What the hub sends on a person's own event stream.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersonalEvent {
    /// Their notifications changed: how many are unread now.
    Notifications { unread: u32 },
}
