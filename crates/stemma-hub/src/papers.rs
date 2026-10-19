//! Working papers: the readable account of what the group is doing, each a
//! LaTeX repository of the group's organization in the forge (see
//! [`crate::forge`]), `paper-<slug>`, made from the template in
//! `latex/template` when an admin or a member creates it.
//!
//! The hub keeps each paper's title and authors: its creator, and those
//! its authors add. Each new commit of its `main` is compiled by a
//! workshop machine with LaTeX, with `latexmk`, which uploads the PDF and
//! says how it went; the hub keeps the last PDF that was made, which a
//! build that fails leaves as it is, and the newest build's log. The
//! workshop changes papers too, with the architecture `write`, and one of
//! a paper's authors approves the changes others asked for (see
//! [`crate::workshop::review`]).

use std::path::PathBuf;

use axum::http::StatusCode;
use stemma_core::address::is_slug;
use stemma_core::api::{BuildState, Event, PAPER_MAIN, Paper, PaperBuild, PaperPdf, Role, User};
use stemma_core::protocol::{PaperBuilt, PaperCompile};

use crate::api::Membership;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge::{self, MAIN};
use crate::jobs;
use crate::state::AppState;

/// The paper a new one starts as: a LaTeX article, its references, and
/// what git leaves out.
const TEMPLATE: &[(&str, &str)] = &[
    (PAPER_MAIN, include_str!("../../../latex/template/main.tex")),
    (
        "references.bib",
        include_str!("../../../latex/template/references.bib"),
    ),
    (
        ".gitignore",
        include_str!("../../../latex/template/gitignore"),
    ),
];

/// The most of a build's log that is kept, in bytes: its end.
const MAX_LOG: usize = 200_000;

/// The longest slug a paper may have.
const MAX_SLUG: usize = 60;

/// The runner of papers' builds, as jobs record it.
pub const LATEX: &str = "latex";

/// The repository of a paper, in its group's organization.
pub fn repository_of(slug: &str) -> String {
    format!("paper-{slug}")
}

/// Where the hub serves a paper over git, relative to it.
pub fn clone_path(group: &str, repository: &str) -> String {
    format!("/git/{group}/{repository}.git")
}

/// Text as LaTeX writes it, its special characters escaped.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => escaped.push_str("\\textbackslash{}"),
            '{' | '}' | '$' | '&' | '#' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(c);
            }
            '^' => escaped.push_str("\\textasciicircum{}"),
            '~' => escaped.push_str("\\textasciitilde{}"),
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

/// The files a paper starts with: the template, for its title, author and
/// group.
pub fn template(title: &str, author: &str, group: &str) -> Vec<(String, String)> {
    TEMPLATE
        .iter()
        .map(|(file, text)| {
            let text = text
                .replace("{{title}}", &escape(title))
                .replace("{{author}}", &escape(author))
                .replace("{{group}}", group);
            ((*file).to_owned(), text)
        })
        .collect()
}

// ── Papers ────────────────────────────────────────────────────────────

#[derive(sqlx::FromRow, Clone)]
pub struct PaperRow {
    pub id: i64,
    pub group_id: i64,
    pub slug: String,
    pub title: String,
    pub repository: String,
    created_by: i64,
    creator_username: String,
    creator_display_name: String,
    head: Option<String>,
    created_at: i64,
    group_slug: String,
}

macro_rules! paper_query {
    () => {
        "SELECT p.id, p.group_id, p.slug, p.title, p.repository, p.created_by,
                u.username AS creator_username, u.display_name AS creator_display_name,
                p.head, p.created_at, g.slug AS group_slug
         FROM papers p JOIN users u ON u.id = p.created_by JOIN groups g ON g.id = p.group_id"
    };
}

pub async fn find(state: &AppState, group_id: i64, slug: &str) -> ApiResult<PaperRow> {
    sqlx::query_as(concat!(
        paper_query!(),
        " WHERE p.group_id = ? AND p.slug = ?"
    ))
    .bind(group_id)
    .bind(slug.strip_prefix("p/").unwrap_or(slug))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Paper"))
}

pub async fn by_id(state: &AppState, id: i64) -> ApiResult<Option<PaperRow>> {
    Ok(sqlx::query_as(concat!(paper_query!(), " WHERE p.id = ?"))
        .bind(id)
        .fetch_optional(&state.db)
        .await?)
}

/// The paper whose repository is `repository`, in the group `group`.
pub async fn by_repository(
    state: &AppState,
    group_id: i64,
    repository: &str,
) -> ApiResult<Option<PaperRow>> {
    Ok(sqlx::query_as(concat!(
        paper_query!(),
        " WHERE p.group_id = ? AND p.repository = ?"
    ))
    .bind(group_id)
    .bind(repository)
    .fetch_optional(&state.db)
    .await?)
}

/// A paper's authors, its creator first.
pub async fn authors(state: &AppState, paper_id: i64) -> ApiResult<Vec<User>> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, u.display_name FROM paper_authors a
         JOIN users u ON u.id = a.user_id JOIN papers p ON p.id = a.paper_id
         WHERE a.paper_id = ? ORDER BY u.id = p.created_by DESC, a.added_at, u.id",
    )
    .bind(paper_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, username, display_name)| User {
            id,
            username,
            display_name,
        })
        .collect())
}

/// Whether someone is one of a paper's authors.
pub async fn is_author(state: &AppState, paper_id: i64, user_id: i64) -> ApiResult<bool> {
    let found: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM paper_authors WHERE paper_id = ? AND user_id = ?")
            .bind(paper_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    Ok(found.is_some())
}

impl PaperRow {
    fn creator(&self) -> User {
        User {
            id: self.created_by,
            username: self.creator_username.clone(),
            display_name: self.creator_display_name.clone(),
        }
    }

    /// The paper as people see it.
    pub async fn to_api(&self, state: &AppState) -> ApiResult<Paper> {
        self.to_api_noted(state, true).await
    }

    /// The paper as people see it; with why its newest build waits, if
    /// `noted`, which takes the workers' lock.
    async fn to_api_noted(&self, state: &AppState, noted: bool) -> ApiResult<Paper> {
        let latest = match latest_build(state, self.id).await? {
            Some(row) if noted => Some(row.to_api_noted(state).await?),
            Some(row) => Some(row.to_api(None)),
            None => None,
        };
        Ok(Paper {
            slug: self.slug.clone(),
            title: self.title.clone(),
            repository: self.repository.clone(),
            clone_path: clone_path(&self.group_slug, &self.repository),
            forge_url: state
                .forge
                .as_deref()
                .map(|forge| forge.public_url(&format!("{}/{}", self.group_slug, self.repository))),
            authors: authors(state, self.id).await?,
            created_by: self.creator(),
            created_at: db::from_millis(self.created_at),
            latest,
            pdf: self.pdf(state).await?,
        })
    }

    async fn pdf(&self, state: &AppState) -> ApiResult<Option<PaperPdf>> {
        // The row may predate a completed build; read its current PDF together
        // with that build's metadata, after reading the latest build state.
        let row: Option<(i64, String, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT b.id, b.commit_sha, b.pdf_size, b.finished_at FROM papers p
             JOIN paper_builds b ON b.id = p.pdf_build WHERE p.id = ?",
        )
        .bind(self.id)
        .fetch_optional(&state.db)
        .await?;
        Ok(match row {
            Some((build, commit, Some(size), finished_at)) => Some(PaperPdf {
                build,
                commit,
                size: size as u64,
                url: pdf_url(&self.group_slug, &self.slug),
                built_at: db::from_millis(finished_at.unwrap_or(self.created_at)),
            }),
            _ => None,
        })
    }
}

/// Where the hub serves a paper's PDF, relative to it.
pub fn pdf_url(group: &str, slug: &str) -> String {
    format!("/api/v1/groups/{group}/papers/{slug}/pdf")
}

/// Tells the paper's group how it is now.
pub async fn publish(state: &AppState, paper_id: i64) -> ApiResult<()> {
    if let Some(row) = by_id(state, paper_id).await? {
        let paper = row.to_api(state).await?;
        state.publish(
            row.group_id,
            Event::PaperUpdated {
                paper: Box::new(paper),
            },
        );
    }
    Ok(())
}

/// A group's papers, the most recently made first.
pub async fn list(state: &AppState, group_id: i64) -> ApiResult<Vec<Paper>> {
    let rows: Vec<PaperRow> = sqlx::query_as(concat!(
        paper_query!(),
        " WHERE p.group_id = ? ORDER BY p.created_at DESC, p.id DESC"
    ))
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    let mut papers = Vec::with_capacity(rows.len());
    for row in rows {
        papers.push(row.to_api(state).await?);
    }
    Ok(papers)
}

/// Creates a paper: its repository, from the template, and its first
/// build. Admins and members do; guests do not. Its creator is its first
/// author.
pub async fn create(
    state: &AppState,
    group: &Membership,
    creator: &User,
    slug: &str,
    title: &str,
) -> ApiResult<Paper> {
    if !group.role.can_ask_for_work() {
        return Err(ApiError::forbidden("Guests cannot create papers"));
    }
    let slug = slug.trim();
    if !is_slug(slug) || slug.len() > MAX_SLUG {
        return Err(ApiError::invalid(format!(
            "A paper's short name is lowercase letters, digits and inner dashes, at most \
             {MAX_SLUG} characters"
        )));
    }
    // The web app's page for a new paper is `p/new`.
    if slug == "new" {
        return Err(ApiError::invalid(
            "A paper is not called “new”: choose another short name",
        ));
    }
    let title = crate::api::required_text(title, "The title", 200)?;
    let taken: Option<i64> =
        sqlx::query_scalar("SELECT id FROM papers WHERE group_id = ? AND slug = ?")
            .bind(group.group_id)
            .bind(slug)
            .fetch_optional(&state.db)
            .await?;
    if taken.is_some() {
        return Err(ApiError::conflict(format!(
            "There is a paper p/{slug} already"
        )));
    }
    let forge = forge::require(state)?;
    let repository = repository_of(slug);
    forge::create_repository(
        forge,
        state,
        &group.slug,
        &repository,
        &format!("{title}: a working paper of {}", group.name),
        &template(&title, &creator.display_name, &group.name),
    )
    .await
    .map_err(|error| match error.is_conflict() {
        true => ApiError::conflict(format!(
            "The group's organization has a repository {repository} already"
        )),
        false => forge::api_error(error),
    })?;
    let now = db::now();
    let mut tx = state.db.begin().await?;
    let id = sqlx::query(
        "INSERT INTO papers (group_id, slug, title, repository, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(slug)
    .bind(&title)
    .bind(&repository)
    .bind(creator.id)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO paper_authors (paper_id, user_id, added_by, added_at) VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(creator.id)
    .bind(creator.id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    refresh(state, id, creator.id).await?;
    publish(state, id).await?;
    find(state, group.group_id, slug).await?.to_api(state).await
}

/// Whether someone may change a paper's authors: its authors, and the
/// group's admins.
async fn may_change_authors(
    state: &AppState,
    group: &Membership,
    paper: &PaperRow,
    user_id: i64,
) -> ApiResult<()> {
    if group.role.is_admin() || is_author(state, paper.id, user_id).await? {
        return Ok(());
    }
    Err(ApiError::forbidden(
        "Only the paper's authors, or an admin, change its authors",
    ))
}

/// Adds someone of the group, an admin or a member, to a paper's authors.
pub async fn add_author(
    state: &AppState,
    group: &Membership,
    by: i64,
    slug: &str,
    username: &str,
) -> ApiResult<Paper> {
    let paper = find(state, group.group_id, slug).await?;
    may_change_authors(state, group, &paper, by).await?;
    let found: Option<(i64, String)> = sqlx::query_as(
        "SELECT u.id, m.role FROM users u JOIN memberships m ON m.user_id = u.id
         WHERE u.username = ? AND m.group_id = ?",
    )
    .bind(username.trim().trim_start_matches('@'))
    .bind(group.group_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((user_id, role)) = found else {
        return Err(ApiError::not_found("Member"));
    };
    if Role::parse(&role).is_none_or(|role| !role.can_ask_for_work()) {
        return Err(ApiError::invalid("Guests are not authors of papers"));
    }
    sqlx::query(
        "INSERT INTO paper_authors (paper_id, user_id, added_by, added_at) VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(paper.id)
    .bind(user_id)
    .bind(by)
    .bind(db::now())
    .execute(&state.db)
    .await?;
    publish(state, paper.id).await?;
    paper.to_api(state).await
}

/// Takes someone from a paper's authors, which keeps at least one.
pub async fn remove_author(
    state: &AppState,
    group: &Membership,
    by: i64,
    slug: &str,
    username: &str,
) -> ApiResult<Paper> {
    let paper = find(state, group.group_id, slug).await?;
    may_change_authors(state, group, &paper, by).await?;
    let authors = authors(state, paper.id).await?;
    let Some(author) = authors.iter().find(|a| a.username == username) else {
        return Err(ApiError::not_found("Author"));
    };
    if authors.len() == 1 {
        return Err(ApiError::conflict("A paper keeps at least one author"));
    }
    sqlx::query("DELETE FROM paper_authors WHERE paper_id = ? AND user_id = ?")
        .bind(paper.id)
        .bind(author.id)
        .execute(&state.db)
        .await?;
    publish(state, paper.id).await?;
    paper.to_api(state).await
}

/// What a paper's address resolves to: whether it exists, and its title.
pub async fn resolve(
    state: &AppState,
    group_id: i64,
    slug: &str,
) -> ApiResult<(bool, Option<String>)> {
    match find(state, group_id, slug).await {
        Ok(row) => Ok((true, Some(row.title))),
        Err(error) if error.status == StatusCode::NOT_FOUND => Ok((false, None)),
        Err(error) => Err(error),
    }
}

// ── Builds ────────────────────────────────────────────────────────────

/// The commit a paper's `main` is at, if it is anywhere.
pub async fn main_commit(state: &AppState, paper: &PaperRow) -> ApiResult<Option<String>> {
    let Some(forge) = state.forge.as_deref() else {
        return Ok(None);
    };
    forge
        .branch_commit(&paper.group_slug, &paper.repository, MAIN)
        .await
        .map_err(forge::api_error)
}

/// Looks at where a paper's `main` is; if it moved, queues a build of the
/// new commit, for `requester_id`, in place of those still waiting.
pub async fn refresh(state: &AppState, paper_id: i64, requester_id: i64) -> ApiResult<()> {
    let Some(paper) = by_id(state, paper_id).await? else {
        return Ok(());
    };
    let Some(head) = main_commit(state, &paper).await? else {
        return Ok(());
    };
    if paper.head.as_deref() == Some(head.as_str()) {
        return Ok(());
    }
    sqlx::query("UPDATE papers SET head = ? WHERE id = ?")
        .bind(&head)
        .bind(paper_id)
        .execute(&state.db)
        .await?;
    let built: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM paper_builds WHERE paper_id = ? AND commit_sha = ? AND succeeded = 1",
    )
    .bind(paper_id)
    .bind(&head)
    .fetch_optional(&state.db)
    .await?;
    if built.is_none() {
        queue(state, &paper, &head, requester_id).await?;
    }
    Ok(())
}

/// Queues a build of a commit of a paper, and drops the builds of the
/// paper still waiting: only the newest commit matters.
pub async fn queue(
    state: &AppState,
    paper: &PaperRow,
    commit: &str,
    requester_id: i64,
) -> ApiResult<i64> {
    let waiting: Vec<i64> = sqlx::query_scalar(
        "SELECT j.id FROM paper_builds b JOIN jobs j ON j.id = b.job_id
         WHERE b.paper_id = ? AND j.state = 'queued'",
    )
    .bind(paper.id)
    .fetch_all(&state.db)
    .await?;
    for job in waiting {
        let _ = jobs::stop(state, job).await;
    }
    let now = db::now();
    let job_id = sqlx::query(
        "INSERT INTO jobs (group_id, kind, requester_id, runner, state, created_at, updated_at)
         VALUES (?, 'paper_build', ?, ?, 'queued', ?, ?)",
    )
    .bind(paper.group_id)
    .bind(requester_id)
    .bind(LATEX)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    let build_id = sqlx::query(
        "INSERT INTO paper_builds (paper_id, commit_sha, job_id, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(paper.id)
    .bind(commit)
    .bind(job_id)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    jobs::dispatch(state).await?;
    Ok(build_id)
}

/// Builds a paper's `main` again: an admin or a member.
pub async fn rebuild(
    state: &AppState,
    group: &Membership,
    requester_id: i64,
    slug: &str,
) -> ApiResult<Paper> {
    if !group.role.can_ask_for_work() {
        return Err(ApiError::forbidden("Guests cannot ask for a build"));
    }
    let paper = find(state, group.group_id, slug).await?;
    let head = main_commit(state, &paper)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper's repository"))?;
    queue(state, &paper, &head, requester_id).await?;
    publish(state, paper.id).await?;
    paper.to_api(state).await
}

#[derive(sqlx::FromRow)]
struct BuildRow {
    id: i64,
    paper_id: i64,
    commit_sha: String,
    succeeded: Option<bool>,
    error: Option<String>,
    created_at: i64,
    finished_at: Option<i64>,
    job_id: Option<i64>,
    job_state: Option<String>,
    job_error: Option<String>,
    job_progress: Option<String>,
    job_updated_at: Option<i64>,
}

macro_rules! build_query {
    () => {
        "SELECT b.id, b.paper_id, b.commit_sha, b.succeeded, b.error, b.created_at,
                b.finished_at, b.job_id, j.state AS job_state, j.error AS job_error,
                j.progress AS job_progress, j.updated_at AS job_updated_at
         FROM paper_builds b LEFT JOIN jobs j ON j.id = b.job_id"
    };
}

impl BuildRow {
    fn to_api(&self, note: Option<String>) -> PaperBuild {
        let state = match (self.succeeded, self.job_state.as_deref()) {
            (Some(true), _) => BuildState::Succeeded,
            (Some(false), _) | (None, Some("failed")) => BuildState::Failed,
            (None, Some("queued")) => BuildState::Queued,
            (None, Some("running")) => BuildState::Running,
            _ => BuildState::Cancelled,
        };
        let progress = match state {
            BuildState::Running => self.job_progress.clone(),
            BuildState::Queued => note,
            _ => None,
        };
        let finished_at = self.finished_at.or(match state {
            BuildState::Failed | BuildState::Cancelled => self.job_updated_at,
            _ => None,
        });
        PaperBuild {
            id: self.id,
            commit: self.commit_sha.clone(),
            state,
            progress,
            error: self.error.clone().or_else(|| {
                (state == BuildState::Failed)
                    .then(|| self.job_error.clone())
                    .flatten()
            }),
            created_at: db::from_millis(self.created_at),
            finished_at: finished_at.map(db::from_millis),
        }
    }

    async fn to_api_noted(&self, state: &AppState) -> ApiResult<PaperBuild> {
        let note = match (self.job_state.as_deref(), self.job_id) {
            (Some("queued"), Some(job)) => jobs::note_of(state, job).await?,
            _ => None,
        };
        Ok(self.to_api(note))
    }
}

async fn latest_build(state: &AppState, paper_id: i64) -> ApiResult<Option<BuildRow>> {
    Ok(sqlx::query_as(concat!(
        build_query!(),
        " WHERE b.paper_id = ? ORDER BY b.id DESC LIMIT 1"
    ))
    .bind(paper_id)
    .fetch_optional(&state.db)
    .await?)
}

/// The newest build's log.
pub async fn latest_log(state: &AppState, paper_id: i64) -> ApiResult<String> {
    let log: Option<Option<String>> = sqlx::query_scalar(
        "SELECT log FROM paper_builds WHERE paper_id = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(paper_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(log.flatten().unwrap_or_default())
}

/// Tells the group how the paper of a build's job is, if the job builds
/// one; `note` is why the job waits.
pub async fn publish_of_job(state: &AppState, job_id: i64, note: Option<String>) -> ApiResult<()> {
    let row: Option<BuildRow> = sqlx::query_as(concat!(build_query!(), " WHERE b.job_id = ?"))
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let Some(paper) = by_id(state, row.paper_id).await? else {
        return Ok(());
    };
    // The caller holds the workers' lock.
    let mut api = paper.to_api_noted(state, false).await?;
    if api.latest.as_ref().is_some_and(|b| b.id == row.id) {
        api.latest = Some(row.to_api(note));
    }
    state.publish(
        paper.group_id,
        Event::PaperUpdated {
            paper: Box::new(api),
        },
    );
    Ok(())
}

/// What a workshop machine needs to compile a job's commit.
pub async fn work_for(state: &AppState, job_id: i64) -> ApiResult<PaperCompile> {
    let (build, paper_id, commit): (i64, i64, String) =
        sqlx::query_as("SELECT id, paper_id, commit_sha FROM paper_builds WHERE job_id = ?")
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::internal("a paper's build without its build"))?;
    let paper = by_id(state, paper_id)
        .await?
        .ok_or_else(|| ApiError::internal("a build without its paper"))?;
    Ok(PaperCompile {
        build,
        paper: paper.slug.clone(),
        repository: clone_path(&paper.group_slug, &paper.repository),
        commit,
        main: PAPER_MAIN.into(),
        upload: format!(
            "/api/v1/groups/{}/papers/{}/builds/{build}/pdf",
            paper.group_slug, paper.slug
        ),
    })
}

/// Where a build's PDF is kept.
pub fn pdf_path(state: &AppState, build_id: i64) -> PathBuf {
    state
        .config
        .data_dir
        .join("papers")
        .join(format!("{build_id}.pdf"))
}

/// The PDF a paper has, as a file: where it is, and its size.
pub async fn pdf_file(state: &AppState, paper: &PaperRow) -> ApiResult<(PathBuf, u64)> {
    let pdf = paper
        .pdf(state)
        .await?
        .ok_or_else(|| ApiError::not_found("PDF"))?;
    Ok((pdf_path(state, pdf.build), pdf.size))
}

/// Keeps the PDF a machine uploads for a build it is running: only the
/// job's own token uploads it, while the job runs.
pub async fn upload(
    state: &AppState,
    token_id: i64,
    paper: &PaperRow,
    build_id: i64,
    pdf: &[u8],
) -> ApiResult<()> {
    let running: Option<i64> = sqlx::query_scalar(
        "SELECT b.id FROM paper_builds b JOIN jobs j ON j.id = b.job_id
         WHERE b.id = ? AND b.paper_id = ? AND j.token_id = ? AND j.state = 'running'",
    )
    .bind(build_id)
    .bind(paper.id)
    .bind(token_id)
    .fetch_optional(&state.db)
    .await?;
    if running.is_none() {
        return Err(ApiError::forbidden(
            "Only the machine building it uploads a build's PDF",
        ));
    }
    if !pdf.starts_with(b"%PDF-") {
        return Err(ApiError::invalid("This is not a PDF"));
    }
    let path = pdf_path(state, build_id);
    tokio::fs::create_dir_all(path.parent().expect("in a folder")).await?;
    tokio::fs::write(&path, pdf).await?;
    sqlx::query("UPDATE paper_builds SET pdf_size = ? WHERE id = ?")
        .bind(pdf.len() as i64)
        .bind(build_id)
        .execute(&state.db)
        .await?;
    Ok(())
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

/// Keeps what a machine says of a paper's build: its log, and if it made
/// the PDF it uploaded, that PDF becomes the paper's, in place of the one
/// before. A build that failed leaves the paper's PDF as it was.
pub async fn build_finished(state: &AppState, job_id: i64, built: PaperBuilt) -> ApiResult<()> {
    let (build_id, paper_id, size): (i64, i64, Option<i64>) =
        sqlx::query_as("SELECT id, paper_id, pdf_size FROM paper_builds WHERE job_id = ?")
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::internal("a paper's build without its build"))?;
    let succeeded = built.succeeded && built.pdf && size.is_some();
    let error = match (&built.error, succeeded) {
        (Some(error), _) => Some(error.chars().take(2_000).collect::<String>()),
        (None, false) => Some("The build made no PDF".to_owned()),
        (None, true) => None,
    };
    // Publish completion and the PDF pointer together: readers must never see
    // a successful build before its PDF is available.
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE paper_builds SET succeeded = ?, error = ?, log = ?, finished_at = ?,
                                 pdf_size = CASE WHEN ? THEN pdf_size END
         WHERE id = ?",
    )
    .bind(succeeded)
    .bind(&error)
    .bind(tail(&built.log))
    .bind(db::now())
    .bind(succeeded)
    .bind(build_id)
    .execute(&mut *tx)
    .await?;
    let mut gone = Vec::new();
    if succeeded {
        let before: Option<Option<i64>> =
            sqlx::query_scalar("SELECT pdf_build FROM papers WHERE id = ?")
                .bind(paper_id)
                .fetch_optional(&mut *tx)
                .await?;
        match before.flatten() {
            // A newer build made the paper's PDF already.
            Some(newer) if newer > build_id => gone.push(build_id),
            before => {
                sqlx::query("UPDATE papers SET pdf_build = ? WHERE id = ?")
                    .bind(build_id)
                    .bind(paper_id)
                    .execute(&mut *tx)
                    .await?;
                gone.extend(before);
            }
        }
    } else {
        gone.push(build_id);
    }
    for build in &gone {
        sqlx::query("UPDATE paper_builds SET pdf_size = NULL WHERE id = ?")
            .bind(build)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    for build in gone {
        let _ = tokio::fs::remove_file(pdf_path(state, build)).await;
    }
    Ok(())
}

// ── Source ────────────────────────────────────────────────────────────

/// A paper's source files at its `main`, with the commit.
pub async fn source(
    state: &AppState,
    paper: &PaperRow,
) -> ApiResult<(String, Vec<stemma_core::api::PaperFile>)> {
    let forge = forge::require(state)?;
    let commit = main_commit(state, paper)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper's repository"))?;
    let files = forge
        .tree(&paper.group_slug, &paper.repository, &commit)
        .await
        .map_err(forge::api_error)?
        .unwrap_or_default()
        .into_iter()
        .map(|entry| stemma_core::api::PaperFile {
            path: entry.path,
            size: entry.size,
        })
        .collect();
    Ok((commit, files))
}

/// One of a paper's source files at its `main`, as text.
pub async fn source_file(
    state: &AppState,
    paper: &PaperRow,
    path: &str,
) -> ApiResult<(String, String)> {
    let path = path.trim_matches('/');
    if path.is_empty() || path.split('/').any(|part| part.is_empty() || part == "..") {
        return Err(ApiError::invalid(
            "A file is named by its path in the paper",
        ));
    }
    let forge = forge::require(state)?;
    let commit = main_commit(state, paper)
        .await?
        .ok_or_else(|| ApiError::not_found("Paper's repository"))?;
    let bytes = forge
        .raw_file(&paper.group_slug, &paper.repository, &commit, path)
        .await
        .map_err(forge::api_error)?
        .ok_or_else(|| ApiError::not_found("File"))?;
    let text =
        String::from_utf8(bytes).map_err(|_| ApiError::invalid(format!("{path} is not text")))?;
    Ok((commit, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_escaped_for_latex() {
        assert_eq!(
            escape("Sets & maps: 100% of x_1"),
            "Sets \\& maps: 100\\% of x\\_1"
        );
        assert_eq!(escape("a\\b{c}"), "a\\textbackslash{}b\\{c\\}");
    }

    #[test]
    fn the_template_is_filled_in() {
        let files = template("Gluing & charts", "Ada", "Geometry");
        let main = &files.iter().find(|(f, _)| f == PAPER_MAIN).unwrap().1;
        assert!(main.contains("\\title{Gluing \\& charts}"), "{main}");
        assert!(main.contains("\\author{Ada}"));
        assert!(main.contains("a working paper of Geometry"));
        assert!(files.iter().any(|(f, _)| f == ".gitignore"));
        assert!(files.iter().any(|(f, _)| f == "references.bib"));
    }
}
