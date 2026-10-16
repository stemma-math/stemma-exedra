//! The workshop: what works on the library in the background.
//!
//! A task is handed over in conversation: an agent answering in a thread
//! calls `create_task` when someone asks it to ("formalize this tonight"),
//! or a message mentions the workshop's agent (`@workshop formalize the
//! lemma in t12#48`). Each task is a job, `task_run`, for a group worker
//! that offers the workshop, with Lean and the workshop agent's runner; or,
//! during the group's night window, for a member's worker its owner lends
//! to the workshop. A group's tasks run one at a time, and a task asked for
//! the night waits for the night window.
//!
//! The worker pushes what it did to `main` with the job's token, and tells
//! how it went: the build and inspection of what it pushed become the
//! library's state, one message in the thread the task came from reports
//! on it, and the editor is asked to update the papers that reference what
//! changed, or that the thread is about.
//!
//! At night the editor also brings up to date the papers whose threads said
//! something new since their last revision (see [`update_papers`]).

use std::collections::{HashMap, HashSet};

use sqlx::FromRow;
use stemma_core::Address;
use stemma_core::api::{
    AgentProfile, AgentRole, AgentScope, Event, JobKind, Task, TaskPage, TaskSchedule, TaskState,
    User, WorkshopSettings,
};
use stemma_core::protocol::{Conversation, PaperSpec, TaskEnd, TaskOutcome, TaskRun};

use crate::api::{Membership, forum, papers};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs;
use crate::library;
use crate::state::AppState;

/// The most of a run's log that is kept, in bytes: its end.
const MAX_LOG: usize = 200_000;

/// The most of a runner's report that goes into the thread, in characters.
const MAX_REPORT_CHARS: usize = 20_000;

/// How many papers a run is given with their text.
const MAX_PAPERS: usize = 4;

/// How many changed declarations a report names.
const MAX_CHANGED: usize = 40;

// ── The night ─────────────────────────────────────────────────────────

/// A group's night window, as dispatching sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Night {
    /// Whether it is night now.
    pub now: bool,
    /// The window, for what people read: `01:00–07:00 (Europe/Madrid)`.
    pub window: String,
    /// How long the window is, in minutes.
    pub length: i64,
    /// The members who lend their workers to the workshop at night.
    pub lenders: HashSet<i64>,
}

/// Minutes after midnight, from `HH:MM`.
pub fn parse_clock(text: &str) -> Option<i64> {
    let (hours, minutes) = text.trim().split_once(':')?;
    if hours.is_empty() || hours.len() > 2 || minutes.len() != 2 {
        return None;
    }
    let (hours, minutes): (i64, i64) = (hours.parse().ok()?, minutes.parse().ok()?);
    ((0..24).contains(&hours) && (0..60).contains(&minutes)).then_some(hours * 60 + minutes)
}

/// `HH:MM`, from minutes after midnight.
pub fn clock(minutes: i64) -> String {
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

/// The time zone a group names, or UTC if it is unknown here.
fn zone(name: &str) -> jiff::tz::TimeZone {
    jiff::tz::TimeZone::get(name).unwrap_or(jiff::tz::TimeZone::UTC)
}

/// Whether `at` is in the window from `from` to `to`, minutes after
/// midnight in the time zone `zone`; a window may span midnight, and one
/// that starts where it ends is empty.
pub fn is_night(at: jiff::Timestamp, zone_name: &str, from: i64, to: i64) -> bool {
    let local = at.to_zoned(zone(zone_name));
    let minute = i64::from(local.hour()) * 60 + i64::from(local.minute());
    if from <= to {
        from <= minute && minute < to
    } else {
        minute >= from || minute < to
    }
}

/// Every group's night, now.
pub async fn nights(state: &AppState) -> ApiResult<HashMap<i64, Night>> {
    let groups: Vec<(i64, i64, i64, String)> =
        sqlx::query_as("SELECT id, night_from, night_to, time_zone FROM groups")
            .fetch_all(&state.db)
            .await?;
    let lenders: Vec<(i64, i64)> =
        sqlx::query_as("SELECT group_id, user_id FROM memberships WHERE lends_worker = 1")
            .fetch_all(&state.db)
            .await?;
    let now = jiff::Timestamp::now();
    let mut nights: HashMap<i64, Night> = groups
        .into_iter()
        .map(|(id, from, to, time_zone)| {
            let night = Night {
                now: is_night(now, &time_zone, from, to),
                window: format!("{}–{} ({time_zone})", clock(from), clock(to)),
                length: (to - from).rem_euclid(24 * 60),
                lenders: HashSet::new(),
            };
            (id, night)
        })
        .collect();
    for (group_id, user_id) in lenders {
        if let Some(night) = nights.get_mut(&group_id) {
            night.lenders.insert(user_id);
        }
    }
    Ok(nights)
}

/// Hands waiting tasks to workers when a night begins or ends somewhere,
/// and updates the papers once a night: run every minute or so.
pub async fn tick(state: &AppState) -> ApiResult<()> {
    let now = nights(state).await?;
    let asked = update_papers(state, &now).await?;
    let changed = {
        let workers = state.workers.lock().await;
        workers.nights.iter().any(|(id, night)| {
            now.get(id)
                .is_some_and(|n| n.now != night.now || n.lenders != night.lenders)
        }) || now.keys().any(|id| !workers.nights.contains_key(id))
    };
    if asked || changed {
        jobs::dispatch(state).await?;
    }
    Ok(())
}

/// Once a night, in each group's night window, asks the editor to update
/// the papers whose threads said something new since their last revision.
/// An editor of the group's runs on the group's workers, for whoever said
/// it; a personal one, only for a member who lends their worker at night.
/// Returns whether it asked for any update.
pub async fn update_papers(state: &AppState, nights: &HashMap<i64, Night>) -> ApiResult<bool> {
    let now = db::now();
    let mut asked = false;
    for (&group_id, night) in nights {
        if !night.now {
            continue;
        }
        let Some(editor) = editor(state, group_id).await? else {
            continue;
        };
        let mut lenders: Vec<i64> = night.lenders.iter().copied().collect();
        lenders.sort_unstable();
        // Wait for someone to lend their worker, if the editor needs one.
        if editor.scope == AgentScope::Personal && lenders.is_empty() {
            continue;
        }
        // Not again within a window's length of the last time: once a night.
        let tonight: Option<i64> = sqlx::query_scalar(
            "UPDATE groups SET papers_updated_at = ?1
             WHERE id = ?2 AND (papers_updated_at IS NULL OR papers_updated_at < ?1 - ?3)
             RETURNING id",
        )
        .bind(now)
        .bind(group_id)
        .bind(night.length * 60 * 1000)
        .fetch_optional(&state.db)
        .await?;
        if tonight.is_some() {
            asked |= update_group_papers(state, group_id, &editor, &lenders).await?;
        }
    }
    Ok(asked)
}

async fn update_group_papers(
    state: &AppState,
    group_id: i64,
    editor: &AgentProfile,
    lenders: &[i64],
) -> ApiResult<bool> {
    // The papers, but not those an editor is updating already.
    let papers: Vec<(String, i64)> = sqlx::query_as(
        "SELECT p.slug, p.updated_at FROM papers p
         WHERE p.group_id = ?1
           AND NOT EXISTS (SELECT 1 FROM jobs j
                           WHERE j.group_id = ?1 AND j.kind = 'paper_update'
                             AND j.paper_slug = p.slug AND j.state IN ('queued', 'running'))
         ORDER BY p.updated_at",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    let mut asked = false;
    for (slug, updated_at) in papers {
        // The thread about it where someone said something new last.
        let mut said = None;
        for (thread_id, _) in papers::threads_about(state, group_id, &slug).await? {
            let author: Option<i64> = sqlx::query_scalar(
                "SELECT author_id FROM messages
                 WHERE thread_id = ? AND agent IS NULL AND deleted_at IS NULL AND created_at > ?
                 ORDER BY number DESC LIMIT 1",
            )
            .bind(thread_id)
            .bind(updated_at)
            .fetch_optional(&state.db)
            .await?;
            if let Some(author) = author {
                said = Some((thread_id, author));
                break;
            }
        }
        let Some((thread_id, author)) = said else {
            continue;
        };
        let requester = match editor.scope {
            AgentScope::Personal if !lenders.contains(&author) => lenders[0],
            _ => author,
        };
        let last: i64 = sqlx::query_scalar("SELECT last_message FROM threads WHERE id = ?")
            .bind(thread_id)
            .fetch_one(&state.db)
            .await?;
        jobs::queue_agent_job(
            state,
            group_id,
            thread_id,
            last as u32,
            requester,
            editor,
            JobKind::PaperUpdate,
            Some(&slug),
            true,
        )
        .await?;
        asked = true;
    }
    Ok(asked)
}

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
    schedule: String,
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
                u.display_name, k.agent, k.state, k.schedule, k.job_id, k.runner, k.model,
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
            schedule: TaskSchedule::parse(&self.schedule).unwrap_or_default(),
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
    profile_with_role(state, group_id, AgentRole::Workshop, "workshop").await
}

/// The group's editor: the agent whose role it is, `@editor` first.
async fn editor(state: &AppState, group_id: i64) -> ApiResult<Option<AgentProfile>> {
    profile_with_role(state, group_id, AgentRole::Editor, "editor").await
}

async fn profile_with_role(
    state: &AppState,
    group_id: i64,
    role: AgentRole,
    first: &str,
) -> ApiResult<Option<AgentProfile>> {
    let handle: Option<String> = sqlx::query_scalar(
        "SELECT handle FROM agent_profiles WHERE group_id = ? AND role = ?
         ORDER BY handle = ? DESC, handle LIMIT 1",
    )
    .bind(group_id)
    .bind(role.as_str())
    .bind(first)
    .fetch_optional(&state.db)
    .await?;
    match handle {
        Some(handle) => jobs::find_profile(state, group_id, &handle).await,
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
    pub schedule: TaskSchedule,
}

/// Hands a task to the workshop, and queues its run; the caller then
/// dispatches. Returns the task.
pub async fn create(state: &AppState, group: &Membership, new: NewTask) -> ApiResult<Task> {
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
                            requester_id, agent, state, schedule, job_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'open', ?, ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(number)
    .bind(&title)
    .bind(&instructions)
    .bind(new.thread_id)
    .bind(new.origin_message)
    .bind(new.requester_id)
    .bind(agent)
    .bind(new.schedule.as_str())
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

/// The title and schedule of a task asked for in a message to the
/// workshop: its first sentence, without the mention; at night if it says
/// "tonight" or "overnight".
pub fn from_message(body: &str, handle: &str) -> (String, TaskSchedule) {
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
    let lower = body.to_lowercase();
    let night = ["tonight", "overnight", "at night", "this night"]
        .iter()
        .any(|word| lower.contains(word));
    let schedule = if night {
        TaskSchedule::Night
    } else {
        TaskSchedule::Now
    };
    (title, schedule)
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
/// from (with the workshop's agent, as the worker is to run it), the papers
/// the conversation is about, and the library.
pub async fn work_for(
    state: &AppState,
    group: &Membership,
    job_id: i64,
    conversation: Conversation,
) -> ApiResult<TaskRun> {
    let row = of_job(state, job_id)
        .await?
        .ok_or_else(|| ApiError::internal("a task's job without its task"))?;
    let thread: stemma_core::api::Thread =
        forum::find_thread_by_id(state, row.thread_id).await?.into();

    // The papers: the one the thread is about, then those it references.
    let mut slugs: Vec<String> = Vec::new();
    if let Some(Address::Paper { slug, .. }) = &thread.anchor {
        slugs.push(slug.clone());
    }
    let mut texts: Vec<&str> = vec![&row.instructions];
    for item in &conversation.context {
        if let stemma_core::protocol::ContextItem::Message { message } = item {
            texts.push(&message.body);
        }
    }
    for text in texts {
        for reference in stemma_core::references(text) {
            if let Ok(Address::Paper { slug, .. }) = reference.address
                && !slugs.contains(&slug)
            {
                slugs.push(slug);
            }
        }
    }
    let mut papers = Vec::new();
    for slug in slugs.into_iter().take(MAX_PAPERS) {
        if let Some(current) = papers::current(state, group.group_id, &slug).await? {
            papers.push(PaperSpec {
                slug,
                title: Some(current.title),
                revision: current.revision,
                body: current.body,
            });
        }
    }

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
        papers,
        repository: format!("/git/{}.git", group.slug),
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
/// one message in its thread, and updates of the papers it concerns.
/// Returns the number of the message. Callers may hold the workers' lock.
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

    // The papers to update: those that reference what changed, and the
    // one the thread is about.
    let mut slugs: Vec<String> = Vec::new();
    if end == TaskState::Done && !outcome.commits.is_empty() {
        let thread: stemma_core::api::Thread =
            forum::find_thread_by_id(state, row.thread_id).await?.into();
        if let Some(Address::Paper { slug, .. }) = &thread.anchor {
            slugs.push(slug.clone());
        }
        for slug in papers_referencing(state, row.group_id, &changed).await? {
            if !slugs.contains(&slug) {
                slugs.push(slug);
            }
        }
    }
    let editor = if slugs.is_empty() {
        None
    } else {
        editor(state, row.group_id).await?
    };

    let text = report_message(&row, end, &outcome, &changed, &slugs, editor.as_ref());
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

    if let Some(editor) = editor {
        for slug in &slugs {
            jobs::queue_agent_job(
                state,
                row.group_id,
                row.thread_id,
                number,
                row.user_id,
                &editor,
                JobKind::PaperUpdate,
                Some(slug),
                false,
            )
            .await?;
        }
    }
    Ok(number)
}

/// The one message that reports on a task's run.
fn report_message(
    row: &TaskRow,
    end: TaskState,
    outcome: &TaskOutcome,
    changed: &[String],
    papers: &[String],
    editor: Option<&AgentProfile>,
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
    if let Some(editor) = editor {
        let papers: Vec<String> = papers.iter().map(|slug| format!("[[p/{slug}]]")).collect();
        text.push_str(&format!(
            "\n\n{} will update {}.",
            editor.display_name,
            papers.join(", ")
        ));
    }
    text
}

/// The papers whose text references any of `names`, as `[[lean:Name]]`.
async fn papers_referencing(
    state: &AppState,
    group_id: i64,
    names: &[String],
) -> ApiResult<Vec<String>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let papers: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.slug, r.body FROM papers p
         JOIN paper_revisions r ON r.paper_id = p.id AND r.number = p.revision
         WHERE p.group_id = ? ORDER BY p.updated_at DESC",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
    Ok(papers
        .into_iter()
        .filter(|(_, body)| {
            stemma_core::references(body).iter().any(|reference| {
                matches!(&reference.address, Ok(Address::Declaration(name)) if wanted.contains(name.as_str()))
            })
        })
        .map(|(slug, _)| slug)
        .collect())
}

// ── Settings ──────────────────────────────────────────────────────────

/// How the group's workshop works, for `viewer_id`.
pub async fn settings(
    state: &AppState,
    group_id: i64,
    viewer_id: i64,
) -> ApiResult<WorkshopSettings> {
    let (from, to, time_zone): (i64, i64, String) =
        sqlx::query_as("SELECT night_from, night_to, time_zone FROM groups WHERE id = ?")
            .bind(group_id)
            .fetch_one(&state.db)
            .await?;
    let lenders: Vec<crate::api::UserColumns> = sqlx::query_as(
        "SELECT u.id AS user_id, u.username, u.display_name
         FROM memberships m JOIN users u ON u.id = m.user_id
         WHERE m.group_id = ? AND m.lends_worker = 1 ORDER BY u.display_name",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    let lenders: Vec<User> = lenders.into_iter().map(User::from).collect();
    Ok(WorkshopSettings {
        night_from: clock(from),
        night_to: clock(to),
        night_now: is_night(jiff::Timestamp::now(), &time_zone, from, to),
        time_zone,
        lend: lenders.iter().any(|u| u.id == viewer_id),
        lenders,
    })
}

/// Changes the night window and time zone.
pub async fn set_night(
    state: &AppState,
    group_id: i64,
    from: Option<&str>,
    to: Option<&str>,
    time_zone: Option<&str>,
) -> ApiResult<()> {
    let minutes = |text: Option<&str>, what: &str| -> ApiResult<Option<i64>> {
        text.map(|t| {
            parse_clock(t)
                .ok_or_else(|| ApiError::invalid(format!("The night {what} is a time, as 01:00")))
        })
        .transpose()
    };
    let from = minutes(from, "starts at")?;
    let to = minutes(to, "ends at")?;
    let time_zone = time_zone.map(str::trim);
    if let Some(name) = time_zone
        && jiff::tz::TimeZone::get(name).is_err()
    {
        return Err(ApiError::invalid(format!(
            "{name} is not a time zone: write one as Europe/Madrid, or UTC"
        )));
    }
    sqlx::query(
        "UPDATE groups SET night_from = coalesce(?, night_from), night_to = coalesce(?, night_to),
                           time_zone = coalesce(?, time_zone)
         WHERE id = ?",
    )
    .bind(from)
    .bind(to)
    .bind(time_zone)
    .bind(group_id)
    .execute(&state.db)
    .await?;
    Ok(())
}

/// Whether a member lends their worker to the workshop at night.
pub async fn set_lend(state: &AppState, group_id: i64, user_id: i64, lend: bool) -> ApiResult<()> {
    sqlx::query("UPDATE memberships SET lends_worker = ? WHERE group_id = ? AND user_id = ?")
        .bind(lend)
        .bind(group_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_clocks() {
        assert_eq!(parse_clock("01:00"), Some(60));
        assert_eq!(parse_clock("7:30"), Some(450));
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("1:5"), None);
        assert_eq!(clock(420), "07:00");
    }

    #[test]
    fn nights_span_midnight() {
        let at = |text: &str| text.parse::<jiff::Timestamp>().unwrap();
        // 01:00–07:00 in Madrid, which is UTC+2 in summer.
        assert!(is_night(
            at("2026-07-01T00:30:00Z"),
            "Europe/Madrid",
            60,
            420
        ));
        assert!(!is_night(
            at("2026-07-01T05:30:00Z"),
            "Europe/Madrid",
            60,
            420
        ));
        // 22:00–06:00 in UTC.
        assert!(is_night(at("2026-07-01T23:00:00Z"), "UTC", 1320, 360));
        assert!(is_night(at("2026-07-01T05:00:00Z"), "UTC", 1320, 360));
        assert!(!is_night(at("2026-07-01T12:00:00Z"), "UTC", 1320, 360));
        // An empty window.
        assert!(!is_night(at("2026-07-01T12:00:00Z"), "UTC", 600, 600));
    }

    #[test]
    fn tasks_from_messages() {
        let (title, schedule) = from_message(
            "@workshop formalize the lemma in t12#48 tonight. It is the gluing one.",
            "workshop",
        );
        assert_eq!(title, "Formalize the lemma in t12#48 tonight");
        assert_eq!(schedule, TaskSchedule::Night);
        let (title, schedule) = from_message("@workshop: prove [[lean:Foo.bar]]", "workshop");
        assert_eq!(title, "Prove [[lean:Foo.bar]]");
        assert_eq!(schedule, TaskSchedule::Now);
    }
}
