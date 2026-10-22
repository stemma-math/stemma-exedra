//! The links that let people in, of two kinds: a group's, which adds an
//! existing account to the group, and the hub's, which makes an account and
//! nothing else. Both expire, run out of uses and can be revoked.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use stemma_core::api::{
    CreateInvitation, CreatedInvitation, Invitation, InvitationInfo, NewAccount, Role, SignUpInfo,
};

use super::{Json, Membership, UserColumns};
use crate::auth::{self, Viewer};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::forge;
use crate::secret;
use crate::state::AppState;

#[derive(sqlx::FromRow)]
struct InvitationRow {
    id: i64,
    role: Option<String>,
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
            role: row.role.as_deref().and_then(Role::parse),
            created_by: row.creator.into(),
            created_at: db::from_millis(row.created_at),
            expires_at: row.expires_at.map(db::from_millis),
            max_uses: row.max_uses.map(|n| n as u32),
            uses: row.uses as u32,
        }
    }
}

/// The links of a group, or the hub's if `group_id` is `None`, that can
/// still be used.
async fn usable(state: &AppState, group_id: Option<i64>) -> ApiResult<Vec<Invitation>> {
    let rows: Vec<InvitationRow> = sqlx::query_as(
        "SELECT i.id, i.role, i.created_at, i.expires_at, i.max_uses, i.uses,
                u.id AS user_id, u.username, u.display_name
         FROM invitations i JOIN users u ON u.id = i.created_by
         WHERE i.group_id IS ? AND i.revoked_at IS NULL
           AND (i.expires_at IS NULL OR i.expires_at > ?)
           AND (i.max_uses IS NULL OR i.uses < i.max_uses)
         ORDER BY i.created_at DESC, i.id DESC",
    )
    .bind(group_id)
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    Ok(rows.into_iter().map(Invitation::from).collect())
}

/// Makes a link for a group with `role`, or for the hub.
async fn create(
    state: &AppState,
    viewer: &Viewer,
    group_id: Option<i64>,
    role: Option<Role>,
    body: &CreateInvitation,
) -> ApiResult<CreatedInvitation> {
    if body.max_uses == Some(0) {
        return Err(ApiError::invalid("A link is used at least once"));
    }
    let now = db::now();
    let expires_at = body.expires_at.map(db::to_millis);
    if expires_at.is_some_and(|at| at <= now) {
        return Err(ApiError::invalid("A link cannot expire in the past"));
    }
    let token = secret::token("");
    let id = sqlx::query(
        "INSERT INTO invitations (group_id, token_hash, role, created_by, created_at, expires_at, max_uses)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(group_id)
    .bind(secret::token_hash(&token))
    .bind(role.map(|role| role.as_str()))
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
    Ok(CreatedInvitation { invitation, token })
}

async fn revoke(state: &AppState, group_id: Option<i64>, id: i64) -> ApiResult<StatusCode> {
    let done = sqlx::query(
        "UPDATE invitations SET revoked_at = ?
         WHERE id = ? AND group_id IS ? AND revoked_at IS NULL",
    )
    .bind(db::now())
    .bind(id)
    .bind(group_id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Invitation"));
    }
    Ok(StatusCode::NO_CONTENT)
}

struct Valid {
    id: i64,
    group: Option<(i64, String, String, Role)>,
    invited_by: stemma_core::api::User,
}

/// The link whose secret is `token`, if it can still be used.
async fn valid(state: &AppState, token: &str) -> ApiResult<Valid> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: i64,
        group_id: Option<i64>,
        slug: Option<String>,
        name: Option<String>,
        role: Option<String>,
        #[sqlx(flatten)]
        creator: UserColumns,
    }
    let row: Option<Row> = sqlx::query_as(
        "SELECT i.id, i.group_id, g.slug, g.name, i.role,
                u.id AS user_id, u.username, u.display_name
         FROM invitations i JOIN users u ON u.id = i.created_by
         LEFT JOIN groups g ON g.id = i.group_id
         WHERE i.token_hash = ? AND i.revoked_at IS NULL
           AND (i.expires_at IS NULL OR i.expires_at > ?)
           AND (i.max_uses IS NULL OR i.uses < i.max_uses)",
    )
    .bind(secret::token_hash(token))
    .bind(db::now())
    .fetch_optional(&state.db)
    .await?;
    let row = row.ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "invalid_invitation",
            "This invitation is not valid: it may have expired, been used up or been revoked",
        )
    })?;
    let group = match (row.group_id, row.slug, row.name, row.role) {
        (Some(id), Some(slug), Some(name), Some(role)) => {
            Some((id, slug, name, Role::parse(&role).unwrap_or(Role::Guest)))
        }
        _ => None,
    };
    Ok(Valid {
        id: row.id,
        group,
        invited_by: row.creator.into(),
    })
}

/// Counts a use of the link `id`, unless it has none left.
async fn use_once(tx: &mut sqlx::SqliteConnection, id: i64) -> ApiResult<()> {
    let done = sqlx::query(
        "UPDATE invitations SET uses = uses + 1
         WHERE id = ? AND (max_uses IS NULL OR uses < max_uses)",
    )
    .bind(id)
    .execute(tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::conflict("This invitation has just been used up"));
    }
    Ok(())
}

// ── A group's links ──────────────────────────────────────────────────

pub async fn group_list(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Invitation>>> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    Ok(Json(usable(&state, Some(group.group_id)).await?))
}

pub async fn group_create(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(slug): Path<String>,
    Json(body): Json<CreateInvitation>,
) -> ApiResult<(StatusCode, Json<CreatedInvitation>)> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    let role = body.role.unwrap_or(Role::Member);
    let created = create(&state, &viewer, Some(group.group_id), Some(role), &body).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn group_revoke(
    State(state): State<AppState>,
    viewer: Viewer,
    Path((slug, id)): Path<(String, i64)>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let group = Membership::of(&state, &viewer, &slug).await?;
    group.require_admin()?;
    revoke(&state, Some(group.group_id), id).await
}

/// What a group's link is to. A hub's link is not one.
pub async fn info(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<Json<InvitationInfo>> {
    let Some((_, group_slug, group_name, role)) = valid(&state, &token).await?.group else {
        return Err(ApiError::not_found("Group invitation"));
    };
    Ok(Json(InvitationInfo {
        group_slug,
        group_name,
        role,
    }))
}

/// Joins the group of a link, as the person signed in. It never makes an
/// account: that takes the hub's link.
pub async fn accept(
    State(state): State<AppState>,
    viewer: Option<Viewer>,
    Path(token): Path<String>,
) -> ApiResult<Json<stemma_core::api::Me>> {
    let viewer = viewer.ok_or_else(ApiError::unauthenticated)?;
    viewer.require_full()?;
    let invitation = valid(&state, &token).await?;
    let Some((group_id, _, _, role)) = invitation.group else {
        return Err(ApiError::not_found("Group invitation"));
    };

    let mut tx = state.db.begin().await?;
    let joined = sqlx::query(
        "INSERT INTO memberships (group_id, user_id, role, joined_at) VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(viewer.id)
    .bind(role.as_str())
    .bind(db::now())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if joined > 0 {
        use_once(&mut tx, invitation.id).await?;
    }
    tx.commit().await?;
    if joined > 0 {
        forge::sync_members_or_log(&state, group_id).await;
    }
    Ok(Json(super::account::me_of(&state, viewer.id).await?))
}

// ── The hub's links ──────────────────────────────────────────────────

pub async fn hub_list(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<Invitation>>> {
    viewer.require_hub_admin()?;
    Ok(Json(usable(&state, None).await?))
}

pub async fn hub_create(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<CreateInvitation>,
) -> ApiResult<(StatusCode, Json<CreatedInvitation>)> {
    viewer.require_hub_admin()?;
    if body.role.is_some() {
        return Err(ApiError::invalid(
            "The hub's links make accounts; a group's give roles",
        ));
    }
    let created = create(&state, &viewer, None, None, &body).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn hub_revoke(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    viewer.require_hub_admin()?;
    revoke(&state, None, id).await
}

/// Who invited, for the sign-up page. A group's link is not one.
pub async fn sign_up_info(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> ApiResult<Json<SignUpInfo>> {
    let invitation = valid(&state, &token).await?;
    if invitation.group.is_some() {
        return Err(ApiError::not_found("Hub invitation"));
    }
    Ok(Json(SignUpInfo {
        invited_by: invitation.invited_by,
    }))
}

/// Makes an account with the hub's link, and signs it in.
pub async fn sign_up(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(account): Json<NewAccount>,
) -> ApiResult<impl IntoResponse> {
    let invitation = valid(&state, &token).await?;
    if invitation.group.is_some() {
        return Err(ApiError::not_found("Hub invitation"));
    }
    // The use is counted first, so a link with one use left makes one
    // account even when two people race for it.
    let mut tx = state.db.begin().await?;
    use_once(&mut tx, invitation.id).await?;
    tx.commit().await?;
    let id = match super::account::create_user(&state, account, false).await {
        Ok(id) => id,
        Err(error) => {
            sqlx::query("UPDATE invitations SET uses = uses - 1 WHERE id = ?")
                .bind(invitation.id)
                .execute(&state.db)
                .await?;
            return Err(error);
        }
    };
    let cookie = auth::start_session(&state, id).await?;
    Ok((
        StatusCode::CREATED,
        cookie,
        Json(super::account::me_of(&state, id).await?),
    ))
}
