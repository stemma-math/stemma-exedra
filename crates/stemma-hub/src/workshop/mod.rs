//! The workshop: where the group's repositories are changed by agents, in
//! the background, the only place where they are.
//!
//! Conversations ask it for work. A task is handed over in conversation:
//! an agent answering in a thread calls `create_task` when someone asks it
//! to ("formalize this"), or a message mentions the workshop
//! (`@workshop formalize the lemma in t12#48`). Only admins and members
//! ask for work: guests cannot. Each task runs with an architecture (see
//! [`crate::architectures`]): a kind of work the workshop knows.
//!
//! A task is a job, `task_run`, for one of the group's workshop machines
//! (see [`machines`]), with Lean, run with the first of the machine's
//! agents under its daily limit. A group's tasks run one at a time.
//!
//! The machine pushes what it did to the task's branch, `workshop/w7`,
//! with the job's token, which pushes nowhere else; the hub opens a pull
//! request in the forge, and reports in the thread the task came from.
//! The changes go in only once approved (see [`review`]).

pub mod machines;
pub mod review;

use sqlx::FromRow;
use stemma_core::Address;
use stemma_core::api::{
    AgentProfile, Approval, Changes, Event, Task, TaskPage, TaskState, User, WORKSHOP,
};
use stemma_core::protocol::{Conversation, TaskEnd, TaskOutcome, TaskRun};

use crate::api::{Membership, forum};
use crate::architectures;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge;
use crate::jobs;
use crate::library;
use crate::state::AppState;

/// The most of a run's log that is kept, in bytes: its end.
const MAX_LOG: usize = 200_000;

/// The most of a runner's report that goes into the thread, in characters.
const MAX_REPORT_CHARS: usize = 20_000;

/// How many changed declarations a report names.
const MAX_CHANGED: usize = 40;

/// The branch a task's changes are pushed to.
pub fn branch_of(number: u32) -> String {
    format!("workshop/w{number}")
}

/// The workshop, as an agent: what signs its messages and commits. Its
/// runner is that of the machine's agent that runs a task.
pub fn profile() -> AgentProfile {
    AgentProfile {
        handle: WORKSHOP.into(),
        display_name: "Workshop".into(),
        runner: WORKSHOP.into(),
        model: None,
        args: Vec::new(),
    }
}

// ── Tasks ─────────────────────────────────────────────────────────────

#[derive(FromRow)]
pub(crate) struct TaskRow {
    pub id: i64,
    pub group_id: i64,
    pub number: i64,
    title: String,
    instructions: String,
    pub architecture: String,
    pub thread_id: i64,
    thread_number: i64,
    pub origin_message: i64,
    pub user_id: i64,
    username: String,
    display_name: String,
    agent: Option<String>,
    state: String,
    pub job_id: Option<i64>,
    runner: Option<String>,
    model: Option<String>,
    reason: Option<String>,
    pub repository: Option<String>,
    pub branch: Option<String>,
    commit_sha: Option<String>,
    pub pull: Option<i64>,
    pull_url: Option<String>,
    pub big: bool,
    why_big: String,
    merged_commit: Option<String>,
    rejected_by: Option<i64>,
    rejected_username: Option<String>,
    rejected_display_name: Option<String>,
    report_message: Option<i64>,
    created_at: i64,
    updated_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    job_progress: Option<String>,
}

macro_rules! task_query {
    () => {
        "SELECT k.id, k.group_id, k.number, k.title, k.instructions, k.architecture, k.thread_id,
                t.number AS thread_number, k.origin_message, u.id AS user_id, u.username,
                u.display_name, k.agent, k.state, k.job_id, k.runner, k.model, k.reason,
                k.repository, k.branch, k.commit_sha, k.pull, k.pull_url, k.big, k.why_big,
                k.merged_commit, k.rejected_by, r.username AS rejected_username,
                r.display_name AS rejected_display_name, k.report_message, k.created_at,
                k.updated_at, k.started_at, k.finished_at, j.progress AS job_progress
         FROM tasks k JOIN threads t ON t.id = k.thread_id
              JOIN users u ON u.id = k.requester_id
              LEFT JOIN users r ON r.id = k.rejected_by
              LEFT JOIN jobs j ON j.id = k.job_id"
    };
}

impl TaskRow {
    pub fn state(&self) -> TaskState {
        TaskState::parse(&self.state).unwrap_or(TaskState::Failed)
    }

    fn origin(&self) -> Address {
        Address::Message {
            thread: self.thread_number as u32,
            message: self.origin_message as u32,
        }
    }

    pub fn requester(&self) -> User {
        User {
            id: self.user_id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }

    /// The task as people see it; `note` says why it waits, if it does.
    fn to_api(&self, note: Option<String>, approvals: Vec<Approval>) -> Task {
        let state = self.state();
        let changes = match (&self.branch, &self.commit_sha, self.pull, &self.pull_url) {
            (Some(branch), Some(commit), Some(pull), Some(url)) => Some(Changes {
                repository: self.repository.clone().unwrap_or_default(),
                branch: branch.clone(),
                commit: commit.clone(),
                pull: pull as u32,
                url: url.clone(),
                big: self.big,
                why_big: serde_json::from_str(&self.why_big).unwrap_or_default(),
                needs: if state == TaskState::Review {
                    review::missing(self, &approvals)
                } else {
                    Vec::new()
                },
                approvals,
                rejected_by: match (
                    self.rejected_by,
                    &self.rejected_username,
                    &self.rejected_display_name,
                ) {
                    (Some(id), Some(username), Some(display_name)) => Some(User {
                        id,
                        username: username.clone(),
                        display_name: display_name.clone(),
                    }),
                    _ => None,
                },
                merged: self.merged_commit.clone(),
            }),
            _ => None,
        };
        Task {
            number: self.number as u32,
            title: self.title.clone(),
            instructions: self.instructions.clone(),
            architecture: self.architecture.clone(),
            origin: self.origin(),
            requester: self.requester(),
            agent: self.agent.clone(),
            state,
            runner: self.runner.clone(),
            model: self.model.clone(),
            progress: match state {
                TaskState::Running => self.job_progress.clone(),
                TaskState::Open => note,
                _ => None,
            },
            reason: self.reason.clone(),
            changes,
            report_message: self.report_message.map(|message| Address::Message {
                thread: self.thread_number as u32,
                message: message as u32,
            }),
            created_at: db::from_millis(self.created_at),
            updated_at: db::from_millis(self.updated_at),
            started_at: self.started_at.map(db::from_millis),
            finished_at: self.finished_at.map(db::from_millis),
        }
    }

    async fn to_api_noted(&self, state: &AppState) -> ApiResult<Task> {
        let note = match (self.state(), self.job_id) {
            (TaskState::Open, Some(job)) => jobs::note_of(state, job).await?,
            _ => None,
        };
        let approvals = review::approvals(state, self.id).await?;
        Ok(self.to_api(note, approvals))
    }
}

pub(crate) async fn find(state: &AppState, group_id: i64, number: u32) -> ApiResult<TaskRow> {
    sqlx::query_as(concat!(
        task_query!(),
        " WHERE k.group_id = ? AND k.number = ?"
    ))
    .bind(group_id)
    .bind(number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Task"))
}

async fn by_id(state: &AppState, task_id: i64) -> ApiResult<Option<TaskRow>> {
    Ok(sqlx::query_as(concat!(task_query!(), " WHERE k.id = ?"))
        .bind(task_id)
        .fetch_optional(&state.db)
        .await?)
}

async fn of_job(state: &AppState, job_id: i64) -> ApiResult<Option<TaskRow>> {
    Ok(
        sqlx::query_as(concat!(task_query!(), " WHERE k.job_id = ?"))
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?,
    )
}

/// Tells the task's group how the task of a job is, if the job runs one;
/// `note` is why the job waits, as the job's own news says.
pub async fn publish_of_job(state: &AppState, job_id: i64, note: Option<String>) -> ApiResult<()> {
    if let Some(row) = of_job(state, job_id).await? {
        let approvals = review::approvals(state, row.id).await?;
        state.publish(
            row.group_id,
            Event::TaskUpdated {
                task: Box::new(row.to_api(note, approvals)),
            },
        );
    }
    Ok(())
}

/// Tells the task's group how it is. It does not say why an open task
/// waits: the news of its job, which follows, does. Callers may hold the
/// workers' lock.
pub(crate) async fn publish(state: &AppState, task_id: i64) -> ApiResult<()> {
    if let Some(row) = by_id(state, task_id).await? {
        let approvals = review::approvals(state, row.id).await?;
        let task = row.to_api(None, approvals);
        state.publish(
            row.group_id,
            Event::TaskUpdated {
                task: Box::new(task),
            },
        );
    }
    Ok(())
}

/// The group's tasks, newest first; only those in `only`, if given.
pub async fn list(
    state: &AppState,
    group_id: i64,
    only: Option<TaskState>,
    limit: u32,
) -> ApiResult<Vec<Task>> {
    let rows: Vec<TaskRow> = sqlx::query_as(concat!(
        task_query!(),
        " WHERE k.group_id = ?1 AND (?2 IS NULL OR k.state = ?2) ORDER BY k.number DESC LIMIT ?3"
    ))
    .bind(group_id)
    .bind(only.map(TaskState::as_str))
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    let mut tasks = Vec::with_capacity(rows.len());
    for row in rows {
        tasks.push(row.to_api_noted(state).await?);
    }
    Ok(tasks)
}

/// A task, with its report and its log.
pub async fn page(state: &AppState, group_id: i64, number: u32) -> ApiResult<TaskPage> {
    let row = find(state, group_id, number).await?;
    let (report, log): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT report, log FROM tasks WHERE id = ?")
            .bind(row.id)
            .fetch_one(&state.db)
            .await?;
    Ok(TaskPage {
        task: row.to_api_noted(state).await?,
        report,
        log: log.unwrap_or_default(),
    })
}

/// What a task's address resolves to: whether it exists, and its title.
pub async fn resolve(
    state: &AppState,
    group_id: i64,
    number: u32,
) -> ApiResult<(bool, Option<String>)> {
    match find(state, group_id, number).await {
        Ok(row) => Ok((true, Some(format!("w{}: {}", row.number, row.title)))),
        Err(error) if error.status == axum::http::StatusCode::NOT_FOUND => Ok((false, None)),
        Err(error) => Err(error),
    }
}

/// A task as it is handed over.
pub struct NewTask {
    pub title: String,
    pub instructions: String,
    /// The architecture to run it with; the default if none.
    pub architecture: Option<String>,
    /// The thread and the number of the message that asks.
    pub thread_id: i64,
    pub origin_message: u32,
    pub requester_id: i64,
    /// The agent handing it over for its requester, if one does.
    pub agent: Option<String>,
}

/// Hands a task to the workshop, and queues its run; the caller then
/// dispatches. Returns the task. `group` is the requester's membership:
/// guests cannot ask for work.
pub async fn create(state: &AppState, group: &Membership, new: NewTask) -> ApiResult<Task> {
    if !group.role.can_ask_for_work() {
        return Err(ApiError::forbidden(
            "Guests cannot ask the workshop for work",
        ));
    }
    let architecture = new
        .architecture
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .unwrap_or(architectures::DEFAULT);
    let Some(architecture) = architectures::find(architecture) else {
        let known: Vec<String> = architectures::all().into_iter().map(|a| a.name).collect();
        return Err(ApiError::invalid(format!(
            "The workshop has no architecture {architecture}: it knows {}",
            known.join(", ")
        )));
    };
    let title = crate::api::required_text(&new.title, "The title", 200)?;
    let instructions = crate::api::required_text(&new.instructions, "The instructions", 20_000)?;
    let agent = new
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty());
    let profile = profile();
    let now = db::now();
    let mut tx = state.db.begin().await?;
    let number: i64 = sqlx::query_scalar(
        "UPDATE groups SET last_task = last_task + 1 WHERE id = ? RETURNING last_task",
    )
    .bind(group.group_id)
    .fetch_one(&mut *tx)
    .await?;
    let job_id = sqlx::query(
        "INSERT INTO jobs (group_id, thread_id, kind, trigger_message, requester_id, agent_handle,
                           agent, runner, state, created_at, updated_at)
         VALUES (?, ?, 'task_run', ?, ?, ?, ?, ?, 'queued', ?, ?)",
    )
    .bind(group.group_id)
    .bind(new.thread_id)
    .bind(new.origin_message)
    .bind(new.requester_id)
    .bind(&profile.handle)
    .bind(serde_json::to_string(&profile).map_err(ApiError::internal)?)
    .bind(&profile.runner)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    let task_id = sqlx::query(
        "INSERT INTO tasks (group_id, number, title, instructions, architecture, thread_id,
                            origin_message, requester_id, agent, state, job_id, created_at,
                            updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(number)
    .bind(&title)
    .bind(&instructions)
    .bind(&architecture.name)
    .bind(new.thread_id)
    .bind(new.origin_message)
    .bind(new.requester_id)
    .bind(agent)
    .bind(job_id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    tx.commit().await?;
    publish(state, task_id).await?;
    find(state, group.group_id, number as u32)
        .await?
        .to_api_noted(state)
        .await
}

/// The title of a task asked for in a message to the workshop: its first
/// sentence, without the mention.
pub fn title_of(body: &str, handle: &str) -> String {
    let mention = format!("@{handle}");
    let text: String = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.trim() != mention)
        .unwrap_or_default()
        .split_whitespace()
        .filter(|word| {
            !word
                .trim_end_matches([',', ':', '.'])
                .eq_ignore_ascii_case(&mention)
        })
        .collect::<Vec<_>>()
        .join(" ");
    // The first sentence: up to a stop followed by a space, or the end.
    let end = text
        .char_indices()
        .find(|&(i, c)| {
            matches!(c, '.' | '?' | '!')
                && i > 10
                && text[i + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map(|(i, _)| i);
    let sentence = match end {
        Some(end) => &text[..end],
        None => text.as_str(),
    };
    let mut title: String = sentence.trim().chars().take(80).collect();
    if sentence.trim().chars().count() > 80 {
        title.push('…');
    }
    if let Some(first) = title.chars().next() {
        title = first.to_uppercase().chain(title.chars().skip(1)).collect();
    }
    if title.is_empty() {
        title = "A task from the thread".into();
    }
    title
}

/// Cancels a task that is open or running: its requester, or an admin.
/// One whose changes wait for approval is rejected instead.
pub async fn cancel(
    state: &AppState,
    group: &Membership,
    user_id: i64,
    number: u32,
) -> ApiResult<()> {
    let row = find(state, group.group_id, number).await?;
    if row.user_id != user_id {
        group.require_admin()?;
    }
    match row.state() {
        TaskState::Review => {
            return Err(ApiError::conflict(
                "This task's changes wait for approval: reject them instead",
            ));
        }
        state if state.is_final() => return Err(ApiError::conflict("This task is over")),
        _ => {}
    }
    match row.job_id {
        // Stopping the job cancels the task.
        Some(job) => jobs::stop(state, job).await,
        None => {
            set_state(state, row.id, TaskState::Cancelled, None).await?;
            publish(state, row.id).await
        }
    }
}

pub(crate) async fn set_state(
    state: &AppState,
    task_id: i64,
    to: TaskState,
    reason: Option<&str>,
) -> ApiResult<()> {
    let now = db::now();
    sqlx::query(
        "UPDATE tasks SET state = ?, reason = coalesce(?, reason), updated_at = ?,
                          finished_at = CASE WHEN ? THEN ? ELSE finished_at END
         WHERE id = ?",
    )
    .bind(to.as_str())
    .bind(reason)
    .bind(now)
    .bind(to.is_final())
    .bind(now)
    .bind(task_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

// ── Runs ──────────────────────────────────────────────────────────────

/// What a workshop machine needs to run a task: the task, its
/// architecture, the conversation it came from (with the workshop's agent,
/// as the machine is to run it), and the repository.
pub async fn work_for(
    state: &AppState,
    group: &Membership,
    job_id: i64,
    conversation: Conversation,
) -> ApiResult<TaskRun> {
    let row = of_job(state, job_id)
        .await?
        .ok_or_else(|| ApiError::internal("a task's job without its task"))?;
    let architecture = architectures::find(&row.architecture).ok_or_else(|| {
        ApiError::invalid(format!(
            "The workshop no longer knows the architecture {}",
            row.architecture
        ))
    })?;
    let data_sorry = match library::current_build_id(state, group.group_id).await? {
        Some(build) => {
            sqlx::query_scalar(
                "SELECT name FROM library_declarations WHERE build_id = ? AND data_sorry = 1
                 ORDER BY name",
            )
            .bind(build)
            .fetch_all(&state.db)
            .await?
        }
        None => Vec::new(),
    };
    Ok(TaskRun {
        task: row.number as u32,
        title: row.title.clone(),
        instructions: row.instructions.clone(),
        architecture: architectures::run_of(&architecture).map_err(ApiError::internal)?,
        conversation,
        repository: format!("/git/{}/{}.git", group.slug, architecture.works_on),
        branch: branch_of(row.number as u32),
        data_sorry,
    })
}

/// A task's run started, on a machine, with `runner`.
pub async fn started(
    state: &AppState,
    job_id: i64,
    runner: &str,
    model: Option<&str>,
) -> ApiResult<()> {
    let now = db::now();
    sqlx::query(
        "UPDATE tasks SET state = 'running', runner = ?, model = ?, reason = NULL,
                          started_at = ?, updated_at = ?
         WHERE job_id = ? AND state IN ('open', 'running')",
    )
    .bind(runner)
    .bind(model)
    .bind(now)
    .bind(now)
    .bind(job_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// A task's run will start again: its machine went away.
pub async fn requeued(state: &AppState, job_id: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE tasks SET state = 'open', updated_at = ? WHERE job_id = ? AND state = 'running'",
    )
    .bind(db::now())
    .bind(job_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// A task's job ended without the run's outcome: it failed, with `error`,
/// or was cancelled. A task that fails says so in its thread. Callers may
/// hold the workers' lock.
pub async fn job_ended(
    state: &AppState,
    job_id: i64,
    cancelled: bool,
    error: Option<&str>,
) -> ApiResult<()> {
    let Some(row) = of_job(state, job_id).await? else {
        return Ok(());
    };
    if row.state().is_final() || row.state() == TaskState::Review {
        return Ok(());
    }
    if cancelled {
        set_state(state, row.id, TaskState::Cancelled, None).await?;
    } else {
        let reason = error.unwrap_or("The run failed");
        set_state(state, row.id, TaskState::Failed, Some(reason)).await?;
        let text = format!(
            "**Failed:** [[w{}]], “{}”. {}.\n\nNothing was proposed.",
            row.number,
            row.title,
            one_line(reason).trim_end_matches('.')
        );
        let number = say(state, &row, &text).await?;
        sqlx::query("UPDATE tasks SET report_message = ? WHERE id = ?")
            .bind(number)
            .bind(row.id)
            .execute(&state.db)
            .await?;
    }
    publish(state, row.id).await
}

/// Posts a message in a task's thread, by the workshop for its requester,
/// in reply to the message that asked. Returns its number.
pub(crate) async fn say(state: &AppState, row: &TaskRow, text: &str) -> ApiResult<u32> {
    let group = Membership::of_group(state, row.group_id).await?;
    let message = forum::post(
        state,
        &group,
        row.thread_id,
        row.user_id,
        Some(&profile().display_name),
        text,
        Some(row.origin_message as u32),
        &[],
    )
    .await?;
    Ok(message.number)
}

pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The end of a log, when it is too long.
fn tail(log: &str) -> String {
    if log.len() <= MAX_LOG {
        return log.to_owned();
    }
    let mut start = log.len() - MAX_LOG;
    while !log.is_char_boundary(start) {
        start += 1;
    }
    format!("[… the beginning is cut]\n{}", &log[start..])
}

/// What a task proposes, once its pull request is open.
struct Proposed {
    pull: u32,
    url: String,
    why_big: Vec<String>,
}

/// Opens the pull request of a task's branch, and says whether its changes
/// are big.
async fn propose(state: &AppState, row: &TaskRow, report: &str) -> ApiResult<Proposed> {
    let forge = forge::require(state)?;
    let architecture = architectures::find(&row.architecture)
        .ok_or_else(|| ApiError::internal("a task without its architecture"))?;
    let group = Membership::of_group(state, row.group_id).await?;
    let repository = &architecture.works_on;
    let branch = branch_of(row.number as u32);
    let body = format!(
        "Task w{} of Stemma, asked by {} in {}.\n\n{}\n\nIt is approved, or rejected, in Stemma.",
        row.number,
        row.display_name,
        row.origin(),
        report.trim()
    );
    let title = format!("w{}: {}", row.number, row.title);
    let pull = forge
        .create_pull(&group.slug, repository, &branch, forge::MAIN, &title, &body)
        .await
        .map_err(forge::api_error)?;
    let files = forge
        .pull_files(&group.slug, repository, pull.number)
        .await
        .map_err(forge::api_error)?;
    let touched: Vec<architectures::Touched> = files
        .iter()
        .map(|f| architectures::Touched {
            path: &f.filename,
            previous: f.previous_filename.as_deref(),
            status: &f.status,
        })
        .collect();
    Ok(Proposed {
        pull: pull.number,
        url: forge.public_url(&format!(
            "{}/{repository}/pulls/{}/files",
            group.slug, pull.number
        )),
        why_big: architectures::why_big(&architecture, &touched),
    })
}

/// Keeps how a task's run ended. Work that changed something is proposed
/// in a pull request, and waits for approval; one message in its thread
/// reports on it. Returns the number of the message. Callers may hold the
/// workers' lock.
pub async fn finished(state: &AppState, job_id: i64, outcome: TaskOutcome) -> ApiResult<u32> {
    let row = of_job(state, job_id)
        .await?
        .ok_or_else(|| ApiError::internal("a task's job without its task"))?;
    let report = outcome.report.trim().to_owned();
    let mut end = match outcome.end {
        TaskEnd::Done if outcome.commit.is_some() => TaskState::Review,
        TaskEnd::Done => TaskState::Done,
        TaskEnd::Failed => TaskState::Failed,
    };
    let mut reason = outcome.reason.as_deref().map(one_line);
    let mut changed: Vec<String> = Vec::new();
    let mut proposed = None;
    if end == TaskState::Review {
        if let Some(report) = outcome.built.as_ref().and_then(|b| b.report.as_ref()) {
            changed = library::changed_declarations(state, row.group_id, report).await?;
        }
        match propose(state, &row, &report).await {
            Ok(done) => proposed = Some(done),
            Err(error) => {
                end = TaskState::Failed;
                reason = Some(format!(
                    "The changes could not be proposed: {}",
                    error.message
                ));
            }
        }
    }
    let architecture = architectures::find(&row.architecture);
    let now = db::now();
    sqlx::query(
        "UPDATE tasks SET state = ?, reason = ?, report = ?, log = ?, runner = coalesce(?, runner),
                          model = coalesce(?, model), repository = ?, branch = ?,
                          commit_sha = ?, pull = ?, pull_url = ?, big = ?, why_big = ?,
                          finished_at = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(end.as_str())
    .bind(&reason)
    .bind(&report)
    .bind(tail(&outcome.log))
    .bind(&outcome.runner)
    .bind(&outcome.model)
    .bind(architecture.as_ref().map(|a| a.works_on.clone()))
    .bind(proposed.as_ref().map(|_| branch_of(row.number as u32)))
    .bind(proposed.as_ref().and(outcome.commit.as_deref()))
    .bind(proposed.as_ref().map(|p| p.pull))
    .bind(proposed.as_ref().map(|p| p.url.clone()))
    .bind(proposed.as_ref().is_some_and(|p| !p.why_big.is_empty()))
    .bind(
        serde_json::to_string(
            &proposed
                .as_ref()
                .map(|p| p.why_big.clone())
                .unwrap_or_default(),
        )
        .map_err(ApiError::internal)?,
    )
    .bind(end.is_final().then_some(now))
    .bind(now)
    .bind(row.id)
    .execute(&state.db)
    .await?;

    let row = by_id(state, row.id)
        .await?
        .ok_or_else(|| ApiError::internal("a task gone"))?;
    let text = report_message(&row, end, reason.as_deref(), &report, &changed);
    let number = say(state, &row, &text).await?;
    sqlx::query("UPDATE tasks SET report_message = ? WHERE id = ?")
        .bind(number)
        .bind(row.id)
        .execute(&state.db)
        .await?;
    Ok(number)
}

/// The one message that reports on a task's run.
fn report_message(
    row: &TaskRow,
    end: TaskState,
    reason: Option<&str>,
    report: &str,
    changed: &[String],
) -> String {
    let task = format!("[[w{}]], “{}”", row.number, row.title);
    let reason = |default: &str| {
        one_line(reason.unwrap_or(default))
            .trim_end_matches('.')
            .to_owned()
    };
    let mut text = match end {
        TaskState::Review => format!("**Ready for review:** {task}."),
        TaskState::Done => format!("**Done:** {task}. Nothing in the library changed."),
        _ => format!("**Failed:** {task}. {}.", reason("The run failed")),
    };
    if !report.is_empty() {
        text.push_str("\n\n");
        if report.chars().count() > MAX_REPORT_CHARS {
            text.extend(report.chars().take(MAX_REPORT_CHARS));
            text.push_str("\n\n[The report was cut here.]");
        } else {
            text.push_str(report);
        }
    }
    match (end, &row.pull_url, &row.branch) {
        (TaskState::Review, Some(url), Some(branch)) => {
            text.push_str(&format!(
                "\n\nThe changes are on the branch `{branch}`: [see them]({url})"
            ));
            if !changed.is_empty() {
                let shown: Vec<String> = changed
                    .iter()
                    .take(MAX_CHANGED)
                    .map(|name| format!("[[lean:{name}]]"))
                    .collect();
                text.push_str(&format!("; they change {}", shown.join(", ")));
                if changed.len() > MAX_CHANGED {
                    text.push_str(&format!(" and {} more", changed.len() - MAX_CHANGED));
                }
            }
            text.push_str(&format!(
                ".\n\nThey go in once {} approves them",
                row.display_name
            ));
            let why: Vec<String> = serde_json::from_str(&row.why_big).unwrap_or_default();
            if row.big {
                text.push_str(&format!(
                    ", and an admin too, for they are big ({})",
                    why.join("; ")
                ));
            }
            text.push_str(&format!(": approve or reject them in [[w{}]].", row.number));
        }
        (TaskState::Failed, _, _) => text.push_str("\n\nNothing was proposed."),
        _ => {}
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_from_messages() {
        assert_eq!(
            title_of(
                "@workshop formalize the lemma in t12#48. It is the gluing one.",
                "workshop",
            ),
            "Formalize the lemma in t12#48"
        );
        assert_eq!(
            title_of("@workshop: prove [[lean:Foo.bar]]", "workshop"),
            "Prove [[lean:Foo.bar]]"
        );
    }
}
