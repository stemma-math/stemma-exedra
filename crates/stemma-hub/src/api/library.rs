//! The library over the API: how it stands, its builds, and its
//! declarations, from the report of its last successful build.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use sqlx::AssertSqlSafe;
use stemma_core::Address;
use stemma_core::api::{
    Build, BuildLog, DeclarationPage, DeclarationState, DeclarationSummary, ForgeLinks,
    LibraryState, ModuleSummary, ReferenceSite,
};

use super::{Json, Membership};
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::forge::LIBRARY;
use crate::library::{self, DECLARATION_COLUMNS, DeclarationRow};
use crate::state::AppState;

/// How the library stands.
pub async fn state(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(group): Path<String>,
) -> ApiResult<Json<LibraryState>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let head: Option<Option<String>> =
        sqlx::query_scalar("SELECT head FROM libraries WHERE group_id = ?")
            .bind(group.group_id)
            .fetch_optional(&state.db)
            .await?;
    let available = state.forge.is_some() && head.is_some();
    let latest = library::builds(&state, group.group_id, 1).await?.pop();
    let current = library::current(&state, group.group_id).await?;
    let mut modules = Vec::new();
    if let Some(build) = &current {
        let rows: Vec<(String, i64, i64)> = sqlx::query_as(
            "SELECT module, COUNT(*), SUM(sorry) FROM library_declarations
             WHERE build_id = ? GROUP BY module ORDER BY module",
        )
        .bind(build.id)
        .fetch_all(&state.db)
        .await?;
        modules = rows
            .into_iter()
            .map(|(name, declarations, sorry)| ModuleSummary {
                name,
                declarations: declarations as u32,
                sorry: sorry as u32,
            })
            .collect();
    }
    Ok(Json(LibraryState {
        available,
        clone_path: library::clone_path(&group.slug),
        forge: state
            .forge
            .as_deref()
            .filter(|_| available)
            .map(|forge| ForgeLinks {
                organization: forge.public_url(&group.slug),
                repository: forge.public_url(&format!("{}/{LIBRARY}", group.slug)),
            }),
        head: head.flatten(),
        latest,
        declarations: modules.iter().map(|m| m.declarations).sum(),
        sorry: modules.iter().map(|m| m.sorry).sum(),
        modules,
        current,
    }))
}

/// The library's builds, newest first.
pub async fn builds(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(group): Path<String>,
) -> ApiResult<Json<Vec<Build>>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    Ok(Json(library::builds(&state, group.group_id, 30).await?))
}

/// A build, with its log.
pub async fn build(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, id)): Path<(String, i64)>,
) -> ApiResult<Json<BuildLog>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let (build, log) = library::build(&state, group.group_id, id).await?;
    Ok(Json(BuildLog { build, log }))
}

/// Builds `main` again, as when a workshop machine with Lean was missing.
pub async fn rebuild(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(group): Path<String>,
) -> ApiResult<(StatusCode, Json<Build>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &group).await?;
    if !group.role.can_ask_for_work() {
        return Err(ApiError::forbidden("Guests cannot ask for a build"));
    }
    let head = library::main_commit(&state, group.group_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Library"))?;
    let id = library::queue(&state, group.group_id, &head, viewer.id).await?;
    let (build, _) = library::build(&state, group.group_id, id).await?;
    Ok((StatusCode::CREATED, Json(build)))
}

#[derive(Deserialize)]
pub struct DeclarationQuery {
    /// Words their name, type or docstring all contain.
    q: Option<String>,
    module: Option<String>,
    state: Option<DeclarationState>,
    limit: Option<u32>,
}

/// The declarations of the library's state, by module and name.
pub async fn declarations(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(group): Path<String>,
    Query(query): Query<DeclarationQuery>,
) -> ApiResult<Json<Vec<DeclarationSummary>>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let Some(build) = library::current_build_id(&state, group.group_id).await? else {
        return Ok(Json(Vec::new()));
    };
    let words: Vec<String> = query
        .q
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .take(8)
        .map(str::to_lowercase)
        .collect();
    // Only placeholders are added to the query: the words are bound.
    let mut sql = format!(
        "SELECT {DECLARATION_COLUMNS} FROM library_declarations
         WHERE build_id = ?1 AND (?2 IS NULL OR module = ?2) AND (?3 IS NULL OR sorry = ?3)"
    );
    for (i, _) in words.iter().enumerate() {
        let n = i + 5;
        sql.push_str(&format!(
            " AND (instr(lower(name), ?{n}) > 0 OR instr(lower(type), ?{n}) > 0
                   OR instr(lower(coalesce(doc, '')), ?{n}) > 0)"
        ));
    }
    sql.push_str(" ORDER BY module, start_line, name LIMIT ?4");
    let sorry = match query.state {
        Some(DeclarationState::Sorry) => Some(true),
        Some(DeclarationState::Proved) => Some(false),
        Some(DeclarationState::Missing) => return Ok(Json(Vec::new())),
        None => None,
    };
    let mut rows = sqlx::query_as::<_, DeclarationRow>(AssertSqlSafe(sql))
        .bind(build)
        .bind(query.module.as_deref())
        .bind(sorry)
        .bind(query.limit.unwrap_or(500).clamp(1, 2_000));
    for word in &words {
        rows = rows.bind(word);
    }
    let rows = rows.fetch_all(&state.db).await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| DeclarationSummary {
                state: row.state(),
                name: row.name,
                module: row.module,
                kind: row.kind,
                type_: row.type_,
                doc: row.doc,
            })
            .collect(),
    ))
}

/// A declaration, with what uses it, where it is referenced and the threads
/// about it.
pub async fn declaration(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((group, name)): Path<(String, String)>,
) -> ApiResult<Json<DeclarationPage>> {
    let group = Membership::of(&state, &viewer, &group).await?;
    let build = library::current_build_id(&state, group.group_id)
        .await?
        .ok_or_else(|| ApiError::not_found("Declaration"))?;
    let row = library::find_declaration(&state, build, &name)
        .await?
        .ok_or_else(|| ApiError::not_found("Declaration"))?;
    let used_by: Vec<String> = sqlx::query_scalar(
        "SELECT user FROM library_uses WHERE build_id = ? AND used = ? ORDER BY user",
    )
    .bind(build)
    .bind(&name)
    .fetch_all(&state.db)
    .await?;
    let commit: String = sqlx::query_scalar("SELECT commit_sha FROM library_builds WHERE id = ?")
        .bind(build)
        .fetch_one(&state.db)
        .await?;
    let reference = format!("[[lean:{name}]]");
    let messages: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT t.number, m.number, t.title FROM messages m JOIN threads t ON t.id = m.thread_id
         WHERE t.group_id = ? AND m.deleted_at IS NULL AND instr(m.body, ?) > 0
         ORDER BY m.created_at DESC LIMIT 50",
    )
    .bind(group.group_id)
    .bind(&reference)
    .fetch_all(&state.db)
    .await?;
    // The threads about it, as "Discuss it" starts them.
    let threads: Vec<(i64, String)> = sqlx::query_as(
        "SELECT number, title FROM threads WHERE group_id = ? AND anchor = ?
         ORDER BY last_activity_at DESC LIMIT 50",
    )
    .bind(group.group_id)
    .bind(format!("lean:{name}"))
    .fetch_all(&state.db)
    .await?;
    let mut references: Vec<ReferenceSite> = Vec::new();
    for (number, title) in threads {
        references.push(ReferenceSite {
            address: Address::Thread(number as u32),
            title,
        });
    }
    references.extend(
        messages
            .into_iter()
            .map(|(thread, message, title)| ReferenceSite {
                address: Address::Message {
                    thread: thread as u32,
                    message: message as u32,
                },
                title,
            }),
    );
    Ok(Json(DeclarationPage {
        declaration: row.declaration(),
        state: row.state(),
        commit,
        used_by,
        references,
    }))
}
