//! Streaming authenticated git access to the forge. Job tokens only fetch.

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::StreamExt;
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
    if matches!(scope, Scope::Agent(_) | Scope::Build { .. }) {
        return Err("An agent's token cannot push");
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
    if let Scope::Build {
        work,
        group_id,
        attempt,
    } = viewer.scope
    {
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1
             FROM work_leases
             WHERE id = ?
             AND group_id = ?
             AND repository = ?
             AND attempts = ?
             AND state = 'running')",
        )
        .bind(work)
        .bind(group_id)
        .bind(name)
        .bind(attempt)
        .fetch_one(&state.db)
        .await
        .unwrap_or(false);
        if group_id != membership.group_id || !valid {
            return refused(
                StatusCode::FORBIDDEN,
                "This build token fetches only its repository",
            );
        }
    }
    let process_branch = if let Scope::Process { work, .. } = viewer.scope {
        if service == Service::Push {
            let raw: Option<String> =
                sqlx::query_scalar("SELECT branches FROM workshop_processes WHERE work_id=?")
                    .bind(work)
                    .fetch_optional(&state.db)
                    .await
                    .unwrap_or(None);
            let branches: Vec<stemma_core::workshop::ProcessBranch> = raw
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            match branches.into_iter().find(|b| b.repository == name) {
                Some(branch) => Some(format!("refs/heads/{}", branch.branch)),
                None => {
                    return refused(
                        StatusCode::FORBIDDEN,
                        "This process cannot push to this repository",
                    );
                }
            }
        } else {
            None
        }
    } else {
        None
    };
    let request = if service == Service::Push && request.method() == Method::POST {
        if request.headers().contains_key(header::CONTENT_ENCODING) {
            return refused(StatusCode::BAD_REQUEST, "Encoded pushes are not supported");
        }
        match check_scoped_push(request, membership.role, process_branch.as_deref()).await {
            Ok(request) => request,
            Err(error) => {
                let why = error.message;
                let fatal = if error.sideband {
                    format!("\x03{why}\n")
                } else {
                    format!("ERR {why}\n")
                };
                return (
                    [(
                        header::CONTENT_TYPE,
                        "application/x-git-receive-pack-result",
                    )],
                    format!("{:04x}{fatal}0000", fatal.len() + 4),
                )
                    .into_response();
            }
        }
    } else {
        request
    };
    if service == Service::Push
        && request.method() == Method::POST
        && let Scope::Process { work, .. } = viewer.scope
    {
        // Conservatively fence retries before streaming a possible push. A lost
        // response can mean the forge accepted it, even if the machine never heard.
        if sqlx::query("UPDATE workshop_processes SET pushed=1 WHERE work_id=?")
            .bind(work)
            .execute(&state.db)
            .await
            .is_err()
        {
            return refused(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not record the process push",
            );
        }
    }
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

/// The most of a push's request read before its pack: its command list.
const MAX_COMMANDS: usize = 1024 * 1024;

/// Parse the receive-pack command list, stopping at its flush before the pack.
/// `None` means more bytes are needed. The returned offset starts the pack.
/// A shallow clone's `shallow` lines come first, and name no ref.
pub fn ref_updates(bytes: &[u8]) -> Result<Option<(Vec<String>, usize)>, &'static str> {
    let mut offset = 0;
    let mut refs = Vec::new();
    let is_id = |s: &str| s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit());
    loop {
        if offset > MAX_COMMANDS {
            return Err("Git's command list is too large");
        }
        let Some(header) = bytes.get(offset..offset + 4) else {
            return Ok(None);
        };
        let length = std::str::from_utf8(header)
            .ok()
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or("Invalid git packet length")?;
        if length == 0 {
            return Ok(Some((refs, offset + 4)));
        }
        if !(5..=65520).contains(&length) {
            return Err("Invalid git packet length");
        }
        let Some(line) = bytes.get(offset + 4..offset + length) else {
            return Ok(None);
        };
        let line = std::str::from_utf8(line).map_err(|_| "Invalid git command")?;
        let command = line.split('\0').next().unwrap().trim_end_matches('\n');
        let fields: Vec<_> = command.split(' ').collect();
        match fields[..] {
            ["shallow", id] if refs.is_empty() && is_id(id) => {}
            [old, new, name] if is_id(old) && is_id(new) => refs.push(name.to_owned()),
            _ => return Err("Invalid git ref update"),
        }
        offset += length;
    }
}

/// Why `role` may not make these ref updates, if it may not.
fn refused_ref(refs: &[String], role: Role) -> Option<&'static str> {
    refs.iter().find_map(|name| {
        if !name.starts_with("refs/heads/") && !name.starts_with("refs/tags/") {
            Some("Pushes may update only branches and tags")
        } else if name == "refs/heads/main" && role != Role::Admin {
            Some("Only the group's admins may push to main; push a branch and open a pull request")
        } else {
            None
        }
    })
}

#[derive(Debug)]
struct PushError {
    message: &'static str,
    sideband: bool,
}

fn push_error(message: &'static str, commands: &[u8]) -> PushError {
    PushError {
        message,
        sideband: commands
            .split(|b| *b == 0)
            .nth(1)
            .is_some_and(|caps| caps.windows(b"side-band".len()).any(|w| w == b"side-band")),
    }
}

/// Reads a push's command list, refuses it if `role` may not make its
/// updates, and otherwise gives back the request as it came, streaming the
/// pack after what was read.
#[cfg(test)]
async fn check_push(request: Request, role: Role) -> Result<Request, PushError> {
    check_scoped_push(request, role, None).await
}
async fn check_scoped_push(
    request: Request,
    role: Role,
    branch: Option<&str>,
) -> Result<Request, PushError> {
    let (parts, body) = request.into_parts();
    let mut stream = body.into_data_stream();
    let mut read = Vec::new();
    loop {
        // A command list complete with this chunk is checked at once: a
        // push that deletes sends no pack to wait for.
        match ref_updates(&read) {
            Err(why) => return Err(push_error(why, &read)),
            Ok(Some((refs, _))) => {
                if branch.is_some_and(|branch| refs.iter().any(|name| name != branch)) {
                    return Err(push_error(
                        "A process may push only its assigned branch",
                        &read,
                    ));
                }
                if let Some(why) = refused_ref(&refs, role) {
                    return Err(push_error(why, &read));
                }
                let first = futures_util::stream::once(async move {
                    Ok::<_, axum::Error>(axum::body::Bytes::from(read))
                });
                return Ok(Request::from_parts(
                    parts,
                    Body::from_stream(first.chain(stream)),
                ));
            }
            Ok(None) => {}
        }
        let Some(chunk) = stream.next().await else {
            return Err(push_error("Incomplete git command list", &read));
        };
        let chunk = chunk.map_err(|_| push_error("Could not read git commands", &read))?;
        read.extend_from_slice(&chunk);
        if read.len() > MAX_COMMANDS + 65_520 {
            return Err(push_error("Git's command list is too large", &read));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn forwards_commands_without_waiting_for_the_pack() {
        let line = format!(
            "{} {} refs/heads/main\0report-status side-band-64k\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let bytes = axum::body::Bytes::from(format!("{:04x}{line}0000", line.len() + 4));
        let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(bytes)])
            .chain(futures_util::stream::pending());
        let request = Request::new(Body::from_stream(stream));
        let forwarded = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            check_push(request, Role::Admin),
        )
        .await
        .unwrap()
        .unwrap();
        let first = forwarded
            .into_body()
            .into_data_stream()
            .next()
            .await
            .unwrap()
            .unwrap();
        assert!(first.ends_with(b"0000"));
    }

    #[test]
    fn parses_updates_and_stops_before_pack() {
        let mut bytes = Vec::new();
        for (i, name) in ["refs/heads/main", "refs/heads/topic", "refs/tags/v1"]
            .iter()
            .enumerate()
        {
            let line = format!(
                "{} {} {name}{}\n",
                "a".repeat(40),
                "0".repeat(40),
                if i == 0 {
                    "\0report-status side-band-64k"
                } else {
                    ""
                }
            );
            bytes.extend(format!("{:04x}{line}", line.len() + 4).bytes());
        }
        let end = bytes.len() + 4;
        bytes.extend(b"0000PACKanything");
        let (refs, offset) = ref_updates(&bytes).unwrap().unwrap();
        assert_eq!(
            refs,
            ["refs/heads/main", "refs/heads/topic", "refs/tags/v1"]
        );
        assert_eq!(offset, end);
        for cut in 0..end {
            assert_eq!(ref_updates(&bytes[..cut]).unwrap(), None);
        }
        assert_eq!(ref_updates(b"0000").unwrap(), Some((vec![], 4)));
        assert!(ref_updates(b"nope").is_err());
        assert!(ref_updates(b"0001").is_err());
    }

    #[test]
    fn shallow_clones_push_too() {
        let shallow = format!("shallow {}", "c".repeat(40));
        let update = format!(
            "{} {} refs/heads/topic\0report-status\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let bytes = format!(
            "{:04x}{shallow}{:04x}{update}0000",
            shallow.len() + 4,
            update.len() + 4
        );
        let (refs, _) = ref_updates(bytes.as_bytes()).unwrap().unwrap();
        assert_eq!(refs, ["refs/heads/topic"]);
        // Never after a ref update.
        let late = format!(
            "{:04x}{update}{:04x}{shallow}0000",
            update.len() + 4,
            shallow.len() + 4
        );
        assert!(ref_updates(late.as_bytes()).is_err());
    }

    #[test]
    fn only_admins_update_main() {
        let main = ["refs/heads/main".to_owned()];
        assert!(refused_ref(&main, Role::Member).is_some());
        assert!(refused_ref(&main, Role::Admin).is_none());
        assert!(
            refused_ref(
                &["refs/heads/topic".to_owned(), "refs/tags/v1".to_owned()],
                Role::Member
            )
            .is_none()
        );
        assert!(refused_ref(&["refs/notes/x".to_owned()], Role::Admin).is_some());
    }
}
