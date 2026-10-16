//! The jobs of a thread, as people follow them, and cancelling one.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::api::Job;

use super::forum::find_thread;
use super::{Json, Membership};
use crate::auth::Viewer;
use crate::error::ApiResult;
use crate::jobs;
use crate::state::AppState;

/// The thread's jobs on their way, and those that failed lately.
pub async fn of_thread(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<Json<Vec<Job>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let thread = find_thread(&state, group.group_id, number).await?;
    Ok(Json(jobs::of_thread(&state, thread.id).await?))
}

/// Cancels a job on its way, or dismisses one that failed.
pub async fn cancel(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, id)): Path<(String, i64)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    jobs::cancel(&state, &group, viewer.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
