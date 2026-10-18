//! Groups, their members and the invitations that bring people in.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use stemma_core::address::is_slug;
use stemma_core::api::{
    AcceptInvitation, CreateGroup, CreateInvitation, CreatedInvitation, Group, Invitation,
    InvitationInfo, Member, Role, UpdateMember,
};

use super::{Json, Membership, UserColumns, required_text};
use crate::auth::{self, Viewer};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge;
use crate::library;
use crate::secret;
use crate::state::AppState;

pub async fn groups_of(state: &AppState, user_id: i64) -> ApiResult<Vec<Group>> {
    let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT g.slug, g.name, m.role, g.created_at
         FROM groups g JOIN memberships m ON m.group_id = g.id
         WHERE m.user_id = ? ORDER BY g.name",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(slug, name, role, created_at)| Group {
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
    if let Some(job) = viewer.job() {
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
    viewer.require_full()?;
    if !viewer.hub_admin {
        return Err(ApiError::forbidden(
            "Only the hub's admins can create groups",
        ));
    }
    let forge = forge::require(&state)?;
    let slug = body.slug.trim().to_owned();
    if !is_slug(&slug) || slug.len() > 40 {
        return Err(ApiError::invalid(
            "A group's short name is up to 40 lowercase letters, digits and inner dashes",
        ));
    }
    if forge::is_reserved(&slug) {
        return Err(ApiError::invalid(format!(
            "{slug} is a name the forge keeps for itself"
        )));
    }
    let name = required_text(&body.name, "The name", 80)?;
    let taken: Option<i64> = sqlx::query_scalar("SELECT id FROM groups WHERE slug = ?")
        .bind(&slug)
        .fetch_optional(&state.db)
        .await?;
    if taken.is_some() {
        return Err(ApiError::conflict(format!(
            "There is already a group {slug}"
        )));
    }
    forge::create_group(
        forge,
        &state,
        &slug,
        &name,
        &library::template(&slug, &name),
    )
    .await?;
    match insert_group(&state, &viewer, &slug, &name).await {
        Ok(group) => {
            library::created(&state, group, viewer.id).await?;
            forge::sync_members_or_log(&state, group).await;
        }
        Err(error) => {
            // The organization is the group's alone: without the group, it
            // goes too.
            let _ = forge.delete_repository(&slug, forge::LIBRARY).await;
            let _ = forge.delete_organization(&slug).await;
            return Err(error);
        }
    }
    let group = Membership::of(&state, &viewer, &slug).await?.group();
    Ok((StatusCode::CREATED, Json(group)))
}

/// Keeps a new group, with `viewer` as its admin.
async fn insert_group(state: &AppState, viewer: &Viewer, slug: &str, name: &str) -> ApiResult<i64> {
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

#[derive(sqlx::FromRow)]
struct InvitationRow {
    id: i64,
    role: String,
    created_at: i64,
    expires_at: Option<i64>,
    max_uses: Option<i64>,
    uses: i64,
    #[sqlx(flatten)]
    creator: UserColumns,
}

impl From<InvitationRow> for Invitation {
    fn from(row: InvitationRow) -> Self {
        Invitation {
            id: row.id,
            role: Role::parse(&row.role).unwrap_or(Role::Guest),
            created_by: row.creator.into(),
            created_at: db::from_millis(row.created_at),
            expires_at: row.expires_at.map(db::from_millis),
            max_uses: row.max_uses.map(|n| n as u32),
            uses: row.uses as u32,
        }
    }
}

/// The invitations that can still be used.
pub async fn invitations(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Invitation>>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let rows: Vec<InvitationRow> = sqlx::query_as(
        "SELECT i.id, i.role, i.created_at, i.expires_at, i.max_uses, i.uses,
                u.id AS user_id, u.username, u.display_name
         FROM invitations i JOIN users u ON u.id = i.created_by
         WHERE i.group_id = ? AND i.revoked_at IS NULL
           AND (i.expires_at IS NULL OR i.expires_at > ?)
           AND (i.max_uses IS NULL OR i.uses < i.max_uses)
         ORDER BY i.created_at DESC",
    )
    .bind(group.group_id)
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(Invitation::from).collect()))
}

/// Creates an invitation that grants a role: a member's, if it names none.
pub async fn create_invitation(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<CreateInvitation>,
) -> ApiResult<(StatusCode, Json<CreatedInvitation>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    if body.max_uses == Some(0) || body.expires_in_days == Some(0) {
        return Err(ApiError::invalid(
            "An invitation is used at least once and lasts at least a day",
        ));
    }
    let role = body.role.unwrap_or(Role::Member);
    let token = secret::token("");
    let now = db::now();
    let expires_at = body
        .expires_in_days
        .map(|days| now + i64::from(days) * 24 * 60 * 60 * 1000);
    let id = sqlx::query(
        "INSERT INTO invitations (group_id, token_hash, role, created_by, created_at, expires_at, max_uses)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group.group_id)
    .bind(secret::token_hash(&token))
    .bind(role.as_str())
    .bind(viewer.id)
    .bind(now)
    .bind(expires_at)
    .bind(body.max_uses.map(i64::from))
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    let invitation = Invitation {
        id,
        role,
        created_by: viewer.user(),
        created_at: db::from_millis(now),
        expires_at: expires_at.map(db::from_millis),
        max_uses: body.max_uses,
        uses: 0,
    };
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvitation { invitation, token }),
    ))
}

pub async fn revoke_invitation(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, id)): Path<(String, i64)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let done = sqlx::query(
        "UPDATE invitations SET revoked_at = ? WHERE id = ? AND group_id = ? AND revoked_at IS NULL",
    )
    .bind(db::now())
    .bind(id)
    .bind(group.group_id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Invitation"));
    }
    Ok(StatusCode::NO_CONTENT)
}

struct ValidInvitation {
    id: i64,
    group_id: i64,
    group_slug: String,
    group_name: String,
    role: Role,
}

async fn valid_invitation(state: &AppState, token: &str) -> ApiResult<ValidInvitation> {
    let row: Option<(i64, i64, String, String, String)> = sqlx::query_as(
        "SELECT i.id, g.id, g.slug, g.name, i.role
         FROM invitations i JOIN groups g ON g.id = i.group_id
         WHERE i.token_hash = ? AND i.revoked_at IS NULL
           AND (i.expires_at IS NULL OR i.expires_at > ?)
           AND (i.max_uses IS NULL OR i.uses < i.max_uses)",
    )
    .bind(secret::token_hash(token))
    .bind(db::now())
    .fetch_optional(&state.db)
    .await?;
    let (id, group_id, group_slug, group_name, role) = row.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "invalid_invitation",
            "This invitation is not valid: it may have expired, been used up or been revoked",
        )
    })?;
    Ok(ValidInvitation {
        id,
        group_id,
        group_slug,
        group_name,
        role: Role::parse(&role).unwrap_or(Role::Guest),
    })
}

pub async fn invitation_info(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<Json<InvitationInfo>> {
    let invitation = valid_invitation(&state, &token).await?;
    Ok(Json(InvitationInfo {
        group_slug: invitation.group_slug,
        group_name: invitation.group_name,
        role: invitation.role,
    }))
}

/// Joins the group of an invitation: as the person signed in, or as a new
/// account, which is then signed in.
pub async fn accept_invitation(
    State(state): State<AppState>,
    viewer: Option<Viewer>,
    Path(token): Path<String>,
    Json(body): Json<AcceptInvitation>,
) -> ApiResult<impl IntoResponse> {
    let invitation = valid_invitation(&state, &token).await?;
    let (user_id, cookie) = match (body.account, viewer) {
        (Some(account), _) => {
            let id = super::account::create_user(&state, account, false).await?;
            (id, auth::start_session(&state, id).await?)
        }
        (None, Some(viewer)) => {
            viewer.require_full()?;
            (viewer.id, HeaderMap::new())
        }
        (None, None) => return Err(ApiError::unauthenticated()),
    };

    let mut tx = state.db.begin().await?;
    let joined = sqlx::query(
        "INSERT INTO memberships (group_id, user_id, role, joined_at) VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(invitation.group_id)
    .bind(user_id)
    .bind(invitation.role.as_str())
    .bind(db::now())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if joined > 0 {
        sqlx::query("UPDATE invitations SET uses = uses + 1 WHERE id = ?")
            .bind(invitation.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if joined > 0 {
        forge::sync_members_or_log(&state, invitation.group_id).await;
    }

    let me = super::account::me_of(&state, user_id).await?;
    Ok((cookie, Json(me)))
}
