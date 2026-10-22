//! Groups and their members.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use stemma_core::api::{CreateGroup, Group, Member, Role, UpdateMember};

use super::{Json, Membership, UserColumns, required_text};
use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge;
use crate::library;
use crate::state::AppState;

pub async fn groups_of(state: &AppState, user_id: i64) -> ApiResult<Vec<Group>> {
    let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT g.id, g.slug, g.name, m.role, g.created_at
         FROM groups g JOIN memberships m ON m.group_id = g.id
         WHERE m.user_id = ? ORDER BY g.name",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, slug, name, role, created_at)| Group {
            id,
            slug,
            name,
            role: Role::parse(&role).unwrap_or(Role::Guest),
            created_at: db::from_millis(created_at),
        })
        .collect())
}

/// The viewer's groups. A job's token sees only its job's group.
pub async fn list(State(state): State<AppState>, viewer: Viewer) -> ApiResult<Json<Vec<Group>>> {
    let mut groups = groups_of(&state, viewer.id).await?;
    if let Some(job) = viewer.agent() {
        let slug: String = sqlx::query_scalar("SELECT slug FROM groups WHERE id = ?")
            .bind(job.group_id)
            .fetch_one(&state.db)
            .await?;
        groups.retain(|g| g.slug == slug);
    }
    Ok(Json(groups))
}

pub async fn get(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Group>> {
    Ok(Json(Membership::of(&state, &viewer, &slug).await?.group()))
}

/// Creates a group, with its creator as its admin: its organization in
/// the forge, with its teams and its library, then the group. Only for hub
/// admins, and only with a forge.
pub async fn create(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<CreateGroup>,
) -> ApiResult<(StatusCode, Json<Group>)> {
    viewer.require_hub_admin()?;
    let _guard = state.group_changes.lock().await;
    let group = create_for(&state, &viewer, body, None).await?;
    Ok((StatusCode::CREATED, Json(group)))
}

pub(super) async fn available_slug(state: &AppState, value: &str) -> ApiResult<String> {
    crate::names::available(state, "group", value, None).await
}

/// The caller serializes group changes; approval and membership are kept together.
pub(super) async fn create_for(
    state: &AppState,
    owner: &Viewer,
    body: CreateGroup,
    request: Option<i64>,
) -> ApiResult<Group> {
    let forge = forge::require(state)?;
    let slug = available_slug(state, &body.slug).await?;
    let name = required_text(&body.name, "The name", 80)?;
    forge::create_group(forge, state, &slug, &name, &library::template(&slug, &name)).await?;
    match insert_group(state, owner, &slug, &name, request).await {
        Ok(group) => {
            library::created(state, group, owner.id).await?;
            forge::sync_members_or_log(state, group).await;
        }
        Err(error) => {
            // The organization is the group's alone: without the group, it
            // goes too.
            let _ = forge.delete_repository(&slug, forge::LIBRARY).await;
            let _ = forge.delete_organization(&slug).await;
            return Err(error);
        }
    }
    Ok(Membership::of(state, owner, &slug).await?.group())
}

/// Keeps a new group, with `viewer` as its admin.
async fn insert_group(
    state: &AppState,
    viewer: &Viewer,
    slug: &str,
    name: &str,
    request: Option<i64>,
) -> ApiResult<i64> {
    let now = db::now();
    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query("INSERT INTO groups (slug, name, created_at) VALUES (?, ?, ?)")
        .bind(slug)
        .bind(name)
        .bind(now)
        .execute(&mut *tx)
        .await;
    let group_id = match inserted {
        Ok(done) => done.last_insert_rowid(),
        Err(e) if db::is_unique_violation(&e) => {
            return Err(ApiError::conflict(format!(
                "There is already a group {slug}"
            )));
        }
        Err(e) => return Err(e.into()),
    };
    sqlx::query(
        "INSERT INTO memberships (group_id, user_id, role, joined_at) VALUES (?, ?, 'admin', ?)",
    )
    .bind(group_id)
    .bind(viewer.id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if let Some(id) = request {
        sqlx::query("UPDATE group_requests SET status = 'approved' WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(group_id)
}

#[derive(sqlx::FromRow)]
struct MemberRow {
    #[sqlx(flatten)]
    user: UserColumns,
    role: String,
    joined_at: i64,
}

pub async fn members(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Member>>> {
    let group = Membership::of(&state, &viewer, &slug).await?;
    let rows: Vec<MemberRow> = sqlx::query_as(
        "SELECT u.id AS user_id, u.username, u.display_name, m.role, m.joined_at
         FROM memberships m JOIN users u ON u.id = m.user_id
         WHERE m.group_id = ? ORDER BY u.display_name",
    )
    .bind(group.group_id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| Member {
                user: row.user.into(),
                role: Role::parse(&row.role).unwrap_or(Role::Guest),
                joined_at: db::from_millis(row.joined_at),
            })
            .collect(),
    ))
}

async fn admin_count(state: &AppState, group_id: i64) -> ApiResult<i64> {
    Ok(
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM memberships WHERE group_id = ? AND role = 'admin'",
        )
        .bind(group_id)
        .fetch_one(&state.db)
        .await?,
    )
}

async fn role_of(state: &AppState, group_id: i64, user_id: i64) -> ApiResult<Role> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE group_id = ? AND user_id = ?")
            .bind(group_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    role.and_then(|r| Role::parse(&r))
        .ok_or_else(|| ApiError::not_found("Member"))
}

/// Changes a member's role: only an admin, and never leaving the group
/// without one.
pub async fn update_member(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, user_id)): Path<(String, i64)>,
    Json(body): Json<UpdateMember>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let current = role_of(&state, group.group_id, user_id).await?;
    if current == Role::Admin
        && body.role != Role::Admin
        && admin_count(&state, group.group_id).await? == 1
    {
        return Err(ApiError::conflict("A group needs at least one admin"));
    }
    sqlx::query("UPDATE memberships SET role = ? WHERE group_id = ? AND user_id = ?")
        .bind(body.role.as_str())
        .bind(group.group_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    forge::sync_members_or_log(&state, group.group_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a member: an admin removes anyone, a member only leaves.
pub async fn remove_member(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, user_id)): Path<(String, i64)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    if user_id != viewer.id {
        group.require_admin()?;
    }
    let role = role_of(&state, group.group_id, user_id).await?;
    if role == Role::Admin && admin_count(&state, group.group_id).await? == 1 {
        return Err(ApiError::conflict("A group needs at least one admin"));
    }
    sqlx::query("DELETE FROM memberships WHERE group_id = ? AND user_id = ?")
        .bind(group.group_id)
        .bind(user_id)
        .execute(&state.db)
        .await?;
    forge::sync_members_or_log(&state, group.group_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// A group's admins may change either name, preserving its identity.
pub async fn update(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<Group>> {
    viewer.require_full()?;
    let _guard = state.group_changes.lock().await;
    let _forge_guard = state.forge_sync.lock().await;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let next = match body.get("slug") {
        Some(value) => {
            crate::names::available(
                &state,
                "group",
                value.as_str().unwrap_or(""),
                Some(group.group_id),
            )
            .await?
        }
        None => group.slug.clone(),
    };
    let name = required_text(
        body.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&group.name),
        "The name",
        80,
    )?;
    if let Some(forge) = state.forge.as_ref() {
        forge
            .edit_organization(&group.slug, &name)
            .await
            .map_err(forge::api_error)?;
        if next != group.slug {
            forge
                .rename_organization(&group.slug, &next)
                .await
                .map_err(forge::api_error)?;
        }
    }
    let mut workers = state.workers.lock().await;
    let mut tx = state.db.begin().await?;
    if next != group.slug {
        sqlx::query("INSERT OR IGNORE INTO group_names (name, group_id) VALUES (?, ?)")
            .bind(&group.slug)
            .bind(group.group_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE groups SET slug = ?, name = ? WHERE id = ?")
        .bind(&next)
        .bind(&name)
        .bind(group.group_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    workers.rename_group(&group.slug, &next);
    drop(workers);
    crate::text::names_changed(&state).await?;
    Ok(Json(Membership::of(&state, &viewer, &next).await?.group()))
}
