//! Who is asking: a session cookie from the web app, or a personal API
//! token in an `Authorization: Bearer` header.

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue};
use stemma_core::api::User;

use crate::db;
use crate::error::ApiError;
use crate::secret;
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "stemma_session";
pub const API_TOKEN_PREFIX: &str = "stm_";
const SESSION_DAYS: i64 = 30;

/// The person making a request.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub hub_admin: bool,
}

impl Viewer {
    pub fn user(&self) -> User {
        User {
            id: self.id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ViewerRow {
    id: i64,
    username: String,
    display_name: String,
    hub_admin: bool,
}

impl From<ViewerRow> for Viewer {
    fn from(row: ViewerRow) -> Self {
        Viewer {
            id: row.id,
            username: row.username,
            display_name: row.display_name,
            hub_admin: row.hub_admin,
        }
    }
}

impl FromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        authenticate(&parts.headers, state)
            .await?
            .ok_or_else(ApiError::unauthenticated)
    }
}

impl OptionalFromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, ApiError> {
        authenticate(&parts.headers, state).await
    }
}

async fn authenticate(headers: &HeaderMap, state: &AppState) -> Result<Option<Viewer>, ApiError> {
    let now = db::now();

    if let Some(token) = bearer_token(headers) {
        let hash = secret::token_hash(token);
        let row = sqlx::query_as::<_, ViewerRow>(
            "SELECT u.id, u.username, u.display_name, u.hub_admin
             FROM api_tokens t JOIN users u ON u.id = t.user_id
             WHERE t.token_hash = ? AND t.revoked_at IS NULL",
        )
        .bind(&hash)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                axum::http::StatusCode::UNAUTHORIZED,
                "invalid_token",
                "This token is not valid, or was revoked",
            )
        })?;
        // Note the use, at most once a minute.
        sqlx::query(
            "UPDATE api_tokens SET last_used_at = ?
             WHERE token_hash = ? AND (last_used_at IS NULL OR last_used_at < ?)",
        )
        .bind(now)
        .bind(&hash)
        .bind(now - 60_000)
        .execute(&state.db)
        .await?;
        return Ok(Some(row.into()));
    }

    let Some(token) = cookie(headers, SESSION_COOKIE) else {
        return Ok(None);
    };
    let row = sqlx::query_as::<_, ViewerRow>(
        "SELECT u.id, u.username, u.display_name, u.hub_admin
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(secret::token_hash(token))
    .bind(now)
    .fetch_optional(&state.db)
    .await?;
    Ok(row.map(Viewer::from))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// The value of a cookie in the request.
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Starts a session for `user_id`; returns the `Set-Cookie` header.
pub async fn start_session(state: &AppState, user_id: i64) -> Result<HeaderMap, ApiError> {
    let token = secret::token("");
    let now = db::now();
    let max_age = SESSION_DAYS * 24 * 60 * 60;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(secret::token_hash(&token))
    .bind(user_id)
    .bind(now)
    .bind(now + max_age * 1000)
    .execute(&state.db)
    .await?;
    Ok(session_cookie(state, &token, max_age))
}

/// Ends the session in the request, if any; returns the `Set-Cookie`
/// header that clears it.
pub async fn end_session(state: &AppState, headers: &HeaderMap) -> Result<HeaderMap, ApiError> {
    if let Some(token) = cookie(headers, SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(secret::token_hash(token))
            .execute(&state.db)
            .await?;
    }
    Ok(session_cookie(state, "", 0))
}

fn session_cookie(state: &AppState, value: &str, max_age: i64) -> HeaderMap {
    let secure = if state.config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{SESSION_COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("a cookie is a valid header"),
    );
    headers
}

/// Checks a password off the async runtime.
pub async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || secret::verify_password(&password, &hash))
        .await
        .unwrap_or(false)
}

/// Hashes a password off the async runtime.
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || secret::hash_password(&password))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("a=1; stemma_session=xyz"));
        headers.append(COOKIE, HeaderValue::from_static("b=2"));
        assert_eq!(cookie(&headers, SESSION_COOKIE), Some("xyz"));
        assert_eq!(cookie(&headers, "b"), Some("2"));
        assert_eq!(cookie(&headers, "c"), None);
    }
}
