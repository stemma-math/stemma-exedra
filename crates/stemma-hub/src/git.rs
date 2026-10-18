//! Git over HTTP for the groups' repositories, `/git/<group>/<name>.git`,
//! passed on to the forge, which keeps them.
//!
//! Workers and people with a personal token talk only to the hub, which
//! decides what each may do and then asks Forgejo as its admin: no
//! credential of Forgejo ever leaves the hub. A personal token is the
//! password (with any user name), or a bearer token. Everyone in the group
//! fetches; admins and members push, guests never do. A job's token only
//! fetches, unless it is a workshop task's, which pushes to its task's
//! branch and nowhere else: the hub reads the refs a push updates before
//! passing it on. Only git's smart protocol is served.
//!
//! Forgejo itself keeps `main` from being rewritten or deleted, and tells
//! the hub of each push to it with a webhook (see [`crate::forge::hook`]).

use axum::Router;
use axum::body::{Body, Bytes};
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
    if let Scope::Job(job) = scope
        && job.push_ref.is_none()
    {
        return Err("A job's token pushes only for a task of the workshop");
    }
    if !role.can_ask_for_work() {
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
    // A task's token pushes only to its branch.
    let request = match viewer.job().and_then(|job| job.push_ref.as_deref()) {
        Some(only) if service == Service::Push && request.method() == Method::POST => {
            match only_to(request, only).await {
                Ok(request) => request,
                Err(why) => return refused(StatusCode::FORBIDDEN, &why),
            }
        }
        _ => request,
    };
    let upstream = format!("{}/{name}.git/{path}", membership.slug);
    match pass_on(forge, &upstream, request).await {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(%error, upstream, "could not reach the forge for git");
            refused(StatusCode::BAD_GATEWAY, "The hub could not reach the forge")
        }
    }
}

/// The most a push's commands may take, before its pack.
const MAX_COMMANDS: usize = 64 * 1024;

/// The refs a push's request updates, from the start of its body, and
/// where its commands end; nothing if they have not all come yet.
pub fn pushed_refs(body: &[u8]) -> Result<Option<(Vec<String>, usize)>, String> {
    let mut refs = Vec::new();
    let mut at = 0;
    loop {
        let Some(head) = body.get(at..at + 4) else {
            return Ok(None);
        };
        let length = std::str::from_utf8(head)
            .ok()
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or("This is not a push git understands")?;
        if length == 0 {
            return Ok(Some((refs, at + 4)));
        }
        if length < 4 {
            return Err("This is not a push git understands".into());
        }
        let Some(line) = body.get(at + 4..at + length) else {
            return Ok(None);
        };
        let line = String::from_utf8_lossy(line);
        let command = line.split('\0').next().unwrap_or_default().trim_end();
        if command.starts_with("shallow ") {
            // Not an update.
        } else if command.starts_with("push-cert") {
            return Err("Signed pushes are not served".into());
        } else {
            let reference = command
                .split(' ')
                .nth(2)
                .ok_or("This is not a push git understands")?;
            refs.push(reference.to_owned());
        }
        at += length;
    }
}

/// The request, if every ref it pushes is `only`; read so far, and passed
/// on whole.
async fn only_to(request: Request, only: &str) -> Result<Request, String> {
    let (parts, body) = request.into_parts();
    let mut stream = body.into_data_stream();
    let mut head: Vec<u8> = Vec::new();
    let refs = loop {
        if let Some((refs, _)) = pushed_refs(&head)? {
            break refs;
        }
        if head.len() > MAX_COMMANDS {
            return Err("This push updates too much".into());
        }
        match stream.next().await {
            Some(Ok(chunk)) => head.extend_from_slice(&chunk),
            Some(Err(_)) | None => return Err("The push ended early".into()),
        }
    };
    if let Some(other) = refs.iter().find(|r| *r != only) {
        return Err(format!(
            "This token pushes only to {only}, and this push updates {other}"
        ));
    }
    let first = futures_util::stream::once(async move { Ok::<_, axum::Error>(Bytes::from(head)) });
    let body = Body::from_stream(first.chain(stream));
    Ok(Request::from_parts(parts, body))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::JobScope;

    fn job(can_push: bool) -> Scope {
        Scope::Job(JobScope {
            token_id: 1,
            group_id: 1,
            thread_id: None,
            agent: "Lean".into(),
            can_post: false,
            push_ref: can_push.then(|| "refs/heads/workshop/w1".into()),
        })
    }

    fn pkt(line: &str) -> String {
        format!("{:04x}{line}", line.len() + 4)
    }

    #[test]
    fn reads_the_refs_a_push_updates() {
        let zero = "0".repeat(40);
        let one = "1".repeat(40);
        let body = format!(
            "{}{}{}0000PACK...",
            pkt(&format!("shallow {one}\n")),
            pkt(&format!(
                "{zero} {one} refs/heads/workshop/w1\0report-status\n"
            )),
            pkt(&format!("{one} {zero} refs/heads/main\n")),
        );
        let (refs, end) = pushed_refs(body.as_bytes()).unwrap().unwrap();
        assert_eq!(refs, ["refs/heads/workshop/w1", "refs/heads/main"]);
        assert_eq!(&body[end..], "PACK...");
        // Not all there yet.
        assert_eq!(pushed_refs(&body.as_bytes()[..30]).unwrap(), None);
        assert!(pushed_refs(b"zzzz").is_err());
    }

    #[test]
    fn only_the_smart_protocol_is_served() {
        let get = Method::GET;
        let post = Method::POST;
        let fetch = Some("service=git-upload-pack");
        let push = Some("service=git-receive-pack");
        assert_eq!(service(&get, "info/refs", fetch), Some(Service::Fetch));
        assert_eq!(service(&get, "info/refs", push), Some(Service::Push));
        assert_eq!(
            service(&post, "git-upload-pack", None),
            Some(Service::Fetch)
        );
        assert_eq!(
            service(&post, "git-receive-pack", None),
            Some(Service::Push)
        );
        // A push is a push, whatever the query says.
        assert_eq!(
            service(&post, "git-receive-pack", fetch),
            Some(Service::Push)
        );
        // The dumb protocol, other files and other methods are not.
        assert_eq!(service(&get, "info/refs", None), None);
        assert_eq!(service(&get, "HEAD", None), None);
        assert_eq!(service(&get, "objects/info/packs", None), None);
        assert_eq!(service(&get, "git-upload-pack", None), None);
        assert_eq!(service(&Method::PUT, "git-receive-pack", None), None);
        assert_eq!(service(&get, "info/refs", Some("service=git-other")), None);
        // Asking for two services at once is asking for neither.
        assert_eq!(
            service(
                &get,
                "info/refs",
                Some("service=git-upload-pack&service=git-receive-pack")
            ),
            None
        );
    }

    #[test]
    fn everyone_fetches() {
        for role in [Role::Admin, Role::Member, Role::Guest] {
            assert!(allowed(&Scope::Full, role, Service::Fetch).is_ok());
            assert!(allowed(&job(false), role, Service::Fetch).is_ok());
        }
    }

    #[test]
    fn admins_and_members_push_and_guests_never_do() {
        assert!(allowed(&Scope::Full, Role::Admin, Service::Push).is_ok());
        assert!(allowed(&Scope::Full, Role::Member, Service::Push).is_ok());
        assert!(allowed(&Scope::Full, Role::Guest, Service::Push).is_err());
        assert!(allowed(&job(true), Role::Guest, Service::Push).is_err());
    }

    #[test]
    fn only_a_tasks_job_token_pushes() {
        assert!(allowed(&job(false), Role::Member, Service::Push).is_err());
        assert!(allowed(&job(true), Role::Member, Service::Push).is_ok());
    }

    #[test]
    fn repository_names() {
        assert!(is_repository_name("library"));
        assert!(is_repository_name("paper-2.v1"));
        for name in ["", ".", "..", ".git", "a/b", "a b", "%2e%2e"] {
            assert!(!is_repository_name(name), "{name}");
        }
    }
}
