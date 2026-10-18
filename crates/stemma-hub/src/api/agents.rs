//! People's own agents, which they keep in their settings. See
//! [`crate::agents`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::api::{PersonalAgent, WORKSHOP, runners};
use stemma_core::mention::is_handle;

use super::{Json, required_text};
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

pub(crate) fn runner_of(text: &str) -> ApiResult<String> {
    let runner = text.trim().to_owned();
    if !runners::ALL.contains(&runner.as_str()) {
        return Err(ApiError::invalid(format!(
            "The runner is one of {}",
            runners::ALL.join(", ")
        )));
    }
    Ok(runner)
}

pub(crate) fn optional(value: Option<String>, what: &str, max: usize) -> ApiResult<Option<String>> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => required_text(text, what, max).map(Some),
    }
}

pub(crate) fn args_of(args: Vec<String>) -> ApiResult<Vec<String>> {
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

/// Refuses the workshop's handle: it names every group's workshop.
fn check_not_the_workshop(handle: &str) -> ApiResult<()> {
    if handle == WORKSHOP {
        return Err(ApiError::conflict(format!(
            "@{WORKSHOP} is every group's workshop: choose another handle"
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
    check_not_the_workshop(&agent.handle)?;
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
