//! A stand-in for Forgejo in Stemma's tests: the part of its API the hub
//! uses, kept in memory (accounts, organizations, teams, repositories,
//! branches and pull requests, which it merges for real), and git's smart
//! HTTP protocol, served by `git http-backend` over bare repositories in a
//! temporary folder, with the webhooks Forgejo sends after a push to
//! `main`. It needs `git`.
//!
//! It knows one admin, [`ADMIN`], whose token is [`TOKEN`]; anything else
//! is refused. It does not enforce teams: the hub talks to it as its admin.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post, put};
use base64::Engine;
use futures_util::TryStreamExt;
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio_util::io::{ReaderStream, StreamReader};

/// The admin's login.
pub const ADMIN: &str = "forge-admin";

/// The admin's token.
pub const TOKEN: &str = "fake-forge-admin-token";

/// A Forgejo of its own, serving on a local port while it lives.
pub struct FakeForge {
    pub url: String,
    inner: Arc<Inner>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Drop for FakeForge {
    fn drop(&mut self) {
        self.server.abort();
    }
}

struct Inner {
    root: PathBuf,
    data: Mutex<Data>,
    http: reqwest::Client,
    refuse_repositories: AtomicBool,
}

#[derive(Default)]
struct Data {
    next_id: i64,
    users: Vec<User>,
    orgs: HashMap<String, Vec<Team>>,
    repos: HashMap<(String, String), Repo>,
}

#[derive(Clone)]
struct User {
    id: i64,
    login: String,
    login_name: String,
    source_id: i64,
}

impl User {
    fn json(&self) -> Value {
        json!({
            "id": self.id, "login": self.login,
            "login_name": self.login_name, "source_id": self.source_id,
        })
    }
}

#[derive(Clone)]
struct Team {
    id: i64,
    name: String,
    permission: String,
    members: BTreeSet<String>,
}

#[derive(Default)]
struct Repo {
    hooks: Vec<(String, String)>,
    pulls: Vec<Pull>,
}

/// A pull request.
#[derive(Clone)]
struct Pull {
    number: u32,
    head: String,
    base: String,
    title: String,
    open: bool,
    merged: Option<String>,
}

impl Pull {
    fn json(&self, owner: &str, name: &str) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "state": if self.open { "open" } else { "closed" },
            "html_url": format!("/{owner}/{name}/pulls/{}", self.number),
            "head": {"ref": self.head},
            "base": {"ref": self.base},
            "merged": self.merged.is_some(),
            "merge_commit_sha": self.merged,
        })
    }
}

/// A pull request, as a test sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullState {
    pub number: u32,
    pub head: String,
    pub open: bool,
    pub merged: Option<String>,
}

impl Data {
    fn id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    fn name_taken(&self, name: &str) -> bool {
        self.orgs.contains_key(name) || self.users.iter().any(|u| u.login == name)
    }

    fn team(&mut self, id: i64) -> Option<&mut Team> {
        self.orgs
            .values_mut()
            .flat_map(|teams| teams.iter_mut())
            .find(|t| t.id == id)
    }
}

impl FakeForge {
    pub async fn start() -> FakeForge {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = tempfile::tempdir().expect("a temporary folder");
        let inner = Arc::new(Inner {
            root: dir.path().to_owned(),
            data: Mutex::new(Data {
                next_id: 1,
                users: vec![User {
                    id: 1,
                    login: ADMIN.into(),
                    login_name: String::new(),
                    source_id: 0,
                }],
                ..Default::default()
            }),
            http: reqwest::Client::new(),
            refuse_repositories: AtomicBool::new(false),
        });
        let app = router(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let url = format!("http://{}", listener.local_addr().expect("an address"));
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        FakeForge {
            url,
            inner,
            server,
            _dir: dir,
        }
    }

    /// The logins in a team of an organization, in order.
    pub fn team_members(&self, org: &str, team: &str) -> Vec<String> {
        let data = self.inner.data.lock().unwrap();
        data.orgs
            .get(org)
            .and_then(|teams| teams.iter().find(|t| t.name == team))
            .map(|t| t.members.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The permission of a team of an organization.
    pub fn team_permission(&self, org: &str, team: &str) -> Option<String> {
        let data = self.inner.data.lock().unwrap();
        data.orgs
            .get(org)?
            .iter()
            .find(|t| t.name == team)
            .map(|t| t.permission.clone())
    }

    /// The account linked to the authentication source as `login_name`.
    pub fn linked_login(&self, source_id: i64, login_name: &str) -> Option<String> {
        let data = self.inner.data.lock().unwrap();
        data.users
            .iter()
            .find(|u| u.source_id == source_id && u.login_name == login_name)
            .map(|u| u.login.clone())
    }

    /// Makes an account, as someone could in Forgejo itself.
    pub fn add_user(&self, login: &str) {
        let mut data = self.inner.data.lock().unwrap();
        let id = data.id();
        data.users.push(User {
            id,
            login: login.into(),
            login_name: String::new(),
            source_id: 0,
        });
    }

    pub fn organization_exists(&self, name: &str) -> bool {
        self.inner.data.lock().unwrap().orgs.contains_key(name)
    }

    pub fn repository_exists(&self, owner: &str, name: &str) -> bool {
        let data = self.inner.data.lock().unwrap();
        data.repos.contains_key(&(owner.into(), name.into()))
    }

    /// The webhooks of a repository: their addresses.
    pub fn hooks(&self, owner: &str, name: &str) -> Vec<String> {
        let data = self.inner.data.lock().unwrap();
        data.repos
            .get(&(owner.into(), name.into()))
            .map(|r| r.hooks.iter().map(|(url, _)| url.clone()).collect())
            .unwrap_or_default()
    }

    /// From now on, whether it refuses to make repositories.
    pub fn refuse_repositories(&self, refuse: bool) {
        self.inner
            .refuse_repositories
            .store(refuse, Ordering::SeqCst);
    }

    /// The bare repository's folder.
    pub fn repository_path(&self, owner: &str, name: &str) -> PathBuf {
        self.inner.root.join(owner).join(format!("{name}.git"))
    }

    /// The pull requests of a repository, in order.
    pub fn pulls(&self, owner: &str, name: &str) -> Vec<PullState> {
        let data = self.inner.data.lock().unwrap();
        data.repos
            .get(&(owner.into(), name.into()))
            .map(|r| {
                r.pulls
                    .iter()
                    .map(|p| PullState {
                        number: p.number,
                        head: p.head.clone(),
                        open: p.open,
                        merged: p.merged.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Commits to a new branch of a repository, from `main`: `write`
    /// files (path and text) and `delete` others, as someone could push.
    /// Returns the commit.
    pub async fn push_branch(
        &self,
        owner: &str,
        name: &str,
        branch: &str,
        write: &[(&str, &str)],
        delete: &[&str],
    ) -> String {
        let bare = self.repository_path(owner, name);
        let work = tempfile::tempdir().expect("a temporary folder");
        let target = bare.to_string_lossy().into_owned();
        let dir = work.path();
        run_git(dir, &["clone", "-q", "-b", "main", &target, "."])
            .await
            .expect("cloned");
        for (path, text) in write {
            let path = dir.join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a folder");
            }
            std::fs::write(path, text).expect("written");
        }
        for path in delete {
            std::fs::remove_file(dir.join(path)).expect("deleted");
        }
        run_git(dir, &["add", "-A"]).await.expect("added");
        run_git(dir, &["commit", "-q", "-m", "Work"])
            .await
            .expect("committed");
        let refspec = format!("HEAD:refs/heads/{branch}");
        run_git(dir, &["push", "-q", "--force", "origin", &refspec])
            .await
            .expect("pushed");
        run_git(dir, &["rev-parse", "HEAD"])
            .await
            .expect("a commit")
    }

    /// The text of a file on a branch, if it is there.
    pub async fn file(&self, owner: &str, name: &str, branch: &str, path: &str) -> Option<String> {
        let bare = self.repository_path(owner, name);
        run_git(&bare, &["show", &format!("{branch}:{path}")])
            .await
            .ok()
    }

    /// The commit a branch of a repository is at, if it is anywhere.
    pub async fn branch_commit(&self, owner: &str, name: &str, branch: &str) -> Option<String> {
        main_of(&self.repository_path(owner, name), branch).await
    }
}

fn router(inner: Arc<Inner>) -> Router {
    Router::new()
        .route("/api/v1/user", get(me))
        .route("/api/v1/admin/users", get(search_users).post(create_user))
        .route("/api/v1/admin/users/{login}/rename", post(rename_user))
        .route("/api/v1/orgs/{org}/rename", post(rename_org))
        .route("/api/v1/orgs", post(create_org))
        .route(
            "/api/v1/orgs/{org}",
            get(get_org).delete(delete_org).patch(get_org),
        )
        .route("/api/v1/orgs/{org}/teams", get(teams).post(create_team))
        .route("/api/v1/orgs/{org}/repos", post(create_repo))
        .route("/api/v1/teams/{id}/members", get(team_members))
        .route(
            "/api/v1/teams/{id}/members/{login}",
            put(add_member).delete(remove_member),
        )
        .route("/api/v1/repos/{owner}/{name}", delete(delete_repo))
        .route("/api/v1/repos/{owner}/{name}/contents", post(commit_files))
        .route(
            "/api/v1/repos/{owner}/{name}/branch_protections",
            post(protect),
        )
        .route("/api/v1/repos/{owner}/{name}/hooks", post(add_hook))
        .route(
            "/api/v1/repos/{owner}/{name}/branches/{*branch}",
            get(branch).delete(delete_branch),
        )
        .route("/api/v1/repos/{owner}/{name}/pulls", post(create_pull))
        .route(
            "/api/v1/repos/{owner}/{name}/pulls/{number}",
            get(get_pull).patch(update_pull),
        )
        .route(
            "/api/v1/repos/{owner}/{name}/pulls/{number}/files",
            get(pull_files),
        )
        .route(
            "/api/v1/repos/{owner}/{name}/pulls/{number}/merge",
            post(merge_pull),
        )
        .route("/api/v1/repos/{owner}/{name}/git/trees/{commit}", get(tree))
        .route("/api/v1/repos/{owner}/{name}/raw/{*path}", get(raw))
        .route("/{owner}/{repo}/{*path}", any(git))
        .layer(axum::middleware::from_fn(authenticate))
        .with_state(inner)
}

/// Refuses whatever does not come with the admin's token: as a token
/// header for the API, as the password of basic credentials for git.
async fn authenticate(request: Request, next: axum::middleware::Next) -> Response {
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let allowed = authorization == format!("token {TOKEN}")
        || authorization
            .strip_prefix("Basic ")
            .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|b| b.split_once(':').map(|(_, p)| p == TOKEN))
            .unwrap_or(false);
    if !allowed {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    next.run(request).await
}

fn message(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({"message": message}))).into_response()
}

#[derive(Deserialize)]
struct Page {
    page: Option<usize>,
    limit: Option<usize>,
}

impl Page {
    fn of<T: Clone>(&self, items: &[T]) -> Vec<T> {
        let limit = self.limit.unwrap_or(30).max(1);
        let page = self.page.unwrap_or(1).max(1);
        items
            .iter()
            .skip((page - 1) * limit)
            .take(limit)
            .cloned()
            .collect()
    }
}

async fn me() -> Response {
    axum::Json(json!({"id": 1, "login": ADMIN})).into_response()
}

#[derive(Deserialize)]
struct UserSearch {
    source_id: Option<i64>,
    login_name: Option<String>,
}

async fn search_users(State(inner): State<Arc<Inner>>, Query(q): Query<UserSearch>) -> Response {
    let data = inner.data.lock().unwrap();
    let found: Vec<Value> = data
        .users
        .iter()
        .filter(|u| q.source_id.is_none_or(|s| s == u.source_id))
        .filter(|u| q.login_name.as_ref().is_none_or(|l| *l == u.login_name))
        .map(User::json)
        .collect();
    axum::Json(found).into_response()
}

async fn create_user(
    State(inner): State<Arc<Inner>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    let login = body["username"].as_str().unwrap_or_default().to_owned();
    if login.is_empty() || body["email"].as_str().unwrap_or_default().is_empty() {
        return message(StatusCode::UNPROCESSABLE_ENTITY, "invalid");
    }
    if data.name_taken(&login) {
        return message(StatusCode::UNPROCESSABLE_ENTITY, "user already exists");
    }
    let id = data.id();
    let user = User {
        id,
        login,
        login_name: body["login_name"].as_str().unwrap_or_default().into(),
        source_id: body["source_id"].as_i64().unwrap_or_default(),
    };
    data.users.push(user.clone());
    (StatusCode::CREATED, axum::Json(user.json())).into_response()
}

async fn create_org(
    State(inner): State<Arc<Inner>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    let name = body["username"].as_str().unwrap_or_default().to_owned();
    if data.name_taken(&name) {
        return message(StatusCode::UNPROCESSABLE_ENTITY, "user already exists");
    }
    let id = data.id();
    data.orgs.insert(
        name.clone(),
        vec![Team {
            id,
            name: "Owners".into(),
            permission: "owner".into(),
            members: BTreeSet::from([ADMIN.to_owned()]),
        }],
    );
    (StatusCode::CREATED, axum::Json(json!({"username": name}))).into_response()
}

async fn get_org(State(inner): State<Arc<Inner>>, UrlPath(org): UrlPath<String>) -> Response {
    match inner.data.lock().unwrap().orgs.contains_key(&org) {
        true => axum::Json(json!({"username": org})).into_response(),
        false => message(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn delete_org(State(inner): State<Arc<Inner>>, UrlPath(org): UrlPath<String>) -> Response {
    let mut data = inner.data.lock().unwrap();
    if data.repos.keys().any(|(owner, _)| *owner == org) {
        return message(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the organization has repositories",
        );
    }
    match data.orgs.remove(&org) {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => message(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn teams(
    State(inner): State<Arc<Inner>>,
    UrlPath(org): UrlPath<String>,
    Query(page): Query<Page>,
) -> Response {
    let data = inner.data.lock().unwrap();
    let Some(teams) = data.orgs.get(&org) else {
        return message(StatusCode::NOT_FOUND, "not found");
    };
    let teams: Vec<Value> = page
        .of(teams)
        .iter()
        .map(|t| json!({"id": t.id, "name": t.name, "permission": t.permission}))
        .collect();
    axum::Json(teams).into_response()
}

async fn create_team(
    State(inner): State<Arc<Inner>>,
    UrlPath(org): UrlPath<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    let id = data.id();
    let Some(teams) = data.orgs.get_mut(&org) else {
        return message(StatusCode::NOT_FOUND, "not found");
    };
    let name = body["name"].as_str().unwrap_or_default().to_owned();
    if teams.iter().any(|t| t.name == name) {
        return message(StatusCode::UNPROCESSABLE_ENTITY, "team already exists");
    }
    if body["units_map"].as_object().is_none_or(|m| m.is_empty()) {
        return message(
            StatusCode::INTERNAL_SERVER_ERROR,
            "units permission should not be empty",
        );
    }
    teams.push(Team {
        id,
        name: name.clone(),
        permission: body["permission"].as_str().unwrap_or_default().into(),
        members: BTreeSet::new(),
    });
    (
        StatusCode::CREATED,
        axum::Json(json!({"id": id, "name": name})),
    )
        .into_response()
}

async fn team_members(
    State(inner): State<Arc<Inner>>,
    UrlPath(id): UrlPath<i64>,
    Query(page): Query<Page>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    let Some(team) = data.team(id) else {
        return message(StatusCode::NOT_FOUND, "not found");
    };
    let members: Vec<String> = team.members.iter().cloned().collect();
    let users: Vec<Value> = page
        .of(&members)
        .iter()
        .map(|login| json!({"id": 0, "login": login}))
        .collect();
    axum::Json(users).into_response()
}

async fn add_member(
    State(inner): State<Arc<Inner>>,
    UrlPath((id, login)): UrlPath<(i64, String)>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    if !data.users.iter().any(|u| u.login == login) {
        return message(StatusCode::NOT_FOUND, "no such user");
    }
    match data.team(id) {
        Some(team) => {
            team.members.insert(login);
            StatusCode::NO_CONTENT.into_response()
        }
        None => message(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn remove_member(
    State(inner): State<Arc<Inner>>,
    UrlPath((id, login)): UrlPath<(i64, String)>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    let removed = data
        .team(id)
        .is_some_and(|team| team.members.remove(&login));
    match removed {
        true => StatusCode::NO_CONTENT.into_response(),
        false => message(StatusCode::NOT_FOUND, "not found"),
    }
}

/// Runs git in `dir`, failing with what it wrote.
async fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Forgejo",
            "-c",
            "user.email=forgejo@localhost",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn create_repo(
    State(inner): State<Arc<Inner>>,
    UrlPath(org): UrlPath<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if inner.refuse_repositories.load(Ordering::SeqCst) {
        return message(
            StatusCode::INTERNAL_SERVER_ERROR,
            "refused, as the test asked",
        );
    }
    let name = body["name"].as_str().unwrap_or_default().to_owned();
    {
        let mut data = inner.data.lock().unwrap();
        if !data.orgs.contains_key(&org) {
            return message(StatusCode::NOT_FOUND, "not found");
        }
        let key = (org.clone(), name.clone());
        if data.repos.contains_key(&key) {
            return message(StatusCode::CONFLICT, "repository already exists");
        }
        data.repos.insert(key, Repo::default());
    }
    let dir = inner.root.join(&org);
    let _ = tokio::fs::create_dir_all(&dir).await;
    let path = format!("{name}.git");
    if let Err(error) = run_git(&dir, &["init", "-q", "--bare", "-b", "main", &path]).await {
        return message(StatusCode::INTERNAL_SERVER_ERROR, &error);
    }
    let repo = dir.join(&path);
    for (key, value) in [("http.receivepack", "true"), ("http.uploadpack", "true")] {
        let _ = run_git(&repo, &["config", key, value]).await;
    }
    (
        StatusCode::CREATED,
        axum::Json(json!({"name": name, "html_url": format!("/{org}/{name}")})),
    )
        .into_response()
}

async fn delete_repo(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name)): UrlPath<(String, String)>,
) -> Response {
    let removed = inner
        .data
        .lock()
        .unwrap()
        .repos
        .remove(&(owner.clone(), name.clone()));
    if removed.is_none() {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let _ = tokio::fs::remove_dir_all(inner.root.join(&owner).join(format!("{name}.git"))).await;
    StatusCode::NO_CONTENT.into_response()
}

fn repo_exists(inner: &Inner, owner: &str, name: &str) -> bool {
    inner
        .data
        .lock()
        .unwrap()
        .repos
        .contains_key(&(owner.into(), name.into()))
}

async fn commit_files(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    let Ok(work) = tempfile::tempdir() else {
        return message(StatusCode::INTERNAL_SERVER_ERROR, "no folder");
    };
    let made = async {
        run_git(work.path(), &["init", "-q", "-b", "main"]).await?;
        for file in body["files"].as_array().into_iter().flatten() {
            let path = work.path().join(file["path"].as_str().unwrap_or_default());
            let text = base64::engine::general_purpose::STANDARD
                .decode(file["content"].as_str().unwrap_or_default())
                .map_err(|e| e.to_string())?;
            if let Some(dir) = path.parent() {
                tokio::fs::create_dir_all(dir)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tokio::fs::write(path, text)
                .await
                .map_err(|e| e.to_string())?;
        }
        run_git(work.path(), &["add", "-A"]).await?;
        let message = body["message"].as_str().unwrap_or("Files");
        run_git(work.path(), &["commit", "-q", "-m", message]).await?;
        let target = bare.to_string_lossy().into_owned();
        run_git(work.path(), &["push", "-q", &target, "main"]).await
    }
    .await;
    match made {
        Ok(_) => (StatusCode::CREATED, axum::Json(json!({}))).into_response(),
        Err(error) => message(StatusCode::INTERNAL_SERVER_ERROR, &error),
    }
}

/// What keeps a protected branch from being rewritten or deleted: a hook,
/// as the rule's branch is in `$BRANCH`.
const PROTECT: &str = r#"#!/bin/sh
zero=0000000000000000000000000000000000000000
while read old new ref; do
  [ "$ref" = "refs/heads/$BRANCH" ] || continue
  if [ "$new" = "$zero" ]; then echo "$BRANCH is protected" >&2; exit 1; fi
  [ "$old" = "$zero" ] && continue
  git merge-base --is-ancestor "$old" "$new" || { echo "$BRANCH is protected" >&2; exit 1; }
done
"#;

async fn protect(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    let branch = body["rule_name"].as_str().unwrap_or("main");
    let hook = bare.join("hooks").join("pre-receive");
    let script = PROTECT.replace("$BRANCH", branch);
    let written = async {
        tokio::fs::create_dir_all(bare.join("hooks")).await?;
        tokio::fs::write(&hook, script).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).await?;
        }
        std::io::Result::Ok(())
    }
    .await;
    if let Err(error) = written {
        return message(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    (StatusCode::CREATED, axum::Json(json!({}))).into_response()
}

/// The files of a commit, all on one page.
async fn tree(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, commit)): UrlPath<(String, String, String)>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    let Ok(listed) = run_git(&bare, &["ls-tree", "-r", "-l", &commit]).await else {
        return message(StatusCode::NOT_FOUND, "commit not found");
    };
    let entries: Vec<Value> = listed
        .lines()
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut meta = meta.split_whitespace();
            let (_, kind, _, size) = (meta.next()?, meta.next()?, meta.next()?, meta.next()?);
            Some(json!({"path": path, "type": kind, "size": size.parse::<u64>().unwrap_or(0)}))
        })
        .collect();
    axum::Json(json!({"sha": commit, "tree": entries, "truncated": false})).into_response()
}

#[derive(Deserialize)]
struct Ref {
    #[serde(rename = "ref")]
    reference: Option<String>,
}

/// A file at a commit, as it is.
async fn raw(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, path)): UrlPath<(String, String, String)>,
    Query(at): Query<Ref>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    let reference = at.reference.unwrap_or_else(|| "main".into());
    let output = Command::new("git")
        .current_dir(&bare)
        .args(["show", &format!("{reference}:{path}")])
        .output()
        .await;
    match output {
        Ok(output) if output.status.success() => output.stdout.into_response(),
        _ => message(StatusCode::NOT_FOUND, "file not found"),
    }
}

async fn add_hook(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut data = inner.data.lock().unwrap();
    let Some(repo) = data.repos.get_mut(&(owner, name)) else {
        return message(StatusCode::NOT_FOUND, "not found");
    };
    let url = body["config"]["url"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let secret = body["config"]["secret"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    repo.hooks.push((url, secret));
    (StatusCode::CREATED, axum::Json(json!({}))).into_response()
}

async fn main_of(bare: &Path, branch: &str) -> Option<String> {
    run_git(
        bare,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .ok()
    .filter(|c| !c.is_empty())
}

async fn branch(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, branch)): UrlPath<(String, String, String)>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    match main_of(&bare, &branch).await {
        Some(commit) => {
            axum::Json(json!({"name": branch, "commit": {"id": commit}})).into_response()
        }
        None => message(StatusCode::NOT_FOUND, "branch not found"),
    }
}

async fn delete_branch(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, branch)): UrlPath<(String, String, String)>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) || branch == "main" {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    if main_of(&bare, &branch).await.is_none() {
        return message(StatusCode::NOT_FOUND, "branch not found");
    }
    let reference = format!("refs/heads/{branch}");
    match run_git(&bare, &["update-ref", "-d", &reference]).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => message(StatusCode::INTERNAL_SERVER_ERROR, &error),
    }
}

fn find_pull<T>(
    inner: &Inner,
    owner: &str,
    name: &str,
    number: u32,
    then: impl FnOnce(&mut Pull) -> T,
) -> Option<T> {
    let mut data = inner.data.lock().unwrap();
    let repo = data.repos.get_mut(&(owner.into(), name.into()))?;
    repo.pulls.iter_mut().find(|p| p.number == number).map(then)
}

async fn create_pull(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name)): UrlPath<(String, String)>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    if !repo_exists(&inner, &owner, &name) {
        return message(StatusCode::NOT_FOUND, "not found");
    }
    let head = body["head"].as_str().unwrap_or_default().to_owned();
    let base = body["base"].as_str().unwrap_or("main").to_owned();
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    if main_of(&bare, &head).await.is_none() || main_of(&bare, &base).await.is_none() {
        return message(StatusCode::NOT_FOUND, "the branch does not exist");
    }
    let mut data = inner.data.lock().unwrap();
    let repo = data
        .repos
        .get_mut(&(owner.clone(), name.clone()))
        .expect("checked");
    if repo.pulls.iter().any(|p| p.open && p.head == head) {
        return message(
            StatusCode::CONFLICT,
            "a pull request of this branch is already open",
        );
    }
    let pull = Pull {
        number: repo.pulls.len() as u32 + 1,
        head,
        base,
        title: body["title"].as_str().unwrap_or_default().to_owned(),
        open: true,
        merged: None,
    };
    repo.pulls.push(pull.clone());
    (StatusCode::CREATED, axum::Json(pull.json(&owner, &name))).into_response()
}

async fn get_pull(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, number)): UrlPath<(String, String, u32)>,
) -> Response {
    match find_pull(&inner, &owner, &name, number, |p| p.clone()) {
        Some(pull) => axum::Json(pull.json(&owner, &name)).into_response(),
        None => message(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn update_pull(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, number)): UrlPath<(String, String, u32)>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let closing = body["state"] == "closed";
    let updated = find_pull(&inner, &owner, &name, number, |p| {
        if closing && p.merged.is_none() {
            p.open = false;
        }
        p.clone()
    });
    match updated {
        Some(pull) => (StatusCode::CREATED, axum::Json(pull.json(&owner, &name))).into_response(),
        None => message(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn pull_files(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, number)): UrlPath<(String, String, u32)>,
    Query(page): Query<Page>,
) -> Response {
    let Some(pull) = find_pull(&inner, &owner, &name, number, |p| p.clone()) else {
        return message(StatusCode::NOT_FOUND, "not found");
    };
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    let range = format!("refs/heads/{}...refs/heads/{}", pull.base, pull.head);
    let listed = match run_git(&bare, &["diff", "--name-status", "-M", &range]).await {
        Ok(listed) => listed,
        Err(error) => return message(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    let files: Vec<Value> = listed
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let status = fields.next()?;
            let first = fields.next()?;
            Some(match status.chars().next()? {
                'A' => json!({"filename": first, "status": "added"}),
                'D' => json!({"filename": first, "status": "deleted"}),
                'R' => json!({"filename": fields.next()?, "previous_filename": first,
                              "status": "renamed"}),
                _ => json!({"filename": first, "status": "modified"}),
            })
        })
        .collect();
    axum::Json(page.of(&files)).into_response()
}

async fn merge_pull(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, name, number)): UrlPath<(String, String, u32)>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let Some(pull) = find_pull(&inner, &owner, &name, number, |p| p.clone()) else {
        return message(StatusCode::NOT_FOUND, "not found");
    };
    if !pull.open {
        return message(StatusCode::METHOD_NOT_ALLOWED, "the pull request is closed");
    }
    let bare = inner.root.join(&owner).join(format!("{name}.git"));
    let before = main_of(&bare, &pull.base).await;
    let Ok(work) = tempfile::tempdir() else {
        return message(StatusCode::INTERNAL_SERVER_ERROR, "no folder");
    };
    let text = body["merge_message_field"]
        .as_str()
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Merge pull request #{number}"));
    let target = bare.to_string_lossy().into_owned();
    let merged = async {
        run_git(
            work.path(),
            &["clone", "-q", "-b", &pull.base, &target, "."],
        )
        .await?;
        let head = format!("origin/{}", pull.head);
        if let Err(error) =
            run_git(work.path(), &["merge", "--no-ff", "-q", "-m", &text, &head]).await
        {
            return Err(format!("merge conflict: {error}"));
        }
        let branch = format!("HEAD:refs/heads/{}", pull.base);
        run_git(work.path(), &["push", "-q", "origin", &branch]).await?;
        run_git(work.path(), &["rev-parse", "HEAD"]).await
    }
    .await;
    let commit = match merged {
        Ok(commit) => commit,
        Err(error) if error.starts_with("merge conflict") => {
            return message(StatusCode::CONFLICT, &error);
        }
        Err(error) => return message(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    find_pull(&inner, &owner, &name, number, |p| {
        p.open = false;
        p.merged = Some(commit.clone());
    });
    if body["delete_branch_after_merge"] == true {
        let reference = format!("refs/heads/{}", pull.head);
        let _ = run_git(&bare, &["update-ref", "-d", &reference]).await;
    }
    if pull.base == "main" {
        notify(&inner, &owner, &name, before, Some(commit)).await;
    }
    StatusCode::OK.into_response()
}

/// Git's smart HTTP protocol, through `git http-backend`; after a push,
/// the repository's webhooks are told.
async fn git(
    State(inner): State<Arc<Inner>>,
    UrlPath((owner, repo, path)): UrlPath<(String, String, String)>,
    request: Request,
) -> Response {
    let Some(name) = repo.strip_suffix(".git").map(str::to_owned) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !repo_exists(&inner, &owner, &name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let pushing = path == "git-receive-pack";
    let bare = inner.root.join(&owner).join(&repo);
    let before = main_of(&bare, "main").await;
    match backend(&inner.root, &owner, &repo, &path, request).await {
        Ok((response, child)) => {
            if pushing {
                let inner = inner.clone();
                tokio::spawn(async move {
                    let mut child = child;
                    let _ = child.wait().await;
                    let after = main_of(&bare, "main").await;
                    if after != before {
                        notify(&inner, &owner, &name, before, after).await;
                    }
                });
            }
            response
        }
        Err(error) => message(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

/// Tells the repository's webhooks that `main` moved, signed as Forgejo
/// signs them.
async fn notify(
    inner: &Inner,
    owner: &str,
    name: &str,
    before: Option<String>,
    after: Option<String>,
) {
    let hooks = inner
        .data
        .lock()
        .unwrap()
        .repos
        .get(&(owner.into(), name.into()))
        .map(|r| r.hooks.clone())
        .unwrap_or_default();
    let zero = "0".repeat(40);
    let body = json!({
        "ref": "refs/heads/main",
        "before": before.unwrap_or_else(|| zero.clone()),
        "after": after.unwrap_or(zero),
        "repository": {"name": name, "owner": {"login": owner}},
        "pusher": {"login": ADMIN},
    })
    .to_string();
    for (url, secret) in hooks {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("any key");
        mac.update(body.as_bytes());
        let signature: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let _ = inner
            .http
            .post(url)
            .header("content-type", "application/json")
            .header("x-forgejo-event", "push")
            .header("x-forgejo-signature", signature)
            .body(body.clone())
            .send()
            .await;
    }
}

/// Runs `git http-backend` for one request, streaming both ways.
async fn backend(
    root: &Path,
    owner: &str,
    repo: &str,
    path: &str,
    request: Request,
) -> std::io::Result<(Response, tokio::process::Child)> {
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
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("PATH_INFO", format!("/{owner}/{repo}/{path}"))
        .env("REQUEST_METHOD", request.method().as_str())
        .env("QUERY_STRING", request.uri().query().unwrap_or_default())
        .env("CONTENT_TYPE", header_value(header::CONTENT_TYPE))
        .env("REMOTE_USER", ADMIN)
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("GIT_COMMITTER_NAME", ADMIN)
        .env("GIT_COMMITTER_EMAIL", "forge-admin@localhost")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let encoding = header_value(header::CONTENT_ENCODING);
    if !encoding.is_empty() {
        command.env("HTTP_CONTENT_ENCODING", encoding);
    }
    if let Some(protocol) = headers.get("git-protocol").and_then(|v| v.to_str().ok()) {
        command.env("GIT_PROTOCOL", protocol);
    }
    let mut child = command.spawn()?;

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
    let mut response = Response::new(Body::from_stream(ReaderStream::new(stdout)));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;
    Ok((response, child))
}

async fn rename_user(
    State(inner): State<Arc<Inner>>,
    UrlPath(old): UrlPath<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let new = body["new_username"].as_str().unwrap_or_default();
    let mut data = inner.data.lock().unwrap();
    if data.name_taken(new) {
        return message(StatusCode::UNPROCESSABLE_ENTITY, "name taken");
    }
    let Some(user) = data.users.iter_mut().find(|u| u.login == old) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    user.login = new.into();
    for teams in data.orgs.values_mut() {
        for team in teams {
            if team.members.remove(&old) {
                team.members.insert(new.into());
            }
        }
    }
    StatusCode::NO_CONTENT.into_response()
}
async fn rename_org(
    State(inner): State<Arc<Inner>>,
    UrlPath(old): UrlPath<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let new = body["new_name"].as_str().unwrap_or_default();
    let mut data = inner.data.lock().unwrap();
    if data.name_taken(new) {
        return message(StatusCode::UNPROCESSABLE_ENTITY, "name taken");
    }
    let Some(teams) = data.orgs.remove(&old) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if inner.root.join(&old).exists() {
        std::fs::rename(inner.root.join(&old), inner.root.join(new)).unwrap();
    }
    data.orgs.insert(new.into(), teams);
    let repositories: Vec<_> = data
        .repos
        .keys()
        .filter(|(owner, _)| owner == &old)
        .cloned()
        .collect();
    for key in repositories {
        let repo = data.repos.remove(&key).unwrap();
        data.repos.insert((new.into(), key.1), repo);
    }
    StatusCode::NO_CONTENT.into_response()
}
