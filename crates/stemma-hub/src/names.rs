//! Name ownership and canonical addresses. IDs remain the identity.
use axum::extract::{Path, Query, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::json;

use crate::api::Json;
use crate::auth::Viewer;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub async fn available(
    state: &AppState,
    kind: &str,
    value: &str,
    owner: Option<i64>,
) -> ApiResult<String> {
    let name = if kind == "user" {
        value.trim().to_lowercase()
    } else {
        value.trim().to_owned()
    };
    let valid = match kind {
        "user" => crate::api::valid_username(&name),
        "group" => stemma_core::address::is_slug(&name) && name.len() <= 40,
        _ => return Err(ApiError::not_found("Name kind")),
    };
    if !valid {
        return Err(ApiError::invalid(if kind == "user" {
            "A username is 1 to 32 lowercase letters, digits, '.', '_' or '-', starting and ending with a letter or digit, with no adjacent punctuation"
        } else {
            "A group's short name is up to 40 lowercase letters, digits and inner dashes"
        }));
    }
    if crate::forge::is_reserved(&name) {
        return Err(ApiError::invalid(format!(
            "{name} is reserved for Stemma or the forge"
        )));
    }
    if kind == "user" && stemma_core::api::AgentProfile::default_for(&name).is_some() {
        return Err(ApiError::conflict(format!("@{name} belongs to an agent")));
    }
    let user: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?1 UNION SELECT user_id FROM user_names WHERE name = ?1")
        .bind(&name).fetch_optional(&state.db).await?;
    let group: Option<i64> = sqlx::query_scalar("SELECT id FROM groups WHERE slug = ?1 UNION SELECT group_id FROM group_names WHERE name = ?1")
        .bind(&name).fetch_optional(&state.db).await?;
    if user.is_some_and(|id| kind != "user" || Some(id) != owner)
        || group.is_some_and(|id| kind != "group" || Some(id) != owner)
    {
        return Err(ApiError::conflict(format!(
            "{name} belongs to another account or group, as a current or former name"
        )));
    }
    Ok(name)
}

#[derive(Deserialize)]
pub struct Check {
    name: String,
    group: Option<String>,
}
pub async fn check(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(kind): Path<String>,
    Query(query): Query<Check>,
) -> ApiResult<Json<serde_json::Value>> {
    viewer.require_full()?;
    let owner = if kind == "group" {
        match query.group {
            Some(slug) => {
                let group = crate::api::Membership::of(&state, &viewer, &slug).await?;
                group.require_admin()?;
                Some(group.group_id)
            }
            None => None,
        }
    } else {
        Some(viewer.id)
    };
    Ok(Json(
        match available(&state, &kind, &query.name, owner).await {
            Ok(_) => json!({"available": true, "reason": null}),
            Err(error) => json!({"available": false, "reason": error.message}),
        },
    ))
}

pub async fn user(
    State(state): State<AppState>,
    viewer: Viewer,
    Path(username): Path<String>,
) -> ApiResult<Json<stemma_core::api::User>> {
    viewer.require_full()?;
    let (id, username, display_name) = sqlx::query_as(
        "SELECT id, username, display_name FROM users WHERE username = ? AND deleted_at IS NULL",
    )
    .bind(username)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Account"))?;
    Ok(Json(stemma_core::api::User {
        id,
        username,
        display_name,
    }))
}

/// Redirect before routing, preserving the suffix and query even for git/API paths.
pub async fn redirect(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    for (prefix, table, column, target, id_column) in [
        ("/g/", "group_names", "slug", "groups", "group_id"),
        (
            "/api/v1/groups/",
            "group_names",
            "slug",
            "groups",
            "group_id",
        ),
        ("/git/", "group_names", "slug", "groups", "group_id"),
        (
            "/api/v1/users/",
            "user_names",
            "username",
            "users",
            "user_id",
        ),
        ("/u/", "user_names", "username", "users", "user_id"),
    ] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let (name, suffix) = rest
                .split_once('/')
                .map(|(n, s)| (n, format!("/{s}")))
                .unwrap_or((rest, String::new()));
            let current: Result<Option<String>, _> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT t.{column} FROM {table} n JOIN {target} t ON t.id = n.{id_column} WHERE n.name = ? AND t.{column} != n.name")))
                .bind(name).fetch_optional(&state.db).await;
            if let Ok(Some(current)) = current {
                let query = request
                    .uri()
                    .query()
                    .map(|q| format!("?{q}"))
                    .unwrap_or_default();
                return Redirect::permanent(&format!("{prefix}{current}{suffix}{query}"))
                    .into_response();
            }
        }
    }
    let token = crate::auth::bearer_token(request.headers()).map(crate::secret::token_hash);
    let mut response = next.run(request).await;
    let username = if let Some(hash) = token {
        sqlx::query_scalar::<_, String>("SELECT u.username FROM users u JOIN api_tokens t ON t.user_id = u.id WHERE t.token_hash = ? AND t.kind = 'personal' AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > ?) AND u.deleted_at IS NULL")
            .bind(hash).bind(crate::db::now()).fetch_optional(&state.db).await.ok().flatten()
    } else {
        None
    };
    if let Some(username) = username
        && let Ok(value) = username.parse()
    {
        response.headers_mut().insert("x-stemma-user", value);
    }
    response
}
