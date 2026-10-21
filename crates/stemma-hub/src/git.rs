//! Streaming authenticated git access to the forge. Job tokens only fetch.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use stemma_core::api::Role;

use crate::api::Membership;
use crate::auth::{Scope, Viewer};
use crate::forge::Forge;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/git/{group}/{repository}/{*path}", any(serve))
        .with_state(state)
}

/// What a request to git asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    /// Clone or fetch: `git-upload-pack`.
    Fetch,
    /// Push: `git-receive-pack`.
    Push,
}

/// Which of the smart protocol's requests this is, if it is one: the
/// advertisement of a service's refs, or the service itself.
pub fn service(method: &Method, path: &str, query: Option<&str>) -> Option<Service> {
    let named = |name: &str| match name {
        "git-upload-pack" => Some(Service::Fetch),
        "git-receive-pack" => Some(Service::Push),
        _ => None,
    };
    match (method, path) {
        (&Method::GET, "info/refs") => {
            let mut asked = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
                .filter(|(key, _)| key == "service");
            let (_, value) = asked.next()?;
            // One service, and only one.
            if asked.next().is_some() {
                return None;
            }
            named(&value)
        }
        (&Method::POST, path) => named(path),
        _ => None,
    }
}

/// Whether a request with `scope`, from someone with `role` in the group,
/// may do `service`; if not, why.
pub fn allowed(scope: &Scope, role: Role, service: Service) -> Result<(), &'static str> {
    if service == Service::Fetch {
        return Ok(());
    }
    if matches!(scope, Scope::Job(_)) {
        return Err("A job's token cannot push");
    }
    if !role.can_write() {
        return Err("Guests cannot push to the group's repositories");
    }
    Ok(())
}

/// Whether `name` can be a repository of a group: as Forgejo's names, but
/// never `.` or `..`.
fn is_repository_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// What asks for a password, so that git prompts for one.
fn ask_for_token() -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        "Use a personal token from Stemma's settings as the password.\n",
    )
        .into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"Stemma\""),
    );
    response
}

fn refused(status: StatusCode, message: &str) -> Response {
    (status, format!("{message}\n")).into_response()
}

async fn serve(
    State(state): State<AppState>,
    viewer: Option<Viewer>,
    Path((group, repository, path)): Path<(String, String, String)>,
    request: Request,
) -> Response {
    // Git is for tokens: a session does not reach it.
    let Some(viewer) = viewer.filter(|v| v.with_token) else {
        return ask_for_token();
    };
    let Some(forge) = state.forge.as_deref() else {
        return refused(StatusCode::NOT_FOUND, "This hub keeps no repositories");
    };
    let name = repository.strip_suffix(".git").unwrap_or(&repository);
    if !is_repository_name(name) {
        return refused(StatusCode::NOT_FOUND, "Not found");
    }
    let membership = match Membership::of(&state, &viewer, &group).await {
        Ok(membership) => membership,
        Err(error) => return error.into_response(),
    };
    let Some(service) = service(request.method(), &path, request.uri().query()) else {
        return refused(StatusCode::NOT_FOUND, "Only git's smart protocol is served");
    };
    if let Err(why) = allowed(&viewer.scope, membership.role, service) {
        return refused(StatusCode::FORBIDDEN, why);
    }
    // A task's token pushes only to its task's repository.
    let upstream = format!("{}/{name}.git/{path}", membership.slug);
    match pass_on(forge, &upstream, request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(%error, upstream, "could not reach the forge for git");
            refused(StatusCode::BAD_GATEWAY, "The hub could not reach the forge")
        }
    }
}

/// The request headers git's protocol needs, passed on as they are.
const REQUEST_HEADERS: &[header::HeaderName] = &[
    header::CONTENT_TYPE,
    header::CONTENT_ENCODING,
    header::ACCEPT,
    header::ACCEPT_ENCODING,
    header::USER_AGENT,
];

/// The answer's headers, passed back as they are.
const RESPONSE_HEADERS: &[header::HeaderName] = &[
    header::CONTENT_TYPE,
    header::CONTENT_ENCODING,
    header::CACHE_CONTROL,
    header::EXPIRES,
    header::PRAGMA,
];

/// Sends the request on to Forgejo, as its admin, streaming both ways.
async fn pass_on(forge: &Forge, path: &str, request: Request) -> anyhow::Result<Response> {
    let (parts, body) = request.into_parts();
    let mut upstream = forge.git(parts.method.clone(), path, parts.uri.query());
    for name in REQUEST_HEADERS {
        if let Some(value) = parts.headers.get(name) {
            upstream = upstream.header(name, value);
        }
    }
    if let Some(protocol) = parts.headers.get("git-protocol") {
        upstream = upstream.header("git-protocol", protocol);
    }
    if parts.method == Method::POST {
        upstream = upstream.body(reqwest::Body::wrap_stream(body.into_data_stream()));
    }
    let answer = upstream.send().await?;
    let status = answer.status();
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        anyhow::bail!("Forgejo refused the hub's own credentials ({status})");
    }
    let mut response = Response::builder().status(status.as_u16());
    for name in RESPONSE_HEADERS {
        if let Some(value) = answer.headers().get(name) {
            response = response.header(name, value);
        }
    }
    Ok(response.body(Body::from_stream(answer.bytes_stream()))?)
}
