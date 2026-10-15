//! Jobs: what agents and workers are asked to do, queued in the database
//! and handed to the workers connected to the hub.
//!
//! A message that mentions an agent (`@claude`), or any message in a chat
//! with a resident agent, creates a job per agent: an answer, or for an
//! editor, an update of the paper the message names; for the workshop, a
//! task (see [`crate::workshop`]). A new commit of a group's library
//! creates a build. A job waits until a worker that can run it is connected
//! and free:
//!
//! - an agent of personal scope runs on a worker of the person who asked;
//! - an agent of group scope, on a worker of the group with its runner;
//! - a build, on a worker of the group with Lean, one at a time per group;
//! - a task, on a worker of the group that offers the workshop, with Lean
//!   and the workshop agent's runner, or during the group's night window on
//!   a member's worker its owner lends; one at a time per group, and those
//!   asked for the night only in the night window.
//!
//! An answer is posted in the thread as a message by the agent, for the
//! person who asked; an editor's revision is stored as the paper's next,
//! and a message in the thread says so (the editor also updates papers at
//! night, see [`crate::workshop::update_papers`]); a build's report becomes
//! the library's state; a task's run is reported in its thread.
//!
//! Jobs live in the database, so they survive the hub's restarts; a job
//! whose worker goes away is queued again, up to [`MAX_ATTEMPTS`] times.

use std::collections::{HashMap, HashSet};

use sqlx::FromRow;
use stemma_core::api::{
    AgentProfile, AgentRole, AgentScope, Event, Job, JobKind, JobState, Message, Thread,
    ThreadKind, User, runners,
};
use stemma_core::protocol::{
    Conversation, GroupRef, HubMessage, JobSpec, Outcome, PaperSpec, PaperUpdate, RelatedThread,
    ThreadRef, Work,
};
use stemma_core::{Address, locate};
use tokio::sync::mpsc;

use crate::api::forum;
use crate::api::papers::{self, Written};
use crate::api::{Membership, papers::Stored};
use crate::auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::library;
use crate::state::AppState;
use crate::workshop::{self, Night};

/// How many times a job is handed to a worker before it fails.
pub const MAX_ATTEMPTS: i64 = 3;

/// How long a job's token lasts, in milliseconds.
const JOB_TOKEN_MILLIS: i64 = 2 * 60 * 60 * 1000;

/// How long a build's or a task's token lasts: they can be long.
const BUILD_TOKEN_MILLIS: i64 = 12 * 60 * 60 * 1000;

/// About how many characters of a thread an agent is given.
const CONTEXT_BUDGET: usize = 60_000;

/// About how many characters of each thread about a paper an editor is
/// given, and how many such threads.
const RELATED_BUDGET: usize = 15_000;
const MAX_RELATED: usize = 6;

/// The most an answer may be, as a message.
const MAX_ANSWER_CHARS: usize = 100_000;

/// The runner of builds, as jobs record it.
pub const LEAN: &str = "lean";

/// The workers connected now.
#[derive(Default)]
pub struct Workers {
    connected: HashMap<i64, Connected>,
    /// Each group's night, as the last dispatch saw it.
    pub nights: HashMap<i64, Night>,
}

/// A connected worker.
pub struct Connected {
    /// Which connection this is, so that a stale one does not remove a
    /// newer one of the same worker.
    pub connection: u64,
    pub group_id: Option<i64>,
    pub owner_id: Option<i64>,
    pub runners: Vec<String>,
    pub lean: bool,
    /// Whether it works on the workshop's tasks.
    pub workshop: bool,
    pub concurrency: u32,
    pub running: HashSet<i64>,
    pub sender: mpsc::UnboundedSender<HubMessage>,
}

impl Workers {
    pub fn is_online(&self, worker_id: i64) -> bool {
        self.connected.contains_key(&worker_id)
    }

    fn can_run(&self, worker: &Connected, job: &JobRow) -> bool {
        match job.kind() {
            JobKind::LibraryBuild => return worker.lean && worker.group_id == Some(job.group_id),
            JobKind::TaskRun => return self.task_runner(worker, job).is_some(),
            _ => {}
        }
        worker.runners.contains(&job.runner)
            && match AgentScope::parse(&job.scope) {
                Some(AgentScope::Personal) => worker.owner_id == Some(job.requester_id),
                Some(AgentScope::Group) => worker.group_id == Some(job.group_id),
                None => false,
            }
    }

    /// The runner a worker would work on a task with, if it can now: a
    /// group worker, with the workshop agent's runner; a member's worker
    /// lent to the group, at night, with that runner or another of the
    /// workshop's it has. Tasks asked for the night wait for it.
    fn task_runner(&self, worker: &Connected, job: &JobRow) -> Option<String> {
        if !(worker.lean && worker.workshop) {
            return None;
        }
        let night = self.nights.get(&job.group_id);
        let night_now = night.is_some_and(|n| n.now);
        if job.task_schedule.as_deref() == Some("night") && !night_now {
            return None;
        }
        let offers = |runner: &str| worker.runners.iter().any(|r| r == runner);
        if worker.group_id == Some(job.group_id) {
            return offers(&job.runner).then(|| job.runner.clone());
        }
        let lent = worker
            .owner_id
            .is_some_and(|owner| night.is_some_and(|n| n.now && n.lenders.contains(&owner)));
        if !lent {
            return None;
        }
        if offers(&job.runner) {
            return Some(job.runner.clone());
        }
        runners::WORKSHOP
            .iter()
            .find(|runner| offers(runner))
            .map(|runner| (*runner).to_owned())
    }

    /// A connected worker that can run `job` and has room for it.
    fn free_for(&self, job: &JobRow) -> Option<i64> {
        self.connected
            .iter()
            .filter(|(_, w)| self.can_run(w, job) && (w.running.len() as u32) < w.concurrency)
            .min_by_key(|(_, w)| w.running.len())
            .map(|(id, _)| *id)
    }

    /// Why a queued job waits.
    fn note(&self, job: &JobRow) -> String {
        let any = self.connected.values().any(|w| self.can_run(w, job));
        if job.kind() == JobKind::TaskRun {
            let night = self.nights.get(&job.group_id);
            if job.task_schedule.as_deref() == Some("night") && !night.is_some_and(|n| n.now) {
                return match night {
                    Some(night) => format!("Waiting for the night, {}", night.window),
                    None => "Waiting for the night".into(),
                };
            }
            return if any {
                "Waiting for the workshop to be free".into()
            } else {
                format!(
                    "Waiting for a group worker with Lean and {} to come online",
                    runner_name(&job.runner)
                )
            };
        }
        if job.kind() == JobKind::LibraryBuild {
            return if any {
                "Waiting for a free worker with Lean".into()
            } else {
                "Waiting for a group worker with Lean to come online".into()
            };
        }
        let runner = runner_name(&job.runner);
        match (any, AgentScope::parse(&job.scope)) {
            (true, _) => "Waiting for a free worker".into(),
            (false, Some(AgentScope::Personal)) => format!(
                "Waiting for {}'s worker, with {runner}, to come online",
                job.display_name
            ),
            _ => format!("Waiting for a group worker with {runner} to come online"),
        }
    }
}

/// How a runner is called in what people read.
pub fn runner_name(runner: &str) -> &str {
    match runner {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        "api" => "an API model",
        "opencode" => "OpenCode",
        other => other,
    }
}

// ── Agent profiles ────────────────────────────────────────────────────

#[derive(FromRow)]
struct ProfileRow {
    handle: String,
    display_name: String,
    runner: String,
    model: Option<String>,
    endpoint: Option<String>,
    instructions: Option<String>,
    scope: String,
    role: String,
}

impl From<ProfileRow> for AgentProfile {
    fn from(row: ProfileRow) -> Self {
        AgentProfile {
            handle: row.handle,
            display_name: row.display_name,
            runner: row.runner,
            model: row.model,
            endpoint: row.endpoint,
            instructions: row.instructions,
            scope: AgentScope::parse(&row.scope).unwrap_or(AgentScope::Group),
            role: AgentRole::parse(&row.role).unwrap_or_default(),
        }
    }
}

macro_rules! profile_query {
    () => {
        "SELECT handle, display_name, runner, model, endpoint, instructions, scope, role
         FROM agent_profiles"
    };
}

pub async fn profiles(state: &AppState, group_id: i64) -> ApiResult<Vec<AgentProfile>> {
    let rows: Vec<ProfileRow> = sqlx::query_as(concat!(
        profile_query!(),
        " WHERE group_id = ? ORDER BY handle"
    ))
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(AgentProfile::from).collect())
}

pub async fn find_profile(
    state: &AppState,
    group_id: i64,
    handle: &str,
) -> ApiResult<Option<AgentProfile>> {
    let row: Option<ProfileRow> = sqlx::query_as(concat!(
        profile_query!(),
        " WHERE group_id = ? AND handle = ?"
    ))
    .bind(group_id)
    .bind(handle)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(AgentProfile::from))
}

/// The agents every group starts with: each person's own Claude and
/// Codex, an editor of papers, with each person's own Claude, and the
/// workshop, on the group's workers, with OpenCode.
pub fn default_profiles() -> Vec<AgentProfile> {
    let personal = |handle: &str, name: &str, runner: &str, role: AgentRole| AgentProfile {
        handle: handle.into(),
        display_name: name.into(),
        runner: runner.into(),
        model: None,
        endpoint: None,
        instructions: None,
        scope: AgentScope::Personal,
        role,
    };
    vec![
        personal("claude", "Claude", "claude-code", AgentRole::Answer),
        personal("codex", "Codex", "codex", AgentRole::Answer),
        personal("editor", "Editor", "claude-code", AgentRole::Editor),
        AgentProfile {
            scope: AgentScope::Group,
            ..personal(
                "workshop",
                "Workshop",
                runners::OPENCODE,
                AgentRole::Workshop,
            )
        },
    ]
}

// ── Jobs ──────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct JobRow {
    id: i64,
    group_id: i64,
    kind: String,
    thread_id: Option<i64>,
    thread_number: Option<i64>,
    trigger_message: Option<i64>,
    requester_id: i64,
    username: String,
    display_name: String,
    agent_handle: Option<String>,
    agent: Option<String>,
    runner: String,
    scope: String,
    state: String,
    worker_id: Option<i64>,
    token_id: Option<i64>,
    attempts: i64,
    progress: Option<String>,
    error: Option<String>,
    answer: Option<i64>,
    paper_slug: Option<String>,
    base_revision: Option<i64>,
    created_at: i64,
    updated_at: i64,
    /// For a task's run, the task's number and schedule.
    task_number: Option<i64>,
    task_schedule: Option<String>,
    /// For an editor's job, whether it is a paper's nightly update.
    nightly: bool,
}

macro_rules! job_query {
    () => {
        "SELECT j.id, j.group_id, j.kind, j.thread_id, t.number AS thread_number,
        j.trigger_message, j.requester_id, u.username, u.display_name, j.agent_handle, j.agent,
        j.runner, j.scope, j.state, j.worker_id, j.token_id, j.attempts, j.progress, j.error,
        j.answer, j.paper_slug, j.base_revision, j.created_at, j.updated_at,
        k.number AS task_number, k.schedule AS task_schedule, j.nightly
         FROM jobs j LEFT JOIN threads t ON t.id = j.thread_id
              JOIN users u ON u.id = j.requester_id
              LEFT JOIN tasks k ON k.job_id = j.id"
    };
}

impl JobRow {
    fn state(&self) -> JobState {
        JobState::parse(&self.state).unwrap_or(JobState::Failed)
    }

    fn kind(&self) -> JobKind {
        JobKind::parse(&self.kind).unwrap_or(JobKind::Reply)
    }

    fn profile(&self) -> ApiResult<AgentProfile> {
        let agent = self
            .agent
            .as_deref()
            .ok_or_else(|| ApiError::internal("an agent's job without its agent"))?;
        serde_json::from_str(agent).map_err(ApiError::internal)
    }

    fn requester(&self) -> User {
        User {
            id: self.requester_id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }

    fn to_api(&self, workers: &Workers) -> Job {
        let state = self.state();
        let (agent, agent_name) = match self.kind() {
            JobKind::LibraryBuild => (LEAN.to_owned(), "Lean".to_owned()),
            _ => {
                let handle = self.agent_handle.clone().unwrap_or_default();
                let name = self
                    .profile()
                    .map(|p| p.display_name)
                    .unwrap_or_else(|_| handle.clone());
                (handle, name)
            }
        };
        Job {
            id: self.id,
            kind: self.kind(),
            thread: self.thread_number.map(|n| n as u32),
            trigger: self.trigger_message.map(|n| n as u32),
            paper: self.paper_slug.clone(),
            task: self.task_number.map(|n| n as u32),
            agent,
            agent_name,
            requester: self.requester(),
            state,
            progress: self.progress.clone().filter(|_| state == JobState::Running),
            note: (state == JobState::Queued).then(|| workers.note(self)),
            error: self.error.clone(),
            answer: self.answer.map(|n| n as u32),
            created_at: db::from_millis(self.created_at),
            updated_at: db::from_millis(self.updated_at),
        }
    }
}

async fn load_job(state: &AppState, job_id: i64) -> ApiResult<Option<JobRow>> {
    Ok(sqlx::query_as(concat!(job_query!(), " WHERE j.id = ?"))
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?)
}

/// Tells the job's group how it is now: and for a build, how the build is.
async fn publish(state: &AppState, workers: &Workers, job_id: i64) -> ApiResult<()> {
    let Some(row) = load_job(state, job_id).await? else {
        return Ok(());
    };
    let job = row.to_api(workers);
    if row.kind() == JobKind::LibraryBuild
        && let Some(build) = library::build_of_job(state, job_id, job.note.clone()).await?
    {
        state.publish(row.group_id, Event::BuildUpdated { build });
    }
    if row.kind() == JobKind::TaskRun {
        workshop::publish_of_job(state, job_id, job.note.clone()).await?;
    }
    state.publish(row.group_id, Event::JobUpdated { job });
    Ok(())
}

/// The jobs of a thread worth showing: those on their way, and those that
/// failed in the last day and nobody dismissed.
pub async fn of_thread(state: &AppState, thread_id: i64) -> ApiResult<Vec<Job>> {
    let rows: Vec<JobRow> = sqlx::query_as(concat!(
        job_query!(),
        " WHERE j.thread_id = ?
           AND (j.state IN ('queued', 'running') OR (j.state = 'failed' AND j.updated_at > ?))
         ORDER BY j.id"
    ))
    .bind(thread_id)
    .bind(db::now() - 24 * 60 * 60 * 1000)
    .fetch_all(&state.db)
    .await?;
    let workers = state.workers.lock().await;
    Ok(rows.iter().map(|row| row.to_api(&workers)).collect())
}

/// Why a job waits, if it does: for a build's page.
pub async fn note_of(state: &AppState, job_id: i64) -> ApiResult<Option<String>> {
    let Some(row) = load_job(state, job_id).await? else {
        return Ok(None);
    };
    let workers = state.workers.lock().await;
    Ok(row.to_api(&workers).note)
}

/// The paper a message asks an editor to update: the first it references
/// or names (`p/compactification`), or else the one the thread is
/// anchored to.
pub fn paper_named(body: &str, anchor: Option<&Address>) -> Option<String> {
    for reference in stemma_core::references(body) {
        if let Ok(Address::Paper { slug, .. }) = reference.address {
            return Some(slug);
        }
    }
    let punctuation: &[char] = &['.', ',', ';', ':', '!', '?', '(', ')', '`', '*', '"', '\''];
    for word in body.split_whitespace() {
        let word = word.trim_matches(punctuation);
        if !word.starts_with("p/") && !word.contains("/p/") {
            continue;
        }
        if let Ok(located) = locate(word)
            && let Address::Paper { slug, .. } = located.address
        {
            return Some(slug);
        }
    }
    match anchor {
        Some(Address::Paper { slug, .. }) => Some(slug.clone()),
        _ => None,
    }
}

/// Asks the agents a message summons for what they do: those it mentions,
/// and in a chat, its resident agent. Messages written by agents summon
/// none.
pub async fn summon(
    state: &AppState,
    group: &Membership,
    thread_id: i64,
    thread: &Thread,
    message: &Message,
) -> ApiResult<()> {
    if message.agent.is_some() || message.deleted {
        return Ok(());
    }
    let mut handles = stemma_core::mentions(&message.body);
    if thread.kind == ThreadKind::Chat
        && let Some(resident) = &thread.resident_agent
        && !handles.contains(resident)
    {
        handles.insert(0, resident.clone());
    }
    let mut created = false;
    for handle in handles {
        let Some(profile) = find_profile(state, group.group_id, &handle).await? else {
            continue;
        };
        if profile.role == AgentRole::Workshop {
            let (title, schedule) = workshop::from_message(&message.body, &profile.handle);
            let task = workshop::NewTask {
                title,
                instructions: message.body.clone(),
                thread_id,
                origin_message: message.number,
                requester_id: message.author.id,
                agent: None,
                schedule,
            };
            match workshop::create(state, group, task).await {
                Ok(_) => created = true,
                Err(error) if error.status.is_client_error() => {
                    tracing::warn!(error = error.message, "could not hand a task over");
                }
                Err(error) => return Err(error),
            }
            continue;
        }
        let (kind, paper, error) = match profile.role {
            AgentRole::Answer | AgentRole::Workshop => (JobKind::Reply, None, None),
            AgentRole::Editor => match paper_named(&message.body, thread.anchor.as_ref()) {
                Some(slug) => (JobKind::PaperUpdate, Some(slug), None),
                None => (
                    JobKind::PaperUpdate,
                    None,
                    Some(format!(
                        "Say which paper @{} should update, as p/its-name",
                        profile.handle
                    )),
                ),
            },
        };
        let job_id = insert_agent_job(
            state,
            group.group_id,
            thread_id,
            message.number,
            message.author.id,
            &profile,
            kind,
            paper.as_deref(),
            error.as_deref(),
        )
        .await?;
        if error.is_some() {
            let workers = state.workers.lock().await;
            publish(state, &workers, job_id).await?;
        } else {
            created = true;
        }
    }
    if created {
        dispatch(state).await?;
    }
    Ok(())
}

/// Records an agent's job for a message: queued, or failed with `error`.
#[allow(clippy::too_many_arguments)]
async fn insert_agent_job(
    state: &AppState,
    group_id: i64,
    thread_id: i64,
    trigger: u32,
    requester_id: i64,
    profile: &AgentProfile,
    kind: JobKind,
    paper: Option<&str>,
    error: Option<&str>,
) -> ApiResult<i64> {
    let now = db::now();
    Ok(sqlx::query(
        "INSERT INTO jobs (group_id, thread_id, kind, trigger_message, requester_id,
                           agent_handle, agent, runner, scope, state, error, paper_slug,
                           created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(thread_id)
    .bind(kind.as_str())
    .bind(trigger)
    .bind(requester_id)
    .bind(&profile.handle)
    .bind(serde_json::to_string(profile).map_err(ApiError::internal)?)
    .bind(&profile.runner)
    .bind(profile.scope.as_str())
    .bind(if error.is_some() { "failed" } else { "queued" })
    .bind(error)
    .bind(paper)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid())
}

/// Queues an agent's job for a message, as when the workshop asks the
/// editor to update a paper; the caller then dispatches.
#[allow(clippy::too_many_arguments)]
pub async fn queue_agent_job(
    state: &AppState,
    group_id: i64,
    thread_id: i64,
    trigger: u32,
    requester_id: i64,
    profile: &AgentProfile,
    kind: JobKind,
    paper: Option<&str>,
) -> ApiResult<i64> {
    insert_agent_job(
        state,
        group_id,
        thread_id,
        trigger,
        requester_id,
        profile,
        kind,
        paper,
        None,
    )
    .await
}

/// Queues a build of a group's library, for `requester_id`: whoever made
/// the commit happen. Returns the job.
pub async fn queue_build(state: &AppState, group_id: i64, requester_id: i64) -> ApiResult<i64> {
    let now = db::now();
    let id = sqlx::query(
        "INSERT INTO jobs (group_id, kind, requester_id, runner, scope, state, created_at,
                           updated_at)
         VALUES (?, 'library_build', ?, ?, 'group', 'queued', ?, ?)",
    )
    .bind(group_id)
    .bind(requester_id)
    .bind(LEAN)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// Hands the queued jobs to the workers that can run them, oldest first,
/// and tells everyone how the queued ones are.
pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    let mut workers = state.workers.lock().await;
    workers.nights = workshop::nights(state).await?;
    let queued: Vec<JobRow> = sqlx::query_as(concat!(
        job_query!(),
        " WHERE j.state = 'queued' ORDER BY j.id LIMIT 500"
    ))
    .fetch_all(&state.db)
    .await?;
    // A group's library is built by one worker at a time.
    let mut building: HashSet<i64> = sqlx::query_scalar(
        "SELECT group_id FROM jobs WHERE kind = 'library_build' AND state = 'running'",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    // And a group's tasks, one at a time.
    let mut tasking: HashSet<i64> = sqlx::query_scalar(
        "SELECT group_id FROM jobs WHERE kind = 'task_run' AND state = 'running'",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    for job in queued {
        let is_build = job.kind() == JobKind::LibraryBuild;
        let is_task = job.kind() == JobKind::TaskRun;
        let busy = (is_build && building.contains(&job.group_id))
            || (is_task && tasking.contains(&job.group_id));
        let worker_id = if busy { None } else { workers.free_for(&job) };
        let Some(worker_id) = worker_id else {
            publish(state, &workers, job.id).await?;
            continue;
        };
        let runner = if is_task {
            workers
                .connected
                .get(&worker_id)
                .and_then(|worker| workers.task_runner(worker, &job))
        } else {
            None
        };
        match start(state, &job, worker_id, runner).await {
            Ok(spec) => {
                let worker = workers
                    .connected
                    .get_mut(&worker_id)
                    .expect("found just now");
                worker.running.insert(job.id);
                let _ = worker.sender.send(HubMessage::Job(Box::new(spec)));
                if is_build {
                    building.insert(job.group_id);
                }
                if is_task {
                    tasking.insert(job.group_id);
                }
            }
            Err(error) => {
                tracing::error!(job = job.id, error = error.message, "could not start a job");
                fail(state, job.id, "The hub could not prepare this job").await?;
            }
        }
        publish(state, &workers, job.id).await?;
    }
    Ok(())
}

/// Marks a job as running on a worker, with a new token, and returns what
/// the worker needs; a task's run, with `runner`.
async fn start(
    state: &AppState,
    job: &JobRow,
    worker_id: i64,
    runner: Option<String>,
) -> ApiResult<JobSpec> {
    let group = Membership::of_group(state, job.group_id).await?;
    let now = db::now();
    let mut task_profile = None;
    let (work, base_revision) = match job.kind() {
        JobKind::Reply => (Work::Reply(conversation(state, &group, job).await?), None),
        JobKind::PaperUpdate => {
            let update = paper_update(state, &group, job).await?;
            let base = update.paper.revision;
            (Work::PaperUpdate(update), Some(base))
        }
        JobKind::LibraryBuild => (
            Work::LibraryBuild(library::work_for(state, &group, job.id).await?),
            None,
        ),
        JobKind::TaskRun => {
            // The worker's runner, if it is not the agent's: then the
            // agent's model is not for it.
            let mut profile = job.profile()?;
            let runner = runner.unwrap_or_else(|| profile.runner.clone());
            if runner != profile.runner {
                profile.runner = runner;
                profile.model = None;
            }
            let mut conversation = conversation(state, &group, job).await?;
            conversation.agent = profile.clone();
            let run = workshop::work_for(state, &group, job.id, conversation).await?;
            task_profile = Some(profile);
            (Work::TaskRun(Box::new(run)), None)
        }
    };
    let (agent, lasts) = match job.kind() {
        JobKind::LibraryBuild => ("Lean".to_owned(), BUILD_TOKEN_MILLIS),
        JobKind::TaskRun => (job.profile()?.display_name, BUILD_TOKEN_MILLIS),
        _ => (job.profile()?.display_name, JOB_TOKEN_MILLIS),
    };
    let (token_id, token) = auth::create_job_token(
        &state.db,
        job.requester_id,
        job.group_id,
        None,
        &agent,
        now + lasts,
    )
    .await?;
    if task_profile.is_some() {
        auth::grant_push(&state.db, token_id).await?;
    }
    sqlx::query(
        "UPDATE jobs SET state = 'running', worker_id = ?, token_id = ?, attempts = attempts + 1,
                         progress = NULL, base_revision = ?, updated_at = ?
         WHERE id = ? AND state = 'queued'",
    )
    .bind(worker_id)
    .bind(token_id)
    .bind(base_revision)
    .bind(now)
    .bind(job.id)
    .execute(&state.db)
    .await?;
    if let Some(profile) = &task_profile {
        workshop::started(state, job.id, &profile.runner, profile.model.as_deref()).await?;
    }
    Ok(JobSpec {
        id: job.id,
        group: GroupRef {
            slug: group.slug.clone(),
            name: group.name.clone(),
        },
        token,
        work,
    })
}

/// The thread of an agent's job, up to the message that asked.
async fn conversation(
    state: &AppState,
    group: &Membership,
    job: &JobRow,
) -> ApiResult<Conversation> {
    let thread_id = job
        .thread_id
        .ok_or_else(|| ApiError::internal("an agent's job without its thread"))?;
    let thread: Thread = forum::find_thread_by_id(state, thread_id).await?.into();
    let messages = forum::load_messages(state, group, thread_id, 1, u32::MAX).await?;
    let trigger = job.trigger_message.unwrap_or(1) as u32;
    Ok(Conversation {
        agent: job.profile()?,
        context: stemma_core::context::trim(&messages, thread.number, trigger, CONTEXT_BUDGET),
        thread: ThreadRef {
            number: thread.number,
            title: thread.title,
            kind: thread.kind,
        },
        trigger,
        requester: job.requester(),
    })
}

/// An editor's job: the paper as it is, the conversation that asked, and
/// the other threads about the paper.
async fn paper_update(
    state: &AppState,
    group: &Membership,
    job: &JobRow,
) -> ApiResult<PaperUpdate> {
    let slug = job
        .paper_slug
        .clone()
        .ok_or_else(|| ApiError::internal("an editor's job without its paper"))?;
    let conversation = conversation(state, group, job).await?;
    let paper = match papers::current(state, group.group_id, &slug).await? {
        Some(current) => PaperSpec {
            slug: slug.clone(),
            title: Some(current.title),
            revision: current.revision,
            body: current.body,
        },
        None => PaperSpec {
            slug: slug.clone(),
            title: None,
            revision: 0,
            body: String::new(),
        },
    };
    let mut related = Vec::new();
    for (thread_id, anchor) in papers::threads_about(state, group.group_id, &slug).await? {
        if Some(thread_id) == job.thread_id || related.len() >= MAX_RELATED {
            continue;
        }
        let thread: Thread = forum::find_thread_by_id(state, thread_id).await?.into();
        let messages = forum::load_messages(state, group, thread_id, 1, u32::MAX).await?;
        related.push(RelatedThread {
            context: stemma_core::context::trim(
                &messages,
                thread.number,
                thread.last_message,
                RELATED_BUDGET,
            ),
            thread: ThreadRef {
                number: thread.number,
                title: thread.title,
                kind: thread.kind,
            },
            anchor,
        });
    }
    Ok(PaperUpdate {
        conversation,
        paper,
        related,
        nightly: job.nightly,
    })
}

async fn revoke_token(state: &AppState, token_id: Option<i64>) -> ApiResult<()> {
    if let Some(id) = token_id {
        sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(db::now())
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    Ok(())
}

async fn fail(state: &AppState, job_id: i64, error: &str) -> ApiResult<()> {
    let token: Option<Option<i64>> = sqlx::query_scalar(
        "UPDATE jobs SET state = 'failed', error = ?, progress = NULL, updated_at = ?
         WHERE id = ? RETURNING token_id",
    )
    .bind(error)
    .bind(db::now())
    .bind(job_id)
    .fetch_optional(&state.db)
    .await?;
    revoke_token(state, token.flatten()).await?;
    workshop::job_ended(state, job_id, false, Some(error)).await
}

/// Marks a job done, with the message that tells of it, if any.
async fn done(state: &AppState, job: &JobRow, answer: Option<u32>) -> ApiResult<()> {
    sqlx::query(
        "UPDATE jobs SET state = 'done', answer = ?, progress = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(answer)
    .bind(db::now())
    .bind(job.id)
    .execute(&state.db)
    .await?;
    revoke_token(state, job.token_id).await
}

/// A job a worker runs, if it is running on that worker.
async fn running_on(state: &AppState, worker_id: i64, job_id: i64) -> ApiResult<Option<JobRow>> {
    Ok(load_job(state, job_id)
        .await?
        .filter(|job| job.state() == JobState::Running && job.worker_id == Some(worker_id)))
}

/// What a worker says a job is doing.
pub async fn progress(state: &AppState, worker_id: i64, job_id: i64, text: &str) -> ApiResult<()> {
    let workers = state.workers.lock().await;
    if running_on(state, worker_id, job_id).await?.is_none() {
        return Ok(());
    }
    let text: String = text.trim().chars().take(200).collect();
    sqlx::query("UPDATE jobs SET progress = ?, updated_at = ? WHERE id = ?")
        .bind(text)
        .bind(db::now())
        .bind(job_id)
        .execute(&state.db)
        .await?;
    publish(state, &workers, job_id).await
}

/// How a worker says a job ended: an answer is posted in the thread, a
/// paper's revision is stored, a build's report is kept.
pub async fn finished(
    state: &AppState,
    worker_id: i64,
    job_id: i64,
    outcome: Outcome,
) -> ApiResult<()> {
    {
        let mut workers = state.workers.lock().await;
        if let Some(worker) = workers.connected.get_mut(&worker_id) {
            worker.running.remove(&job_id);
        }
        let Some(job) = running_on(state, worker_id, job_id).await? else {
            return Ok(());
        };
        let failure = match (job.kind(), outcome) {
            (_, Outcome::Failed { error }) => Some(error.trim().chars().take(2_000).collect()),
            (JobKind::Reply, Outcome::Answer { body }) => answered(state, &job, &body).await?,
            (
                JobKind::PaperUpdate,
                Outcome::Paper {
                    title,
                    body,
                    reason,
                },
            ) => revised(state, &job, &title, &body, &reason).await?,
            (JobKind::LibraryBuild, Outcome::Built(built)) => {
                library::build_finished(state, job.group_id, job_id, *built).await?;
                done(state, &job, None).await?;
                None
            }
            (JobKind::TaskRun, Outcome::Task(outcome)) => {
                let number = workshop::finished(state, job_id, *outcome).await?;
                done(state, &job, Some(number)).await?;
                None
            }
            _ => Some("The worker answered with something else than was asked".into()),
        };
        if let Some(error) = failure {
            fail(state, job_id, &error).await?;
        }
        publish(state, &workers, job_id).await?;
    }
    dispatch(state).await
}

/// Posts an agent's answer; or says why it cannot be.
async fn answered(state: &AppState, job: &JobRow, answer: &str) -> ApiResult<Option<String>> {
    if answer.trim().is_empty() {
        return Ok(Some("The agent gave no answer".into()));
    }
    let mut text: String = forum::check_body(answer).unwrap_or_default();
    if text.chars().count() >= MAX_ANSWER_CHARS {
        text = text.chars().take(MAX_ANSWER_CHARS - 40).collect();
        text.push_str("\n\n[The answer was cut here.]");
    }
    let number = say(state, job, &text).await?;
    done(state, job, Some(number)).await?;
    Ok(None)
}

/// Posts a message in a job's thread, by its agent, in reply to the
/// message that asked. Returns its number.
async fn say(state: &AppState, job: &JobRow, text: &str) -> ApiResult<u32> {
    let thread_id = job
        .thread_id
        .ok_or_else(|| ApiError::internal("an agent's job without its thread"))?;
    let profile = job.profile()?;
    let group = Membership::of_group(state, job.group_id).await?;
    let message = forum::post(
        state,
        &group,
        thread_id,
        job.requester_id,
        Some(&profile.display_name),
        text,
        job.trigger_message.map(|n| n as u32),
        &[],
    )
    .await?;
    Ok(message.number)
}

/// Stores an editor's revision of a paper, and says so in the thread; or
/// says why it cannot be.
async fn revised(
    state: &AppState,
    job: &JobRow,
    title: &str,
    body: &str,
    reason: &str,
) -> ApiResult<Option<String>> {
    let Some(slug) = &job.paper_slug else {
        return Ok(Some("This job had no paper".into()));
    };
    let profile = job.profile()?;
    // At night, a paper with nothing to change stays as it is, quietly.
    if job.nightly
        && let Some(current) = papers::current(state, job.group_id, slug).await?
        && current.body.trim_end() == body.trim_end()
        && current.title == title.trim()
    {
        done(state, job, None).await?;
        return Ok(None);
    }
    let group = Membership::of_group(state, job.group_id).await?;
    let written = Written {
        author_id: job.requester_id,
        agent: Some(profile.display_name.clone()),
        title: Some(title.to_owned()),
        body: body.to_owned(),
        reason: reason.to_owned(),
    };
    let base = job.base_revision.unwrap_or(0) as u32;
    let paper = match papers::store(state, &group, slug, base, written).await {
        Ok(Stored::Revised(paper)) => paper,
        Ok(Stored::Moved { current }) => {
            return Ok(Some(format!(
                "p/{slug} changed while {} worked on it (it is at revision {current} now): ask \
                 again",
                profile.display_name
            )));
        }
        Err(error) if error.status.is_client_error() => {
            return Ok(Some(format!(
                "The revision was not stored: {}",
                error.message
            )));
        }
        Err(error) => return Err(error),
    };
    let reason = reason.trim();
    let news = if paper.revision == 1 {
        format!("Started [[p/{slug}]]: {reason}")
    } else {
        format!(
            "Updated [[p/{slug}]] to revision {}: {reason}",
            paper.revision
        )
    };
    let number = say(state, job, &news).await?;
    done(state, job, Some(number)).await?;
    Ok(None)
}

/// Registers a connected worker, replacing an older connection of it,
/// whose jobs are queued again.
pub async fn connected(state: &AppState, worker_id: i64, worker: Connected) -> ApiResult<()> {
    let old = state
        .workers
        .lock()
        .await
        .connected
        .insert(worker_id, worker);
    if let Some(old) = old {
        requeue(state, worker_id, old.running).await?;
    }
    dispatch(state).await
}

/// Forgets a worker's connection, if it is still the current one, and
/// queues its jobs again.
pub async fn disconnected(state: &AppState, worker_id: i64, connection: u64) -> ApiResult<()> {
    let gone = {
        let mut workers = state.workers.lock().await;
        match workers.connected.get(&worker_id) {
            Some(w) if w.connection == connection => workers.connected.remove(&worker_id),
            _ => None,
        }
    };
    if let Some(gone) = gone {
        requeue(state, worker_id, gone.running).await?;
    }
    dispatch(state).await
}

/// Disconnects a worker, as when its token is revoked.
pub async fn disconnect(state: &AppState, worker_id: i64) -> ApiResult<()> {
    let gone = state.workers.lock().await.connected.remove(&worker_id);
    if let Some(gone) = gone {
        requeue(state, worker_id, gone.running).await?;
    }
    dispatch(state).await
}

/// Queues again the jobs a worker was running, unless they ran too often.
async fn requeue(state: &AppState, worker_id: i64, jobs: HashSet<i64>) -> ApiResult<()> {
    for job_id in jobs {
        let Some(job) = running_on(state, worker_id, job_id).await? else {
            continue;
        };
        revoke_token(state, job.token_id).await?;
        if job.attempts >= MAX_ATTEMPTS {
            fail(
                state,
                job_id,
                "The workers that took this job went away before finishing it",
            )
            .await?;
        } else {
            sqlx::query(
                "UPDATE jobs SET state = 'queued', worker_id = NULL, token_id = NULL,
                                 progress = NULL, updated_at = ?
                 WHERE id = ?",
            )
            .bind(db::now())
            .bind(job_id)
            .execute(&state.db)
            .await?;
            if job.kind() == JobKind::TaskRun {
                workshop::requeued(state, job_id).await?;
            }
        }
    }
    Ok(())
}

/// After a restart no worker is connected: queue again what was running.
pub async fn recover(state: &AppState) -> anyhow::Result<()> {
    let tokens: Vec<Option<i64>> = sqlx::query_scalar(
        "UPDATE jobs SET state = 'queued', worker_id = NULL, token_id = NULL, progress = NULL
         WHERE state = 'running' RETURNING token_id",
    )
    .fetch_all(&state.db)
    .await?;
    for token in tokens {
        revoke_token(state, token)
            .await
            .map_err(|e| anyhow::anyhow!(e.message))?;
    }
    sqlx::query("UPDATE tasks SET state = 'open' WHERE state = 'running'")
        .execute(&state.db)
        .await?;
    Ok(())
}

/// Cancels a job on its way, or dismisses one that failed: its requester,
/// or an admin of its group.
pub async fn cancel(
    state: &AppState,
    group: &Membership,
    user_id: i64,
    job_id: i64,
) -> ApiResult<()> {
    let job = load_job(state, job_id)
        .await?
        .filter(|job| job.group_id == group.group_id)
        .ok_or_else(|| ApiError::not_found("Job"))?;
    if job.requester_id != user_id {
        group.require_admin()?;
    }
    stop(state, job_id).await
}

/// Stops a job on its way, or dismisses one that failed.
pub async fn stop(state: &AppState, job_id: i64) -> ApiResult<()> {
    let mut workers = state.workers.lock().await;
    let job = load_job(state, job_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Job"))?;
    if matches!(job.state(), JobState::Done | JobState::Cancelled) {
        return Err(ApiError::conflict("This job is over"));
    }
    if job.state() == JobState::Running
        && let Some(worker) = job.worker_id.and_then(|id| workers.connected.get_mut(&id))
    {
        worker.running.remove(&job_id);
        let _ = worker.sender.send(HubMessage::Cancel { job: job_id });
    }
    sqlx::query(
        "UPDATE jobs SET state = 'cancelled', progress = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(db::now())
    .bind(job_id)
    .execute(&state.db)
    .await?;
    revoke_token(state, job.token_id).await?;
    workshop::job_ended(state, job_id, true, None).await?;
    publish(state, &workers, job_id).await?;
    drop(workers);
    dispatch(state).await
}

#[cfg(test)]
mod tests {
    use super::paper_named;

    #[test]
    fn finds_the_paper_a_message_names() {
        let named = |body: &str| paper_named(body, None);
        assert_eq!(
            named("@editor update p/compactification with this."),
            Some("compactification".into())
        );
        assert_eq!(
            named("@editor see [[p/gluing#charts]], and p/other"),
            Some("gluing".into())
        );
        assert_eq!(named("@editor (`p/notes`)"), Some("notes".into()));
        assert_eq!(
            named("@editor https://stemma.example.org/g/geometry/p/notes"),
            Some("notes".into())
        );
        assert_eq!(named("@editor update the paper"), None);
        let anchor: stemma_core::Address = "p/anchored#a".parse().unwrap();
        assert_eq!(
            paper_named("@editor update it", Some(&anchor)),
            Some("anchored".into())
        );
    }
}
