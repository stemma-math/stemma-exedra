//! The group library and archived build results. Pushes update main; new builds are paused.

use stemma_core::api::{Build, BuildState, Declaration, DeclarationState, SourceRange};

use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge::{self, LIBRARY, MAIN};
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
pub async fn created(state: &AppState, group_id: i64, _requester_id: i64) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO libraries (group_id, created_at) VALUES (?, ?) ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(db::now())
    .execute(&state.db)
    .await?;
    refresh(state, group_id, _requester_id).await
}

/// Update the recorded main commit without scheduling a build.
pub async fn refresh(state: &AppState, group_id: i64, _requester_id: i64) -> ApiResult<()> {
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
    // A commit built already, as when `main` comes back to it, is not
    // built again.
    Ok(())
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
}

macro_rules! build_query {
    () => {
        "SELECT b.id, b.commit_sha, b.succeeded, b.error, b.lean, b.created_at, b.finished_at
         FROM library_builds b"
    };
}

impl BuildRow {
    fn to_api(&self) -> Build {
        Build {
            id: self.id,
            commit: self.commit_sha.clone(),
            state: match self.succeeded {
                Some(true) => BuildState::Succeeded,
                Some(false) => BuildState::Failed,
                None => BuildState::Cancelled,
            },
            progress: None,
            error: self.error.clone(),
            lean: self.lean.clone(),
            created_at: db::from_millis(self.created_at),
            finished_at: self.finished_at.map(db::from_millis),
        }
    }
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
        builds.push(row.to_api());
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
    Ok((row.to_api(), log.unwrap_or_default()))
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
    Ok(row.map(|row| row.to_api()))
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
