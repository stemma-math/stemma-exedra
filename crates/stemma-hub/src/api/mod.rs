//! The JSON API, version 1, under `/api/v1`.

mod account;
pub use account::create_user;
pub(crate) mod agents;
mod attachments;
mod events;
pub(crate) mod forum;
mod groups;
mod jobs;
mod library;
mod papers;
mod tasks;
pub mod workers;
mod workshop;

use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use serde::Serialize;
use serde::de::DeserializeOwned;
use stemma_core::api::{Role, User};

use crate::auth::Viewer;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    // Room for the multipart framing around the largest file.
    let upload_limit = (state.config.max_upload_mb as usize + 1) * 1024 * 1024;
    let api = Router::new()
        .route("/health", get(account::health))
        .route("/setup", get(account::setup_state).post(account::setup))
        .route("/session", post(account::sign_in).delete(account::sign_out))
        .route("/me/login-handoff", post(crate::handoff::issue))
        .route("/me", get(account::me).patch(account::update_me))
        .route("/me/password", post(account::change_password))
        .route(
            "/me/tokens",
            get(account::list_tokens).post(account::create_token),
        )
        .route("/me/tokens/{id}", delete(account::revoke_token))
        .route("/invitations/{token}", get(groups::invitation_info))
        .route(
            "/invitations/{token}/accept",
            post(groups::accept_invitation),
        )
        .route("/groups", get(groups::list).post(groups::create))
        .route("/groups/{group}", get(groups::get))
        .route("/groups/{group}/members", get(groups::members))
        .route(
            "/groups/{group}/members/{user}",
            patch(groups::update_member).delete(groups::remove_member),
        )
        .route(
            "/groups/{group}/invitations",
            get(groups::invitations).post(groups::create_invitation),
        )
        .route(
            "/groups/{group}/invitations/{id}",
            delete(groups::revoke_invitation),
        )
        .route(
            "/groups/{group}/threads",
            get(forum::list_threads).post(forum::create_thread),
        )
        .route(
            "/groups/{group}/threads/{thread}",
            get(forum::get_thread).patch(forum::update_thread),
        )
        .route(
            "/groups/{group}/threads/{thread}/messages",
            get(forum::list_messages).post(forum::post_message),
        )
        .route(
            "/groups/{group}/threads/{thread}/messages/{message}",
            get(forum::get_message)
                .patch(forum::edit_message)
                .delete(forum::delete_message),
        )
        .route("/groups/{group}/library", get(library::state))
        .route(
            "/groups/{group}/library/builds",
            get(library::builds).post(library::rebuild),
        )
        .route("/groups/{group}/library/builds/{id}", get(library::build))
        .route(
            "/groups/{group}/library/declarations",
            get(library::declarations),
        )
        .route(
            "/groups/{group}/library/declarations/{name}",
            get(library::declaration),
        )
        .route(
            "/groups/{group}/papers",
            get(papers::list).post(papers::create),
        )
        .route("/groups/{group}/papers/{paper}", get(papers::get))
        .route(
            "/groups/{group}/papers/{paper}/authors/{username}",
            put(papers::add_author).delete(papers::remove_author),
        )
        .route(
            "/groups/{group}/papers/{paper}/builds",
            post(papers::rebuild),
        )
        .route(
            "/groups/{group}/papers/{paper}/builds/{build}/pdf",
            put(papers::upload_pdf).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route("/groups/{group}/papers/{paper}/pdf", get(papers::pdf))
        .route("/groups/{group}/papers/{paper}/files", get(papers::files))
        .route(
            "/groups/{group}/papers/{paper}/files/{*path}",
            get(papers::file),
        )
        .route("/groups/{group}/papers/{paper}/tasks", post(papers::edit))
        .route("/groups/{group}/resolve", post(forum::resolve))
        .route("/groups/{group}/search", get(forum::search))
        .route("/workshop/architectures", get(workshop::architectures))
        .route(
            "/groups/{group}/workshop/machines",
            get(workshop::list_machines).post(workshop::add_machine),
        )
        .route(
            "/groups/{group}/workshop/machines/{id}",
            delete(workshop::remove_machine),
        )
        .route(
            "/groups/{group}/workshop/consumption",
            get(workshop::consumption),
        )
        .route(
            "/me/machines",
            get(workers::list_machines).post(workers::create_machine),
        )
        .route(
            "/me/machines/{id}",
            patch(workers::update_machine).delete(workers::revoke_machine),
        )
        .route(
            "/me/machines/{id}/workshop-agents",
            put(workers::set_workshop_agents),
        )
        .route(
            "/me/agents",
            get(agents::list_mine).post(agents::create_mine),
        )
        .route(
            "/me/agents/{handle}",
            put(agents::update_mine).delete(agents::remove_mine),
        )
        .route(
            "/groups/{group}/threads/{thread}/jobs",
            get(jobs::of_thread),
        )
        .route("/groups/{group}/jobs/{id}", delete(jobs::cancel))
        .route(
            "/groups/{group}/tasks",
            get(tasks::list).post(tasks::create),
        )
        .route(
            "/groups/{group}/tasks/{number}",
            get(tasks::get).delete(tasks::cancel),
        )
        .route(
            "/groups/{group}/tasks/{number}/approve",
            post(workshop::approve),
        )
        .route(
            "/groups/{group}/tasks/{number}/reject",
            post(workshop::reject),
        )
        .route(stemma_core::protocol::WORKER_PATH, get(workers::connect))
        .route(
            "/groups/{group}/attachments",
            post(attachments::upload).layer(DefaultBodyLimit::max(upload_limit)),
        )
        .route(
            "/groups/{group}/attachments/{id}/{name}",
            get(attachments::download),
        )
        .route("/groups/{group}/events", get(events::stream))
        .fallback(|| async { ApiError::not_found("Route") });

    Router::new()
        .nest(stemma_core::api::PREFIX, api)
        .with_state(state)
}

/// A JSON body, whose errors are API errors.
pub struct Json<T>(pub T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Json<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Json(value)),
            Err(rejection) => Err(json_rejection(rejection)),
        }
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

fn json_rejection(rejection: JsonRejection) -> ApiError {
    let status = match rejection {
        JsonRejection::MissingJsonContentType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        _ => StatusCode::BAD_REQUEST,
    };
    ApiError::new(status, "bad_request", rejection.body_text())
}

/// A group the viewer belongs to, with their role in it.
pub struct Membership {
    pub group_id: i64,
    pub slug: String,
    pub name: String,
    pub created_at: i64,
    pub role: Role,
}

impl Membership {
    /// Finds the group `slug` among the viewer's. A group they are not in
    /// is not found, so its existence is not revealed.
    ///
    /// A job's token sees only its job's group, with its requester's role
    /// there, but never as an admin.
    pub async fn of(state: &AppState, viewer: &Viewer, slug: &str) -> ApiResult<Membership> {
        let row: Option<(i64, String, String, i64, String)> = sqlx::query_as(
            "SELECT g.id, g.slug, g.name, g.created_at, m.role
             FROM groups g JOIN memberships m ON m.group_id = g.id
             WHERE g.slug = ? AND m.user_id = ?",
        )
        .bind(slug)
        .bind(viewer.id)
        .fetch_optional(&state.db)
        .await?;
        let (group_id, slug, name, created_at, role) =
            row.ok_or_else(|| ApiError::not_found("Group"))?;
        let role = Role::parse(&role).unwrap_or(Role::Guest);
        let role = match viewer.job() {
            Some(job) if job.group_id != group_id => return Err(ApiError::not_found("Group")),
            Some(_) if role == Role::Admin => Role::Member,
            _ => role,
        };
        Ok(Membership {
            group_id,
            slug,
            name,
            created_at,
            role,
        })
    }

    /// A group as the hub itself acts in it, for what it posts.
    pub async fn of_group(state: &AppState, group_id: i64) -> ApiResult<Membership> {
        let (slug, name, created_at): (String, String, i64) =
            sqlx::query_as("SELECT slug, name, created_at FROM groups WHERE id = ?")
                .bind(group_id)
                .fetch_optional(&state.db)
                .await?
                .ok_or_else(|| ApiError::not_found("Group"))?;
        Ok(Membership {
            group_id,
            slug,
            name,
            created_at,
            role: Role::Member,
        })
    }

    pub fn require_admin(&self) -> ApiResult<()> {
        if self.role.is_admin() {
            Ok(())
        } else {
            Err(ApiError::forbidden("Only the group's admins can do this"))
        }
    }

    pub fn group(&self) -> stemma_core::api::Group {
        stemma_core::api::Group {
            slug: self.slug.clone(),
            name: self.name.clone(),
            role: self.role,
            created_at: db::from_millis(self.created_at),
        }
    }
}

/// A user, as joined into another row under these column names.
#[derive(sqlx::FromRow)]
pub struct UserColumns {
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
}

impl From<UserColumns> for User {
    fn from(u: UserColumns) -> Self {
        User {
            id: u.user_id,
            username: u.username,
            display_name: u.display_name,
        }
    }
}

/// Trims a required piece of text and checks its length in characters.
pub fn required_text(value: &str, what: &str, max: usize) -> ApiResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::invalid(format!("{what} cannot be empty")));
    }
    if value.chars().count() > max {
        return Err(ApiError::invalid(format!(
            "{what} is too long (at most {max} characters)"
        )));
    }
    Ok(value.to_owned())
}
