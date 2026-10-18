//! Git over HTTP for the groups' libraries: `/git/<group>.git`, through
//! `git http-backend`.
//!
//! Members clone and push with a personal token as the password (any user
//! name), or as a bearer token; guests only clone. A job's token only
//! clones, unless it is a task's, which pushes too. After a person's push the hub looks at `main`,
//! and builds its new commit; after a task's, the task's own build and
//! inspection of what it pushed become the library's state when the task
//! ends (see [`crate::workshop`]).

use std::process::Stdio;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::TryStreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::api::Membership;
use crate::auth::Viewer;
use crate::error::ApiError;
use crate::library;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/git/{repo}", any(serve_root))
        .route("/git/{repo}/{*path}", any(serve))
        .with_state(state)
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

async fn serve_root(
    state: State<AppState>,
    viewer: Option<Viewer>,
    Path(repo): Path<String>,
    request: Request,
) -> Response {
    handle(state, viewer, repo, String::new(), request).await
}

async fn serve(
    state: State<AppState>,
    viewer: Option<Viewer>,
    Path((repo, path)): Path<(String, String)>,
    request: Request,
) -> Response {
    handle(state, viewer, repo, path, request).await
}

async fn handle(
    State(state): State<AppState>,
    viewer: Option<Viewer>,
    repo: String,
    path: String,
    request: Request,
) -> Response {
    let Some(viewer) = viewer else {
        return ask_for_token();
    };
    let slug = repo.strip_suffix(".git").unwrap_or(&repo);
    let group = match Membership::of(&state, &viewer, slug).await {
        Ok(group) => group,
        Err(error) => return error.into_response(),
    };
    let query = request.uri().query().unwrap_or_default().to_owned();
    let pushing = path == "git-receive-pack" || query.contains("service=git-receive-pack");
    if pushing && viewer.job().is_some_and(|job| !job.can_push) {
        return refused(
            StatusCode::FORBIDDEN,
            "A job's token cannot push to the library",
        );
    }
    if pushing && !group.role.can_ask_for_work() {
        return refused(StatusCode::FORBIDDEN, "Guests cannot push to the library");
    }
    // Only the smart protocol's three endpoints.
    if !matches!(
        path.as_str(),
        "info/refs" | "git-upload-pack" | "git-receive-pack"
    ) {
        return refused(StatusCode::NOT_FOUND, "Not found");
    }
    let repository = library::repository(&state, group.group_id);
    if !state.git || !repository.exists() {
        return refused(StatusCode::NOT_FOUND, "This group has no library");
    }
    match backend(
        &state,
        &viewer,
        group.group_id,
        &path,
        &query,
        request,
        pushing,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => ApiError::internal(error).into_response(),
    }
}

/// Runs `git http-backend` for one request, streaming both ways.
async fn backend(
    state: &AppState,
    viewer: &Viewer,
    group_id: i64,
    path: &str,
    query: &str,
    request: Request,
    pushing: bool,
) -> anyhow::Result<Response> {
    let root = std::path::absolute(state.config.libraries_dir())?;
    let headers: HeaderMap = request.headers().clone();
    let header_value = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    let mut command = Command::new("git");
    command
        .arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_PROJECT_ROOT", &root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", format!("/{group_id}.git/{path}"))
        .env("REQUEST_METHOD", request.method().as_str())
        .env("QUERY_STRING", query)
        .env("CONTENT_TYPE", header_value(header::CONTENT_TYPE))
        .env("REMOTE_USER", &viewer.username)
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("GIT_COMMITTER_NAME", &viewer.display_name)
        .env("GIT_COMMITTER_EMAIL", format!("{}@stemma", viewer.username))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let encoding = header_value(header::CONTENT_ENCODING);
    if !encoding.is_empty() {
        command.env("HTTP_CONTENT_ENCODING", encoding);
    }
    if let Some(protocol) = headers.get("git-protocol").and_then(|v| v.to_str().ok()) {
        command.env("GIT_PROTOCOL", protocol);
    }
    let mut child = command.spawn()?;

    // The request's body goes to the backend as it comes.
    let mut stdin = child.stdin.take().expect("piped");
    let is_post = request.method() == Method::POST;
    let body = request
        .into_body()
        .into_data_stream()
        .map_err(std::io::Error::other);
    tokio::spawn(async move {
        if is_post {
            let mut reader = StreamReader::new(body);
            let _ = tokio::io::copy(&mut reader, &mut stdin).await;
        }
        let _ = stdin.shutdown().await;
    });

    // Its answer: CGI headers, then the body.
    let mut stdout = BufReader::new(child.stdout.take().expect("piped"));
    let mut status = StatusCode::OK;
    let mut response_headers = HeaderMap::new();
    loop {
        let mut line = String::new();
        if stdout.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            let code = value.split_whitespace().next().unwrap_or("500");
            status = code.parse().unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        } else if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.trim().as_bytes()),
            HeaderValue::from_str(value),
        ) {
            response_headers.append(name, value);
        }
    }

    // After a person's push, the hub looks at where `main` is.
    let (state, user) = (state.clone(), viewer.id);
    let refresh = pushing && viewer.job().is_none();
    tokio::spawn(async move {
        let _ = child.wait().await;
        if refresh && let Err(error) = library::refresh(&state, group_id, user).await {
            tracing::error!(group_id, error = error.message, "could not look at a push");
        }
    });

    let mut response = Response::new(Body::from_stream(ReaderStream::new(stdout)));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok(response)
}
