//! Browser sessions and API tokens.
//!
//! A personal token (a device's or a script's) can do whatever its owner
//! can. Agents hold tokens of their own, limited to one group: a job's
//! token, for an answer, only reads it; an agent's token, for a person's
//! session with their agent, reads it and posts or opens threads there,
//! always as the agent. Neither pushes, attaches files, or reaches the
//! account, its tokens or the group's settings.

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
/// A browser session lasts a day from sign-in: an account can send work to
/// its owner's computers. Tokens have their own lifetimes.
const SESSION_HOURS: i64 = 24;

/// The person making a request.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub hub_admin: bool,
    /// Whether the request came with an API token, not a session.
    pub with_token: bool,
    /// A device's or a script's token: not an agent's, nor a browser session.
    pub personal_token: bool,
    pub scope: Scope,
}

/// What a request may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Whatever the person can: a session or a personal token.
    Full,
    /// An agent's token: a job's or a session's.
    Agent(AgentScope),
    Process {
        agent: AgentScope,
        work: i64,
        attempt: u32,
    },
    Build {
        work: i64,
        attempt: u32,
        group_id: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentScope {
    /// The token's own id, which its job records.
    pub token_id: i64,
    pub attachments: stemma_core::api::AttachmentRule,
    pub group_id: i64,
    /// The thread it may post in, if it may post.
    pub thread_id: Option<i64>,
    /// The agent its posts are written by.
    pub agent: String,
    pub can_post: bool,
    /// A session's token, which may also open threads; else a job's.
    pub session: bool,
}

impl Viewer {
    pub fn require_hub_admin(&self) -> Result<(), ApiError> {
        self.require_full()?;
        if self.hub_admin {
            Ok(())
        } else {
            Err(ApiError::forbidden("Only the hub's admins can do this"))
        }
    }

    pub fn user(&self) -> User {
        User {
            id: self.id,
            username: self.username.clone(),
            display_name: self.display_name.clone(),
        }
    }

    pub fn agent(&self) -> Option<&AgentScope> {
        match &self.scope {
            Scope::Full | Scope::Build { .. } => None,
            Scope::Agent(agent) | Scope::Process { agent, .. } => Some(agent),
        }
    }

    /// Fails for an agent's token: what follows needs the person.
    pub fn require_full(&self) -> Result<(), ApiError> {
        match self.scope {
            Scope::Full => Ok(()),
            Scope::Agent(_) | Scope::Build { .. } | Scope::Process { .. } => {
                Err(ApiError::forbidden("An agent's token cannot do this"))
            }
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
            personal_token: false,
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
    build_work: Option<i64>,
    lease_attempt: Option<i64>,
    group_id: Option<i64>,
    thread_id: Option<i64>,
    agent: Option<String>,
    can_post: bool,
    attachments: String,
}

impl FromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let viewer = authenticate(&parts.headers, state)
            .await?
            .ok_or_else(ApiError::unauthenticated)?;
        build_path(&viewer, parts)?;
        Ok(viewer)
    }
}

impl OptionalFromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, ApiError> {
        let viewer = authenticate(&parts.headers, state).await?;
        if let Some(viewer) = &viewer {
            build_path(viewer, parts)?;
        }
        Ok(viewer)
    }
}

async fn authenticate(headers: &HeaderMap, state: &AppState) -> Result<Option<Viewer>, ApiError> {
    let now = db::now();

    if let Some(token) = request_token(headers) {
        let hash = secret::token_hash(&token);
        let row = sqlx::query_as::<_, TokenRow>(
            "SELECT u.id, u.username, u.display_name, u.hub_admin, t.id AS token_id,
                    t.kind, t.build_work, t.lease_attempt, t.group_id, t.thread_id, t.agent, t.can_post, t.attachments
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
            ("build", Some(group_id)) => Scope::Build {
                work: row
                    .build_work
                    .ok_or_else(|| ApiError::internal("build token without work"))?,
                attempt: row.lease_attempt.unwrap_or_default() as u32,
                group_id,
            },
            ("process", Some(group_id)) => Scope::Process {
                work: row
                    .build_work
                    .ok_or_else(|| ApiError::internal("process token without work"))?,
                attempt: row.lease_attempt.unwrap_or_default() as u32,
                agent: AgentScope {
                    token_id: row.token_id,
                    attachments: row.attachments.parse().map_err(ApiError::internal)?,
                    group_id,
                    thread_id: row.thread_id,
                    agent: row.agent.clone().unwrap_or_else(|| "Agent".into()),
                    can_post: true,
                    session: false,
                },
            },
            (kind @ ("job" | "agent"), Some(group_id)) => Scope::Agent(AgentScope {
                token_id: row.token_id,
                attachments: row.attachments.parse().map_err(ApiError::internal)?,
                group_id,
                thread_id: row.thread_id,
                agent: row.agent.unwrap_or_else(|| "Agent".into()),
                can_post: row.can_post,
                session: kind == "agent",
            }),
            ("job" | "agent" | "build" | "process", None) => {
                return Err(ApiError::internal("a scoped token without a group"));
            }
            _ => Scope::Full,
        };
        if let Scope::Process { work, attempt, .. } = &scope {
            let alive: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1
                 FROM work_leases w
                 JOIN workshop_processes p ON p.work_id = w.id
                 WHERE w.id = ?
                 AND w.attempts = ?
                 AND w.state = 'running'
                 AND w.heartbeat > ?
                 AND p.deadline > ?)",
            )
            .bind(work)
            .bind(*attempt)
            .bind(now - crate::lease::EXPIRY_MILLIS)
            .bind(now)
            .fetch_one(&state.db)
            .await?;
            if !alive {
                return Err(ApiError::forbidden("This process lease ended"));
            }
        }
        let mut viewer = row.user.viewer(true, scope);
        viewer.personal_token = row.kind == "personal";
        return Ok(Some(viewer));
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
/// `expires_at`: it only reads the group. Returns its id and secret.
pub async fn create_job_token(
    db: &sqlx::SqlitePool,
    user_id: i64,
    group_id: i64,
    agent: &str,
    expires_at: i64,
    attachments: stemma_core::api::AttachmentRule,
) -> Result<(i64, String), ApiError> {
    let secret = secret::token(JOB_TOKEN_PREFIX);
    let prefix: String = secret.chars().take(JOB_TOKEN_PREFIX.len() + 6).collect();
    let id = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_hash, prefix, created_at, kind, group_id,
                                 agent, can_post, expires_at, attachments)
         VALUES (?, ?, ?, ?, ?, 'job', ?, ?, 0, ?, ?)",
    )
    .bind(user_id)
    .bind(format!("{agent}, for a job"))
    .bind(secret::token_hash(&secret))
    .bind(&prefix)
    .bind(db::now())
    .bind(group_id)
    .bind(agent)
    .bind(expires_at)
    .bind(attachments.as_str())
    .execute(db)
    .await?
    .last_insert_rowid();
    Ok((id, secret))
}

/// How long an agent's session token lasts, in milliseconds: a long day.
pub const AGENT_TOKEN_MILLIS: i64 = 16 * 60 * 60 * 1000;

/// A token for `agent`'s session with `user_id`, in `group_id`: it reads
/// the group, and posts and opens threads there as the agent. Returns its
/// id and secret.
pub async fn create_agent_token(
    db: &sqlx::SqlitePool,
    user_id: i64,
    group_id: i64,
    agent: &str,
    name: &str,
    attachments: stemma_core::api::AttachmentRule,
) -> Result<(i64, String, i64), ApiError> {
    let secret = secret::token(API_TOKEN_PREFIX);
    let prefix: String = secret.chars().take(API_TOKEN_PREFIX.len() + 6).collect();
    let now = db::now();
    let expires_at = now + AGENT_TOKEN_MILLIS;
    let id = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_hash, prefix, created_at, kind, group_id,
                                 agent, can_post, expires_at, attachments)
         VALUES (?, ?, ?, ?, ?, 'agent', ?, ?, 1, ?, ?)",
    )
    .bind(user_id)
    .bind(name)
    .bind(secret::token_hash(&secret))
    .bind(&prefix)
    .bind(now)
    .bind(group_id)
    .bind(agent)
    .bind(expires_at)
    .bind(attachments.as_str())
    .execute(db)
    .await?
    .last_insert_rowid();
    Ok((id, secret, expires_at))
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
    let max_age = SESSION_HOURS * 60 * 60;
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

fn build_path(viewer: &Viewer, parts: &Parts) -> Result<(), ApiError> {
    if let Scope::Process { .. } = &viewer.scope {
        let path = parts
            .extensions
            .get::<axum::extract::OriginalUri>()
            .map(|uri| uri.0.path())
            .unwrap_or_else(|| parts.uri.path());
        if path.starts_with("/git/")
            || (parts.method == axum::http::Method::GET && path.starts_with("/api/v1/groups/"))
            || (parts.method == axum::http::Method::POST
                && path.starts_with("/api/v1/groups/")
                && (path.ends_with("/messages")
                    || path.ends_with("/attachments")
                    || path.ends_with("/resolve")))
        {
            return Ok(());
        }
        return Err(ApiError::forbidden(
            "A process token only reads its group and posts in its request",
        ));
    }
    if let Scope::Build { work, .. } = viewer.scope {
        let path = parts
            .extensions
            .get::<axum::extract::OriginalUri>()
            .map(|uri| uri.0.path())
            .unwrap_or_else(|| parts.uri.path());
        if path.starts_with("/git/")
            || (parts.method == axum::http::Method::POST
                && [
                    format!("/api/v1/builds/{work}/pdf"),
                    format!("/api/v1/builds/{work}/result"),
                ]
                .contains(&path.to_owned()))
        {
            return Ok(());
        }
        return Err(ApiError::forbidden(
            "A build token only fetches its repository and reports its build",
        ));
    }
    Ok(())
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
