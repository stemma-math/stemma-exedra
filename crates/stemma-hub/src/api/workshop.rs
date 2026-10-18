//! The workshop over the API: its architectures, its machines, what it
//! consumed, and approving or rejecting its changes.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use stemma_core::api::{AddWorkshopMachine, Architecture, ConsumptionDay, WorkshopMachine};

use super::{Json, Membership};
use crate::architectures;
use crate::auth::Viewer;
use crate::consumption;
use crate::error::ApiResult;
use crate::state::AppState;
use crate::workshop::{machines, review};

/// The kinds of work the workshop knows.
pub async fn architectures(_viewer: Viewer) -> Json<Vec<Architecture>> {
    Json(architectures::all())
}

/// The machines of the group's workshop, with their agents.
pub async fn list_machines(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<WorkshopMachine>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    Ok(Json(machines::of_group(&state, group.group_id).await?))
}

/// Makes one of the admin's own machines work for the group's workshop.
pub async fn add_machine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<AddWorkshopMachine>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    machines::add(&state, &group, viewer.id, body.machine).await?;
    Ok(StatusCode::CREATED)
}

/// Takes a machine from the group's workshop: an admin, or its owner.
pub async fn remove_machine(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, id)): Path<(String, i64)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    machines::remove(&state, &group, viewer.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct DaysQuery {
    days: Option<u32>,
}

/// What the group's workshop consumed, day by day, the newest first.
pub async fn consumption(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Query(query): Query<DaysQuery>,
) -> ApiResult<Json<Vec<ConsumptionDay>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let days = query.days.unwrap_or(14).clamp(1, 90);
    Ok(Json(
        consumption::workshop_days(&state, group.group_id, days).await?,
    ))
}

/// Approves a task's changes, as what the viewer may approve them as.
pub async fn approve(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    review::approve(&state, &group, &viewer.user(), number).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Rejects a task's changes: its asker, or an admin.
pub async fn reject(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, number)): Path<(String, u32)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    review::reject(&state, &group, &viewer.user(), number).await?;
    Ok(StatusCode::NO_CONTENT)
}
