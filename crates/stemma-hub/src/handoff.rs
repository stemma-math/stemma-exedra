//! One-use, short-lived browser sign-in from a person's device token.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use stemma_core::api::LoginHandoff;

use crate::api::Json;
use crate::auth::{self, Viewer};
use crate::error::{ApiError, ApiResult};
use crate::{db, secret, state::AppState};

pub async fn issue(State(state): State<AppState>, viewer: Viewer) -> ApiResult<impl IntoResponse> {
    if !viewer.personal_token {
        return Err(ApiError::forbidden(
            "Only a person's own token can open a browser session",
        ));
    }
    let now = db::now();
    // Bound storage as well as lifetime; the plaintext code is never kept.
    sqlx::query("DELETE FROM login_handoffs WHERE expires_at <= ?")
        .bind(now)
        .execute(&state.db)
        .await?;
    let code = secret::token("");
    sqlx::query("INSERT INTO login_handoffs (code_hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(secret::token_hash(&code))
        .bind(viewer.id)
        .bind(now + 60_000)
        .execute(&state.db)
        .await?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(LoginHandoff { code }),
    ))
}

#[derive(Deserialize)]
pub struct Destination {
    #[serde(default)]
    code: String,
    next: Option<String>,
}

/// Exactly the login page's destination rules, with control characters refused too.
fn relative(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
}

pub async fn exchange(
    State(state): State<AppState>,
    Query(query): Query<Destination>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let next = query.next.as_deref().unwrap_or("/");
    if !relative(next) {
        return Err(ApiError::invalid(
            "The next address must be a relative path on this hub",
        ));
    }
    // DELETE RETURNING makes concurrent exchanges single-use too. Creating the
    // session uses the same cookie code as password login.
    let desktop = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("StemmaDesktop/"));
    let user: Option<i64> = if desktop {
        sqlx::query_scalar(
            "DELETE FROM login_handoffs WHERE code_hash = ? AND expires_at > ? RETURNING user_id",
        )
        .bind(secret::token_hash(&query.code))
        .bind(db::now())
        .fetch_optional(&state.db)
        .await?
    } else {
        None
    };
    let (headers, target) = if let Some(user) = user {
        (auth::start_session(&state, user).await?, next.to_owned())
    } else {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("handoff", "expired")
            .append_pair("next", next)
            .finish();
        (HeaderMap::new(), format!("/login?{query}"))
    };
    Ok(replace(headers, &target))
}

fn replace(mut headers: HeaderMap, target: &str) -> Response {
    let nonce = secret::token("");
    // Escape HTML delimiters even though this is a JSON string inside a script.
    let target = serde_json::to_string(target)
        .expect("serialize address")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026");
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_str(&format!("default-src 'none'; script-src 'nonce-{nonce}'; base-uri 'none'; frame-ancestors 'none'")).expect("CSP header"));
    (headers, Html(format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>Stemma</title><script nonce=\"{nonce}\">location.replace({target});</script></html>"))).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn destinations_stay_on_this_hub() {
        for path in ["/", "/g/algebra", "/oauth/authorize?x=1"] {
            assert!(super::relative(path));
        }
        for path in [
            "",
            "https://evil.test",
            "//evil.test",
            "/\\evil.test",
            "/a\\b",
            "/\nevil.test",
        ] {
            assert!(!super::relative(path), "{path}");
        }
    }
}
