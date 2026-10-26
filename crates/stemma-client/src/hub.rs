//! The hub's API, over HTTP.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use stemma_core::api::{
    ApiToken, CreateAgentToken, CreatePaper, CreateThread, CreateToken, CreatedToken,
    DeclarationPage, DeclarationSummary, ErrorBody, LibraryState, Me, Message, PREFIX, Paper,
    PaperFileText, PaperPage, PaperSource, PostMessage, SearchHit, SendNote, SignIn, Thread,
    ThreadKind, ThreadPage,
};

/// An error the hub answered with, or not reaching it.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{message}")]
    Hub {
        status: StatusCode,
        code: String,
        message: String,
    },
    #[error("Could not reach {url}: {source}")]
    Unreachable { url: String, source: reqwest::Error },
}

impl ApiError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            ApiError::Hub { status, .. } => Some(*status),
            ApiError::Unreachable { .. } => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

/// A hub, as one person (or one job) reaches it.
#[derive(Clone, Debug)]
pub struct Hub {
    base: String,
    token: Option<String>,
    /// A session cookie, while signing in to make a token.
    cookie: Option<String>,
    http: reqwest::Client,
    local: Option<crate::config::Paths>,
}

/// Makes rustls use ring, once per process.
pub fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

impl Hub {
    /// A hub at `base` (such as `https://stemma.example.org`), with a token.
    pub fn new(base: &str, token: Option<String>) -> anyhow::Result<Hub> {
        install_crypto();
        let http = reqwest::Client::builder()
            .user_agent(concat!("stemma/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Hub {
            base: normalize_url(base)?,
            token,
            cookie: None,
            http,
            local: None,
        })
    }

    /// Only a hub constructed from local credentials may update local files.
    pub fn local(mut self, paths: crate::config::Paths) -> Self {
        self.local = Some(paths);
        self
    }

    /// Download an attachment by the id shown by the reading tools. The hub
    /// checks both group membership and the token's attachment rule.
    pub async fn download_attachment(&self, group: &str, id: i64) -> Result<reqwest::Response> {
        let mut request = self.http.get(format!(
            "{}{PREFIX}/groups/{group}/attachments/{id}/file",
            self.base
        ));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(cookie) = &self.cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        let response = request.send().await.map_err(|e| self.unreachable(e))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .json::<ErrorBody>()
                .await
                .map_err(|e| self.unreachable(e))?;
            return Err(ApiError::Hub {
                status,
                code: body.error.code,
                message: body.error.message,
            });
        }
        Ok(response)
    }

    /// The hub's address, without a trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    fn unreachable(&self, source: reqwest::Error) -> ApiError {
        ApiError::Unreachable {
            url: self.base.clone(),
            source,
        }
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&(impl Serialize + ?Sized)>,
    ) -> Result<(T, reqwest::header::HeaderMap)> {
        let mut request = self
            .http
            .request(method, format!("{}{PREFIX}{path}", self.base))
            .query(query);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        if let Some(cookie) = &self.cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|source| self.unreachable(source))?;
        let status = response.status();
        let headers = response.headers().clone();
        if let (Some(paths), Some(token), Some(username)) = (
            &self.local,
            &self.token,
            headers.get("x-stemma-user").and_then(|v| v.to_str().ok()),
        ) && let Err(error) = paths.update_username(&self.base, token, username)
        {
            tracing::warn!(%error, "could not update the saved username");
        }

        if !status.is_success() {
            let text = response
                .text()
                .await
                .map_err(|source| self.unreachable(source))?;
            return Err(match serde_json::from_str::<ErrorBody>(&text) {
                Ok(body) => ApiError::Hub {
                    status,
                    code: body.error.code,
                    message: body.error.message,
                },
                Err(_) => ApiError::Hub {
                    status,
                    code: "unknown".into(),
                    message: format!(
                        "{} did not answer as a Stemma hub (HTTP {status}).",
                        self.base
                    ),
                },
            });
        }
        let text = response
            .text()
            .await
            .map_err(|source| self.unreachable(source))?;
        let text = if text.trim().is_empty() {
            "null"
        } else {
            &text
        };
        let value = serde_json::from_str(text).map_err(|e| ApiError::Hub {
            status,
            code: "unreadable".into(),
            message: format!("{} did not answer as a Stemma hub: {e}", self.base),
        })?;
        Ok((value, headers))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        Ok(self.send(Method::GET, path, query, None::<&()>).await?.0)
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &(impl Serialize + ?Sized),
    ) -> Result<T> {
        Ok(self.send(Method::POST, path, &[], Some(body)).await?.0)
    }

    async fn delete(&self, path: &str) -> Result<()> {
        self.send::<serde_json::Value>(Method::DELETE, path, &[], None::<&()>)
            .await?;
        Ok(())
    }

    /// Upload a local file; the hub still enforces the token's attachment policy.
    pub async fn upload_attachment(
        &self,
        group: &str,
        path: &std::path::Path,
    ) -> Result<stemma_core::api::Attachment> {
        let form = reqwest::multipart::Form::new()
            .file("file", path)
            .await
            .map_err(|e| ApiError::Hub {
                status: reqwest::StatusCode::BAD_REQUEST,
                code: "file".into(),
                message: e.to_string(),
            })?;
        let mut request = self
            .http
            .post(format!(
                "{}{PREFIX}/groups/{}/attachments",
                self.base,
                encode(group)
            ))
            .multipart(form);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| self.unreachable(e))?;
        let status = response.status();
        let value: serde_json::Value = response.json().await.map_err(|e| self.unreachable(e))?;
        if !status.is_success() {
            return Err(ApiError::Hub {
                status,
                code: value["error"]["code"].as_str().unwrap_or("upload").into(),
                message: value["error"]["message"]
                    .as_str()
                    .unwrap_or("Upload refused")
                    .into(),
            });
        }
        serde_json::from_value(value).map_err(|e| ApiError::Hub {
            status,
            code: "unreadable".into(),
            message: e.to_string(),
        })
    }

    /// Workshop API, shared by the command catalog and MCP.
    pub async fn workshop_read(
        &self,
        group: &str,
        resource: &str,
        query: &[(&str, String)],
    ) -> Result<serde_json::Value> {
        self.get(
            &format!("/groups/{}/workshop/{resource}", encode(group)),
            query,
        )
        .await
    }
    pub async fn workshop_ask(
        &self,
        group: &str,
        ask: &stemma_core::workshop::Ask,
    ) -> Result<stemma_core::workshop::Request> {
        self.post(&format!("/groups/{}/workshop/requests", encode(group)), ask)
            .await
    }
    pub async fn workshop_cancel(
        &self,
        group: &str,
        request: u32,
    ) -> Result<stemma_core::workshop::Request> {
        self.post(
            &format!("/groups/{}/workshop/requests/{request}", encode(group)),
            &serde_json::json!({"action":"cancel"}),
        )
        .await
    }

    // ── Accounts ──────────────────────────────────────────────────────

    pub async fn group(&self, slug: &str) -> Result<stemma_core::api::Group> {
        self.get(&format!("/groups/{}", encode(slug)), &[]).await
    }
    pub async fn me(&self) -> Result<Me> {
        let me: Me = self.get("/me", &[]).await?;
        if let Some(paths) = &self.local
            && let Err(error) = paths.reconcile_groups(&self.base, &me.groups)
        {
            tracing::warn!(%error, "could not update group names; will retry");
        }
        Ok(me)
    }

    /// Signs in with a password, for what follows, until [`Hub::sign_out`].
    pub async fn sign_in(&mut self, username: &str, password: &str) -> Result<Me> {
        let body = SignIn {
            username: username.into(),
            password: password.into(),
        };
        let (me, headers): (Me, _) = self
            .send(Method::POST, "/session", &[], Some(&body))
            .await
            .map_err(|error| match error {
                ApiError::Hub {
                    status: StatusCode::UNAUTHORIZED,
                    ref code,
                    ..
                } if code == "wrong_credentials" => ApiError::Hub {
                    status: StatusCode::UNAUTHORIZED,
                    code: "invalid_credentials".into(),
                    message: "That username and password don't match.".into(),
                },
                other => other,
            })?;
        self.cookie = headers
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("stemma_session="))
            .and_then(|v| v.split(';').next())
            .map(str::to_owned);
        Ok(me)
    }

    pub async fn sign_out(&mut self) -> Result<()> {
        self.delete("/session").await?;
        self.cookie = None;
        Ok(())
    }

    pub async fn create_token(&self, name: &str) -> Result<CreatedToken> {
        let body = CreateToken {
            name: name.into(),
            expires_in_minutes: None,
        };
        self.post("/me/tokens", &body).await
    }

    /// A token for a session with the person's agent of `runner`, in
    /// `group`, on the computer `host`: it reads the group, and posts and
    /// opens threads there as the agent, for a long day at most.
    pub async fn create_agent_token(
        &self,
        group: &str,
        runner: &str,
        host: &str,
        attachments: stemma_core::api::AttachmentRule,
    ) -> Result<CreatedToken> {
        let body = CreateAgentToken {
            attachments,
            group: group.into(),
            runner: runner.into(),
            host: host.into(),
        };
        self.post("/me/agent-tokens", &body).await
    }

    pub async fn token_identity(&self) -> Result<ApiToken> {
        self.get("/me/token", &[]).await
    }

    pub async fn tokens(&self) -> Result<Vec<ApiToken>> {
        self.get("/me/tokens", &[]).await
    }

    pub async fn revoke_token(&self, id: i64) -> Result<()> {
        self.delete(&format!("/me/tokens/{id}")).await
    }

    // ── Agents and machines ───────────────────────────────────────────

    // ── The forum ─────────────────────────────────────────────────────

    pub async fn threads(
        &self,
        group: &str,
        kind: Option<ThreadKind>,
        limit: u32,
    ) -> Result<Vec<Thread>> {
        let mut query = vec![("limit", limit.to_string())];
        if let Some(kind) = kind {
            query.push(("kind", kind.as_str().to_owned()));
        }
        self.get(&format!("{}/threads", g(group)), &query).await
    }

    pub async fn create_thread(&self, group: &str, thread: &CreateThread) -> Result<ThreadPage> {
        self.post(&format!("{}/threads", g(group)), thread).await
    }

    pub async fn thread(&self, group: &str, number: u32) -> Result<ThreadPage> {
        self.get(&format!("{}/threads/{number}", g(group)), &[])
            .await
    }

    /// Messages `from` to `to` of a thread, both included.
    pub async fn messages(
        &self,
        group: &str,
        thread: u32,
        from: u32,
        to: u32,
    ) -> Result<Vec<Message>> {
        let query = [("from", from.to_string()), ("to", to.to_string())];
        self.get(&format!("{}/threads/{thread}/messages", g(group)), &query)
            .await
    }

    pub async fn post_message(
        &self,
        group: &str,
        thread: u32,
        message: &PostMessage,
    ) -> Result<Message> {
        self.post(&format!("{}/threads/{thread}/messages", g(group)), message)
            .await
    }

    /// Leaves a note for some of the group's members.
    pub async fn send_note(&self, group: &str, note: &SendNote) -> Result<()> {
        self.post::<serde_json::Value>(&format!("{}/notes", g(group)), note)
            .await?;
        Ok(())
    }

    pub async fn search(&self, group: &str, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
        let query = [("q", query.to_owned()), ("limit", limit.to_string())];
        self.get(&format!("{}/search", g(group)), &query).await
    }
}

impl Hub {
    // ── The library ───────────────────────────────────────────────────

    pub async fn library(&self, group: &str) -> Result<LibraryState> {
        self.get(&format!("{}/library", g(group)), &[]).await
    }

    pub async fn declaration(&self, group: &str, name: &str) -> Result<DeclarationPage> {
        let path = format!("{}/library/declarations/{}", g(group), encode(name));
        self.get(&path, &[]).await
    }

    /// Declarations of the library, as `query` says: `q` (words of their
    /// names, types or docstrings), `module`, `state` and `limit`.
    pub async fn declarations(
        &self,
        group: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<DeclarationSummary>> {
        self.get(&format!("{}/library/declarations", g(group)), query)
            .await
    }
}

impl Hub {
    // ── Papers ────────────────────────────────────────────────────────

    pub async fn papers(&self, group: &str) -> Result<Vec<Paper>> {
        self.get(&format!("{}/papers", g(group)), &[]).await
    }

    pub async fn paper(&self, group: &str, slug: &str) -> Result<PaperPage> {
        self.get(&format!("{}/papers/{}", g(group), encode(slug)), &[])
            .await
    }

    pub async fn create_paper(&self, group: &str, slug: &str, title: &str) -> Result<Paper> {
        let body = CreatePaper {
            slug: slug.into(),
            title: title.into(),
        };
        self.post(&format!("{}/papers", g(group)), &body).await
    }

    /// A paper's source files, at its `main`.
    pub async fn paper_files(&self, group: &str, slug: &str) -> Result<PaperSource> {
        self.get(&format!("{}/papers/{}/files", g(group), encode(slug)), &[])
            .await
    }

    /// One of a paper's source files, at its `main`.
    pub async fn paper_file(&self, group: &str, slug: &str, path: &str) -> Result<PaperFileText> {
        let path: Vec<String> = path.split('/').map(encode).collect();
        self.get(
            &format!(
                "{}/papers/{}/files/{}",
                g(group),
                encode(slug),
                path.join("/")
            ),
            &[],
        )
        .await
    }
}

impl Hub {}

fn g(group: &str) -> String {
    format!("/groups/{}", encode(group))
}

/// Percent-encodes a path segment.
fn encode(segment: &str) -> String {
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

/// A hub's address: `http` or `https`, without a trailing slash; a bare
/// host is taken as `https://`.
pub fn normalize_url(url: &str) -> anyhow::Result<String> {
    let url = url.trim().trim_end_matches('/');
    anyhow::ensure!(!url.is_empty(), "the hub's address is empty");
    let url = if url.contains("://") {
        url.to_owned()
    } else {
        format!("https://{url}")
    };
    anyhow::ensure!(
        url.starts_with("http://") || url.starts_with("https://"),
        "a hub's address starts with http:// or https://"
    );
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_urls() {
        assert_eq!(
            normalize_url("stemma.example.org/").unwrap(),
            "https://stemma.example.org"
        );
        assert_eq!(
            normalize_url("http://127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url(" ").is_err());
    }

    #[test]
    fn encodes_segments() {
        assert_eq!(encode("geometry"), "geometry");
        assert_eq!(encode("a b/c"), "a%20b%2Fc");
    }
}
