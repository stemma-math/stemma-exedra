//! Jobs: what agents are asked to do, queued in the database and handed to
//! the workers connected to the hub.
//!
//! A message that mentions an agent (`@claude`), or any message in a chat
//! with a resident agent, creates a job per agent. A job waits until a
//! worker that can run it is connected and free: for an agent of personal
//! scope, a worker of the person who asked; for one of group scope, a
//! worker of the group. The worker's answer is posted in the thread as a
//! message by the agent, for that person.
//!
//! Jobs live in the database, so they survive the hub's restarts; a job
//! whose worker goes away is queued again, up to [`MAX_ATTEMPTS`] times.

use std::collections::{HashMap, HashSet};

use sqlx::FromRow;
use stemma_core::api::{
    AgentProfile, AgentScope, Event, Job, JobState, Message, Thread, ThreadKind, User,
};
use stemma_core::protocol::{GroupRef, HubMessage, JobKind, JobSpec, Outcome, ThreadRef};
use tokio::sync::mpsc;

use crate::api::Membership;
use crate::api::forum;
use crate::auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// How many times a job is handed to a worker before it fails.
pub const MAX_ATTEMPTS: i64 = 3;

/// How long a job's token lasts, in milliseconds.
const JOB_TOKEN_MILLIS: i64 = 2 * 60 * 60 * 1000;

/// About how many characters of a thread an agent is given.
const CONTEXT_BUDGET: usize = 60_000;

/// The most an answer may be, as a message.
const MAX_ANSWER_CHARS: usize = 100_000;

/// The workers connected now.
#[derive(Default)]
pub struct Workers {
    connected: HashMap<i64, Connected>,
}

/// A connected worker.
pub struct Connected {
    /// Which connection this is, so that a stale one does not remove a
    /// newer one of the same worker.
    pub connection: u64,
    pub group_id: Option<i64>,
    pub owner_id: Option<i64>,
    pub runners: Vec<String>,
    pub concurrency: u32,
    pub running: HashSet<i64>,
    pub sender: mpsc::UnboundedSender<HubMessage>,
}

impl Workers {
    pub fn is_online(&self, worker_id: i64) -> bool {
        self.connected.contains_key(&worker_id)
    }

    fn can_run(worker: &Connected, job: &JobRow) -> bool {
        worker.runners.contains(&job.runner)
            && match AgentScope::parse(&job.scope) {
                Some(AgentScope::Personal) => worker.owner_id == Some(job.requester_id),
                Some(AgentScope::Group) => worker.group_id == Some(job.group_id),
                None => false,
            }
    }

    /// A connected worker that can run `job` and has room for it.
    fn free_for(&self, job: &JobRow) -> Option<i64> {
        self.connected
            .iter()
            .filter(|(_, w)| Self::can_run(w, job) && (w.running.len() as u32) < w.concurrency)
            .min_by_key(|(_, w)| w.running.len())
            .map(|(id, _)| *id)
    }

    /// Why a queued job waits.
    fn note(&self, job: &JobRow) -> String {
        let any = self.connected.values().any(|w| Self::can_run(w, job));
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
        }
    }
}

macro_rules! profile_query {
    () => {
        "SELECT handle, display_name, runner, model, endpoint, instructions, scope
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

/// The agents every group starts with: each person's own Claude and Codex.
pub fn default_profiles() -> Vec<AgentProfile> {
    let personal = |handle: &str, name: &str, runner: &str| AgentProfile {
        handle: handle.into(),
        display_name: name.into(),
        runner: runner.into(),
        model: None,
        endpoint: None,
        instructions: None,
        scope: AgentScope::Personal,
    };
    vec![
        personal("claude", "Claude", "claude-code"),
        personal("codex", "Codex", "codex"),
    ]
}

// ── Jobs ──────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct JobRow {
    id: i64,
    group_id: i64,
    thread_id: i64,
    thread_number: i64,
    trigger_message: i64,
    requester_id: i64,
    username: String,
    display_name: String,
    agent_handle: String,
    agent: String,
    runner: String,
    scope: String,
    state: String,
    worker_id: Option<i64>,
    token_id: Option<i64>,
    attempts: i64,
    progress: Option<String>,
    error: Option<String>,
    answer: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

macro_rules! job_query {
    () => {
        "SELECT j.id, j.group_id, j.thread_id, t.number AS thread_number,
        j.trigger_message, j.requester_id, u.username, u.display_name, j.agent_handle, j.agent,
        j.runner, j.scope, j.state, j.worker_id, j.token_id, j.attempts, j.progress, j.error,
        j.answer, j.created_at, j.updated_at
         FROM jobs j JOIN threads t ON t.id = j.thread_id JOIN users u ON u.id = j.requester_id"
    };
}

impl JobRow {
    fn state(&self) -> JobState {
        JobState::parse(&self.state).unwrap_or(JobState::Failed)
    }

    fn profile(&self) -> ApiResult<AgentProfile> {
        serde_json::from_str(&self.agent).map_err(ApiError::internal)
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
        let agent_name = self
            .profile()
            .map(|p| p.display_name)
            .unwrap_or_else(|_| self.agent_handle.clone());
        Job {
            id: self.id,
            thread: self.thread_number as u32,
            trigger: self.trigger_message as u32,
            agent: self.agent_handle.clone(),
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

/// Tells the job's group how it is now.
async fn publish(state: &AppState, workers: &Workers, job_id: i64) -> ApiResult<()> {
    if let Some(row) = load_job(state, job_id).await? {
        state.publish(
            row.group_id,
            Event::JobUpdated {
                job: row.to_api(workers),
            },
        );
    }
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

/// Asks the agents a message summons to answer it: those it mentions, and
/// in a chat, its resident agent. Messages written by agents summon none.
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
        let now = db::now();
        sqlx::query(
            "INSERT INTO jobs (group_id, thread_id, kind, trigger_message, requester_id,
                               agent_handle, agent, runner, scope, state, created_at, updated_at)
             VALUES (?, ?, 'reply', ?, ?, ?, ?, ?, ?, 'queued', ?, ?)",
        )
        .bind(group.group_id)
        .bind(thread_id)
        .bind(message.number)
        .bind(message.author.id)
        .bind(&profile.handle)
        .bind(serde_json::to_string(&profile).map_err(ApiError::internal)?)
        .bind(&profile.runner)
        .bind(profile.scope.as_str())
        .bind(now)
        .bind(now)
        .execute(&state.db)
        .await?;
        created = true;
    }
    if created {
        dispatch(state).await?;
    }
    Ok(())
}

/// Hands the queued jobs to the workers that can run them, oldest first,
/// and tells everyone how the queued ones are.
pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    let mut workers = state.workers.lock().await;
    let queued: Vec<JobRow> = sqlx::query_as(concat!(
        job_query!(),
        " WHERE j.state = 'queued' ORDER BY j.id LIMIT 500"
    ))
    .fetch_all(&state.db)
    .await?;
    for job in queued {
        let Some(worker_id) = workers.free_for(&job) else {
            state.publish(
                job.group_id,
                Event::JobUpdated {
                    job: job.to_api(&workers),
                },
            );
            continue;
        };
        match start(state, &job, worker_id).await {
            Ok(spec) => {
                let worker = workers
                    .connected
                    .get_mut(&worker_id)
                    .expect("found just now");
                worker.running.insert(job.id);
                let _ = worker.sender.send(HubMessage::Job(Box::new(spec)));
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
/// the worker needs.
async fn start(state: &AppState, job: &JobRow, worker_id: i64) -> ApiResult<JobSpec> {
    let profile = job.profile()?;
    let now = db::now();
    let (token_id, token) = auth::create_job_token(
        &state.db,
        job.requester_id,
        job.group_id,
        None,
        &profile.display_name,
        now + JOB_TOKEN_MILLIS,
    )
    .await?;
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

    let group = Membership::of_group(state, job.group_id).await?;
    let thread: Thread = forum::find_thread_by_id(state, job.thread_id).await?.into();
    let messages = forum::load_messages(state, &group, job.thread_id, 1, u32::MAX).await?;
    let trigger = job.trigger_message as u32;
    Ok(JobSpec {
        id: job.id,
        kind: JobKind::Reply,
        agent: profile,
        group: GroupRef {
            slug: group.slug.clone(),
            name: group.name.clone(),
        },
        context: stemma_core::context::trim(&messages, thread.number, trigger, CONTEXT_BUDGET),
        thread: ThreadRef {
            number: thread.number,
            title: thread.title,
            kind: thread.kind,
        },
        trigger,
        requester: job.requester(),
        token,
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
    revoke_token(state, token.flatten()).await
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

/// How a worker says a job ended: an answer is posted in the thread.
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
        let answer = match outcome {
            Outcome::Answer { body } if !body.trim().is_empty() => body,
            Outcome::Answer { .. } => {
                fail(state, job_id, "The agent gave no answer").await?;
                publish(state, &workers, job_id).await?;
                drop(workers);
                return dispatch(state).await;
            }
            Outcome::Failed { error } => {
                let error: String = error.trim().chars().take(2_000).collect();
                fail(state, job_id, &error).await?;
                publish(state, &workers, job_id).await?;
                drop(workers);
                return dispatch(state).await;
            }
        };
        let mut text: String = forum::check_body(&answer).unwrap_or_default();
        if text.chars().count() >= MAX_ANSWER_CHARS {
            text = text.chars().take(MAX_ANSWER_CHARS - 40).collect();
            text.push_str("\n\n[The answer was cut here.]");
        }
        let profile = job.profile()?;
        let group = Membership::of_group(state, job.group_id).await?;
        let message = forum::post(
            state,
            &group,
            job.thread_id,
            job.requester_id,
            Some(&profile.display_name),
            &text,
            Some(job.trigger_message as u32),
            &[],
        )
        .await?;
        sqlx::query(
            "UPDATE jobs SET state = 'done', answer = ?, progress = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(message.number)
        .bind(db::now())
        .bind(job_id)
        .execute(&state.db)
        .await?;
        revoke_token(state, job.token_id).await?;
        publish(state, &workers, job_id).await?;
    }
    dispatch(state).await
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
    let mut workers = state.workers.lock().await;
    let job = load_job(state, job_id)
        .await?
        .filter(|job| job.group_id == group.group_id)
        .ok_or_else(|| ApiError::not_found("Job"))?;
    if job.requester_id != user_id {
        group.require_admin()?;
    }
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
    publish(state, &workers, job_id).await?;
    drop(workers);
    dispatch(state).await
}
