//! Accounts: the first one, signing in and out, the profile and API tokens.

use std::sync::LazyLock;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use stemma_core::api::{
    ApiToken, ChangePassword, CreateToken, CreatedToken, Health, Me, NewAccount, Setup, SetupState,
    SignIn, UpdateMe,
};

use super::{Json, required_text};
use crate::auth::{self, API_TOKEN_PREFIX, Viewer};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::secret;
use crate::state::AppState;

pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

pub async fn setup_state(State(state): State<AppState>) -> ApiResult<Json<SetupState>> {
    Ok(Json(SetupState {
        needed: crate::user_count(&state.db).await? == 0,
    }))
}

/// Creates the first account, a hub admin, with the setup token.
pub async fn setup(
    State(state): State<AppState>,
    Json(body): Json<Setup>,
) -> ApiResult<impl IntoResponse> {
    let expected = state.setup_token.lock().expect("not poisoned").clone();
    if expected.as_deref() != Some(body.token.trim()) {
        return Err(ApiError::forbidden(
            "The setup token is not valid; the hub prints it when it starts without accounts",
        ));
    }
    if crate::user_count(&state.db).await? > 0 {
        return Err(ApiError::conflict("The hub already has accounts"));
    }
    let user_id = create_user(&state, body.account, true).await?;
    *state.setup_token.lock().expect("not poisoned") = None;
    let cookie = auth::start_session(&state, user_id).await?;
    Ok((
        StatusCode::CREATED,
        cookie,
        Json(me_of(&state, user_id).await?),
    ))
}

/// Validates a new account and creates it.
pub async fn create_user(state: &AppState, account: NewAccount, hub_admin: bool) -> ApiResult<i64> {
    let username = account.username.trim().to_lowercase();
    if !valid_username(&username) {
        return Err(ApiError::invalid(
            "A username is 1 to 32 lowercase letters, digits, '.', '_' or '-', starting and \
             ending with a letter or digit, with no two of '.', '_' and '-' in a row",
        ));
    }
    if crate::forge::is_reserved(&username) {
        return Err(ApiError::invalid(format!(
            "{username} is a name the forge keeps for itself"
        )));
    }
    let display_name = required_text(&account.display_name, "The name", 80)?;
    check_password(&account.password)?;
    let hash = auth::hash_password(account.password).await?;
    let result = sqlx::query(
        "INSERT INTO users (username, display_name, password_hash, hub_admin, subject, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&username)
    .bind(&display_name)
    .bind(hash)
    .bind(hub_admin)
    .bind(secret::token(""))
    .bind(db::now())
    .execute(&state.db)
    .await;
    match result {
        Ok(done) => Ok(done.last_insert_rowid()),
        Err(e) if db::is_unique_violation(&e) => Err(ApiError::conflict(format!(
            "The username {username} is taken"
        ))),
        Err(e) => Err(e.into()),
    }
}

/// Whether `name` can be a username: also a valid account name in the
/// forge.
pub fn valid_username(name: &str) -> bool {
    let plain = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes.first().copied().is_some_and(plain)
        && bytes.last().copied().is_some_and(plain)
        && bytes.iter().all(|&b| plain(b) || b"._-".contains(&b))
        && bytes.windows(2).all(|w| plain(w[0]) || plain(w[1]))
}

fn check_password(password: &str) -> ApiResult<()> {
    match password.chars().count() {
        0..8 => Err(ApiError::invalid("A password has at least 8 characters")),
        8..=1024 => Ok(()),
        _ => Err(ApiError::invalid("That password is too long")),
    }
}

static UNKNOWN_USER_HASH: LazyLock<String> =
    LazyLock::new(|| secret::hash_password("no one").unwrap_or_default());

pub async fn sign_in(
    State(state): State<AppState>,
    Json(body): Json<SignIn>,
) -> ApiResult<impl IntoResponse> {
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
            .bind(body.username.trim().to_lowercase())
            .fetch_optional(&state.db)
            .await?;
    // Without the account, check a password all the same, so that the time
    // taken does not tell which usernames exist.
    let (user_id, hash) = match row {
        Some((id, hash)) => (Some(id), hash),
        None => (None, UNKNOWN_USER_HASH.clone()),
    };
    let matches = auth::verify_password(body.password, hash).await;
    let Some(user_id) = user_id.filter(|_| matches) else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "wrong_credentials",
            "Wrong username or password",
        ));
    };
    let cookie = auth::start_session(&state, user_id).await?;
    Ok((cookie, Json(me_of(&state, user_id).await?)))
}

pub async fn sign_out(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let cookie = auth::end_session(&state, &headers).await?;
    Ok((StatusCode::NO_CONTENT, cookie))
}

/// The person signed in. A job's token sees only its job's group.
pub async fn me(State(state): State<AppState>, viewer: Viewer) -> ApiResult<Json<Me>> {
    let mut me = me_of(&state, viewer.id).await?;
    if let Some(job) = viewer.job() {
        let slug: String = sqlx::query_scalar("SELECT slug FROM groups WHERE id = ?")
            .bind(job.group_id)
            .fetch_one(&state.db)
            .await?;
        me.groups.retain(|g| g.slug == slug);
        me.hub_admin = false;
    }
    Ok(Json(me))
}

pub async fn me_of(state: &AppState, user_id: i64) -> ApiResult<Me> {
    let (id, username, display_name, hub_admin): (i64, String, String, bool) =
        sqlx::query_as("SELECT id, username, display_name, hub_admin FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    Ok(Me {
        user: stemma_core::api::User {
            id,
            username,
            display_name,
        },
        hub_admin,
        groups: super::groups::groups_of(state, user_id).await?,
    })
}

pub async fn update_me(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<UpdateMe>,
) -> ApiResult<Json<Me>> {
    viewer.require_full()?;
    let display_name = required_text(&body.display_name, "The name", 80)?;
    sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
        .bind(display_name)
        .bind(viewer.id)
        .execute(&state.db)
        .await?;
    Ok(Json(me_of(&state, viewer.id).await?))
}

/// Changes the password, and ends every other session.
pub async fn change_password(
    State(state): State<AppState>,
    viewer: Viewer,
    headers: HeaderMap,
    Json(body): Json<ChangePassword>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(viewer.id)
        .fetch_one(&state.db)
        .await?;
    if !auth::verify_password(body.current, hash).await {
        return Err(ApiError::forbidden("The current password is wrong"));
    }
    check_password(&body.new)?;
    let hash = auth::hash_password(body.new).await?;
    let current = auth::cookie(&headers, auth::SESSION_COOKIE)
        .map(secret::token_hash)
        .unwrap_or_default();
    let mut tx = state.db.begin().await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(viewer.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(viewer.id)
        .bind(current)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    id: i64,
    name: String,
    prefix: String,
    created_at: i64,
    last_used_at: Option<i64>,
    expires_at: Option<i64>,
}

impl From<TokenRow> for ApiToken {
    fn from(row: TokenRow) -> Self {
        ApiToken {
            id: row.id,
            name: row.name,
            prefix: row.prefix,
            created_at: db::from_millis(row.created_at),
            last_used_at: row.last_used_at.map(db::from_millis),
            expires_at: row.expires_at.map(db::from_millis),
        }
    }
}

pub async fn list_tokens(
    State(state): State<AppState>,
    viewer: Viewer,
) -> ApiResult<Json<Vec<ApiToken>>> {
    viewer.require_full()?;
    let rows: Vec<TokenRow> = sqlx::query_as(
        "SELECT id, name, prefix, created_at, last_used_at, expires_at FROM api_tokens
         WHERE user_id = ? AND kind = 'personal' AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > ?)
         ORDER BY created_at DESC",
    )
    .bind(viewer.id)
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().map(ApiToken::from).collect()))
}

pub async fn create_token(
    State(state): State<AppState>,
    viewer: Viewer,
    Json(body): Json<CreateToken>,
) -> ApiResult<(StatusCode, Json<CreatedToken>)> {
    viewer.require_full()?;
    let name = required_text(&body.name, "The name", 80)?;
    let secret = secret::token(API_TOKEN_PREFIX);
    let prefix: String = secret.chars().take(API_TOKEN_PREFIX.len() + 6).collect();
    let now = db::now();
    let expires_at = match body.expires_in_minutes {
        Some(0) => return Err(ApiError::invalid("A token lasts at least a minute")),
        Some(minutes) => Some(now + i64::from(minutes) * 60_000),
        None => None,
    };
    let id = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_hash, prefix, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(viewer.id)
    .bind(&name)
    .bind(secret::token_hash(&secret))
    .bind(&prefix)
    .bind(now)
    .bind(expires_at)
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    let token = ApiToken {
        id,
        name,
        prefix,
        created_at: db::from_millis(now),
        last_used_at: None,
        expires_at: expires_at.map(db::from_millis),
    };
    Ok((StatusCode::CREATED, Json(CreatedToken { token, secret })))
}

pub async fn revoke_token(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    viewer.require_full()?;
    let done = sqlx::query(
        "UPDATE api_tokens SET revoked_at = ?
         WHERE id = ? AND user_id = ? AND kind = 'personal' AND revoked_at IS NULL",
    )
    .bind(db::now())
    .bind(id)
    .bind(viewer.id)
    .execute(&state.db)
    .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found("Token"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        for name in ["ada", "a", "ada.lovelace", "a_1-b", "1ada"] {
            assert!(valid_username(name), "{name}");
        }
        for name in [
            "",
            "Ada",
            "-ada",
            ".a",
            "ada.",
            "ada__lovelace",
            "ada.-lovelace",
            "ada lovelace",
            "ñ",
            &"a".repeat(33),
        ] {
            assert!(!valid_username(name), "{name}");
        }
    }
}
