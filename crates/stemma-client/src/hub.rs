//! The hub's API, over HTTP.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use stemma_core::api::{
    AddWorkshopMachine, ApiToken, Architecture, ConsumptionDay, CreateMachine, CreatePaper,
    CreateTask, CreateThread, CreateToken, CreatedMachine, CreatedToken, DeclarationPage,
    DeclarationSummary, ErrorBody, LibraryState, Machine, Me, Message, PREFIX, Paper,
    PaperFileText, PaperPage, PaperSource, PersonalAgent, PostMessage, SearchHit, SignIn, Task,
    TaskPage, TaskState, Thread, ThreadKind, ThreadPage, UpdateMachine, WorkshopAgent,
    WorkshopMachine,
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
    #[error("could not reach the hub: {0}")]
    Unreachable(#[from] reqwest::Error),
}

impl ApiError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            ApiError::Hub { status, .. } => Some(*status),
            ApiError::Unreachable(_) => None,
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
        })
    }

    /// The hub's address, without a trailing slash.
    pub fn base(&self) -> &str {
        &self.base
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
        let response = request.send().await?;
        let status = response.status();
        let headers = response.headers().clone();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(match serde_json::from_str::<ErrorBody>(&text) {
                Ok(body) => ApiError::Hub {
                    status,
                    code: body.error.code,
                    message: body.error.message,
                },
                Err(_) => ApiError::Hub {
                    status,
                    code: "unknown".into(),
                    message: format!("The hub answered {status}"),
                },
            });
        }
        let text = response.text().await?;
        let text = if text.trim().is_empty() {
            "null"
        } else {
            &text
        };
        let value = serde_json::from_str(text).map_err(|e| ApiError::Hub {
            status,
            code: "unreadable".into(),
            message: format!("Could not read the hub's answer: {e}"),
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

    /// Uploads a build's PDF, with a job's token, to `path` (relative to
    /// the hub, as the job says).
    pub async fn put_pdf(&self, path: &str, pdf: Vec<u8>) -> Result<()> {
        let mut request = self
            .http
            .put(format!("{}{path}", self.base))
            .header(reqwest::header::CONTENT_TYPE, "application/pdf")
            .timeout(Duration::from_secs(600))
            .body(pdf);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        Err(match serde_json::from_str::<ErrorBody>(&text) {
            Ok(body) => ApiError::Hub {
                status,
                code: body.error.code,
                message: body.error.message,
            },
            Err(_) => ApiError::Hub {
                status,
                code: "unknown".into(),
                message: format!("The hub answered {status}"),
            },
        })
    }

    async fn delete(&self, path: &str) -> Result<()> {
        self.send::<serde_json::Value>(Method::DELETE, path, &[], None::<&()>)
            .await?;
        Ok(())
    }

    // ── Accounts ──────────────────────────────────────────────────────

    pub async fn me(&self) -> Result<Me> {
        self.get("/me", &[]).await
    }

    /// Signs in with a password, for what follows, until [`Hub::sign_out`].
    pub async fn sign_in(&mut self, username: &str, password: &str) -> Result<Me> {
        let body = SignIn {
            username: username.into(),
            password: password.into(),
        };
        let (me, headers): (Me, _) = self
            .send(Method::POST, "/session", &[], Some(&body))
            .await?;
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
            launcher: false,
            expires_in_minutes: None,
        };
        self.post("/me/tokens", &body).await
    }

    /// A token that stops working by itself after `minutes`.
    pub async fn create_expiring_token(&self, name: &str, minutes: u32) -> Result<CreatedToken> {
        let body = CreateToken {
            name: name.into(),
            launcher: true,
            expires_in_minutes: Some(minutes),
        };
        self.post("/me/tokens", &body).await
    }

    pub async fn tokens(&self) -> Result<Vec<ApiToken>> {
        self.get("/me/tokens", &[]).await
    }

    pub async fn revoke_token(&self, id: i64) -> Result<()> {
        self.delete(&format!("/me/tokens/{id}")).await
    }

    // ── Agents and machines ───────────────────────────────────────────

    /// The person's own agents.
    pub async fn my_agents(&self) -> Result<Vec<PersonalAgent>> {
        self.get("/me/agents", &[]).await
    }

    /// The person's machines.
    pub async fn machines(&self) -> Result<Vec<Machine>> {
        self.get("/me/machines", &[]).await
    }

    pub async fn create_machine(&self, name: &str, preferred: bool) -> Result<CreatedMachine> {
        let body = CreateMachine {
            name: name.into(),
            preferred,
        };
        self.post("/me/machines", &body).await
    }

    pub async fn prefer_machine(&self, id: i64, preferred: bool) -> Result<Machine> {
        let body = UpdateMachine { preferred };
        Ok(self
            .send(
                Method::PATCH,
                &format!("/me/machines/{id}"),
                &[],
                Some(&body),
            )
            .await?
            .0)
    }

    pub async fn revoke_machine(&self, id: i64) -> Result<()> {
        self.delete(&format!("/me/machines/{id}")).await
    }

    /// Sets the agents one of the person's machines offers the workshop,
    /// in order of preference.
    pub async fn set_workshop_agents(
        &self,
        id: i64,
        agents: &[WorkshopAgent],
    ) -> Result<Vec<WorkshopAgent>> {
        Ok(self
            .send(
                Method::PUT,
                &format!("/me/machines/{id}/workshop-agents"),
                &[],
                Some(agents),
            )
            .await?
            .0)
    }

    // ── The workshop ──────────────────────────────────────────────────

    /// The kinds of work the workshop knows.
    pub async fn architectures(&self) -> Result<Vec<Architecture>> {
        self.get("/workshop/architectures", &[]).await
    }

    pub async fn workshop_machines(&self, group: &str) -> Result<Vec<WorkshopMachine>> {
        self.get(&format!("{}/workshop/machines", g(group)), &[])
            .await
    }

    /// Makes one of the person's machines work for a group's workshop.
    pub async fn add_workshop_machine(&self, group: &str, machine: i64) -> Result<()> {
        self.send::<serde_json::Value>(
            Method::POST,
            &format!("{}/workshop/machines", g(group)),
            &[],
            Some(&AddWorkshopMachine { machine }),
        )
        .await?;
        Ok(())
    }

    pub async fn remove_workshop_machine(&self, group: &str, machine: i64) -> Result<()> {
        self.delete(&format!("{}/workshop/machines/{machine}", g(group)))
            .await
    }

    /// What a group's workshop consumed, day by day.
    pub async fn workshop_consumption(
        &self,
        group: &str,
        days: u32,
    ) -> Result<Vec<ConsumptionDay>> {
        self.get(
            &format!("{}/workshop/consumption", g(group)),
            &[("days", days.to_string())],
        )
        .await
    }

    pub async fn approve_task(&self, group: &str, number: u32) -> Result<()> {
        self.send::<serde_json::Value>(
            Method::POST,
            &format!("{}/tasks/{number}/approve", g(group)),
            &[],
            None::<&()>,
        )
        .await?;
        Ok(())
    }

    pub async fn reject_task(&self, group: &str, number: u32) -> Result<()> {
        self.send::<serde_json::Value>(
            Method::POST,
            &format!("{}/tasks/{number}/reject", g(group)),
            &[],
            None::<&()>,
        )
        .await?;
        Ok(())
    }

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

impl Hub {
    // ── The workshop ──────────────────────────────────────────────────

    pub async fn tasks(
        &self,
        group: &str,
        state: Option<TaskState>,
        limit: u32,
    ) -> Result<Vec<Task>> {
        let mut query = vec![("limit", limit.to_string())];
        if let Some(state) = state {
            query.push(("state", state.as_str().to_owned()));
        }
        self.get(&format!("{}/tasks", g(group)), &query).await
    }

    pub async fn task(&self, group: &str, number: u32) -> Result<TaskPage> {
        self.get(&format!("{}/tasks/{number}", g(group)), &[]).await
    }

    pub async fn create_task(&self, group: &str, task: &CreateTask) -> Result<Task> {
        self.post(&format!("{}/tasks", g(group)), task).await
    }
}

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
