//! Jobs: what agents and machines are asked to do, queued in the database
//! and handed to the machines connected to the hub.
//!
//! A message that mentions an agent (`@claude`), or any message in a chat
//! with a resident agent, creates a job per agent: an answer by the
//! writer's own agent (see [`crate::agents`]); or for the workshop, a task
//! (see [`crate::workshop`]). A new commit of a group's library, or of one
//! of its papers, creates a build. A job waits until a machine that can
//! run it is connected and free:
//!
//! - a person's own agent runs on one of their machines with its runner,
//!   the one they prefer if it can;
//! - a build of the library, on one of the group's workshop machines with
//!   Lean, one at a time per group; of a paper, on one with LaTeX, one at
//!   a time per group;
//! - a task, on one of the group's workshop machines with Lean (with
//!   LaTeX, for a paper's), with the first of the machine's agents for the
//!   workshop that it can run and that is under its daily limit; one at a
//!   time per group.
//!
//! An answer is posted in the thread as a message by the agent, for the
//! person who asked; a build's report becomes the library's state, and a
//! paper's build makes its PDF; a
//! task's run is proposed and reported in its thread. Each run is kept,
//! with what it consumed (see [`crate::consumption`]).
//!
//! Jobs live in the database, so they survive the hub's restarts; a job
//! whose machine goes away is queued again, up to [`MAX_ATTEMPTS`] times.

use std::collections::{HashMap, HashSet};

use sqlx::FromRow;
use stemma_core::api::{
    AgentProfile, Event, Isolation, Job, JobKind, JobState, Message, Role, Thread, ThreadKind,
    User, runners,
};
use stemma_core::protocol::{
    Conversation, GroupRef, HubMessage, JobSpec, Outcome, ThreadRef, Usage, Work,
};
use tokio::sync::mpsc;

use crate::agents::{self, Summoned};
use crate::api::Membership;
use crate::api::forum;
use crate::auth;
use crate::consumption;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::library;
use crate::papers;
use crate::state::AppState;
use crate::workshop::{self, machines::AgentRow};

/// How many times a job is handed to a machine before it fails.
pub const MAX_ATTEMPTS: i64 = 3;

/// How long a job's token lasts, in milliseconds.
const JOB_TOKEN_MILLIS: i64 = 2 * 60 * 60 * 1000;

/// How long a build's or a task's token lasts: they can be long.
const BUILD_TOKEN_MILLIS: i64 = 12 * 60 * 60 * 1000;

/// How many threads the workshop may open in a task's run.
const TASK_THREADS: u32 = 2;

/// About how many characters of a thread an agent is given.
const CONTEXT_BUDGET: usize = 60_000;

/// The most an answer may be, as a message.
const MAX_ANSWER_CHARS: usize = 100_000;

/// The runner of builds, as jobs record it.
pub const LEAN: &str = "lean";

/// The machines connected now, and why the workshop's tasks wait.
#[derive(Default)]
pub struct Workers {
    connected: HashMap<i64, Connected>,
    /// Why each task waits, as dispatching last found.
    waiting: HashMap<i64, String>,
}

/// A connected machine.
pub struct Connected {
    pub personal: bool,
    /// Which connection this is, so that a stale one does not remove a
    /// newer one of the same machine.
    pub connection: u64,
    pub owner_id: i64,
    pub runners: Vec<String>,
    pub lean: bool,
    /// Whether it has LaTeX, for the papers.
    pub latex: bool,
    /// How it isolates the workshop's jobs; none if it cannot, and then it
    /// runs none.
    pub isolation: Option<Isolation>,
    /// The groups whose workshop it works for.
    pub workshop: HashSet<i64>,
    pub concurrency: u32,
    /// Whether it is its owner's preferred machine.
    pub preferred: bool,
    pub running: HashMap<i64, JobKind>,
    pub sender: mpsc::UnboundedSender<HubMessage>,
}

impl Connected {
    fn has_room(&self, kind: JobKind) -> bool {
        // Any paper may occupy the extra slot, freeing a normal slot as other
        // jobs finish. At most concurrency + 1 jobs run, with one being a paper.
        let extra = self.latex
            && (kind == JobKind::PaperBuild
                || self
                    .running
                    .values()
                    .any(|kind| *kind == JobKind::PaperBuild));
        self.running.len() < self.concurrency as usize + usize::from(extra)
    }

    /// Whether it may run the jobs of a group's workshop: it works for
    /// it, has Lean, and isolates them.
    fn for_workshop(&self, group_id: i64) -> bool {
        self.lean && self.isolation.is_some() && self.workshop.contains(&group_id)
    }

    /// Whether it may run the jobs of a group's workshop on its papers: it
    /// works for it, has LaTeX, and isolates them.
    fn for_papers(&self, group_id: i64) -> bool {
        self.latex && self.isolation.is_some() && self.workshop.contains(&group_id)
    }
}

impl Workers {
    pub fn is_online(&self, worker_id: i64) -> bool {
        self.connected.contains_key(&worker_id)
    }

    fn can_run(&self, worker: &Connected, job: &JobRow) -> bool {
        match job.kind() {
            JobKind::Reply => {
                worker.personal
                    && worker.owner_id == job.requester_id
                    && worker.runners.contains(&job.runner)
            }
            JobKind::LibraryBuild => worker.for_workshop(job.group_id),
            JobKind::PaperBuild => worker.for_papers(job.group_id),
            JobKind::TaskRun => {
                (if job.task_paper.is_some() {
                    worker.for_papers(job.group_id)
                } else {
                    worker.for_workshop(job.group_id)
                }) && worker
                    .runners
                    .iter()
                    .any(|r| runners::WORKSHOP.contains(&r.as_str()))
            }
        }
    }

    /// The connected machines that can run `job` and have room for it,
    /// best first: the person's preferred machine, then the least busy.
    fn free_for(&self, job: &JobRow) -> Vec<i64> {
        let mut free: Vec<(&i64, &Connected)> = self
            .connected
            .iter()
            .filter(|(_, w)| self.can_run(w, job) && w.has_room(job.kind()))
            .collect();
        free.sort_by_key(|(id, w)| (!w.preferred, w.running.len(), **id));
        free.into_iter().map(|(id, _)| *id).collect()
    }

    /// Marks which of a person's connected machines is preferred.
    pub fn set_preferred(&mut self, owner_id: i64, preferred: Option<i64>) {
        for (id, worker) in self.connected.iter_mut() {
            if worker.owner_id == owner_id {
                worker.preferred = preferred == Some(*id);
            }
        }
    }

    /// Why a queued job waits.
    fn note(&self, job: &JobRow) -> String {
        let any = self.connected.values().any(|w| self.can_run(w, job));
        let with = if job.task_paper.is_some() || job.kind() == JobKind::PaperBuild {
            "LaTeX"
        } else {
            "Lean"
        };
        match job.kind() {
            JobKind::TaskRun => self.waiting.get(&job.id).cloned().unwrap_or_else(|| {
                if any {
                    "Waiting for the workshop to be free".into()
                } else {
                    format!("Waiting for a workshop machine with {with} to come online")
                }
            }),
            JobKind::PaperBuild => {
                if any {
                    "Waiting for a free workshop machine".into()
                } else {
                    format!("Waiting for a workshop machine with {with} to come online")
                }
            }
            JobKind::LibraryBuild => {
                if any {
                    "Waiting for a free workshop machine".into()
                } else {
                    "Waiting for a workshop machine with Lean to come online".into()
                }
            }
            JobKind::Reply => {
                let runner = runners::name(&job.runner);
                if any {
                    format!("Waiting for a machine of {}'s to be free", job.display_name)
                } else {
                    format!(
                        "Waiting for a machine of {}'s with {runner}",
                        job.display_name
                    )
                }
            }
        }
    }
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
    state: String,
    worker_id: Option<i64>,
    token_id: Option<i64>,
    attempts: i64,
    progress: Option<String>,
    error: Option<String>,
    answer: Option<i64>,
    created_at: i64,
    updated_at: i64,
    /// For a task's run, the task's number.
    task_number: Option<i64>,
    /// For a task's run on a paper, the paper.
    task_paper: Option<i64>,
    /// The requester's role in the group, if they are still in it.
    requester_role: Option<String>,
}

macro_rules! job_query {
    () => {
        "SELECT j.id, j.group_id, j.kind, j.thread_id, t.number AS thread_number,
        j.trigger_message, j.requester_id, u.username, u.display_name, j.agent_handle, j.agent,
        j.runner, j.state, j.worker_id, j.token_id, j.attempts, j.progress, j.error,
        j.answer, j.created_at, j.updated_at, k.number AS task_number, k.paper_id AS task_paper,
        m.role AS requester_role
         FROM jobs j LEFT JOIN threads t ON t.id = j.thread_id
              JOIN users u ON u.id = j.requester_id
              LEFT JOIN tasks k ON k.job_id = j.id
              LEFT JOIN memberships m ON m.group_id = j.group_id AND m.user_id = j.requester_id"
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

    /// Whether its agent may hand tasks to the workshop: one answering in
    /// a thread, for someone who may ask for work.
    fn hands_over(&self) -> bool {
        self.kind() == JobKind::Reply
            && self
                .requester_role
                .as_deref()
                .and_then(Role::parse)
                .is_some_and(Role::can_ask_for_work)
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
            JobKind::PaperBuild => (papers::LATEX.to_owned(), "LaTeX".to_owned()),
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

/// Tells the job's group how it is now: and for a build or a task, how
/// that is.
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
    if row.kind() == JobKind::PaperBuild {
        papers::publish_of_job(state, job_id, job.note.clone()).await?;
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

/// Why a job waits, if it does: for a build's or a task's page.
pub async fn note_of(state: &AppState, job_id: i64) -> ApiResult<Option<String>> {
    let Some(row) = load_job(state, job_id).await? else {
        return Ok(None);
    };
    let workers = state.workers.lock().await;
    Ok(row.to_api(&workers).note)
}

/// Asks the agents a message summons for what they do: those it mentions,
/// and in a chat, its resident agent. Each handle names the writer's own
/// agent, or the workshop. Messages written by agents summon none.
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
    let author = &message.author;
    let mut created = false;
    for handle in handles {
        let profile = match agents::resolve(state, group.group_id, author.id, &handle).await? {
            Summoned::Workshop => {
                // In a thread about a paper, the workshop works on it.
                let paper_id = match &thread.anchor {
                    Some(stemma_core::Address::Paper(slug)) => {
                        match papers::find(state, group.group_id, slug).await {
                            Ok(paper) => Some(paper.id),
                            Err(error) if error.status.is_client_error() => None,
                            Err(error) => return Err(error),
                        }
                    }
                    _ => None,
                };
                if !group.role.can_ask_for_work() {
                    let error = format!("Guests cannot ask @{handle} for work");
                    refuse(
                        state,
                        group,
                        thread_id,
                        message,
                        &workshop::profile(),
                        &error,
                    )
                    .await?;
                    continue;
                }
                let task = workshop::NewTask {
                    title: workshop::title_of(&message.body, &handle),
                    instructions: message.body.clone(),
                    architecture: None,
                    paper_id,
                    thread_id,
                    origin_message: message.number,
                    requester_id: author.id,
                    agent: None,
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
            Summoned::Personal(profile) => {
                in_chat(state, thread_id, thread, &handle, profile).await?
            }
            Summoned::Missing => {
                let profile = AgentProfile {
                    handle: handle.clone(),
                    display_name: format!("@{handle}"),
                    runner: String::new(),
                    model: None,
                    args: Vec::new(),
                };
                let error = format!(
                    "{} has no agent @{handle}: add one in the settings",
                    author.display_name
                );
                refuse(state, group, thread_id, message, &profile, &error).await?;
                continue;
            }
            Summoned::Nothing => continue,
        };
        insert_agent_job(
            state,
            group.group_id,
            thread_id,
            message.number,
            author.id,
            &profile,
            None,
        )
        .await?;
        created = true;
    }
    if created {
        dispatch(state).await?;
    }
    Ok(())
}

/// A chat's resident agent, as the chat asks it to answer: with the model
/// and the effort chosen when it started, if the writer's agent runs with
/// the runner they were chosen for.
async fn in_chat(
    state: &AppState,
    thread_id: i64,
    thread: &Thread,
    handle: &str,
    mut profile: AgentProfile,
) -> ApiResult<AgentProfile> {
    if thread.kind != ThreadKind::Chat || thread.resident_agent.as_deref() != Some(handle) {
        return Ok(profile);
    }
    if thread.resident_model.is_none() && thread.resident_effort.is_none() {
        return Ok(profile);
    }
    let runner: Option<String> =
        sqlx::query_scalar("SELECT resident_runner FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_one(&state.db)
            .await?;
    if runner.as_deref() != Some(profile.runner.as_str()) {
        return Ok(profile);
    }
    if let Some(model) = &thread.resident_model {
        profile.model = Some(model.clone());
    }
    if let Some(args) = thread.resident_effort.and_then(|e| e.args(&profile.runner)) {
        profile.args.extend(args);
    }
    Ok(profile)
}

/// Says in the thread why an agent does not answer, as a job that failed.
async fn refuse(
    state: &AppState,
    group: &Membership,
    thread_id: i64,
    message: &Message,
    profile: &AgentProfile,
    error: &str,
) -> ApiResult<()> {
    let job_id = insert_agent_job(
        state,
        group.group_id,
        thread_id,
        message.number,
        message.author.id,
        profile,
        Some(error),
    )
    .await?;
    let workers = state.workers.lock().await;
    publish(state, &workers, job_id).await
}

/// Records an agent's answer to a message: queued, or failed with `error`.
async fn insert_agent_job(
    state: &AppState,
    group_id: i64,
    thread_id: i64,
    trigger: u32,
    requester_id: i64,
    profile: &AgentProfile,
    error: Option<&str>,
) -> ApiResult<i64> {
    let now = db::now();
    Ok(sqlx::query(
        "INSERT INTO jobs (group_id, thread_id, kind, trigger_message, requester_id,
                           agent_handle, agent, runner, state, error, created_at, updated_at)
         VALUES (?, ?, 'reply', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(thread_id)
    .bind(trigger)
    .bind(requester_id)
    .bind(&profile.handle)
    .bind(serde_json::to_string(profile).map_err(ApiError::internal)?)
    .bind(&profile.runner)
    .bind(if error.is_some() { "failed" } else { "queued" })
    .bind(error)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid())
}

/// Queues a build of a group's library, for `requester_id`: whoever made
/// the commit happen. Returns the job.
pub async fn queue_build(state: &AppState, group_id: i64, requester_id: i64) -> ApiResult<i64> {
    let now = db::now();
    let id = sqlx::query(
        "INSERT INTO jobs (group_id, kind, requester_id, runner, state, created_at, updated_at)
         VALUES (?, 'library_build', ?, ?, 'queued', ?, ?)",
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

/// Hands the queued jobs to the machines that can run them, oldest first,
/// and tells everyone how the queued ones are.
pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    let mut workers = state.workers.lock().await;
    let queued: Vec<JobRow> = sqlx::query_as(concat!(
        job_query!(),
        " WHERE j.state = 'queued' ORDER BY j.id"
    ))
    .fetch_all(&state.db)
    .await?;
    // A group's library is built by one machine at a time.
    let mut building: HashSet<i64> = sqlx::query_scalar(
        "SELECT group_id FROM jobs WHERE kind = 'library_build' AND state = 'running'",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    // A group's papers, too.
    let mut compiling: HashSet<i64> = sqlx::query_scalar(
        "SELECT group_id FROM jobs WHERE kind = 'paper_build' AND state = 'running'",
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
    workers.waiting.clear();
    for job in queued {
        let is_build = job.kind() == JobKind::LibraryBuild;
        let is_compile = job.kind() == JobKind::PaperBuild;
        let is_task = job.kind() == JobKind::TaskRun;
        let busy = (is_build && building.contains(&job.group_id))
            || (is_compile && compiling.contains(&job.group_id))
            || (is_task && tasking.contains(&job.group_id));
        let free = if busy {
            Vec::new()
        } else {
            workers.free_for(&job)
        };
        // A task goes with the first agent, on the best machine, that can
        // run it and is under its limit.
        let mut chosen: Option<(i64, Option<AgentRow>)> = None;
        if is_task {
            for worker_id in &free {
                let offered = workers.connected[worker_id].runners.clone();
                if let Some(agent) = workshop::machines::pick(state, *worker_id, &offered).await? {
                    chosen = Some((*worker_id, Some(agent)));
                    break;
                }
            }
            if chosen.is_none() && !free.is_empty() {
                workers.waiting.insert(
                    job.id,
                    "Waiting for a workshop agent under its daily limit".into(),
                );
            }
        } else {
            chosen = free.first().map(|id| (*id, None));
        }
        let Some((worker_id, agent)) = chosen else {
            publish(state, &workers, job.id).await?;
            continue;
        };
        match start(state, &job, worker_id, agent).await {
            Ok(spec) => {
                let worker = workers
                    .connected
                    .get_mut(&worker_id)
                    .expect("found just now");
                worker.running.insert(job.id, job.kind());
                let _ = worker.sender.send(HubMessage::Job(Box::new(spec)));
                if is_build {
                    building.insert(job.group_id);
                }
                if is_compile {
                    compiling.insert(job.group_id);
                }
                if is_task {
                    tasking.insert(job.group_id);
                }
            }
            Err(error) => {
                tracing::error!(job = job.id, error = error.message, "could not start a job");
                let why = if error.status.is_client_error() {
                    error.message
                } else {
                    "The hub could not prepare this job".into()
                };
                fail(state, job.id, &why).await?;
            }
        }
        publish(state, &workers, job.id).await?;
    }
    Ok(())
}

/// Marks a job as running on a machine, with a new token, and returns what
/// the machine needs; a task's run, with the workshop agent `agent`.
async fn start(
    state: &AppState,
    job: &JobRow,
    worker_id: i64,
    agent: Option<AgentRow>,
) -> ApiResult<JobSpec> {
    let group = Membership::of_group(state, job.group_id).await?;
    let now = db::now();
    let mut task_profile = None;
    let mut branch = None;
    let mut repository = None;
    let work = match job.kind() {
        JobKind::Reply => Work::Reply(Box::new(conversation(state, &group, job).await?)),
        JobKind::LibraryBuild => {
            Work::LibraryBuild(library::work_for(state, &group, job.id).await?)
        }
        JobKind::PaperBuild => Work::PaperBuild(papers::work_for(state, job.id).await?),
        JobKind::TaskRun => {
            let agent = agent
                .as_ref()
                .ok_or_else(|| ApiError::internal("a task without its agent"))?;
            let profile = AgentProfile {
                runner: agent.runner.clone(),
                model: agent.model.clone(),
                args: agent.args(),
                ..workshop::profile()
            };
            let mut conversation = conversation(state, &group, job).await?;
            conversation.agent = profile.clone();
            let run = workshop::work_for(state, &group, job.id, conversation).await?;
            branch = Some(run.branch.clone());
            repository = run
                .repository
                .rsplit('/')
                .next()
                .map(|name| name.trim_end_matches(".git").to_owned());
            task_profile = Some(profile);
            Work::TaskRun(Box::new(run))
        }
    };
    let (name, lasts) = match job.kind() {
        JobKind::LibraryBuild => ("Lean".to_owned(), BUILD_TOKEN_MILLIS),
        JobKind::PaperBuild => ("LaTeX".to_owned(), BUILD_TOKEN_MILLIS),
        JobKind::TaskRun => (job.profile()?.display_name, BUILD_TOKEN_MILLIS),
        _ => (job.profile()?.display_name, JOB_TOKEN_MILLIS),
    };
    let (token_id, token) = auth::create_job_token(
        &state.db,
        job.requester_id,
        job.group_id,
        None,
        &name,
        now + lasts,
    )
    .await?;
    if let (Some(branch), Some(repository)) = (&branch, &repository) {
        auth::grant_task(
            &state.db,
            token_id,
            repository,
            &format!("refs/heads/{branch}"),
            TASK_THREADS,
        )
        .await?;
    }
    sqlx::query(
        "UPDATE jobs SET state = 'running', worker_id = ?, token_id = ?, attempts = attempts + 1,
                         progress = NULL, updated_at = ?
         WHERE id = ? AND state = 'queued'",
    )
    .bind(worker_id)
    .bind(token_id)
    .bind(now)
    .bind(job.id)
    .execute(&state.db)
    .await?;
    let (runner, model) = match (&task_profile, job.kind()) {
        (Some(profile), _) => (profile.runner.clone(), profile.model.clone()),
        (None, JobKind::LibraryBuild) => (LEAN.to_owned(), None),
        (None, JobKind::PaperBuild) => (papers::LATEX.to_owned(), None),
        (None, _) => {
            let profile = job.profile()?;
            (profile.runner, profile.model)
        }
    };
    consumption::started(
        state,
        consumption::Start {
            job_id: job.id,
            group_id: job.group_id,
            kind: job.kind(),
            worker_id,
            workshop_agent_id: agent.as_ref().map(|a| a.id),
            runner: &runner,
            model: model.as_deref(),
        },
    )
    .await?;
    if task_profile.is_some() {
        workshop::started(state, job.id, &runner, model.as_deref()).await?;
    }
    Ok(JobSpec {
        id: job.id,
        group: GroupRef {
            slug: group.slug.clone(),
            name: group.name.clone(),
        },
        token,
        hands_over: job.hands_over(),
        opens_threads: task_profile.is_some(),
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

/// A job a machine runs, if it is running on that machine.
async fn running_on(state: &AppState, worker_id: i64, job_id: i64) -> ApiResult<Option<JobRow>> {
    Ok(load_job(state, job_id)
        .await?
        .filter(|job| job.state() == JobState::Running && job.worker_id == Some(worker_id)))
}

/// What a machine says a job is doing.
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

/// How a machine says a job ended, and what it consumed: an answer is
/// posted in the thread, a build's report is kept, a task's run is
/// proposed and reported.
pub async fn finished(
    state: &AppState,
    worker_id: i64,
    job_id: i64,
    outcome: Outcome,
    usage: Usage,
) -> ApiResult<()> {
    {
        let mut workers = state.workers.lock().await;
        if let Some(worker) = workers.connected.get_mut(&worker_id) {
            worker.running.remove(&job_id);
        }
        let Some(job) = running_on(state, worker_id, job_id).await? else {
            return Ok(());
        };
        let failed = matches!(outcome, Outcome::Failed { .. });
        consumption::ended(
            state,
            job_id,
            if failed { "failed" } else { "done" },
            Some(&usage),
        )
        .await?;
        let failure = match (job.kind(), outcome) {
            (_, Outcome::Failed { error }) => Some(error.trim().chars().take(2_000).collect()),
            (JobKind::Reply, Outcome::Answer { body }) => answered(state, &job, &body).await?,
            (JobKind::LibraryBuild, Outcome::Built(built)) => {
                library::build_finished(state, job.group_id, job_id, *built).await?;
                done(state, &job, None).await?;
                None
            }
            (JobKind::PaperBuild, Outcome::PaperBuilt(built)) => {
                papers::build_finished(state, job_id, *built).await?;
                done(state, &job, None).await?;
                None
            }
            (JobKind::TaskRun, Outcome::Task(outcome)) => {
                let number = workshop::finished(state, job_id, *outcome).await?;
                done(state, &job, Some(number)).await?;
                None
            }
            _ => Some("The machine answered with something else than was asked".into()),
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

/// Registers a connected machine, replacing an older connection of it,
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

/// The groups a machine works for changed: its connection learns it, and
/// the jobs are dispatched again.
pub async fn workshop_changed(state: &AppState, worker_id: i64) -> ApiResult<()> {
    let groups = workshop::machines::groups_of(state, worker_id).await?;
    if let Some(worker) = state.workers.lock().await.connected.get_mut(&worker_id) {
        worker.workshop = groups;
    }
    dispatch(state).await
}

/// Forgets a machine's connection, if it is still the current one, and
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

/// Disconnects a machine, as when its token is revoked.
pub async fn disconnect(state: &AppState, worker_id: i64) -> ApiResult<()> {
    let gone = state.workers.lock().await.connected.remove(&worker_id);
    if let Some(gone) = gone {
        requeue(state, worker_id, gone.running).await?;
    }
    dispatch(state).await
}

/// Queues again the jobs a machine was running, unless they ran too often.
async fn requeue(state: &AppState, worker_id: i64, jobs: HashMap<i64, JobKind>) -> ApiResult<()> {
    for job_id in jobs.into_keys() {
        let Some(job) = running_on(state, worker_id, job_id).await? else {
            continue;
        };
        revoke_token(state, job.token_id).await?;
        consumption::ended(state, job_id, "requeued", None).await?;
        if job.attempts >= MAX_ATTEMPTS {
            fail(
                state,
                job_id,
                "The machines that took this job went away before finishing it",
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

/// After a restart no machine is connected: queue again what was running.
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
    sqlx::query(
        "UPDATE job_runs SET finished_at = ?, outcome = 'requeued' WHERE finished_at IS NULL",
    )
    .bind(db::now())
    .execute(&state.db)
    .await?;
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
    consumption::ended(state, job_id, "cancelled", None).await?;
    workshop::job_ended(state, job_id, true, None).await?;
    publish(state, &workers, job_id).await?;
    drop(workers);
    dispatch(state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paper_can_use_either_slot_and_release_normal_capacity() {
        let (sender, _) = mpsc::unbounded_channel();
        let mut machine = Connected {
            personal: true,
            connection: 1,
            owner_id: 1,
            runners: Vec::new(),
            lean: true,
            latex: true,
            isolation: Some(Isolation::Docker),
            workshop: HashSet::new(),
            concurrency: 1,
            preferred: false,
            running: HashMap::new(),
            sender,
        };
        assert!(machine.has_room(JobKind::PaperBuild));
        machine.running.insert(1, JobKind::PaperBuild);
        assert!(machine.has_room(JobKind::LibraryBuild));
        assert!(machine.has_room(JobKind::PaperBuild));
        machine.running.insert(2, JobKind::PaperBuild);
        assert!(!machine.has_room(JobKind::PaperBuild));
        assert!(!machine.has_room(JobKind::LibraryBuild));
        machine.running.remove(&2);
        machine.running.insert(3, JobKind::LibraryBuild);
        assert!(!machine.has_room(JobKind::PaperBuild));
        machine.running.remove(&1);
        assert!(machine.has_room(JobKind::PaperBuild));
        assert!(!machine.has_room(JobKind::Reply));
        machine.latex = false;
        assert!(!machine.has_room(JobKind::PaperBuild));
    }
}
