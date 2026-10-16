//! Working papers: Markdown narratives of a group's lines of work, with
//! their revisions, and the revision each person read last.
//!
//! A revision is written over a base revision: if the paper moved past it
//! meanwhile, the edit is refused, so that nobody's work is lost
//! silently. People write revisions themselves, and agents write them
//! for people: through a person's API token, or as an editor's job.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::Address;
use stemma_core::address::is_slug;
use stemma_core::api::{
    CreatePaper, EditPaper, Event, MarkRead, Paper, PaperPage, Revision, RevisionText, SectionInfo,
    Thread,
};
use stemma_core::paper::sections;

use super::{Json, Membership, UserColumns, required_text};
use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The longest a paper may be, in characters.
const MAX_PAPER_CHARS: usize = 400_000;

#[derive(sqlx::FromRow)]
struct PaperRow {
    slug: String,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    title: String,
    updated_agent: Option<String>,
    creator_id: i64,
    creator_username: String,
    creator_display_name: String,
    #[sqlx(flatten)]
    updater: UserColumns,
    last_read: Option<i64>,
}

impl From<PaperRow> for Paper {
    fn from(row: PaperRow) -> Self {
        Paper {
            slug: row.slug,
            title: row.title,
            revision: row.revision as u32,
            created_by: stemma_core::api::User {
                id: row.creator_id,
                username: row.creator_username,
                display_name: row.creator_display_name,
            },
            created_at: db::from_millis(row.created_at),
            updated_by: row.updater.into(),
            updated_agent: row.updated_agent,
            updated_at: db::from_millis(row.updated_at),
            last_read: row.last_read.map(|n| n as u32),
        }
    }
}

/// Papers, with what the person `?1` read last.
macro_rules! paper_query {
    () => {
        "SELECT p.slug, p.revision, p.created_at, p.updated_at, r.title,
                r.agent AS updated_agent, c.id AS creator_id, c.username AS creator_username,
                c.display_name AS creator_display_name,
                a.id AS user_id, a.username, a.display_name,
                (SELECT revision FROM paper_reads WHERE paper_id = p.id AND user_id = ?1)
                    AS last_read
         FROM papers p
         JOIN paper_revisions r ON r.paper_id = p.id AND r.number = p.revision
         JOIN users c ON c.id = p.created_by
         JOIN users a ON a.id = r.author_id"
    };
}

async fn find(state: &AppState, viewer_id: i64, group_id: i64, slug: &str) -> ApiResult<Paper> {
    let row: Option<PaperRow> = sqlx::query_as(concat!(
        paper_query!(),
        " WHERE p.group_id = ?2 AND p.slug = ?3"
    ))
    .bind(viewer_id)
    .bind(group_id)
    .bind(slug)
    .fetch_optional(&state.db)
    .await?;
    row.map(Paper::from)
        .ok_or_else(|| ApiError::not_found("Paper"))
}

async fn paper_id(state: &AppState, group_id: i64, slug: &str) -> ApiResult<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT id FROM papers WHERE group_id = ? AND slug = ?")
            .bind(group_id)
            .bind(slug)
            .fetch_optional(&state.db)
            .await?,
    )
}

/// A paper as it stands now.
pub struct Current {
    pub title: String,
    pub revision: u32,
    pub body: String,
}

pub async fn current(state: &AppState, group_id: i64, slug: &str) -> ApiResult<Option<Current>> {
    let row: Option<(String, i64, String)> = sqlx::query_as(
        "SELECT r.title, p.revision, r.body
         FROM papers p JOIN paper_revisions r ON r.paper_id = p.id AND r.number = p.revision
         WHERE p.group_id = ? AND p.slug = ?",
    )
    .bind(group_id)
    .bind(slug)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|(title, revision, body)| Current {
        title,
        revision: revision as u32,
        body,
    }))
}

/// A new revision, as someone writes it.
pub struct Written {
    pub author_id: i64,
    /// The agent that writes it for its author, if one does.
    pub agent: Option<String>,
    /// The title, if it changes; a new paper needs one.
    pub title: Option<String>,
    pub body: String,
    pub reason: String,
}

/// What became of a revision.
pub enum Stored {
    Revised(Box<Paper>),
    /// The paper is past the revision it was written over.
    Moved {
        current: u32,
    },
}

/// Stores a revision of the paper `slug` written over revision `base`, or
/// starts the paper if `base` is 0 and there is none. Tells the group.
pub async fn store(
    state: &AppState,
    group: &Membership,
    slug: &str,
    base: u32,
    written: Written,
) -> ApiResult<Stored> {
    if !is_slug(slug) || slug.len() > 60 {
        return Err(ApiError::invalid(
            "A paper's short name is up to 60 lowercase letters, digits and inner dashes",
        ));
    }
    let title = match &written.title {
        Some(title) => Some(required_text(title, "The title", 200)?),
        None => None,
    };
    let reason = required_text(&written.reason, "The reason", 1_000)?;
    if written.body.chars().count() > MAX_PAPER_CHARS {
        return Err(ApiError::invalid(format!(
            "A paper has at most {MAX_PAPER_CHARS} characters"
        )));
    }
    let body = format!("{}\n", written.body.trim_end());
    let body = if body.trim().is_empty() {
        String::new()
    } else {
        body
    };

    let now = db::now();
    let mut tx = state.db.begin().await?;
    let existing: Option<(i64, i64, String)> = sqlx::query_as(
        "SELECT p.id, p.revision, r.title
         FROM papers p JOIN paper_revisions r ON r.paper_id = p.id AND r.number = p.revision
         WHERE p.group_id = ? AND p.slug = ?",
    )
    .bind(group.group_id)
    .bind(slug)
    .fetch_optional(&mut *tx)
    .await?;
    let (paper_id, number, title) = match existing {
        Some((_, current, _)) if current as u32 != base => {
            return Ok(Stored::Moved {
                current: current as u32,
            });
        }
        Some((id, current, old_title)) => (id, current + 1, title.unwrap_or(old_title)),
        None if base != 0 => return Err(ApiError::not_found("Paper")),
        None => {
            let title = title.ok_or_else(|| ApiError::invalid("A new paper needs a title"))?;
            let id = sqlx::query(
                "INSERT INTO papers (group_id, slug, created_by, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(group.group_id)
            .bind(slug)
            .bind(written.author_id)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
            (id, 1, title)
        }
    };
    sqlx::query(
        "INSERT INTO paper_revisions (paper_id, number, title, body, author_id, agent, reason,
                                      created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(paper_id)
    .bind(number)
    .bind(&title)
    .bind(&body)
    .bind(written.author_id)
    .bind(&written.agent)
    .bind(&reason)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE papers SET revision = ?, updated_at = ? WHERE id = ?")
        .bind(number)
        .bind(now)
        .bind(paper_id)
        .execute(&mut *tx)
        .await?;
    // Whoever writes a revision themselves has read it.
    if written.agent.is_none() {
        mark(&mut *tx, paper_id, written.author_id, number).await?;
    }
    tx.commit().await?;

    let mut paper = find(state, 0, group.group_id, slug).await?;
    state.publish(
        group.group_id,
        Event::PaperUpdated {
            paper: paper.clone(),
        },
    );
    paper.last_read = (written.agent.is_none()).then_some(paper.revision);
    Ok(Stored::Revised(Box::new(paper)))
}

async fn mark<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    paper_id: i64,
    user_id: i64,
    revision: i64,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO paper_reads (paper_id, user_id, revision, read_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (paper_id, user_id) DO UPDATE
         SET revision = max(revision, excluded.revision), read_at = excluded.read_at",
    )
    .bind(paper_id)
    .bind(user_id)
    .bind(revision)
    .bind(db::now())
    .execute(db)
    .await?;
    Ok(())
}

/// The threads about a paper, most recently active first: those anchored
/// to it or a section of it, with their anchor, and those whose messages
/// reference it.
pub async fn threads_about(
    state: &AppState,
    group_id: i64,
    slug: &str,
) -> ApiResult<Vec<(i64, Option<Address>)>> {
    let paper = format!("p/{slug}");
    let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.anchor FROM threads t
         WHERE t.group_id = ?1
           AND (t.anchor = ?2 OR t.anchor LIKE ?2 || '#%'
                OR EXISTS (SELECT 1 FROM messages m
                           WHERE m.thread_id = t.id AND m.deleted_at IS NULL
                             AND (instr(m.body, '[[' || ?2 || ']]') > 0
                                  OR instr(m.body, '[[' || ?2 || '#') > 0)))
         ORDER BY t.last_activity_at DESC LIMIT 50",
    )
    .bind(group_id)
    .bind(&paper)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, anchor)| {
            let anchor = anchor
                .and_then(|a| a.parse::<Address>().ok())
                .filter(|a| matches!(a, Address::Paper { slug: s, .. } if s == slug));
            (id, anchor)
        })
        .collect())
}

/// The group's papers, most recently changed first.
pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(group): Path<String>,
) -> ApiResult<Json<Vec<Paper>>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let rows: Vec<PaperRow> = sqlx::query_as(concat!(
        paper_query!(),
        " WHERE p.group_id = ?2 ORDER BY p.updated_at DESC"
    ))
    .bind(viewer.id)
    .bind(group.group_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(Paper::from).collect()))
}

/// The agent a person's token writes for, if it says so.
fn agent_of(viewer: &Viewer, agent: Option<&str>) -> ApiResult<Option<String>> {
    viewer.require_full()?;
    match agent.map(str::trim) {
        Some(_) if !viewer.with_token => {
            Err(ApiError::invalid("Only an API token writes for an agent"))
        }
        Some(agent) => Ok(Some(required_text(agent, "The agent's name", 60)?)),
        None => Ok(None),
    }
}

/// Starts a paper.
pub async fn create(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(group): Path<String>,
    Json(body): Json<CreatePaper>,
) -> ApiResult<(StatusCode, Json<Paper>)> {
    let agent = agent_of(&viewer, body.agent.as_deref())?;
    let group = Membership::of(&state, &viewer, &group).await?;
    let slug = body.slug.trim().to_owned();
    let written = Written {
        author_id: viewer.id,
        agent,
        title: Some(body.title),
        body: body.body,
        reason: body
            .reason
            .filter(|r| !r.trim().is_empty())
            .unwrap_or_else(|| "Started the paper".into()),
    };
    match store(&state, &group, &slug, 0, written).await? {
        Stored::Revised(paper) => Ok((StatusCode::CREATED, Json(*paper))),
        Stored::Moved { .. } => Err(ApiError::conflict(format!(
            "There is already a paper p/{slug}"
        ))),
    }
}

/// A paper, with what changed since the person read it last and the
/// threads anchored to it.
pub async fn get(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, slug)): Path<(String, String)>,
) -> ApiResult<Json<PaperPage>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let paper = find(&state, viewer.id, group.group_id, &slug).await?;
    let id = paper_id(&state, group.group_id, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper"))?;
    let body = revision_text(&state, id, paper.revision).await?.body;
    let (since_read, read_body) = match paper.last_read {
        Some(read) if read < paper.revision => (
            revisions_of(&state, id, read + 1, paper.revision).await?,
            Some(revision_text(&state, id, read).await?.body),
        ),
        _ => (Vec::new(), None),
    };
    let anchored: Vec<i64> = threads_about(&state, group.group_id, &slug)
        .await?
        .into_iter()
        .filter(|(_, anchor)| anchor.is_some())
        .map(|(id, _)| id)
        .collect();
    let mut threads: Vec<Thread> = Vec::new();
    for thread_id in anchored {
        threads.push(
            super::forum::find_thread_by_id(&state, thread_id)
                .await?
                .into(),
        );
    }
    let sections = sections(&body)
        .into_iter()
        .map(|s| SectionInfo {
            slug: s.slug,
            title: s.title,
            level: s.level,
        })
        .collect();
    Ok(Json(PaperPage {
        paper,
        body,
        sections,
        since_read,
        read_body,
        threads,
    }))
}

/// A new revision of a paper.
pub async fn edit(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, slug)): Path<(String, String)>,
    Json(body): Json<EditPaper>,
) -> ApiResult<Json<Paper>> {
    let agent = agent_of(&viewer, body.agent.as_deref())?;
    let group = Membership::of(&state, &viewer, &group).await?;
    if paper_id(&state, group.group_id, &slug).await?.is_none() {
        return Err(ApiError::not_found("Paper"));
    }
    if body.base_revision == 0 {
        return Err(ApiError::invalid("Say which revision the edit is over"));
    }
    let written = Written {
        author_id: viewer.id,
        agent,
        title: body.title.filter(|t| !t.trim().is_empty()),
        body: body.body,
        reason: body.reason,
    };
    match store(&state, &group, &slug, body.base_revision, written).await? {
        Stored::Revised(paper) => Ok(Json(*paper)),
        Stored::Moved { current } => Err(ApiError::conflict(format!(
            "The paper changed since revision {}: it is at revision {current}. Edit the new \
             text instead",
            body.base_revision
        ))),
    }
}

#[derive(sqlx::FromRow)]
struct RevisionRow {
    number: i64,
    title: String,
    agent: Option<String>,
    reason: String,
    created_at: i64,
    body: String,
    #[sqlx(flatten)]
    author: UserColumns,
}

impl RevisionRow {
    fn revision(self) -> RevisionText {
        RevisionText {
            revision: Revision {
                number: self.number as u32,
                title: self.title,
                author: self.author.into(),
                agent: self.agent,
                reason: self.reason,
                created_at: db::from_millis(self.created_at),
            },
            body: self.body,
        }
    }
}

macro_rules! revision_query {
    () => {
        "SELECT r.number, r.title, r.agent, r.reason, r.created_at, r.body,
                u.id AS user_id, u.username, u.display_name
         FROM paper_revisions r JOIN users u ON u.id = r.author_id"
    };
}

async fn revision_text(state: &AppState, paper_id: i64, number: u32) -> ApiResult<RevisionText> {
    let row: Option<RevisionRow> = sqlx::query_as(concat!(
        revision_query!(),
        " WHERE r.paper_id = ? AND r.number = ?"
    ))
    .bind(paper_id)
    .bind(number)
    .fetch_optional(&state.db)
    .await?;
    row.map(RevisionRow::revision)
        .ok_or_else(|| ApiError::not_found("Revision"))
}

/// Revisions `from` to `to` of a paper, newest first, without their text.
async fn revisions_of(
    state: &AppState,
    paper_id: i64,
    from: u32,
    to: u32,
) -> ApiResult<Vec<Revision>> {
    let rows: Vec<RevisionRow> = sqlx::query_as(concat!(
        revision_query!(),
        " WHERE r.paper_id = ? AND r.number BETWEEN ? AND ? ORDER BY r.number DESC"
    ))
    .bind(paper_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(|r| r.revision().revision).collect())
}

/// A paper's history: its revisions, newest first.
pub async fn revisions(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, slug)): Path<(String, String)>,
) -> ApiResult<Json<Vec<Revision>>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let id = paper_id(&state, group.group_id, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper"))?;
    Ok(Json(revisions_of(&state, id, 1, u32::MAX).await?))
}

/// A revision of a paper, with its text.
pub async fn revision(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, slug, number)): Path<(String, String, u32)>,
) -> ApiResult<Json<RevisionText>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let id = paper_id(&state, group.group_id, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper"))?;
    Ok(Json(revision_text(&state, id, number).await?))
}

/// Notes that the person read a paper up to a revision.
pub async fn mark_read(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, slug)): Path<(String, String)>,
    Json(body): Json<MarkRead>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &group).await?;
    let paper = find(&state, viewer.id, group.group_id, &slug).await?;
    if body.revision == 0 || body.revision > paper.revision {
        return Err(ApiError::invalid(format!(
            "The paper has revisions 1 to {}",
            paper.revision
        )));
    }
    let id = paper_id(&state, group.group_id, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper"))?;
    mark(&state.db, id, viewer.id, i64::from(body.revision)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What a paper's address resolves to: whether it, and its section, exist,
/// and a title.
pub async fn resolve(
    state: &AppState,
    group_id: i64,
    slug: &str,
    section: Option<&str>,
) -> ApiResult<(bool, Option<String>, Option<String>)> {
    let Some(paper) = current(state, group_id, slug).await? else {
        return Ok((false, None, Some("There is no such paper".into())));
    };
    let Some(section) = section else {
        return Ok((true, Some(paper.title), None));
    };
    match sections(&paper.body)
        .into_iter()
        .find(|s| s.slug == section)
    {
        Some(found) => Ok((
            true,
            Some(format!("{} § {}", paper.title, found.title)),
            None,
        )),
        None => Ok((
            false,
            Some(paper.title),
            Some("The paper has no such section".into()),
        )),
    }
}
