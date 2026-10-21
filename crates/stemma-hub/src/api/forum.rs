//! The forum: threads, numbered messages and what references resolve to.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::{AssertSqlSafe, Sqlite, Transaction};
use stemma_core::Address;
use stemma_core::api::{
    CreateThread, EditMessage, Event, Message, PostMessage, Resolve, Resolved, SearchHit, Thread,
    ThreadKind, ThreadPage, UpdateThread,
};

use super::attachments::{self, attachments_of};
use super::{Json, Membership, UserColumns, required_text};
use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const MAX_BODY_CHARS: usize = 100_000;

/// What [`ThreadRow`] reads, from `threads t` and its creator.
const THREAD_COLUMNS: &str =
    "SELECT t.id, t.number, t.title, t.kind, t.resident_agent, t.resident_runner, t.anchor, t.created_at, t.last_activity_at,
            t.last_message, u.id AS user_id, u.username, u.display_name
     FROM threads t JOIN users u ON u.id = t.created_by";

#[derive(sqlx::FromRow)]
pub(crate) struct ThreadRow {
    pub id: i64,
    number: i64,
    title: String,
    kind: String,
    resident_agent: Option<String>,
    resident_runner: Option<String>,
    anchor: Option<String>,
    created_at: i64,
    last_activity_at: i64,
    pub(crate) last_message: i64,
    #[sqlx(flatten)]
    creator: UserColumns,
}

impl From<ThreadRow> for Thread {
    fn from(row: ThreadRow) -> Self {
        Thread {
            number: row.number as u32,
            title: row.title,
            kind: ThreadKind::parse(&row.kind).unwrap_or(ThreadKind::Discussion),
            resident_agent: row.resident_agent,
            resident_runner: row.resident_runner,
            anchor: row.anchor.and_then(|a| a.parse().ok()),
            created_by: row.creator.into(),
            created_at: db::from_millis(row.created_at),
            last_activity_at: db::from_millis(row.last_activity_at),
            last_message: row.last_message as u32,
        }
    }
}

pub(crate) async fn find_thread_by_id(state: &AppState, thread_id: i64) -> ApiResult<ThreadRow> {
    sqlx::query_as(AssertSqlSafe(format!("{THREAD_COLUMNS} WHERE t.id = ?")))
        .bind(thread_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Thread"))
}

pub(crate) async fn find_thread(
    state: &AppState,
    group_id: i64,
    number: u32,
) -> ApiResult<ThreadRow> {
    sqlx::query_as(AssertSqlSafe(format!(
        "{THREAD_COLUMNS} WHERE t.group_id = ? AND t.number = ?"
    )))
    .bind(group_id)
    .bind(number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Thread"))
}

#[derive(Deserialize)]
pub struct ThreadQuery {
    kind: Option<ThreadKind>,
    /// Only threads active before this many milliseconds since the epoch,
    /// to page back.
    before: Option<i64>,
    limit: Option<u32>,
}

/// The group's threads, most recently active first.
pub async fn list_threads(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Query(query): Query<ThreadQuery>,
) -> ApiResult<Json<Vec<Thread>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let rows: Vec<ThreadRow> = sqlx::query_as(AssertSqlSafe(format!(
        "{THREAD_COLUMNS}
         WHERE t.group_id = ?1 AND (?2 IS NULL OR t.kind = ?2) AND t.last_activity_at < ?3
         ORDER BY t.last_activity_at DESC LIMIT ?4"
    )))
    .bind(group.group_id)
    .bind(query.kind.map(ThreadKind::as_str))
    .bind(query.before.unwrap_or(i64::MAX))
    .bind(query.limit.unwrap_or(50).min(200))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(Thread::from).collect()))
}

pub async fn create_thread(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<CreateThread>,
) -> ApiResult<(StatusCode, Json<ThreadPage>)> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let title = required_text(&body.title, "The title", 200)?;
    // A session's agent opens discussions, as itself; a job's never does.
    let agent = match viewer.agent() {
        None => None,
        Some(scope) if scope.session && body.kind == ThreadKind::Discussion => {
            Some(scope.agent.clone())
        }
        Some(_) => {
            return Err(ApiError::forbidden(
                "This agent's token cannot open this thread",
            ));
        }
    };
    if viewer.agent().is_some() && !body.attachments.is_empty() {
        return Err(ApiError::forbidden("An agent's token cannot attach files"));
    }
    let (resident_agent, resident_runner) = match (body.kind, body.resident_agent) {
        (_, None) => (None, None),
        (ThreadKind::Chat, Some(agent)) => {
            let agent = agent.trim().trim_start_matches('@').to_lowercase();
            if !stemma_core::mention::is_handle(&agent) {
                return Err(ApiError::invalid("That is not an agent's handle"));
            }
            // One of the writer's: each person writing in the chat is then
            // answered by theirs.
            let profile = stemma_core::api::AgentProfile::default_for(&agent)
                .ok_or_else(|| ApiError::invalid(format!("Unknown agent @{agent}")))?;
            (Some(agent), Some(profile.runner))
        }
        (ThreadKind::Discussion, Some(_)) => {
            return Err(ApiError::invalid("Only chats have a resident agent"));
        }
    };
    match &body.anchor {
        None | Some(Address::Declaration(_)) => {}
        Some(Address::Paper(paper)) => {
            crate::papers::find(&state, group.group_id, paper).await?;
        }
        Some(_) => {
            return Err(ApiError::invalid(
                "A thread is anchored to a declaration or a paper",
            ));
        }
    }
    let first = match &body.body {
        Some(text) if !text.trim().is_empty() || !body.attachments.is_empty() => {
            Some(check_body(text)?)
        }
        _ if !body.attachments.is_empty() => Some(String::new()),
        _ => None,
    };

    let now = db::now();
    let agent_runner = attribution_runner(&state, &viewer, agent.as_deref()).await?;
    let mut tx = state.db.begin().await?;
    let number: i64 = sqlx::query_scalar(
        "UPDATE groups SET last_thread = last_thread + 1 WHERE id = ? RETURNING last_thread",
    )
    .bind(group.group_id)
    .fetch_one(&mut *tx)
    .await?;
    let thread_id = sqlx::query(
        "INSERT INTO threads (group_id, number, title, kind, resident_agent, resident_runner,
                              anchor, created_by,
                              created_at, last_activity_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(number)
    .bind(&title)
    .bind(body.kind.as_str())
    .bind(&resident_agent)
    .bind(&resident_runner)
    .bind(body.anchor.as_ref().map(ToString::to_string))
    .bind(viewer.id)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    if let Some(text) = &first {
        insert_message(
            &mut tx,
            &group,
            thread_id,
            viewer.id,
            agent.as_deref(),
            agent_runner.as_deref(),
            text,
            None,
            &body.attachments,
        )
        .await?;
    }
    tx.commit().await?;

    let thread: Thread = find_thread(&state, group.group_id, number as u32)
        .await?
        .into();
    let messages = load_messages(&state, &group, thread_id, 1, u32::MAX).await?;
    state.publish(
        group.group_id,
        Event::ThreadCreated {
            thread: thread.clone(),
        },
    );
    if let Some(message) = messages.first() {
        state.publish(
            group.group_id,
            Event::MessageCreated {
                thread: thread.number,
                message: message.clone(),
            },
        );
        crate::jobs::summon(&state, &group, thread_id, &thread, message).await?;
    }
    Ok((StatusCode::CREATED, Json(ThreadPage { thread, messages })))
}

/// A thread with all its messages.
pub async fn get_thread(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<Json<ThreadPage>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    let messages = load_messages(&state, &group, row.id, 1, u32::MAX).await?;
    Ok(Json(ThreadPage {
        thread: row.into(),
        messages,
    }))
}

/// Renames a thread: its creator or a group admin.
pub async fn update_thread(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
    Json(body): Json<UpdateThread>,
) -> ApiResult<Json<Thread>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    if row.creator.user_id != viewer.id {
        group.require_admin()?;
    }
    let title = required_text(&body.title, "The title", 200)?;
    sqlx::query("UPDATE threads SET title = ? WHERE id = ?")
        .bind(&title)
        .bind(row.id)
        .execute(&state.db)
        .await?;
    let thread: Thread = find_thread(&state, group.group_id, number).await?.into();
    state.publish(
        group.group_id,
        Event::ThreadUpdated {
            thread: thread.clone(),
        },
    );
    Ok(Json(thread))
}

#[derive(Deserialize)]
pub struct MessageRange {
    from: Option<u32>,
    to: Option<u32>,
}

/// The messages of a thread, all or from `from` to `to`, both included.
pub async fn list_messages(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
    Query(range): Query<MessageRange>,
) -> ApiResult<Json<Vec<Message>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    let from = range.from.unwrap_or(1);
    let to = range.to.unwrap_or(u32::MAX);
    Ok(Json(load_messages(&state, &group, row.id, from, to).await?))
}

pub async fn get_message(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number, message)): Path<(String, u32, u32)>,
) -> ApiResult<Json<Message>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    load_message(&state, &group, row.id, message)
        .await
        .map(Json)
}

pub async fn post_message(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
    Json(body): Json<PostMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    let agent = match viewer.agent() {
        Some(scope) if scope.can_post && scope.thread_id.is_none_or(|id| id == row.id) => {
            Some(scope.agent.clone())
        }
        Some(_) => {
            return Err(ApiError::forbidden(
                "This agent's token cannot post in this thread",
            ));
        }
        None => match body.agent.as_deref().map(str::trim) {
            Some(_) if !viewer.with_token => {
                return Err(ApiError::invalid("Only an API token posts for an agent"));
            }
            Some(agent) => Some(required_text(agent, "The agent's name", 60)?),
            None => None,
        },
    };
    // An agent's uploads would be its person's, waiting for their message.
    if viewer.agent().is_some() && !body.attachments.is_empty() {
        return Err(ApiError::forbidden("An agent's token cannot attach files"));
    }
    if body.body.trim().is_empty() && body.attachments.is_empty() {
        return Err(ApiError::invalid("A message needs words or a file"));
    }
    let text = check_body(&body.body)?;
    if let Some(reply_to) = body.reply_to
        && (reply_to == 0 || i64::from(reply_to) > row.last_message)
    {
        return Err(ApiError::invalid(format!(
            "There is no message #{reply_to} to reply to"
        )));
    }

    let agent_runner = attribution_runner(&state, &viewer, agent.as_deref()).await?;
    let message = post(
        &state,
        &group,
        row.id,
        viewer.id,
        agent.as_deref(),
        agent_runner.as_deref(),
        &text,
        body.reply_to,
        &body.attachments,
    )
    .await?;
    if agent.is_none() {
        let thread: Thread = find_thread(&state, group.group_id, number).await?.into();
        crate::jobs::summon(&state, &group, row.id, &thread, &message).await?;
    }
    Ok((StatusCode::CREATED, Json(message)))
}
async fn attribution_runner(
    state: &AppState,
    viewer: &Viewer,
    agent: Option<&str>,
) -> ApiResult<Option<String>> {
    if let Some(scope) = viewer.agent().filter(|scope| scope.session) {
        return Ok(stemma_core::api::AgentProfile::defaults()
            .into_iter()
            .find(|agent| agent.display_name == scope.agent)
            .map(|agent| agent.runner));
    }
    if let Some(job) = viewer.agent() {
        let runner: Option<String> =
            sqlx::query_scalar("SELECT runner FROM jobs WHERE token_id = ?")
                .bind(job.token_id)
                .fetch_optional(&state.db)
                .await?;
        return Ok(runner);
    }
    Ok(agent.and_then(|name| {
        stemma_core::api::runners::ALL
            .iter()
            .find(|&&runner| stemma_core::api::runners::name(runner).eq_ignore_ascii_case(name))
            .map(|r| (*r).to_owned())
    }))
}

/// Posts a message in a thread, and tells the group's listeners.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn post(
    state: &AppState,
    group: &Membership,
    thread_id: i64,
    author_id: i64,
    agent: Option<&str>,
    agent_runner: Option<&str>,
    text: &str,
    reply_to: Option<u32>,
    attachments: &[i64],
) -> ApiResult<Message> {
    let mut tx = state.db.begin().await?;
    let number = insert_message(
        &mut tx,
        group,
        thread_id,
        author_id,
        agent,
        agent_runner,
        text,
        reply_to,
        attachments,
    )
    .await?;
    tx.commit().await?;

    let message = load_message(state, group, thread_id, number).await?;
    let thread: Thread = find_thread_by_id(state, thread_id).await?.into();
    state.publish(
        group.group_id,
        Event::MessageCreated {
            thread: thread.number,
            message: message.clone(),
        },
    );
    state.publish(group.group_id, Event::ThreadUpdated { thread });
    Ok(message)
}

pub(crate) fn check_body(text: &str) -> ApiResult<String> {
    if text.chars().count() > MAX_BODY_CHARS {
        return Err(ApiError::invalid(format!(
            "A message has at most {MAX_BODY_CHARS} characters"
        )));
    }
    Ok(text.trim_end().to_owned())
}

/// Adds a message to a thread with the next number, and attaches the
/// author's uploads to it. Returns its number.
#[allow(clippy::too_many_arguments)]
async fn insert_message(
    tx: &mut Transaction<'_, Sqlite>,
    group: &Membership,
    thread_id: i64,
    author_id: i64,
    agent: Option<&str>,
    agent_runner: Option<&str>,
    text: &str,
    reply_to: Option<u32>,
    attachment_ids: &[i64],
) -> ApiResult<u32> {
    let now = db::now();
    let number: i64 = sqlx::query_scalar(
        "UPDATE threads SET last_message = last_message + 1, last_activity_at = ?
         WHERE id = ? RETURNING last_message",
    )
    .bind(now)
    .bind(thread_id)
    .fetch_one(&mut **tx)
    .await?;
    let message_id = sqlx::query(
        "INSERT INTO messages (thread_id, number, author_id, agent, agent_runner, body, reply_to, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(thread_id)
    .bind(number)
    .bind(author_id)
    .bind(agent)
    .bind(agent_runner)
    .bind(text)
    .bind(reply_to)
    .bind(now)
    .execute(&mut **tx)
    .await?
    .last_insert_rowid();
    for &id in attachment_ids {
        let done = sqlx::query(
            "UPDATE attachments SET message_id = ?
             WHERE id = ? AND group_id = ? AND uploader_id = ? AND message_id IS NULL",
        )
        .bind(message_id)
        .bind(id)
        .bind(group.group_id)
        .bind(author_id)
        .execute(&mut **tx)
        .await?;
        if done.rows_affected() == 0 {
            return Err(ApiError::invalid(format!(
                "File {id} is not one of your uploads waiting for a message"
            )));
        }
    }
    Ok(number as u32)
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: i64,
    number: i64,
    agent: Option<String>,
    agent_runner: Option<String>,
    body: String,
    reply_to: Option<i64>,
    created_at: i64,
    edited_at: Option<i64>,
    deleted_at: Option<i64>,
    #[sqlx(flatten)]
    author: UserColumns,
}

pub(crate) async fn load_messages(
    state: &AppState,
    group: &Membership,
    thread_id: i64,
    from: u32,
    to: u32,
) -> ApiResult<Vec<Message>> {
    let rows: Vec<MessageRow> = sqlx::query_as(
        "SELECT m.id, m.number, m.agent, m.agent_runner, m.body, m.reply_to, m.created_at, m.edited_at,
                m.deleted_at, u.id AS user_id, u.username, u.display_name
         FROM messages m JOIN users u ON u.id = m.author_id
         WHERE m.thread_id = ? AND m.number BETWEEN ? AND ?
         ORDER BY m.number",
    )
    .bind(thread_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    let mut files = attachments_of(state, group, thread_id, from, to).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let attachments = files.remove(&row.id).unwrap_or_default();
            to_message(row, attachments)
        })
        .collect())
}

pub(crate) async fn load_message(
    state: &AppState,
    group: &Membership,
    thread_id: i64,
    number: u32,
) -> ApiResult<Message> {
    load_messages(state, group, thread_id, number, number)
        .await?
        .pop()
        .ok_or_else(|| ApiError::not_found("Message"))
}

fn to_message(row: MessageRow, attachments: Vec<stemma_core::api::Attachment>) -> Message {
    Message {
        number: row.number as u32,
        author: row.author.into(),
        agent: row.agent,
        agent_runner: row.agent_runner,
        body: row.body,
        reply_to: row.reply_to.map(|n| n as u32),
        attachments,
        created_at: db::from_millis(row.created_at),
        edited_at: row.edited_at.map(db::from_millis),
        deleted: row.deleted_at.is_some(),
    }
}

/// The message's id and author, if it exists and is not deleted.
async fn editable_message(state: &AppState, thread_id: i64, number: u32) -> ApiResult<(i64, i64)> {
    sqlx::query_as(
        "SELECT id, author_id FROM messages
         WHERE thread_id = ? AND number = ? AND deleted_at IS NULL",
    )
    .bind(thread_id)
    .bind(number)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Message"))
}

/// Edits a message: only its author. Its number stays.
pub async fn edit_message(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number, message)): Path<(String, u32, u32)>,
    Json(body): Json<EditMessage>,
) -> ApiResult<Json<Message>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    let (id, author) = editable_message(&state, row.id, message).await?;
    if author != viewer.id {
        return Err(ApiError::forbidden("Only its author can edit a message"));
    }
    let text = check_body(&body.body)?;
    sqlx::query("UPDATE messages SET body = ?, edited_at = ? WHERE id = ?")
        .bind(text)
        .bind(db::now())
        .bind(id)
        .execute(&state.db)
        .await?;
    let message = load_message(&state, &group, row.id, message).await?;
    state.publish(
        group.group_id,
        Event::MessageUpdated {
            thread: number,
            message: message.clone(),
        },
    );
    Ok(Json(message))
}

/// Deletes a message, with its files: its author or a group admin. The
/// number stays, so references to it still say where it was.
pub async fn delete_message(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number, message)): Path<(String, u32, u32)>,
) -> ApiResult<Json<Message>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    let row = find_thread(&state, group.group_id, number).await?;
    let (id, author) = editable_message(&state, row.id, message).await?;
    if author != viewer.id {
        group.require_admin()?;
    }
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE messages SET body = '', deleted_at = ? WHERE id = ?")
        .bind(db::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let keys: Vec<String> =
        sqlx::query_scalar("DELETE FROM attachments WHERE message_id = ? RETURNING storage_key")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    tx.commit().await?;
    attachments::remove_files(&state, &keys).await;

    let message = load_message(&state, &group, row.id, message).await?;
    state.publish(
        group.group_id,
        Event::MessageUpdated {
            thread: number,
            message: message.clone(),
        },
    );
    Ok(Json(message))
}

/// What each of a list of addresses names in the group, for showing
/// references.
pub async fn resolve(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<Resolve>,
) -> ApiResult<Json<Vec<Resolved>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    if body.addresses.len() > 200 {
        return Err(ApiError::invalid("Resolve at most 200 addresses at once"));
    }
    let mut threads: HashMap<u32, Option<(String, u32)>> = HashMap::new();
    let mut resolved = Vec::with_capacity(body.addresses.len());
    for raw in body.addresses {
        let address = match raw.trim().parse::<Address>() {
            Ok(address) => address,
            Err(error) => {
                resolved.push(Resolved {
                    address: raw,
                    canonical: None,
                    kind: None,
                    exists: false,
                    path: None,
                    title: None,
                    note: Some(error.to_string()),
                    state: None,
                });
                continue;
            }
        };
        let mut entry = Resolved {
            address: raw,
            canonical: Some(address.to_string()),
            kind: Some(address.kind()),
            exists: false,
            path: Some(address.web_path(&group.slug)),
            title: None,
            note: None,
            state: None,
        };
        match address.thread() {
            Some(number) => {
                if let Entry::Vacant(slot) = threads.entry(number) {
                    let found = match find_thread(&state, group.group_id, number).await {
                        Ok(row) => Some((row.title, row.last_message as u32)),
                        Err(e) if e.status == StatusCode::NOT_FOUND => None,
                        Err(e) => return Err(e),
                    };
                    slot.insert(found);
                }
                match &threads[&number] {
                    Some((title, last)) => {
                        let last_needed = match address {
                            Address::Message { message, .. }
                            | Address::Attachment { message, .. } => message,
                            Address::Messages { to, .. } => to,
                            _ => 0,
                        };
                        entry.title = Some(title.clone());
                        entry.exists = last_needed <= *last;
                        if !entry.exists {
                            entry.note = Some(format!("The thread has {last} messages"));
                        }
                    }
                    None => entry.note = Some("There is no such thread".into()),
                }
            }
            None => match &address {
                Address::Declaration(name) => {
                    let found = crate::library::resolve(&state, group.group_id, name).await?;
                    entry.exists = found.exists;
                    entry.title = found.title;
                    entry.note = found.note;
                    entry.state = found.state;
                }

                Address::Paper(slug) => {
                    let (exists, title) =
                        crate::papers::resolve(&state, group.group_id, slug).await?;
                    entry.exists = exists;
                    entry.title = title;
                    if !exists {
                        entry.note = Some("There is no such paper".into());
                    }
                }
                _ => {}
            },
        }
        resolved.push(entry);
    }
    Ok(Json(resolved))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
    limit: Option<u32>,
}

/// Threads whose title, and messages whose text, match a query, with its
/// operators: see [`crate::search`].
pub async fn search(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Vec<SearchHit>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let hits = crate::search::run(&state, group.group_id, &query.q, limit).await?;
    Ok(Json(hits))
}
