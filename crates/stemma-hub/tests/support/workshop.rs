// The board, process protocol, lease and token boundaries, against a real git proxy.

use std::sync::Arc;

use serde_json::{Value, json};
use stemma_core::api::NewAccount;
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};

struct Hub {
    url: String,
    state: stemma_hub::AppState,
    _forge: FakeForge,
    _data: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

async fn hub(model: Option<&stemma_fake_model::FakeModel>) -> Hub {
    let forge = FakeForge::start().await;
    let data = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let config = stemma_hub::Config {
        data_dir: data.path().to_owned(),
        public_url: Some(url.clone()),
        forge: Some(ForgeConfig::new(
            &forge.url,
            "http://forge.test/user/oauth2/stemma/callback",
            ForgeSecrets {
                token: stemma_fake_forge::TOKEN.into(),
                webhook_secret: "the webhook's secret".into(),
                client_secret: "the client's secret".into(),
            },
        )),
        ..Default::default()
    };
    let mut hub = stemma_hub::Hub::new(stemma_hub::db::in_memory().await.unwrap(), config).unwrap();
    if let Some(model) = model {
        hub.model_client.endpoint = Some(model.url.clone());
    }
    let state = Arc::new(hub);
    let app = stemma_hub::app(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Hub {
        url,
        state,
        _forge: forge,
        _data: data,
        server,
    }
}

/// A person's personal token.
async fn person(hub: &Hub, username: &str, admin: bool) -> String {
    let account = NewAccount {
        username: username.into(),
        display_name: username.into(),
        password: "password1".into(),
    };
    stemma_hub::api::create_user(&hub.state, account, admin)
        .await
        .unwrap();
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/api/v1/session", hub.url))
        .json(&json!({"username": username, "password": "password1"}))
        .send()
        .await
        .unwrap();
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let created: Value = client
        .post(format!("{}/api/v1/me/tokens", hub.url))
        .header("cookie", cookie)
        .json(&json!({"name": "tests"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    created["secret"].as_str().unwrap().to_owned()
}

async fn call(hub: &Hub, token: &str, method: &str, path: &str, body: Option<Value>) -> Value {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = reqwest::Client::new()
        .request(method, format!("{}/api/v1{path}", hub.url))
        .bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let value = response.json().await.unwrap_or(Value::Null);
    assert!(status.is_success(), "{path}: {status} {value}");
    value
}

use futures_util::{SinkExt, StreamExt};
use stemma_core::protocol::*;
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn next(socket: &mut Socket) -> HubMessage {
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
async fn send(socket: &mut Socket, msg: WorkerMessage) {
    socket
        .send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
        .await
        .unwrap();
}
async fn machine(hub: &Hub, token: &str) -> Socket {
    let mut req = format!("{}/api/v1/worker", hub.url.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    send(
        &mut socket,
        WorkerMessage::Hello(Hello {
            protocol: PROTOCOL_VERSION,
            version: "1.0.0".into(),
            runners: vec!["codex".into()],
            concurrency: 1,
            agents: GroupAgents {
                attachments: Default::default(),
                enabled: false,
                priority: 0,
                concurrency: None,
                runners: None,
            },
            groups: Default::default(),
            workshop: GroupWorkshop {
                enabled: true,
                build: false,
                build_concurrency: 1,
                concurrency: 2,
                runners: std::collections::BTreeMap::from([("codex".into(), "".into())]),
                others: Others::Ask,
            },
            workshop_groups: Default::default(),
        }),
    )
    .await;
    assert!(matches!(next(&mut socket).await, HubMessage::Welcome(_)));
    socket
}
/// The group `geometry`, by `token`'s person, with the tests' own recipes
/// beside the template's.
async fn group(hub: &Hub, token: &str) {
    call(
        hub,
        token,
        "POST",
        "/groups",
        Some(json!({"slug":"geometry","name":"Geometry"})),
    )
    .await;
    let recipes = [
        ("prove.md", include_str!("recipes/prove.md")),
        ("review-paper.md", include_str!("recipes/review-paper.md")),
    ];
    hub.state
        .forge
        .as_ref()
        .unwrap()
        .commit_files(
            "geometry",
            "recipes",
            "main",
            "Add the tests' recipes",
            &recipes.map(|(name, text)| (name.to_owned(), text.to_owned())),
        )
        .await
        .unwrap();
}
fn ask_body(recipe: &str, assignment: Value) -> Value {
    let inputs = if recipe == "prove" {
        json!({ "statement": "True" })
    } else {
        json!({ "paper_reference": "notes" })
    };
    json!({
        "recipe": recipe,
        "title": "Prove the identity",
        "inputs": inputs,
        "assignment": assignment,
        "runner": "codex",
        "model": null,
        "schedule": { "timezone": "Europe/Madrid" },
        "wait_hours": 24,
    })
}
fn git(dir: &std::path::Path, token: &str, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Process",
            "-c",
            "user.email=worker@stemma.invalid",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .output()
        .unwrap()
}
async fn status(
    hub: &Hub,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> reqwest::StatusCode {
    reqwest::Client::new()
        .request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            format!("{}{path}", hub.url),
        )
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.state.shutdown.cancel();
        self.server.abort();
    }
}
