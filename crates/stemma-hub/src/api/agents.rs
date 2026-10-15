//! A group's agent profiles: the agents its people summon by handle.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::api::{AgentProfile, runners};
use stemma_core::mention::is_handle;

use super::{Json, Membership, required_text};
use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::jobs;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<AgentProfile>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(jobs::profiles(&state, group.group_id).await?))
}

/// Checks a profile and tidies it: trimmed, empty optional text as none.
fn checked(profile: AgentProfile) -> ApiResult<AgentProfile> {
    let handle = profile.handle.trim().trim_start_matches('@').to_lowercase();
    if !is_handle(&handle) {
        return Err(ApiError::invalid(
            "A handle is a lowercase letter, then up to 31 lowercase letters, digits, '-' or '_'",
        ));
    }
    let runner = profile.runner.trim().to_owned();
    if !runners::ALL.contains(&runner.as_str()) {
        return Err(ApiError::invalid(format!(
            "The runner is one of {}",
            runners::ALL.join(", ")
        )));
    }
    let optional = |value: Option<String>, what: &str, max: usize| -> ApiResult<Option<String>> {
        match value.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(text) => required_text(text, what, max).map(Some),
        }
    };
    Ok(AgentProfile {
        handle,
        display_name: required_text(&profile.display_name, "The name", 60)?,
        runner,
        model: optional(profile.model, "The model", 200)?,
        endpoint: optional(profile.endpoint, "The endpoint", 60)?,
        instructions: optional(profile.instructions, "The instructions", 8_000)?,
        scope: profile.scope,
        role: profile.role,
    })
}

pub async fn create(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<AgentProfile>,
) -> ApiResult<(StatusCode, Json<AgentProfile>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let profile = checked(body)?;
    insert(&state.db, group.group_id, &profile)
        .await
        .map_err(|e| {
            if db::is_unique_violation(&e) {
                ApiError::conflict(format!("There is already an agent @{}", profile.handle))
            } else {
                e.into()
            }
        })?;
    Ok((StatusCode::CREATED, Json(profile)))
}

/// Adds a profile to a group.
pub async fn insert<'e>(
    db: impl sqlx::SqliteExecutor<'e>,
    group_id: i64,
    profile: &AgentProfile,
) -> Result<(), sqlx::Error> {
    let now = db::now();
    sqlx::query(
        "INSERT INTO agent_profiles (group_id, handle, display_name, runner, model, endpoint,
                                     instructions, scope, role, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(&profile.handle)
    .bind(&profile.display_name)
    .bind(&profile.runner)
    .bind(&profile.model)
    .bind(&profile.endpoint)
    .bind(&profile.instructions)
    .bind(profile.scope.as_str())
    .bind(profile.role.as_str())
    .bind(now)
    .bind(now)
    .execute(db)
    .await?;
    Ok(())
}

/// Changes a profile; its handle stays. Jobs already asked for keep the
/// profile as it was.
pub async fn update(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, handle)): Path<(String, String)>,
    Json(body): Json<AgentProfile>,
) -> ApiResult<Json<AgentProfile>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let profile = checked(AgentProfile {
        handle: handle.clone(),
        ..body
    })?;
    let done = sqlx::query(
        "UPDATE agent_profiles SET display_name = ?, runner = ?, model = ?, endpoint = ?,
                                   instructions = ?, scope = ?, role = ?, updated_at = ?
         WHERE group_id = ? AND handle = ?",
    )
    .bind(&profile.display_name)
    .bind(&profile.runner)
    .bind(&profile.model)
    .bind(&profile.endpoint)
    .bind(&profile.instructions)
    .bind(profile.scope.as_str())
    .bind(profile.role.as_str())
    .bind(db::now())
    .bind(group.group_id)
    .bind(&profile.handle)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Agent"));
    }
    Ok(Json(profile))
}

pub async fn remove(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, handle)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let done = sqlx::query("DELETE FROM agent_profiles WHERE group_id = ? AND handle = ?")
        .bind(group.group_id)
        .bind(&handle)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Agent"));
    }
    Ok(StatusCode::NO_CONTENT)
}
