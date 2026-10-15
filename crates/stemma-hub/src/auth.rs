//! Who is asking: a session cookie from the web app, or an API token in an
//! `Authorization: Bearer` header (or as the password of basic credentials,
//! as git sends it): a personal token, which can do whatever
//! its owner can, or a job's token, which reads one group for a job's
//! requester while the job lasts, and for a task of the workshop, pushes
//! to the group's library.

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
pub const JOB_TOKEN_PREFIX: &str = "stj_";
const SESSION_DAYS: i64 = 30;

/// The person making a request.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub hub_admin: bool,
    /// Whether the request came with an API token, not a session.
    pub with_token: bool,
    pub scope: Scope,
}

/// What a request may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Whatever the person can: a session or a personal token.
    Full,
    /// A job's token.
    Job(JobScope),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobScope {
    /// The token's own id, which its job records.
    pub token_id: i64,
    pub group_id: i64,
    /// The thread it may post in, if it may post.
    pub thread_id: Option<i64>,
    /// The agent its posts are written by.
    pub agent: String,
    pub can_post: bool,
    /// Whether it pushes to the library, as a task's token does.
    pub can_push: bool,
}

impl Viewer {
    pub fn user(&self) -> User {
        User {
            id: self.id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }

    pub fn job(&self) -> Option<&JobScope> {
        match &self.scope {
            Scope::Full => None,
            Scope::Job(job) => Some(job),
        }
    }

    /// Fails for a job's token: what follows needs the person.
    pub fn require_full(&self) -> Result<(), ApiError> {
        match self.scope {
            Scope::Full => Ok(()),
            Scope::Job(_) => Err(ApiError::forbidden("A job's token cannot do this")),
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

impl ViewerRow {
    fn viewer(self, with_token: bool, scope: Scope) -> Viewer {
        Viewer {
            id: self.id,
            username: self.username,
            display_name: self.display_name,
            hub_admin: self.hub_admin,
            with_token,
            scope,
        }
    }
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    #[sqlx(flatten)]
    user: ViewerRow,
    token_id: i64,
    kind: String,
    group_id: Option<i64>,
    thread_id: Option<i64>,
    agent: Option<String>,
    can_post: bool,
    can_push: bool,
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

    if let Some(token) = request_token(headers) {
        let hash = secret::token_hash(&token);
        let row = sqlx::query_as::<_, TokenRow>(
            "SELECT u.id, u.username, u.display_name, u.hub_admin, t.id AS token_id,
                    t.kind, t.group_id, t.thread_id, t.agent, t.can_post, t.can_push
             FROM api_tokens t JOIN users u ON u.id = t.user_id
             WHERE t.token_hash = ? AND t.revoked_at IS NULL
               AND (t.expires_at IS NULL OR t.expires_at > ?)",
        )
        .bind(&hash)
        .bind(now)
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
        let scope = match (row.kind.as_str(), row.group_id) {
            ("job", Some(group_id)) => Scope::Job(JobScope {
                token_id: row.token_id,
                group_id,
                thread_id: row.thread_id,
                agent: row.agent.unwrap_or_else(|| "Agent".into()),
                can_post: row.can_post,
                can_push: row.can_push,
            }),
            ("job", None) => return Err(ApiError::internal("a job token without a group")),
            _ => Scope::Full,
        };
        return Ok(Some(row.user.viewer(true, scope)));
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
    Ok(row.map(|row| row.viewer(false, Scope::Full)))
}

/// The token a request comes with: a bearer token, or the password of
/// basic credentials, as git sends them.
fn request_token(headers: &HeaderMap) -> Option<String> {
    if let Some(token) = bearer_token(headers) {
        return Some(token.to_owned());
    }
    use base64::Engine;
    let encoded = headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let (_, password) = std::str::from_utf8(&decoded).ok()?.split_once(':')?;
    Some(password.trim().to_owned()).filter(|p| !p.is_empty())
}

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// A token for a job, acting for `user_id` in `group_id` until
/// `expires_at`; it posts only in `post_in`, a thread, if given. Returns
/// its id and secret.
pub async fn create_job_token(
    db: &sqlx::SqlitePool,
    user_id: i64,
    group_id: i64,
    post_in: Option<i64>,
    agent: &str,
    expires_at: i64,
) -> Result<(i64, String), ApiError> {
    let secret = secret::token(JOB_TOKEN_PREFIX);
    let prefix: String = secret.chars().take(JOB_TOKEN_PREFIX.len() + 6).collect();
    let id = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_hash, prefix, created_at, kind, group_id,
                                 thread_id, agent, can_post, expires_at)
         VALUES (?, ?, ?, ?, ?, 'job', ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(format!("{agent}, for a job"))
    .bind(secret::token_hash(&secret))
    .bind(&prefix)
    .bind(db::now())
    .bind(group_id)
    .bind(post_in)
    .bind(agent)
    .bind(post_in.is_some())
    .bind(expires_at)
    .execute(db)
    .await?
    .last_insert_rowid();
    Ok((id, secret))
}

/// Lets a job's token push to its group's library, as a task's does.
pub async fn grant_push(db: &sqlx::SqlitePool, token_id: i64) -> Result<(), ApiError> {
    sqlx::query("UPDATE api_tokens SET can_push = 1 WHERE id = ? AND kind = 'job'")
        .bind(token_id)
        .execute(db)
        .await?;
    Ok(())
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
