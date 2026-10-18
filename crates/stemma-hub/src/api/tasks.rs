//! The workshop over the API: its tasks, handing one over and cancelling
//! one.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use stemma_core::api::{CreateTask, Task, TaskPage, TaskState};
use stemma_core::{Address, locate};

use super::forum::find_thread;
use super::{Json, Membership, required_text};
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::{jobs, workshop};

#[derive(Deserialize)]
pub struct TaskQuery {
    state: Option<TaskState>,
    limit: Option<u32>,
}

/// The group's tasks, newest first.
pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Query(query): Query<TaskQuery>,
) -> ApiResult<Json<Vec<Task>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    Ok(Json(
        workshop::list(&state, group.group_id, query.state, limit).await?,
    ))
}

/// A task, with its report and log.
pub async fn get(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<Json<TaskPage>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(workshop::page(&state, group.group_id, number).await?))
}

/// The message an origin names: `t12#48`; for a range, its last message;
/// for a thread, its last message. It must be in the group.
async fn origin_of(state: &AppState, group: &Membership, written: &str) -> ApiResult<(i64, u32)> {
    let located = locate(written).map_err(|e| {
        ApiError::invalid(format!(
            "The origin is the message that asks, as t12#48: {e}"
        ))
    })?;
    if located.group.as_deref().is_some_and(|g| g != group.slug) {
        return Err(ApiError::invalid("The origin is a message of this group"));
    }
    let (thread, message) = match located.address {
        Address::Message { thread, message } => (thread, Some(message)),
        Address::Messages { thread, to, .. } => (thread, Some(to)),
        Address::Thread(thread) => (thread, None),
        _ => {
            return Err(ApiError::invalid(
                "The origin is the message that asks, as t12#48",
            ));
        }
    };
    let row = find_thread(state, group.group_id, thread).await?;
    let last = row.last_message as u32;
    let message = message.unwrap_or(last);
    if message == 0 || message > last {
        return Err(ApiError::invalid(format!(
            "There is no message t{thread}#{message}"
        )));
    }
    Ok((row.id, message))
}

/// Hands a task to the workshop. A person names the message that asks;
/// an agent answering in a thread, with its job's token, hands over what
/// the message it answers asks, unless it names another. Guests cannot,
/// nor can an agent for them.
pub async fn create(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<CreateTask>,
) -> ApiResult<(StatusCode, Json<Task>)> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    if !group.role.can_ask_for_work() {
        return Err(ApiError::forbidden(
            "Guests cannot ask the workshop for work",
        ));
    }
    let (origin, agent) = match viewer.job() {
        Some(job) => {
            let asked: Option<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
                "SELECT kind, thread_id, trigger_message FROM jobs
                 WHERE token_id = ? AND state = 'running'",
            )
            .bind(job.token_id)
            .fetch_optional(&state.db)
            .await?;
            let Some(("reply", Some(thread_id), Some(trigger))) = asked
                .as_ref()
                .map(|(kind, thread, trigger)| (kind.as_str(), *thread, *trigger))
            else {
                return Err(ApiError::forbidden(
                    "Only an agent answering in a thread hands tasks to the workshop",
                ));
            };
            let origin = match body.origin.as_deref().map(str::trim) {
                Some(written) if !written.is_empty() => origin_of(&state, &group, written).await?,
                _ => (thread_id, trigger as u32),
            };
            (origin, Some(job.agent.clone()))
        }
        None => {
            let written = body
                .origin
                .as_deref()
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .ok_or_else(|| {
                    ApiError::invalid("Say which message asks for the task, as t12#48")
                })?;
            let agent = match body.agent.as_deref().map(str::trim) {
                Some(_) if !viewer.with_token => {
                    return Err(ApiError::invalid(
                        "Only an API token hands a task over for an agent",
                    ));
                }
                Some(agent) => Some(required_text(agent, "The agent's name", 60)?),
                None => None,
            };
            (origin_of(&state, &group, written).await?, agent)
        }
    };
    let task = workshop::create(
        &state,
        &group,
        workshop::NewTask {
            title: body.title,
            instructions: body.instructions,
            architecture: body.architecture,
            thread_id: origin.0,
            origin_message: origin.1,
            requester_id: viewer.id,
            agent,
        },
    )
    .await?;
    jobs::dispatch(&state).await?;
    Ok((StatusCode::CREATED, Json(task)))
}

/// Cancels a task that is open or running: its requester, or an admin.
pub async fn cancel(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    workshop::cancel(&state, &group, viewer.id, number).await?;
    Ok(StatusCode::NO_CONTENT)
}
