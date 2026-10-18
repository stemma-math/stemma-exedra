//! Forgejo's API, as the hub uses it: with an admin's token, to make
//! accounts, organizations, their teams and repositories, and to look at
//! branches.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::ForgeConfig;

/// What Forgejo answered, when it was not what the hub asked for.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("Forgejo could not be reached: {0}")]
    Unreachable(#[from] reqwest::Error),
    #[error("Forgejo refused to {what} ({status}): {message}")]
    Refused {
        what: String,
        status: StatusCode,
        message: String,
    },
}

impl ForgeError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            ForgeError::Refused { status, .. } => Some(*status),
            ForgeError::Unreachable(_) => None,
        }
    }

    /// Whether what was to be made is there already.
    pub fn is_conflict(&self) -> bool {
        matches!(
            self.status(),
            Some(StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY)
        )
    }
}

pub type ForgeResult<T> = Result<T, ForgeError>;

/// An account in Forgejo.
#[derive(Clone, Debug, Deserialize)]
pub struct User {
    pub id: i64,
    pub login: String,
    #[serde(default)]
    pub login_name: String,
    #[serde(default)]
    pub source_id: i64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Team {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Repository {
    pub name: String,
    pub html_url: String,
}

/// A new account, linked to an authentication source: signing in through
/// it, as `login_name` there, lands on this account.
#[derive(Debug, Serialize)]
pub struct NewUser<'a> {
    pub username: &'a str,
    pub full_name: &'a str,
    pub email: &'a str,
    /// Random and never told: the account signs in through its source.
    pub password: &'a str,
    pub must_change_password: bool,
    pub send_notify: bool,
    pub source_id: i64,
    pub login_name: &'a str,
    pub visibility: &'a str,
}

/// What a team may do in every repository of its organization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    Read,
    Write,
}

impl Permission {
    fn as_str(self) -> &'static str {
        match self {
            Permission::Read => "read",
            Permission::Write => "write",
        }
    }
}

/// The parts of a repository a team's permission covers.
const UNITS: &[&str] = &[
    "repo.code",
    "repo.issues",
    "repo.pulls",
    "repo.releases",
    "repo.wiki",
    "repo.projects",
    "repo.packages",
    "repo.actions",
];

/// A client of one Forgejo, acting as its admin.
pub struct Forge {
    http: reqwest::Client,
    base: String,
    public: String,
    token: String,
    source_id: i64,
    admin: tokio::sync::OnceCell<String>,
}

impl Forge {
    pub fn new(config: &ForgeConfig) -> anyhow::Result<Forge> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .user_agent(concat!("stemma/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            // Git's answers are streamed for as long as they take; the API's
            // are bounded per request.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Forge {
            http,
            base: config.base(),
            public: config.public_base(),
            token: config.secrets.token.clone(),
            source_id: config.auth_source_id,
            admin: tokio::sync::OnceCell::new(),
        })
    }

    /// Where people reach an organization or repository: `path` is
    /// `geometry` or `geometry/library`.
    pub fn public_url(&self, path: &str) -> String {
        format!("{}/{path}", self.public)
    }

    /// The authentication source the hub's accounts are linked to.
    pub fn source_id(&self) -> i64 {
        self.source_id
    }

    fn api(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}/api/v1{path}", self.base))
            .header("authorization", format!("token {}", self.token))
            .timeout(Duration::from_secs(60))
    }

    /// A request for a repository's git, as Forgejo's admin: `path` is
    /// `owner/name.git/info/refs` and the like.
    pub fn git(&self, method: Method, path: &str, query: Option<&str>) -> RequestBuilder {
        let mut url = format!("{}/{path}", self.base);
        if let Some(query) = query.filter(|q| !q.is_empty()) {
            url.push('?');
            url.push_str(query);
        }
        self.http
            .request(method, url)
            .basic_auth("stemma", Some(&self.token))
    }

    async fn send(&self, what: &str, request: RequestBuilder) -> ForgeResult<reqwest::Response> {
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let text = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_owned))
            .unwrap_or(text);
        Err(ForgeError::Refused {
            what: what.to_owned(),
            status,
            message: message.chars().take(500).collect(),
        })
    }

    async fn json<T: DeserializeOwned>(
        &self,
        what: &str,
        request: RequestBuilder,
    ) -> ForgeResult<T> {
        Ok(self.send(what, request).await?.json().await?)
    }

    /// `what`, or nothing if Forgejo says it is not there.
    async fn optional<T: DeserializeOwned>(
        &self,
        what: &str,
        request: RequestBuilder,
    ) -> ForgeResult<Option<T>> {
        match self.json(what, request).await {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.status() == Some(StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Every page of a list.
    async fn all<T: DeserializeOwned>(&self, what: &str, path: &str) -> ForgeResult<Vec<T>> {
        const LIMIT: usize = 50;
        let mut items = Vec::new();
        for page in 1.. {
            let batch: Vec<T> = self
                .json(
                    what,
                    self.api(Method::GET, path)
                        .query(&[("page", page), ("limit", LIMIT)]),
                )
                .await?;
            let last = batch.len() < LIMIT;
            items.extend(batch);
            if last {
                break;
            }
        }
        Ok(items)
    }

    /// The login of the admin the hub acts as.
    pub async fn admin_login(&self) -> ForgeResult<String> {
        self.admin
            .get_or_try_init(|| async {
                let me: User = self
                    .json("say who the hub is", self.api(Method::GET, "/user"))
                    .await?;
                Ok(me.login)
            })
            .await
            .cloned()
    }

    // ── Accounts ──────────────────────────────────────────────────────

    /// The account linked to the hub's source as `login_name`, if any.
    pub async fn linked_user(&self, login_name: &str) -> ForgeResult<Option<User>> {
        let users: Vec<User> = self
            .json(
                "search its accounts",
                self.api(Method::GET, "/admin/users").query(&[
                    ("source_id", self.source_id.to_string()),
                    ("login_name", login_name.to_owned()),
                ]),
            )
            .await?;
        // Filtered by Forgejo already; checked again all the same.
        Ok(users
            .into_iter()
            .find(|u| u.source_id == self.source_id && u.login_name == login_name))
    }

    pub async fn create_user(&self, user: &NewUser<'_>) -> ForgeResult<User> {
        self.json(
            &format!("create the account {}", user.username),
            self.api(Method::POST, "/admin/users").json(user),
        )
        .await
    }

    // ── Organizations and teams ───────────────────────────────────────

    pub async fn organization_exists(&self, name: &str) -> ForgeResult<bool> {
        let found: Option<Value> = self
            .optional(
                &format!("find the organization {name}"),
                self.api(Method::GET, &format!("/orgs/{name}")),
            )
            .await?;
        Ok(found.is_some())
    }

    /// A private organization, owned by the hub's admin.
    pub async fn create_organization(&self, name: &str, full_name: &str) -> ForgeResult<()> {
        self.send(
            &format!("create the organization {name}"),
            self.api(Method::POST, "/orgs").json(&json!({
                "username": name,
                "full_name": full_name,
                "visibility": "private",
                "repo_admin_change_team_access": false,
            })),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_organization(&self, name: &str) -> ForgeResult<()> {
        self.send(
            &format!("delete the organization {name}"),
            self.api(Method::DELETE, &format!("/orgs/{name}")),
        )
        .await?;
        Ok(())
    }

    pub async fn teams(&self, org: &str) -> ForgeResult<Vec<Team>> {
        self.all(
            &format!("list the teams of {org}"),
            &format!("/orgs/{org}/teams"),
        )
        .await
    }

    /// A team with `permission` on every repository of `org`, those to
    /// come too.
    pub async fn create_team(
        &self,
        org: &str,
        name: &str,
        description: &str,
        permission: Permission,
    ) -> ForgeResult<Team> {
        let units_map: serde_json::Map<String, Value> = UNITS
            .iter()
            .map(|unit| ((*unit).to_owned(), json!(permission.as_str())))
            .collect();
        self.json(
            &format!("create the team {name} of {org}"),
            self.api(Method::POST, &format!("/orgs/{org}/teams"))
                .json(&json!({
                    "name": name,
                    "description": description,
                    "permission": permission.as_str(),
                    "includes_all_repositories": true,
                    "can_create_org_repo": false,
                    "units": UNITS,
                    "units_map": units_map,
                })),
        )
        .await
    }

    /// The logins of a team's members.
    pub async fn team_members(&self, team: i64) -> ForgeResult<Vec<String>> {
        let users: Vec<User> = self
            .all("list a team's members", &format!("/teams/{team}/members"))
            .await?;
        Ok(users.into_iter().map(|u| u.login).collect())
    }

    pub async fn add_team_member(&self, team: i64, login: &str) -> ForgeResult<()> {
        self.send(
            &format!("add {login} to a team"),
            self.api(Method::PUT, &format!("/teams/{team}/members/{login}")),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_team_member(&self, team: i64, login: &str) -> ForgeResult<()> {
        match self
            .send(
                &format!("remove {login} from a team"),
                self.api(Method::DELETE, &format!("/teams/{team}/members/{login}")),
            )
            .await
        {
            Err(error) if error.status() == Some(StatusCode::NOT_FOUND) => Ok(()),
            done => done.map(|_| ()),
        }
    }

    // ── Repositories ──────────────────────────────────────────────────

    /// An empty private repository of `org`, whose branch is `main`.
    pub async fn create_repository(
        &self,
        org: &str,
        name: &str,
        description: &str,
    ) -> ForgeResult<Repository> {
        self.json(
            &format!("create the repository {org}/{name}"),
            self.api(Method::POST, &format!("/orgs/{org}/repos"))
                .json(&json!({
                    "name": name,
                    "description": description,
                    "private": true,
                    "auto_init": false,
                    "default_branch": "main",
                })),
        )
        .await
    }

    pub async fn delete_repository(&self, owner: &str, name: &str) -> ForgeResult<()> {
        self.send(
            &format!("delete the repository {owner}/{name}"),
            self.api(Method::DELETE, &format!("/repos/{owner}/{name}")),
        )
        .await?;
        Ok(())
    }

    /// Commits `files` (path and text) to `branch`, as Stemma.
    pub async fn commit_files(
        &self,
        owner: &str,
        name: &str,
        branch: &str,
        message: &str,
        files: &[(String, String)],
    ) -> ForgeResult<()> {
        let files: Vec<Value> = files
            .iter()
            .map(|(path, text)| {
                json!({
                    "operation": "create",
                    "path": path,
                    "content": STANDARD.encode(text),
                })
            })
            .collect();
        let signature = json!({"name": "Stemma", "email": "stemma@stemma.invalid"});
        self.send(
            &format!("commit to {owner}/{name}"),
            self.api(Method::POST, &format!("/repos/{owner}/{name}/contents"))
                .json(&json!({
                    "branch": branch,
                    "message": message,
                    "files": files,
                    "author": signature,
                    "committer": signature,
                })),
        )
        .await?;
        Ok(())
    }

    /// Protects `branch`: whoever may write pushes to it, but nobody
    /// rewrites its history or deletes it.
    pub async fn protect_branch(&self, owner: &str, name: &str, branch: &str) -> ForgeResult<()> {
        self.send(
            &format!("protect {branch} of {owner}/{name}"),
            self.api(
                Method::POST,
                &format!("/repos/{owner}/{name}/branch_protections"),
            )
            .json(&json!({
                "rule_name": branch,
                "enable_push": true,
                "enable_push_whitelist": false,
            })),
        )
        .await?;
        Ok(())
    }

    /// A webhook that tells `url` of each push to `branch`, signed with
    /// `secret`.
    pub async fn add_push_hook(
        &self,
        owner: &str,
        name: &str,
        branch: &str,
        url: &str,
        secret: &str,
    ) -> ForgeResult<()> {
        self.send(
            &format!("add a webhook to {owner}/{name}"),
            self.api(Method::POST, &format!("/repos/{owner}/{name}/hooks"))
                .json(&json!({
                    "type": "forgejo",
                    "active": true,
                    "events": ["push"],
                    "branch_filter": branch,
                    "config": {
                        "url": url,
                        "content_type": "json",
                        "secret": secret,
                    },
                })),
        )
        .await?;
        Ok(())
    }

    /// The commit `branch` is at, if the repository and the branch exist.
    pub async fn branch_commit(
        &self,
        owner: &str,
        name: &str,
        branch: &str,
    ) -> ForgeResult<Option<String>> {
        let found: Option<Value> = self
            .optional(
                &format!("find {branch} of {owner}/{name}"),
                self.api(
                    Method::GET,
                    &format!("/repos/{owner}/{name}/branches/{branch}"),
                ),
            )
            .await?;
        Ok(found.and_then(|b| b["commit"]["id"].as_str().map(str::to_owned)))
    }

    /// The files of a repository at `commit`, with their sizes, in order;
    /// none if the repository or the commit is not there.
    pub async fn tree(
        &self,
        owner: &str,
        name: &str,
        commit: &str,
    ) -> ForgeResult<Option<Vec<TreeEntry>>> {
        let mut files = Vec::new();
        for page in 1.. {
            let found: Option<Tree> = self
                .optional(
                    &format!("list the files of {owner}/{name}"),
                    self.api(
                        Method::GET,
                        &format!("/repos/{owner}/{name}/git/trees/{commit}"),
                    )
                    .query(&[("recursive", "true")])
                    .query(&[("page", page), ("per_page", 1000)]),
                )
                .await?;
            let Some(tree) = found else {
                return Ok(None);
            };
            let last = !tree.truncated || tree.tree.is_empty();
            files.extend(tree.tree.into_iter().filter(|e| e.kind == "blob"));
            if last {
                break;
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Some(files))
    }

    /// A file of a repository at `commit`, if it is there.
    pub async fn raw_file(
        &self,
        owner: &str,
        name: &str,
        commit: &str,
        path: &str,
    ) -> ForgeResult<Option<Vec<u8>>> {
        let encoded: Vec<String> = path.split('/').map(encode_segment).collect();
        let request = self
            .api(
                Method::GET,
                &format!("/repos/{owner}/{name}/raw/{}", encoded.join("/")),
            )
            .query(&[("ref", commit)]);
        match self
            .send(&format!("read {path} of {owner}/{name}"), request)
            .await
        {
            Ok(response) => Ok(Some(response.bytes().await?.to_vec())),
            Err(error) if error.status() == Some(StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn delete_branch(&self, owner: &str, name: &str, branch: &str) -> ForgeResult<()> {
        match self
            .send(
                &format!("delete {branch} of {owner}/{name}"),
                self.api(
                    Method::DELETE,
                    &format!("/repos/{owner}/{name}/branches/{branch}"),
                ),
            )
            .await
        {
            Err(error) if error.status() == Some(StatusCode::NOT_FOUND) => Ok(()),
            done => done.map(|_| ()),
        }
    }

    // ── Pull requests ─────────────────────────────────────────────────

    /// A pull request of `head` into `base`.
    pub async fn create_pull(
        &self,
        owner: &str,
        name: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> ForgeResult<Pull> {
        self.json(
            &format!("open a pull request of {head} in {owner}/{name}"),
            self.api(Method::POST, &format!("/repos/{owner}/{name}/pulls"))
                .json(&json!({"head": head, "base": base, "title": title, "body": body})),
        )
        .await
    }

    pub async fn pull(&self, owner: &str, name: &str, number: u32) -> ForgeResult<Pull> {
        self.json(
            &format!("find pull request {number} of {owner}/{name}"),
            self.api(
                Method::GET,
                &format!("/repos/{owner}/{name}/pulls/{number}"),
            ),
        )
        .await
    }

    /// The files a pull request changes.
    pub async fn pull_files(
        &self,
        owner: &str,
        name: &str,
        number: u32,
    ) -> ForgeResult<Vec<ChangedFile>> {
        self.all(
            &format!("list the files of pull request {number} of {owner}/{name}"),
            &format!("/repos/{owner}/{name}/pulls/{number}/files"),
        )
        .await
    }

    /// Merges a pull request, with a merge commit, and deletes its branch.
    pub async fn merge_pull(
        &self,
        owner: &str,
        name: &str,
        number: u32,
        message: &str,
    ) -> ForgeResult<()> {
        self.send(
            &format!("merge pull request {number} of {owner}/{name}"),
            self.api(
                Method::POST,
                &format!("/repos/{owner}/{name}/pulls/{number}/merge"),
            )
            .json(&json!({
                "Do": "merge",
                "merge_message_field": message,
                "delete_branch_after_merge": true,
            })),
        )
        .await?;
        Ok(())
    }

    pub async fn close_pull(&self, owner: &str, name: &str, number: u32) -> ForgeResult<()> {
        self.send(
            &format!("close pull request {number} of {owner}/{name}"),
            self.api(
                Method::PATCH,
                &format!("/repos/{owner}/{name}/pulls/{number}"),
            )
            .json(&json!({"state": "closed"})),
        )
        .await?;
        Ok(())
    }
}

/// A page of a repository's tree, as Forgejo lists it.
#[derive(Clone, Debug, Deserialize)]
struct Tree {
    #[serde(default)]
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

/// A file, or a folder, of a repository's tree.
#[derive(Clone, Debug, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    /// `blob` for a file, `tree` for a folder.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: u64,
}

/// Percent-encodes a segment of a URL's path.
fn encode_segment(segment: &str) -> String {
    segment
        .bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A pull request, as the hub reads it.
#[derive(Clone, Debug, Deserialize)]
pub struct Pull {
    pub number: u32,
    pub html_url: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub merge_commit_sha: Option<String>,
}

/// A file a pull request changes.
#[derive(Clone, Debug, Deserialize)]
pub struct ChangedFile {
    pub filename: String,
    #[serde(default)]
    pub previous_filename: Option<String>,
    /// `added`, `modified`, `deleted`, `renamed`…
    pub status: String,
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::ForgeSecrets;

    fn forge(server: &MockServer) -> Forge {
        let mut config = ForgeConfig::new(
            &server.uri(),
            "https://forge.example.org/user/oauth2/stemma/callback",
            ForgeSecrets {
                token: "admin-token".into(),
                ..Default::default()
            },
        );
        config.public_url = Some("https://forge.example.org/".into());
        config.auth_source_id = 3;
        Forge::new(&config).unwrap()
    }

    #[tokio::test]
    async fn acts_as_the_admin_and_remembers_who_it_is() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user"))
            .and(header("authorization", "token admin-token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": 1, "login": "root"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let forge = forge(&server);
        assert_eq!(forge.admin_login().await.unwrap(), "root");
        assert_eq!(forge.admin_login().await.unwrap(), "root");
        assert_eq!(
            forge.public_url("geometry/library"),
            "https://forge.example.org/geometry/library"
        );
    }

    #[tokio::test]
    async fn finds_and_creates_accounts_linked_to_the_source() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users"))
            .and(query_param("source_id", "3"))
            .and(query_param("login_name", "7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": 9, "login": "ada", "login_name": "7", "source_id": 3}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/users"))
            .and(query_param("login_name", "8"))
            // Not linked as asked: not the one.
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": 10, "login": "bob", "login_name": "", "source_id": 0}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/admin/users"))
            .and(body_partial_json(json!({
                "username": "bob", "source_id": 3, "login_name": "8",
                "must_change_password": false, "send_notify": false,
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": 11, "login": "bob", "login_name": "8", "source_id": 3
            })))
            .expect(1)
            .mount(&server)
            .await;
        let forge = forge(&server);
        assert_eq!(forge.linked_user("7").await.unwrap().unwrap().login, "ada");
        assert!(forge.linked_user("8").await.unwrap().is_none());
        let user = forge
            .create_user(&NewUser {
                username: "bob",
                full_name: "Bob",
                email: "bob@stemma.invalid",
                password: "random",
                must_change_password: false,
                send_notify: false,
                source_id: 3,
                login_name: "8",
                visibility: "private",
            })
            .await
            .unwrap();
        assert_eq!(user.id, 11);
    }

    #[tokio::test]
    async fn makes_teams_that_cover_every_repository() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/orgs/geometry/teams"))
            .and(body_partial_json(json!({
                "name": "Guests",
                "permission": "read",
                "includes_all_repositories": true,
                "units_map": {"repo.code": "read"},
            })))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"id": 4, "name": "Guests"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let team = forge(&server)
            .create_team("geometry", "Guests", "Guests", Permission::Read)
            .await
            .unwrap();
        assert_eq!(team.id, 4);
    }

    #[tokio::test]
    async fn reads_every_page_of_a_list() {
        let server = MockServer::start().await;
        let first: Vec<Value> = (0..50)
            .map(|i| json!({"id": i, "login": format!("u{i}")}))
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/v1/teams/2/members"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/teams/2/members"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([{"id": 50, "login": "last"}])),
            )
            .mount(&server)
            .await;
        let members = forge(&server).team_members(2).await.unwrap();
        assert_eq!(members.len(), 51);
        assert_eq!(members[50], "last");
    }

    #[tokio::test]
    async fn commits_files_protects_main_and_hooks_pushes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/repos/geometry/library/contents"))
            .and(body_partial_json(json!({
                "branch": "main",
                "files": [{"operation": "create", "path": "README.md", "content": "IyBHZW9tZXRyeQ=="}],
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/repos/geometry/library/branch_protections"))
            .and(body_partial_json(
                json!({"rule_name": "main", "enable_push": true}),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/repos/geometry/library/hooks"))
            .and(body_partial_json(json!({
                "events": ["push"],
                "config": {"url": "http://hub/forge/hook", "content_type": "json", "secret": "s"},
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        let forge = forge(&server);
        forge
            .commit_files(
                "geometry",
                "library",
                "main",
                "Start",
                &[("README.md".into(), "# Geometry".into())],
            )
            .await
            .unwrap();
        forge
            .protect_branch("geometry", "library", "main")
            .await
            .unwrap();
        forge
            .add_push_hook("geometry", "library", "main", "http://hub/forge/hook", "s")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn tells_missing_from_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/repos/geometry/library/branches/main"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"commit": {"id": "abc123"}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/orgs/algebra"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "not found"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/orgs"))
            .respond_with(
                ResponseTemplate::new(422).set_body_json(json!({"message": "user already exists"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/teams/2/members/ada"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let forge = forge(&server);
        assert_eq!(
            forge
                .branch_commit("geometry", "library", "main")
                .await
                .unwrap()
                .as_deref(),
            Some("abc123")
        );
        assert!(!forge.organization_exists("algebra").await.unwrap());
        let error = forge
            .create_organization("algebra", "Algebra")
            .await
            .unwrap_err();
        assert!(error.is_conflict());
        assert!(error.to_string().contains("user already exists"), "{error}");
        // Removing someone who is not there is done.
        forge.remove_team_member(2, "ada").await.unwrap();
    }
}
