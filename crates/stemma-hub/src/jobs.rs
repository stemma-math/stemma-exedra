//! Personal reply jobs, queued until a suitable live computer can run them.

use std::collections::HashMap;

use sqlx::FromRow;
use stemma_core::api::{
    AgentProfile, Event, Job, JobKind, JobState, Message, Thread, ThreadKind, User,
};
use stemma_core::protocol::{
    Conversation, GroupRef, HubMessage, JobSpec, Outcome, ThreadRef, Usage, Work,
};
use tokio::sync::mpsc;

use crate::api::Membership;
use crate::api::forum;
use crate::auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use stemma_core::secrets;

/// How long a job's token lasts, in milliseconds.
const JOB_TOKEN_MILLIS: i64 = 2 * 60 * 60 * 1000;

/// About how many characters of a thread an agent is given.
const CONTEXT_BUDGET: usize = 60_000;

/// The most an answer may be, as a message.
const MAX_ANSWER_CHARS: usize = 100_000;

/// Live connections, keyed only by personal token id.
#[derive(Default)]
pub struct Workers {
    connected: HashMap<i64, Connected>,
}
pub struct Connected {
    pub connection: u64,
    pub owner_id: i64,
    pub runners: Vec<String>,
    pub groups: std::collections::BTreeMap<String, stemma_core::protocol::GroupAgents>,
    pub agents: stemma_core::protocol::GroupAgents,
    pub concurrency: u32,
    pub running: HashMap<i64, JobKind>,
    pub sender: mpsc::UnboundedSender<HubMessage>,
}
impl Connected {
    fn scope(&self, group: &str) -> &stemma_core::protocol::GroupAgents {
        self.groups.get(group).unwrap_or(&self.agents)
    }
}
impl Workers {
    pub fn runners(&self, token: i64) -> Vec<String> {
        self.connected
            .get(&token)
            .map(|w| {
                let mut runners = w.runners.clone();
                for group in w.groups.values() {
                    if let Some(found) = &group.runners {
                        for runner in found {
                            if !runners.contains(runner) {
                                runners.push(runner.clone());
                            }
                        }
                    }
                }
                runners
            })
            .unwrap_or_default()
    }
    pub fn is_online(&self, token: i64) -> bool {
        self.connected.contains_key(&token)
    }
    fn free_for(&self, job: &JobRow) -> Vec<i64> {
        let mut free: Vec<_> = self
            .connected
            .iter()
            .filter(|(_, w)| {
                w.owner_id == job.requester_id
                    && w.scope(&job.group_slug).enabled
                    && w.scope(&job.group_slug)
                        .runners
                        .as_ref()
                        .unwrap_or(&w.runners)
                        .contains(&job.runner)
                    && w.running.len()
                        < w.scope(&job.group_slug)
                            .concurrency
                            .unwrap_or(w.concurrency) as usize
            })
            .collect();
        free.sort_by(|(_, a), (_, b)| {
            b.scope(&job.group_slug)
                .priority
                .cmp(&a.scope(&job.group_slug).priority)
                .then(b.connection.cmp(&a.connection))
        });
        free.into_iter().map(|(id, _)| *id).collect()
    }
    fn note(&self, job: &JobRow) -> String {
        format!("Waiting for one of {}'s machines", job.display_name)
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
    runner: String,
    state: String,
    worker_id: Option<i64>,
    machine_name: Option<String>,
    token_id: Option<i64>,
    progress: Option<String>,
    error: Option<String>,
    answer: Option<i64>,
    held_answer: Option<String>,
    created_at: i64,
    updated_at: i64,
    group_slug: String,
}

macro_rules! job_query {
    () => {
        "SELECT j.id, j.group_id, j.kind, j.thread_id, t.number AS thread_number,
        j.trigger_message, j.requester_id, u.username, u.display_name, j.agent_handle,
        j.runner, j.state, j.worker_id, w.name AS machine_name, j.token_id, j.progress, j.error,
        j.answer, j.held_answer, j.created_at, j.updated_at, g.slug AS group_slug
         FROM jobs j LEFT JOIN threads t ON t.id = j.thread_id
              JOIN users u ON u.id = j.requester_id
              JOIN groups g ON g.id = j.group_id
              LEFT JOIN api_tokens w ON w.id = j.worker_id"
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
        stemma_core::api::AgentProfile::default_for(
            self.agent_handle.as_deref().unwrap_or_default(),
        )
        .map(|a| a.profile())
        .ok_or_else(|| ApiError::internal("unknown agent"))
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
        let profile = self.profile().expect("fixed agent");
        let (agent, agent_name) = (profile.handle, profile.display_name);
        Job {
            id: self.id,
            machine: self.machine_name.clone(),
            kind: self.kind(),
            thread: self.thread_number.map(|n| n as u32),
            trigger: self.trigger_message.map(|n| n as u32),
            agent,
            agent_name: agent_name.clone(),
            requester: self.requester(),
            state,
            progress: self.progress.clone().filter(|_| state == JobState::Running),
            note: match state {
                JobState::Queued => Some(workers.note(self)),
                JobState::Held => Some(format!(
                    "{}'s answer is waiting for {}",
                    agent_name, self.display_name
                )),
                _ => None,
            },
            error: self.error.clone(),
            answer: self.answer.map(|n| n as u32),
            // Only ever given to its requester: see [`of_thread`].
            held: None,
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

/// Tells the job's group how it is now. Never with a held answer's text.
async fn publish(state: &AppState, workers: &Workers, job_id: i64) -> ApiResult<()> {
    let Some(row) = load_job(state, job_id).await? else {
        return Ok(());
    };
    let job = row.to_api(workers);
    state.publish(row.group_id, Event::JobUpdated { job });
    Ok(())
}

/// The thread's jobs on their way, held, or failed lately, as `viewer_id`
/// sees them, failed ones for a day: a held answer's text only for its
/// requester.
pub async fn of_thread(state: &AppState, thread_id: i64, viewer_id: i64) -> ApiResult<Vec<Job>> {
    let rows: Vec<JobRow> = sqlx::query_as(concat!(
        job_query!(),
        " WHERE j.thread_id = ?
           AND (j.state IN ('queued', 'running', 'held')
                OR (j.state = 'failed' AND j.updated_at > ?))
         ORDER BY j.id"
    ))
    .bind(thread_id)
    .bind(db::now() - 24 * 60 * 60 * 1000)
    .fetch_all(&state.db)
    .await?;
    let workers = state.workers.lock().await;
    Ok(rows
        .iter()
        .map(|row| {
            let mut job = row.to_api(&workers);
            if row.requester_id == viewer_id {
                job.held = row.held_answer.as_deref().map(secrets::parts);
            }
            job
        })
        .collect())
}

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
        let Some(agent) = AgentProfile::default_for(&handle) else {
            continue;
        };
        let profile = agent.profile();
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
                           agent_handle, runner, state, error, created_at, updated_at)
         VALUES (?, ?, 'reply', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(thread_id)
    .bind(trigger)
    .bind(requester_id)
    .bind(&profile.handle)
    .bind(&profile.runner)
    .bind(if error.is_some() { "failed" } else { "queued" })
    .bind(error)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid())
}

/// Hands the queued jobs to the machines that can run them, oldest first,
/// and tells everyone how the queued ones are.
pub async fn dispatch(state: &AppState) -> ApiResult<()> {
    if state.shutdown.is_cancelled() {
        return Ok(());
    }
    let mut workers = state.workers.lock().await;
    let queued: Vec<JobRow> = sqlx::query_as(concat!(
        job_query!(),
        " WHERE j.state = 'queued' ORDER BY j.id"
    ))
    .fetch_all(&state.db)
    .await?;
    for job in queued {
        let Some(worker_id) = workers.free_for(&job).first().copied() else {
            publish(state, &workers, job.id).await?;
            continue;
        };
        match start(state, &job, worker_id).await {
            Ok(spec) => {
                let worker = workers
                    .connected
                    .get_mut(&worker_id)
                    .expect("found just now");
                worker.running.insert(job.id, job.kind());
                let _ = worker.sender.send(HubMessage::Job(Box::new(spec)));
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
async fn start(state: &AppState, job: &JobRow, worker_id: i64) -> ApiResult<JobSpec> {
    let group = Membership::of_group(state, job.group_id).await?;
    let work = Work::Reply(Box::new(conversation(state, &group, job).await?));
    let (token_id, token) = auth::create_job_token(
        &state.db,
        job.requester_id,
        job.group_id,
        &job.profile()?.display_name,
        db::now() + JOB_TOKEN_MILLIS,
    )
    .await?;
    sqlx::query("UPDATE jobs SET state = 'running', worker_id = ?, token_id = ?, attempts = attempts + 1, progress = NULL, updated_at = ? WHERE id = ? AND state = 'queued'")
        .bind(worker_id).bind(token_id).bind(db::now()).bind(job.id).execute(&state.db).await?;
    Ok(JobSpec {
        id: job.id,
        group: GroupRef {
            slug: group.slug,
            name: group.name,
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
    // An error may end with what a runner printed: never a secret of it.
    let token: Option<Option<i64>> = sqlx::query_scalar(
        "UPDATE jobs SET state = 'failed', error = ?, progress = NULL, updated_at = ?
         WHERE id = ? RETURNING token_id",
    )
    .bind(secrets::hide(error))
    .bind(db::now())
    .bind(job_id)
    .fetch_optional(&state.db)
    .await?;
    revoke_token(state, token.flatten()).await?;
    Ok(())
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
pub async fn progress(
    state: &AppState,
    worker_id: i64,
    connection: u64,
    job_id: i64,
    text: &str,
) -> ApiResult<()> {
    let workers = state.workers.lock().await;
    if !workers
        .connected
        .get(&worker_id)
        .is_some_and(|w| w.connection == connection)
        || running_on(state, worker_id, job_id).await?.is_none()
    {
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
pub async fn finished(
    state: &AppState,
    worker_id: i64,
    connection: u64,
    job_id: i64,
    outcome: Outcome,
    usage: Usage,
) -> ApiResult<()> {
    {
        let mut workers = state.workers.lock().await;
        if !workers
            .connected
            .get(&worker_id)
            .is_some_and(|w| w.connection == connection)
        {
            return Ok(());
        }
        if let Some(worker) = workers.connected.get_mut(&worker_id) {
            worker.running.remove(&job_id);
        }
        let Some(job) = running_on(state, worker_id, job_id).await? else {
            return Ok(());
        };
        let _ = usage;
        let failure = match outcome {
            Outcome::Failed { error } => Some(error.trim().chars().take(2_000).collect()),
            Outcome::Answer { body } => answered(state, &job, &body).await?,
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
    if !secrets::find(&text).is_empty() {
        hold(state, job, &text).await?;
        return Ok(None);
    }
    let number = say(state, job, &text).await?;
    done(state, job, Some(number)).await?;
    Ok(None)
}

/// Keeps an answer that seems to hold a secret, unposted, for its
/// requester to decide.
async fn hold(state: &AppState, job: &JobRow, text: &str) -> ApiResult<()> {
    sqlx::query(
        "UPDATE jobs SET state = 'held', held_answer = ?, progress = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(text)
    .bind(db::now())
    .bind(job.id)
    .execute(&state.db)
    .await?;
    revoke_token(state, job.token_id).await
}

/// A held job of `group`'s, which only its requester decides on.
async fn held_by(
    state: &AppState,
    group: &Membership,
    user_id: i64,
    job_id: i64,
) -> ApiResult<(JobRow, String)> {
    let job = load_job(state, job_id)
        .await?
        .filter(|job| job.group_id == group.group_id && job.requester_id == user_id)
        .ok_or_else(|| ApiError::not_found("Job"))?;
    match (job.state(), job.held_answer.clone()) {
        (JobState::Held, Some(text)) => Ok((job, text)),
        _ => Err(ApiError::conflict("This answer is not held")),
    }
}

/// Posts a held answer, as it is or with its secrets hidden: its
/// requester only.
pub async fn publish_held(
    state: &AppState,
    group: &Membership,
    user_id: i64,
    job_id: i64,
    hidden: bool,
) -> ApiResult<()> {
    let (job, text) = held_by(state, group, user_id, job_id).await?;
    let text = if hidden { secrets::hide(&text) } else { text };
    let number = say(state, &job, &text).await?;
    sqlx::query(
        "UPDATE jobs SET state = 'done', answer = ?, held_answer = NULL, updated_at = ?
         WHERE id = ?",
    )
    .bind(number)
    .bind(db::now())
    .bind(job.id)
    .execute(&state.db)
    .await?;
    let workers = state.workers.lock().await;
    publish(state, &workers, job.id).await
}

/// Discards a held answer: its requester only.
pub async fn discard_held(
    state: &AppState,
    group: &Membership,
    user_id: i64,
    job_id: i64,
) -> ApiResult<()> {
    let (job, _) = held_by(state, group, user_id, job_id).await?;
    sqlx::query(
        "UPDATE jobs SET state = 'cancelled', held_answer = NULL, updated_at = ? WHERE id = ?",
    )
    .bind(db::now())
    .bind(job.id)
    .execute(&state.db)
    .await?;
    let workers = state.workers.lock().await;
    publish(state, &workers, job.id).await
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
        Some(&profile.runner),
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
    let old = {
        let mut workers = state.workers.lock().await;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_tokens WHERE id = ? AND user_id = ? AND kind = 'personal' AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at > ?))").bind(worker_id).bind(worker.owner_id).bind(db::now()).fetch_one(&state.db).await?;
        if !valid {
            return Err(ApiError::unauthenticated());
        }
        workers.connected.insert(worker_id, worker)
    };
    if let Some(old) = old {
        requeue(state, worker_id, old.running).await?;
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
        sqlx::query("UPDATE jobs SET state = 'queued', worker_id = NULL, token_id = NULL, progress = NULL, updated_at = ? WHERE id = ?")
            .bind(db::now()).bind(job_id).execute(&state.db).await?;
    }
    Ok(())
}

/// After a restart no machine is connected: queue again what was running.
pub async fn recover(state: &AppState) -> anyhow::Result<()> {
    let tokens: Vec<Option<i64>> =
        sqlx::query_scalar("SELECT token_id FROM jobs WHERE state = 'running'")
            .fetch_all(&state.db)
            .await?;
    sqlx::query("UPDATE jobs SET state = 'queued', worker_id = NULL, token_id = NULL, progress = NULL WHERE state = 'running'").execute(&state.db).await?;
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
        "UPDATE jobs SET state = 'cancelled', progress = NULL, held_answer = NULL, updated_at = ?
         WHERE id = ?",
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

/// Replace the capabilities of the current connection without reconnecting.
pub async fn hello(
    state: &AppState,
    token: i64,
    connection: u64,
    hello: stemma_core::protocol::Hello,
) -> ApiResult<()> {
    if let Some(worker) = state.workers.lock().await.connected.get_mut(&token)
        && worker.connection == connection
    {
        worker.runners = hello.runners;
        worker.agents = hello.agents;
        worker.groups = hello.groups;
        worker.concurrency = hello.concurrency.max(1);
    }
    dispatch(state).await
}
