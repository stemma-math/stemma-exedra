//! Group proposals and the hub's administration, always acting as a person.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::api::{
    CreateGroup, CreateGroupRequest, Group, GroupRequest, HubGroup, HubPerson, UpdateHubPerson,
};

use super::{Json, UserColumns, groups, required_text};
use crate::auth::{Scope, Viewer};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct RequestRow {
    id: i64,
    slug: String,
    name: String,
    purpose: String,
    status: String,
    created_at: i64,
    #[sqlx(flatten)]
    requester: UserColumns,
}

async fn requests(state: &AppState, user: Option<i64>) -> ApiResult<Json<Vec<GroupRequest>>> {
    let rows: Vec<RequestRow> = sqlx::query_as(
        "SELECT r.id, r.slug, r.name, r.purpose, r.status, r.created_at,
                u.id AS user_id, u.username, u.display_name
         FROM group_requests r JOIN users u ON u.id = r.user_id
         WHERE (? IS NULL AND r.status = 'open') OR r.user_id = ? ORDER BY r.created_at, r.id",
    )
    .bind(user)
    .bind(user)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| GroupRequest {
                id: r.id,
                slug: r.slug,
                name: r.name,
                purpose: r.purpose,
                status: r.status,
                created_at: db::from_millis(r.created_at),
                requester: r.requester.into(),
            })
            .collect(),
    ))
}

pub async fn mine(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<GroupRequest>>> {
    viewer.require_full()?;
    requests(&state, Some(viewer.id)).await
}

pub async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<GroupRequest>>> {
    viewer.require_hub_admin()?;
    requests(&state, None).await
}

pub async fn request(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<CreateGroupRequest>,
) -> ApiResult<(StatusCode, Json<GroupRequest>)> {
    viewer.require_full()?;
    let _guard = state.group_changes.lock().await;
    let slug = groups::available_slug(&state, &body.slug).await?;
    let name = required_text(&body.name, "The name", 80)?;
    let purpose = required_text(&body.purpose, "What the group is for", 2000)?;
    let created_at = db::now();
    let result = sqlx::query("INSERT INTO group_requests (user_id, slug, name, purpose, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(viewer.id).bind(&slug).bind(&name).bind(&purpose).bind(created_at).execute(&state.db).await;
    match result {
        Ok(done) => Ok((
            StatusCode::CREATED,
            Json(GroupRequest {
                id: done.last_insert_rowid(),
                slug,
                name,
                purpose,
                requester: viewer.user(),
                status: "open".into(),
                created_at: db::from_millis(created_at),
            }),
        )),
        Err(e) if db::is_unique_violation(&e) => Err(ApiError::conflict(
            "You already have an open request for that short name.",
        )),
        Err(e) => Err(e.into()),
    }
}

pub async fn approve(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<Json<Group>> {
    viewer.require_hub_admin()?;
    let _guard = state.group_changes.lock().await;
    let row: Option<(String, String, i64, String, String)> = sqlx::query_as(
        "SELECT r.slug, r.name, u.id, u.username, u.display_name FROM group_requests r
         JOIN users u ON u.id = r.user_id WHERE r.id = ? AND r.status = 'open'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    let (slug, name, id_owner, username, display_name) =
        row.ok_or_else(|| ApiError::not_found("Open request"))?;
    let owner = Viewer {
        id: id_owner,
        username,
        display_name,
        hub_admin: false,
        with_token: false,
        personal_token: false,
        scope: Scope::Full,
    };
    Ok(Json(
        groups::create_for(&state, &owner, CreateGroup { slug, name }, Some(id)).await?,
    ))
}

pub async fn decline(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    viewer.require_hub_admin()?;
    let _guard = state.group_changes.lock().await;
    let done = sqlx::query(
        "UPDATE group_requests SET status = 'declined' WHERE id = ? AND status = 'open'",
    )
    .bind(id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Open request"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn all_groups(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<HubGroup>>> {
    viewer.require_hub_admin()?;
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT g.slug, g.name, COUNT(m.user_id), g.created_at FROM groups g
         LEFT JOIN memberships m ON m.group_id = g.id GROUP BY g.id ORDER BY g.name",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(slug, name, members, created_at)| HubGroup {
                slug,
                name,
                members,
                created_at: db::from_millis(created_at),
            })
            .collect(),
    ))
}

pub async fn people(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<HubPerson>>> {
    viewer.require_hub_admin()?;
    let rows: Vec<(i64, String, String, bool)> = sqlx::query_as(
        "SELECT id, username, display_name, hub_admin FROM users ORDER BY display_name, id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, username, display_name, hub_admin)| HubPerson {
                user: stemma_core::api::User {
                    id,
                    username,
                    display_name,
                },
                hub_admin,
            })
            .collect(),
    ))
}

pub async fn update_person(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
    Json(body): Json<UpdateHubPerson>,
) -> ApiResult<StatusCode> {
    viewer.require_hub_admin()?;
    // A single write checks the invariant even when two admins step down together.
    let done = sqlx::query("UPDATE users SET hub_admin = ? WHERE id = ? AND (? OR hub_admin = 0 OR (SELECT COUNT(*) FROM users WHERE hub_admin = 1) > 1)")
        .bind(body.hub_admin).bind(id).bind(body.hub_admin).execute(&state.db).await?;
    if done.rows_affected() == 0 {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
        return Err(if exists {
            ApiError::conflict("The hub needs at least one admin.")
        } else {
            ApiError::not_found("Person")
        });
    }
    Ok(StatusCode::NO_CONTENT)
}
