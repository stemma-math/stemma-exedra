//! Libraries: each group's Lean project, the repository `library` of the
//! group's organization in the forge (see [`crate::forge`]), made from a
//! template when the group is.
//!
//! Each new commit of `main` is built by a group worker with Lean, which
//! runs the inspection and reports on every declaration. The library's
//! state is the report of its last successful build.

use stemma_core::api::{
    Build, BuildState, Declaration, DeclarationState, LibraryReport, SourceRange,
};
use stemma_core::protocol::{Built, LibraryBuild};

use crate::api::Membership;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge::{self, LIBRARY, MAIN};
use crate::jobs;
use crate::state::AppState;

/// The library a group starts with: a Lean project depending on Mathlib.
const TEMPLATE: &[(&str, &str)] = &[
    (
        "lean-toolchain",
        include_str!("../../../lean/template/lean-toolchain"),
    ),
    (
        "lakefile.toml",
        include_str!("../../../lean/template/lakefile.toml"),
    ),
    (
        "lake-manifest.json",
        include_str!("../../../lean/template/lake-manifest.json"),
    ),
    (
        ".gitignore",
        include_str!("../../../lean/template/gitignore"),
    ),
    (
        "README.md",
        include_str!("../../../lean/template/README.md"),
    ),
    (
        "Library/Basic.lean",
        include_str!("../../../lean/template/Library/Basic.lean"),
    ),
];

/// How many successful builds keep their declarations.
const KEEP_REPORTS: i64 = 5;

/// The most of a build's log that is kept, in bytes: its end.
const MAX_LOG: usize = 200_000;

/// The name of a library's root module, from its group's slug:
/// `number-theory` is `NumberTheory`. Names Lean or Mathlib use are
/// avoided.
pub fn root_module(slug: &str) -> String {
    let mut name: String = slug
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert_str(0, "Group");
    }
    const TAKEN: &[&str] = &[
        "Init",
        "Std",
        "Lean",
        "Lake",
        "Mathlib",
        "Batteries",
        "Aesop",
        "Qq",
        "Plausible",
        "ProofWidgets",
        "ImportGraph",
        "LeanSearchClient",
        "Cli",
    ];
    if TAKEN.contains(&name.as_str()) {
        name.push_str("Library");
    }
    name
}

/// The files a group's library starts with: the template, for the group.
pub fn template(slug: &str, name: &str) -> Vec<(String, String)> {
    let root = root_module(slug);
    TEMPLATE
        .iter()
        .map(|(file, text)| {
            let file = file.replace("Library", &root);
            let text = text
                .replace("{{package}}", slug)
                .replace("{{Library}}", &root)
                .replace("{{name}}", name);
            (file, text)
        })
        .collect()
}

/// Where the hub serves a group's library over git, relative to it.
pub fn clone_path(slug: &str) -> String {
    format!("/git/{slug}/{LIBRARY}.git")
}

/// The commit the library's `main` is at, if it is anywhere.
pub async fn main_commit(state: &AppState, group_id: i64) -> ApiResult<Option<String>> {
    let Some(forge) = state.forge.as_deref() else {
        return Ok(None);
    };
    let slug: String = sqlx::query_scalar("SELECT slug FROM groups WHERE id = ?")
        .bind(group_id)
        .fetch_one(&state.db)
        .await?;
    forge
        .branch_commit(&slug, LIBRARY, MAIN)
        .await
        .map_err(forge::api_error)
}

/// Keeps that the group, just made, has a library, as the forge made it,
/// and queues its first build, for `requester_id`.
pub async fn created(state: &AppState, group_id: i64, requester_id: i64) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO libraries (group_id, created_at) VALUES (?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(db::now())
    .execute(&state.db)
    .await?;
    refresh(state, group_id, requester_id).await
}

/// Looks at where `main` is; if it moved, queues a build of the new
/// commit, for `requester_id`, in place of the builds still waiting.
pub async fn refresh(state: &AppState, group_id: i64, requester_id: i64) -> ApiResult<()> {
    let Some(head) = main_commit(state, group_id).await? else {
        return Ok(());
    };
    let known: Option<Option<String>> =
        sqlx::query_scalar("SELECT head FROM libraries WHERE group_id = ?")
            .bind(group_id)
            .fetch_optional(&state.db)
            .await?;
    let Some(known) = known else {
        return Ok(());
    };
    if known.as_deref() == Some(head.as_str()) {
        return Ok(());
    }
    sqlx::query("UPDATE libraries SET head = ? WHERE group_id = ?")
        .bind(&head)
        .bind(group_id)
        .execute(&state.db)
        .await?;
    // A task's run builds what it pushes: its build becomes the library's
    // when the task ends, and none is needed then.
    let built: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM library_builds WHERE group_id = ? AND commit_sha = ? AND succeeded = 1",
    )
    .bind(group_id)
    .bind(&head)
    .fetch_optional(&state.db)
    .await?;
    if built.is_none() {
        queue(state, group_id, &head, requester_id).await?;
    }
    Ok(())
}

/// Drops the builds of the group still waiting for a commit that is not
/// where `main` is now, or that a task's run built already, as after a
/// task's run put its own build of `main` in place; then looks at `main`,
/// for `requester_id`.
pub async fn settle(state: &AppState, group_id: i64, requester_id: i64) -> ApiResult<()> {
    let head: Option<Option<String>> =
        sqlx::query_scalar("SELECT head FROM libraries WHERE group_id = ?")
            .bind(group_id)
            .fetch_optional(&state.db)
            .await?;
    let stale: Vec<i64> = sqlx::query_scalar(
        "SELECT j.id FROM library_builds b JOIN jobs j ON j.id = b.job_id
         WHERE b.group_id = ?1 AND j.state = 'queued'
           AND (b.commit_sha IS NOT ?2 OR EXISTS (
                SELECT 1 FROM library_builds d
                WHERE d.group_id = ?1 AND d.commit_sha = b.commit_sha AND d.succeeded = 1))",
    )
    .bind(group_id)
    .bind(head.flatten())
    .fetch_all(&state.db)
    .await?;
    for job in stale {
        let _ = jobs::stop(state, job).await;
    }
    refresh(state, group_id, requester_id).await
}

/// Queues a build of `commit`, and drops the builds of the group still
/// waiting: only the newest commit matters.
pub async fn queue(
    state: &AppState,
    group_id: i64,
    commit: &str,
    requester_id: i64,
) -> ApiResult<i64> {
    let waiting: Vec<i64> = sqlx::query_scalar(
        "SELECT j.id FROM library_builds b JOIN jobs j ON j.id = b.job_id
         WHERE b.group_id = ? AND j.state = 'queued'",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await?;
    for job in waiting {
        let _ = jobs::stop(state, job).await;
    }
    let job_id = jobs::queue_build(state, group_id, requester_id).await?;
    let build_id = sqlx::query(
        "INSERT INTO library_builds (group_id, commit_sha, job_id, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(commit)
    .bind(job_id)
    .bind(db::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    jobs::dispatch(state).await?;
    Ok(build_id)
}

// ── Builds ────────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct BuildRow {
    id: i64,
    commit_sha: String,
    succeeded: Option<bool>,
    error: Option<String>,
    lean: Option<String>,
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
        "SELECT b.id, b.commit_sha, b.succeeded, b.error, b.lean, b.created_at, b.finished_at,
                b.job_id, j.state AS job_state, j.error AS job_error, j.progress AS job_progress,
                j.updated_at AS job_updated_at
         FROM library_builds b LEFT JOIN jobs j ON j.id = b.job_id"
    };
}

impl BuildRow {
    /// The build as people see it; `note` says why it waits, if it does.
    fn to_api(&self, note: Option<String>) -> Build {
        let job_state = self.job_state.as_deref();
        let state = match (self.succeeded, job_state) {
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
        Build {
            id: self.id,
            commit: self.commit_sha.clone(),
            state,
            progress,
            error: self.error.clone().or_else(|| {
                (state == BuildState::Failed)
                    .then(|| self.job_error.clone())
                    .flatten()
            }),
            lean: self.lean.clone(),
            created_at: db::from_millis(self.created_at),
            finished_at: finished_at.map(db::from_millis),
        }
    }

    async fn to_api_noted(&self, state: &AppState) -> ApiResult<Build> {
        let note = match (self.job_state.as_deref(), self.job_id) {
            (Some("queued"), Some(job)) => jobs::note_of(state, job).await?,
            _ => None,
        };
        Ok(self.to_api(note))
    }
}

/// The build a job does, as it is now.
pub async fn build_of_job(
    state: &AppState,
    job_id: i64,
    note: Option<String>,
) -> ApiResult<Option<Build>> {
    let row: Option<BuildRow> = sqlx::query_as(concat!(build_query!(), " WHERE b.job_id = ?"))
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?;
    Ok(row.map(|row| row.to_api(note)))
}

/// A group's builds, newest first.
pub async fn builds(state: &AppState, group_id: i64, limit: u32) -> ApiResult<Vec<Build>> {
    let rows: Vec<BuildRow> = sqlx::query_as(concat!(
        build_query!(),
        " WHERE b.group_id = ? ORDER BY b.id DESC LIMIT ?"
    ))
    .bind(group_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await?;
    let mut builds = Vec::with_capacity(rows.len());
    for row in rows {
        builds.push(row.to_api_noted(state).await?);
    }
    Ok(builds)
}

/// One of a group's builds, with its log.
pub async fn build(state: &AppState, group_id: i64, id: i64) -> ApiResult<(Build, String)> {
    let row: BuildRow = sqlx::query_as(concat!(
        build_query!(),
        " WHERE b.group_id = ? AND b.id = ?"
    ))
    .bind(group_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Build"))?;
    let log: Option<String> = sqlx::query_scalar("SELECT log FROM library_builds WHERE id = ?")
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok((row.to_api_noted(state).await?, log.unwrap_or_default()))
}

/// The build whose report is the library's state: the last that passed.
pub async fn current(state: &AppState, group_id: i64) -> ApiResult<Option<Build>> {
    let row: Option<BuildRow> = sqlx::query_as(concat!(
        build_query!(),
        " WHERE b.group_id = ? AND b.succeeded = 1 ORDER BY b.id DESC LIMIT 1"
    ))
    .bind(group_id)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(|row| row.to_api(None)))
}

/// What a group worker needs to build a job's commit.
pub async fn work_for(
    state: &AppState,
    group: &Membership,
    job_id: i64,
) -> ApiResult<LibraryBuild> {
    let (build, commit): (i64, String) =
        sqlx::query_as("SELECT id, commit_sha FROM library_builds WHERE job_id = ?")
            .bind(job_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::internal("a build's job without its build"))?;
    Ok(LibraryBuild {
        build,
        repository: clone_path(&group.slug),
        commit,
    })
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

/// Keeps what a worker says of a build: its log, and if it passed, its
/// report, which becomes the library's state.
pub async fn build_finished(
    state: &AppState,
    group_id: i64,
    job_id: i64,
    built: Built,
) -> ApiResult<()> {
    let build_id: i64 =
        sqlx::query_scalar("SELECT id FROM library_builds WHERE job_id = ? AND group_id = ?")
            .bind(job_id)
            .bind(group_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::internal("a build's job without its build"))?;
    keep(state, group_id, build_id, built).await
}

/// Keeps a build that a task's run made of the commit it pushed: when that
/// commit is where `main` is, the build is the library's, as if the hub had
/// asked for it.
pub async fn record_build(
    state: &AppState,
    group_id: i64,
    commit: &str,
    built: Built,
) -> ApiResult<()> {
    if main_commit(state, group_id).await?.as_deref() != Some(commit) {
        return Ok(());
    }
    let now = db::now();
    let build_id = sqlx::query(
        "INSERT INTO library_builds (group_id, commit_sha, created_at) VALUES (?, ?, ?)",
    )
    .bind(group_id)
    .bind(commit)
    .bind(now)
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    sqlx::query("UPDATE libraries SET head = ? WHERE group_id = ?")
        .bind(commit)
        .bind(group_id)
        .execute(&state.db)
        .await?;
    keep(state, group_id, build_id, built).await?;
    if let Some(build) = builds(state, group_id, 1).await?.pop() {
        state.publish(group_id, stemma_core::api::Event::BuildUpdated { build });
    }
    Ok(())
}

/// Stores how a build went: its log, and if it passed, its report.
async fn keep(state: &AppState, group_id: i64, build_id: i64, built: Built) -> ApiResult<()> {
    let succeeded = built.succeeded && built.report.is_some();
    let error = match (&built.error, succeeded) {
        (Some(error), _) => Some(error.chars().take(2_000).collect::<String>()),
        (None, false) => Some("The build gave no report".to_owned()),
        (None, true) => None,
    };
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE library_builds SET succeeded = ?, error = ?, log = ?, lean = ?, finished_at = ?
         WHERE id = ?",
    )
    .bind(succeeded)
    .bind(&error)
    .bind(tail(&built.log))
    .bind(built.report.as_ref().map(|r| r.lean.clone()))
    .bind(db::now())
    .bind(build_id)
    .execute(&mut *tx)
    .await?;
    if let (true, Some(report)) = (succeeded, &built.report) {
        store_report(&mut tx, build_id, report).await?;
        // Older reports than the last few are not needed any more.
        sqlx::query(
            "DELETE FROM library_declarations WHERE build_id IN (
                 SELECT id FROM library_builds WHERE group_id = ?1 AND succeeded = 1
                 ORDER BY id DESC LIMIT -1 OFFSET ?2)",
        )
        .bind(group_id)
        .bind(KEEP_REPORTS)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "DELETE FROM library_uses WHERE build_id IN (
                 SELECT id FROM library_builds WHERE group_id = ?1 AND succeeded = 1
                 ORDER BY id DESC LIMIT -1 OFFSET ?2)",
        )
        .bind(group_id)
        .bind(KEEP_REPORTS)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The declarations a report adds, removes or changes (in kind, type or
/// state) from the library's state, by name.
pub async fn changed_declarations(
    state: &AppState,
    group_id: i64,
    report: &LibraryReport,
) -> ApiResult<Vec<String>> {
    type Seen = (String, String, bool, bool);
    let before: std::collections::HashMap<String, Seen> =
        match current_build_id(state, group_id).await? {
            Some(build) => sqlx::query_as::<_, (String, String, String, bool, bool)>(
                "SELECT name, kind, type, sorry, data_sorry FROM library_declarations
                 WHERE build_id = ?",
            )
            .bind(build)
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .map(|(name, kind, type_, sorry, data)| (name, (kind, type_, sorry, data)))
            .collect(),
            None => Default::default(),
        };
    let mut changed: Vec<String> = report
        .declarations
        .iter()
        .filter(|d| {
            before.get(&d.name) != Some(&(d.kind.clone(), d.type_.clone(), d.sorry, d.data_sorry))
        })
        .map(|d| d.name.clone())
        .collect();
    let now: std::collections::HashSet<&str> = report
        .declarations
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    let mut removed: Vec<String> = before
        .into_keys()
        .filter(|name| !now.contains(name.as_str()))
        .collect();
    removed.sort();
    changed.extend(removed);
    Ok(changed)
}

async fn store_report(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    build_id: i64,
    report: &LibraryReport,
) -> ApiResult<()> {
    for d in &report.declarations {
        let range = d.range;
        sqlx::query(
            "INSERT OR REPLACE INTO library_declarations
                 (build_id, name, module, kind, type, doc, start_line, start_column, end_line,
                  end_column, sorry, data_sorry, axioms, uses)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(build_id)
        .bind(&d.name)
        .bind(&d.module)
        .bind(&d.kind)
        .bind(&d.type_)
        .bind(
            d.doc
                .as_deref()
                .map(str::trim)
                .filter(|doc| !doc.is_empty()),
        )
        .bind(range.map(|r| r.start_line))
        .bind(range.map(|r| r.start_column))
        .bind(range.map(|r| r.end_line))
        .bind(range.map(|r| r.end_column))
        .bind(d.sorry)
        .bind(d.data_sorry)
        .bind(serde_json::to_string(&d.axioms).map_err(ApiError::internal)?)
        .bind(serde_json::to_string(&d.uses).map_err(ApiError::internal)?)
        .execute(&mut **tx)
        .await?;
        for used in &d.uses {
            sqlx::query(
                "INSERT OR IGNORE INTO library_uses (build_id, used, user) VALUES (?, ?, ?)",
            )
            .bind(build_id)
            .bind(used)
            .bind(&d.name)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

// ── Declarations ──────────────────────────────────────────────────────

/// The id of the build whose report is the library's state.
pub async fn current_build_id(state: &AppState, group_id: i64) -> ApiResult<Option<i64>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM library_builds WHERE group_id = ? AND succeeded = 1
         ORDER BY id DESC LIMIT 1",
    )
    .bind(group_id)
    .fetch_optional(&state.db)
    .await?)
}

#[derive(sqlx::FromRow)]
pub struct DeclarationRow {
    pub name: String,
    pub module: String,
    pub kind: String,
    #[sqlx(rename = "type")]
    pub type_: String,
    pub doc: Option<String>,
    start_line: Option<i64>,
    start_column: Option<i64>,
    end_line: Option<i64>,
    end_column: Option<i64>,
    pub sorry: bool,
    data_sorry: bool,
    axioms: String,
    uses: String,
}

impl DeclarationRow {
    pub fn state(&self) -> DeclarationState {
        if self.sorry {
            DeclarationState::Sorry
        } else {
            DeclarationState::Proved
        }
    }

    pub fn declaration(&self) -> Declaration {
        let range = match (
            self.start_line,
            self.start_column,
            self.end_line,
            self.end_column,
        ) {
            (Some(start_line), Some(start_column), Some(end_line), Some(end_column)) => {
                Some(SourceRange {
                    start_line: start_line as u32,
                    start_column: start_column as u32,
                    end_line: end_line as u32,
                    end_column: end_column as u32,
                })
            }
            _ => None,
        };
        Declaration {
            name: self.name.clone(),
            module: self.module.clone(),
            kind: self.kind.clone(),
            type_: self.type_.clone(),
            doc: self.doc.clone(),
            range,
            sorry: self.sorry,
            data_sorry: self.data_sorry,
            axioms: serde_json::from_str(&self.axioms).unwrap_or_default(),
            uses: serde_json::from_str(&self.uses).unwrap_or_default(),
        }
    }
}

pub const DECLARATION_COLUMNS: &str = "name, module, kind, type, doc, start_line, start_column, \
     end_line, end_column, sorry, data_sorry, axioms, uses";

pub async fn find_declaration(
    state: &AppState,
    build_id: i64,
    name: &str,
) -> ApiResult<Option<DeclarationRow>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {DECLARATION_COLUMNS} FROM library_declarations WHERE build_id = ? AND name = ?"
    )))
    .bind(build_id)
    .bind(name)
    .fetch_optional(&state.db)
    .await?)
}

/// What a declaration's address resolves to.
pub struct Found {
    pub exists: bool,
    pub title: Option<String>,
    pub note: Option<String>,
    pub state: Option<DeclarationState>,
}

pub async fn resolve(state: &AppState, group_id: i64, name: &str) -> ApiResult<Found> {
    let Some(build) = current_build_id(state, group_id).await? else {
        return Ok(Found {
            exists: false,
            title: None,
            note: Some("The library has not been built yet".into()),
            state: None,
        });
    };
    Ok(match find_declaration(state, build, name).await? {
        Some(row) => Found {
            exists: true,
            title: Some(format!("{} {}: {}", row.kind, row.name, row.type_)),
            note: None,
            state: Some(row.state()),
        },
        None => Found {
            exists: false,
            title: None,
            note: Some("The library has no such declaration".into()),
            state: Some(DeclarationState::Missing),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::root_module;

    #[test]
    fn root_modules() {
        assert_eq!(root_module("geometry"), "Geometry");
        assert_eq!(root_module("number-theory-2"), "NumberTheory2");
        assert_eq!(root_module("2-adic"), "Group2Adic");
        assert_eq!(root_module("mathlib"), "MathlibLibrary");
    }
}
