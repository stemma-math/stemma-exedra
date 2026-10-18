//! Agents: each person's own, which they keep in their settings, and each
//! group's own, which its admins keep. See [`crate::agents`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::api::{AgentProfile, PersonalAgent, runners};
use stemma_core::mention::is_handle;

use super::{Json, Membership, required_text};
use crate::agents;
use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

/// The most arguments an agent may have, and how long each may be.
const MAX_ARGS: usize = 32;
const MAX_ARG_CHARS: usize = 1_000;

fn handle_of(text: &str) -> ApiResult<String> {
    let handle = text.trim().trim_start_matches('@').to_lowercase();
    if !is_handle(&handle) {
        return Err(ApiError::invalid(
            "A handle is a lowercase letter, then up to 31 lowercase letters, digits, '-' or '_'",
        ));
    }
    Ok(handle)
}

fn runner_of(text: &str) -> ApiResult<String> {
    let runner = text.trim().to_owned();
    if !runners::ALL.contains(&runner.as_str()) {
        return Err(ApiError::invalid(format!(
            "The runner is one of {}",
            runners::ALL.join(", ")
        )));
    }
    Ok(runner)
}

fn optional(value: Option<String>, what: &str, max: usize) -> ApiResult<Option<String>> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => required_text(text, what, max).map(Some),
    }
}

fn args_of(args: Vec<String>) -> ApiResult<Vec<String>> {
    if args.len() > MAX_ARGS {
        return Err(ApiError::invalid(format!(
            "An agent has at most {MAX_ARGS} arguments"
        )));
    }
    if args.iter().any(|a| a.chars().count() > MAX_ARG_CHARS) {
        return Err(ApiError::invalid(format!(
            "An argument is at most {MAX_ARG_CHARS} characters"
        )));
    }
    Ok(args.into_iter().filter(|a| !a.is_empty()).collect())
}

// ── A group's own ─────────────────────────────────────────────────────

pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<AgentProfile>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(agents::of_group(&state, group.group_id).await?))
}

/// Checks a group's agent and tidies it: trimmed, empty optional text as
/// none.
fn checked(profile: AgentProfile) -> ApiResult<AgentProfile> {
    Ok(AgentProfile {
        handle: handle_of(&profile.handle)?,
        display_name: required_text(&profile.display_name, "The name", 60)?,
        runner: runner_of(&profile.runner)?,
        model: optional(profile.model, "The model", 200)?,
        args: args_of(profile.args)?,
        instructions: optional(profile.instructions, "The instructions", 8_000)?,
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
    // A handle someone in the group has for an agent of their own would
    // take it from them.
    if agents::someone_in_group_has(&state, group.group_id, &profile.handle).await? {
        return Err(ApiError::conflict(format!(
            "@{} is someone's own agent here: choose another handle",
            profile.handle
        )));
    }
    agents::insert_group_agent(&state.db, group.group_id, &profile)
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

/// Changes a group's agent; its handle stays. Jobs already asked for keep
/// the agent as it was.
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
        "UPDATE group_agents SET display_name = ?, runner = ?, model = ?, args = ?,
                                 instructions = ?, role = ?, updated_at = ?
         WHERE group_id = ? AND handle = ?",
    )
    .bind(&profile.display_name)
    .bind(&profile.runner)
    .bind(&profile.model)
    .bind(serde_json::to_string(&profile.args).map_err(ApiError::internal)?)
    .bind(&profile.instructions)
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
    let done = sqlx::query("DELETE FROM group_agents WHERE group_id = ? AND handle = ?")
        .bind(group.group_id)
        .bind(&handle)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Agent"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── A person's own ────────────────────────────────────────────────────

/// The viewer's own agents. A job's token sees its requester's too, as
/// the launcher's does.
pub async fn list_mine(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<PersonalAgent>>> {
    Ok(Json(agents::of_person(&state, viewer.id).await?))
}

/// Checks one of a person's agents and tidies it.
fn checked_personal(agent: PersonalAgent) -> ApiResult<PersonalAgent> {
    let handle = handle_of(&agent.handle)?;
    let default = PersonalAgent::default_for(&handle);
    let runner = runner_of(&agent.runner)?;
    if let Some(default) = &default
        && default.runner != runner
    {
        return Err(ApiError::invalid(format!(
            "@{handle} runs {}: add another agent for another runner",
            runners::name(&default.runner)
        )));
    }
    Ok(PersonalAgent {
        handle,
        display_name: required_text(&agent.display_name, "The name", 60)?,
        runner,
        model: optional(agent.model, "The model", 200)?,
        args: args_of(agent.args)?,
        default: default.is_some(),
    })
}

/// Refuses a handle that one of the viewer's groups has for an agent of
/// its own: it would always name the group's.
async fn check_not_a_groups(state: &AppState, viewer: &Viewer, handle: &str) -> ApiResult<()> {
    let groups = agents::groups_with_agent(state, viewer.id, handle).await?;
    if let Some(group) = groups.first() {
        return Err(ApiError::conflict(format!(
            "@{handle} is the agent of the group {group}: choose another handle"
        )));
    }
    Ok(())
}

async fn save_personal(state: &AppState, user_id: i64, agent: &PersonalAgent) -> ApiResult<()> {
    let now = db::now();
    sqlx::query(
        "INSERT INTO personal_agents (user_id, handle, display_name, runner, model, args,
                                      created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (user_id, handle) DO UPDATE SET
             display_name = excluded.display_name, runner = excluded.runner,
             model = excluded.model, args = excluded.args, updated_at = excluded.updated_at",
    )
    .bind(user_id)
    .bind(&agent.handle)
    .bind(&agent.display_name)
    .bind(&agent.runner)
    .bind(&agent.model)
    .bind(serde_json::to_string(&agent.args).map_err(ApiError::internal)?)
    .bind(now)
    .bind(now)
    .execute(&state.db)
    .await?;
    Ok(())
}

pub async fn create_mine(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<PersonalAgent>,
) -> ApiResult<(StatusCode, Json<PersonalAgent>)> {
    viewer.require_full()?;
    let agent = checked_personal(body)?;
    if agent.default
        || agents::personal_agent(&state, viewer.id, &agent.handle)
            .await?
            .is_some()
    {
        return Err(ApiError::conflict(format!(
            "You already have an agent @{}",
            agent.handle
        )));
    }
    check_not_a_groups(&state, &viewer, &agent.handle).await?;
    save_personal(&state, viewer.id, &agent).await?;
    Ok((StatusCode::CREATED, Json(agent)))
}

/// Changes one of the viewer's agents; its handle stays. One everyone has
/// keeps its runner.
pub async fn update_mine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(handle): Path<String>,
    Json(body): Json<PersonalAgent>,
) -> ApiResult<Json<PersonalAgent>> {
    viewer.require_full()?;
    if agents::personal_agent(&state, viewer.id, &handle)
        .await?
        .is_none()
    {
        return Err(ApiError::not_found("Agent"));
    }
    let agent = checked_personal(PersonalAgent { handle, ..body })?;
    save_personal(&state, viewer.id, &agent).await?;
    Ok(Json(agent))
}

/// Removes one of the viewer's agents; one everyone has is put back as it
/// was.
pub async fn remove_mine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(handle): Path<String>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let done = sqlx::query("DELETE FROM personal_agents WHERE user_id = ? AND handle = ?")
        .bind(viewer.id)
        .bind(&handle)
        .execute(&state.db)
        .await?;
    if done.rows_affected() == 0 && PersonalAgent::default_for(&handle).is_none() {
        return Err(ApiError::not_found("Agent"));
    }
    Ok(StatusCode::NO_CONTENT)
}
