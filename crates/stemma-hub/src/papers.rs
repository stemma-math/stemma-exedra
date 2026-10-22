//! Working papers, their source and archived PDFs. New builds are paused.

use std::path::PathBuf;

use axum::http::StatusCode;
use stemma_core::address::is_slug;
use stemma_core::api::{BuildState, Event, PAPER_MAIN, Paper, PaperBuild, PaperPdf, Role, User};

use crate::api::Membership;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge::{self, MAIN};
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
    pub created_by: i64,
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
        let latest = latest_build(state, self.id).await?.map(|row| row.to_api());
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
    if !group.role.can_write() {
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
    let title_ids = crate::text::encode(&mut tx, group.group_id, &title).await?;
    let (title, _) = crate::text::render_with(&mut tx, &title_ids).await?;
    let id = sqlx::query(
        "INSERT INTO papers (group_id, slug, title, title_ids, repository, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(slug)
    .bind(&title)
    .bind(title_ids)
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
    if Role::parse(&role).is_none_or(|role| !role.can_write()) {
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

/// Update the recorded main commit without scheduling a build.
pub async fn refresh(state: &AppState, paper_id: i64, _requester_id: i64) -> ApiResult<()> {
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
    Ok(())
}

#[derive(sqlx::FromRow)]
struct BuildRow {
    id: i64,
    commit_sha: String,
    succeeded: Option<bool>,
    error: Option<String>,
    created_at: i64,
    finished_at: Option<i64>,
}

macro_rules! build_query {
    () => {
        "SELECT b.id, b.commit_sha, b.succeeded, b.error, b.created_at,
                b.finished_at
         FROM paper_builds b"
    };
}

impl BuildRow {
    fn to_api(&self) -> PaperBuild {
        PaperBuild {
            id: self.id,
            commit: self.commit_sha.clone(),
            state: match self.succeeded {
                Some(true) => BuildState::Succeeded,
                Some(false) => BuildState::Failed,
                None => BuildState::Cancelled,
            },
            progress: None,
            error: self.error.clone(),
            created_at: db::from_millis(self.created_at),
            finished_at: self.finished_at.map(db::from_millis),
        }
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
