//! The workshop: what works on the library in the background.
//!
//! A task is handed over in conversation: an agent answering in a thread
//! calls `create_task` when someone asks it to ("formalize this"), or a
//! message mentions the workshop's agent (`@workshop formalize the lemma in
//! t12#48`). Only admins and members ask for work: guests cannot. Each task
//! is a job, `task_run`, for a group worker that offers the workshop, with
//! Lean and the workshop agent's runner. A group's tasks run one at a time.
//!
//! The worker pushes what it did to `main` with the job's token, and tells
//! how it went: the build and inspection of what it pushed become the
//! library's state, and one message in the thread the task came from
//! reports on it.

use sqlx::FromRow;
use stemma_core::Address;
use stemma_core::api::{AgentProfile, AgentRole, Event, Task, TaskPage, TaskState, User};
use stemma_core::protocol::{Conversation, TaskEnd, TaskOutcome, TaskRun};

use crate::api::{Membership, forum};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs;
use crate::library;
use crate::state::AppState;

/// The most of a run's log that is kept, in bytes: its end.
const MAX_LOG: usize = 200_000;

/// The most of a runner's report that goes into the thread, in characters.
const MAX_REPORT_CHARS: usize = 20_000;

/// How many changed declarations a report names.
const MAX_CHANGED: usize = 40;

// ── Tasks ─────────────────────────────────────────────────────────────

#[derive(FromRow)]
struct TaskRow {
    id: i64,
    group_id: i64,
    number: i64,
    title: String,
    instructions: String,
    thread_id: i64,
    thread_number: i64,
    origin_message: i64,
    user_id: i64,
    username: String,
    display_name: String,
    agent: Option<String>,
    state: String,
    job_id: Option<i64>,
    runner: Option<String>,
    model: Option<String>,
    reason: Option<String>,
    commits: String,
    report_message: Option<i64>,
    created_at: i64,
    updated_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    job_progress: Option<String>,
}

macro_rules! task_query {
    () => {
        "SELECT k.id, k.group_id, k.number, k.title, k.instructions, k.thread_id,
                t.number AS thread_number, k.origin_message, u.id AS user_id, u.username,
                u.display_name, k.agent, k.state, k.job_id, k.runner, k.model,
                k.reason, k.commits, k.report_message, k.created_at, k.updated_at,
                k.started_at, k.finished_at, j.progress AS job_progress
         FROM tasks k JOIN threads t ON t.id = k.thread_id
              JOIN users u ON u.id = k.requester_id
              LEFT JOIN jobs j ON j.id = k.job_id"
    };
}

impl TaskRow {
    fn state(&self) -> TaskState {
        TaskState::parse(&self.state).unwrap_or(TaskState::Failed)
    }

    fn origin(&self) -> Address {
        Address::Message {
            thread: self.thread_number as u32,
            message: self.origin_message as u32,
        }
    }

    /// The task as people see it; `note` says why it waits, if it does.
    fn to_api(&self, note: Option<String>) -> Task {
        let state = self.state();
        Task {
            number: self.number as u32,
            title: self.title.clone(),
            instructions: self.instructions.clone(),
            origin: self.origin(),
            requester: User {
                id: self.user_id,
                username: self.username.clone(),
                display_name: self.display_name.clone(),
            },
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
            commits: serde_json::from_str(&self.commits).unwrap_or_default(),
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
        Ok(self.to_api(note))
    }
}

async fn find(state: &AppState, group_id: i64, number: u32) -> ApiResult<TaskRow> {
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
        state.publish(
            row.group_id,
            Event::TaskUpdated {
                task: row.to_api(note),
            },
        );
    }
    Ok(())
}

/// Tells the task's group how it is. It does not say why an open task
/// waits: the news of its job, which follows, does. Callers may hold the
/// workers' lock.
async fn publish(state: &AppState, task_id: i64) -> ApiResult<()> {
    let row: Option<TaskRow> = sqlx::query_as(concat!(task_query!(), " WHERE k.id = ?"))
        .bind(task_id)
        .fetch_optional(&state.db)
        .await?;
    if let Some(row) = row {
        let task = row.to_api(None);
        state.publish(row.group_id, Event::TaskUpdated { task });
    }
    Ok(())
}

/// Looks at where the library's `main` is, soon: a run may have pushed.
/// It is not done here, for callers may hold the workers' lock, which
/// queueing a build takes.
fn refresh_soon(state: &AppState, group_id: i64, requester_id: i64) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(error) = library::settle(&state, group_id, requester_id).await {
            tracing::error!(
                group_id,
                error = error.message,
                "could not look at the library"
            );
        }
    });
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

/// The group's workshop agent: the agent whose role is the workshop,
/// `@workshop` first.
pub async fn profile(state: &AppState, group_id: i64) -> ApiResult<Option<AgentProfile>> {
    let handle: Option<String> = sqlx::query_scalar(
        "SELECT handle FROM group_agents WHERE group_id = ? AND role = ?
         ORDER BY handle = 'workshop' DESC, handle LIMIT 1",
    )
    .bind(group_id)
    .bind(AgentRole::Workshop.as_str())
    .fetch_optional(&state.db)
    .await?;
    match handle {
        Some(handle) => crate::agents::group_agent(state, group_id, &handle).await,
        None => Ok(None),
    }
}

/// A task as it is handed over.
pub struct NewTask {
    pub title: String,
    pub instructions: String,
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
    let profile = profile(state, group.group_id).await?.ok_or_else(|| {
        ApiError::invalid("This group has no workshop: add an agent whose role is the workshop")
    })?;
    let title = crate::api::required_text(&new.title, "The title", 200)?;
    let instructions = crate::api::required_text(&new.instructions, "The instructions", 20_000)?;
    let agent = new
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty());
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
                           agent, runner, scope, state, created_at, updated_at)
         VALUES (?, ?, 'task_run', ?, ?, ?, ?, ?, 'group', 'queued', ?, ?)",
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
        "INSERT INTO tasks (group_id, number, title, instructions, thread_id, origin_message,
                            requester_id, agent, state, job_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(number)
    .bind(&title)
    .bind(&instructions)
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
    if row.state().is_final() {
        return Err(ApiError::conflict("This task is over"));
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

async fn set_state(
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

/// What a worker needs to run a task: the task, the conversation it came
/// from (with the workshop's agent, as the worker is to run it), and the
/// library.
pub async fn work_for(
    state: &AppState,
    group: &Membership,
    job_id: i64,
    conversation: Conversation,
) -> ApiResult<TaskRun> {
    let row = of_job(state, job_id)
        .await?
        .ok_or_else(|| ApiError::internal("a task's job without its task"))?;
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
        conversation,
        repository: library::clone_path(&group.slug),
        data_sorry,
    })
}

/// A task's run started, on a worker, with `runner`.
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

/// A task's run will start again: its worker went away. It may have
/// pushed before it did, so the hub looks at the library.
pub async fn requeued(state: &AppState, job_id: i64) -> ApiResult<()> {
    let Some(row) = of_job(state, job_id).await? else {
        return Ok(());
    };
    sqlx::query(
        "UPDATE tasks SET state = 'open', updated_at = ? WHERE id = ? AND state = 'running'",
    )
    .bind(db::now())
    .bind(row.id)
    .execute(&state.db)
    .await?;
    refresh_soon(state, row.group_id, row.user_id);
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
    if row.state().is_final() {
        return Ok(());
    }
    if cancelled {
        set_state(state, row.id, TaskState::Cancelled, None).await?;
    } else {
        let reason = error.unwrap_or("The run failed");
        set_state(state, row.id, TaskState::Failed, Some(reason)).await?;
        let text = format!(
            "**Failed:** [[w{}]], “{}”. {}.\n\nNothing was put in the library.",
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
    // It may have pushed before it ended.
    refresh_soon(state, row.group_id, row.user_id);
    publish(state, row.id).await
}

/// Posts a message in a task's thread, by the workshop for its requester,
/// in reply to the message that asked. Returns its number.
async fn say(state: &AppState, row: &TaskRow, text: &str) -> ApiResult<u32> {
    let group = Membership::of_group(state, row.group_id).await?;
    let agent = match row.job_id {
        Some(job) => {
            let profile: Option<Option<String>> =
                sqlx::query_scalar("SELECT agent FROM jobs WHERE id = ?")
                    .bind(job)
                    .fetch_optional(&state.db)
                    .await?;
            profile
                .flatten()
                .and_then(|json| serde_json::from_str::<AgentProfile>(&json).ok())
                .map(|p| p.display_name)
        }
        None => None,
    }
    .unwrap_or_else(|| "Workshop".into());
    let message = forum::post(
        state,
        &group,
        row.thread_id,
        row.user_id,
        Some(&agent),
        text,
        Some(row.origin_message as u32),
        &[],
    )
    .await?;
    Ok(message.number)
}

fn one_line(text: &str) -> String {
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

/// Keeps how a task's run ended: the library's new state from its build,
/// and one message in its thread. Returns the number of the message. Callers may hold the workers' lock.
pub async fn finished(state: &AppState, job_id: i64, outcome: TaskOutcome) -> ApiResult<u32> {
    let row = of_job(state, job_id)
        .await?
        .ok_or_else(|| ApiError::internal("a task's job without its task"))?;

    // What it pushed becomes the library's state.
    let mut changed: Vec<String> = Vec::new();
    if let (Some(built), Some(commit)) = (&outcome.built, outcome.commits.last())
        && built.succeeded
        && let Some(report) = &built.report
    {
        changed = library::changed_declarations(state, row.group_id, report).await?;
        library::record_build(state, row.group_id, commit, built.clone()).await?;
    }
    refresh_soon(state, row.group_id, row.user_id);

    let end = match outcome.end {
        TaskEnd::Done => TaskState::Done,
        TaskEnd::Failed => TaskState::Failed,
        TaskEnd::Blocked => TaskState::Blocked,
    };

    let text = report_message(&row, end, &outcome, &changed);
    let number = say(state, &row, &text).await?;

    let now = db::now();
    sqlx::query(
        "UPDATE tasks SET state = ?, reason = ?, report = ?, log = ?, commits = ?, runner = ?,
                          model = ?, report_message = ?, finished_at = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(end.as_str())
    .bind(outcome.reason.as_deref().map(one_line))
    .bind(outcome.report.trim())
    .bind(tail(&outcome.log))
    .bind(serde_json::to_string(&outcome.commits).map_err(ApiError::internal)?)
    .bind(&outcome.runner)
    .bind(&outcome.model)
    .bind(number)
    .bind(now)
    .bind(now)
    .bind(row.id)
    .execute(&state.db)
    .await?;

    Ok(number)
}

/// The one message that reports on a task's run.
fn report_message(
    row: &TaskRow,
    end: TaskState,
    outcome: &TaskOutcome,
    changed: &[String],
) -> String {
    let task = format!("[[w{}]], “{}”", row.number, row.title);
    let reason = |default: &str| {
        one_line(outcome.reason.as_deref().unwrap_or(default))
            .trim_end_matches('.')
            .to_owned()
    };
    let mut text = match end {
        TaskState::Done if outcome.commits.is_empty() => {
            format!("**Done:** {task}. Nothing in the library changed.")
        }
        TaskState::Done => format!("**Done:** {task}."),
        TaskState::Blocked => format!("**Blocked:** {task}. {}.", reason("It could not be pushed")),
        _ => format!("**Failed:** {task}. {}.", reason("The run failed")),
    };
    let report = outcome.report.trim();
    if !report.is_empty() {
        text.push_str("\n\n");
        if report.chars().count() > MAX_REPORT_CHARS {
            text.extend(report.chars().take(MAX_REPORT_CHARS));
            text.push_str("\n\n[The report was cut here.]");
        } else {
            text.push_str(report);
        }
    }
    if !outcome.commits.is_empty() {
        let commits: Vec<String> = outcome
            .commits
            .iter()
            .map(|c| format!("`{}`", &c[..c.len().min(10)]))
            .collect();
        text.push_str(&format!("\n\nIn the library: {}", commits.join(", ")));
        if !changed.is_empty() {
            let shown: Vec<String> = changed
                .iter()
                .take(MAX_CHANGED)
                .map(|name| format!("[[lean:{name}]]"))
                .collect();
            text.push_str(&format!(", which changed {}", shown.join(", ")));
            if changed.len() > MAX_CHANGED {
                text.push_str(&format!(" and {} more", changed.len() - MAX_CHANGED));
            }
        }
        text.push('.');
    } else if end != TaskState::Done {
        text.push_str("\n\nNothing was put in the library.");
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
