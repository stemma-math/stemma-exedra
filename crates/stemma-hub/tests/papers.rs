//! Working papers retain source and archived results without creating builds.

use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::{Value, json};
use stemma_core::api::NewAccount;
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};

struct Hub {
    url: String,
    state: stemma_hub::AppState,
    _forge: FakeForge,
    _data: tempfile::TempDir,
}

async fn hub() -> Hub {
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
    let state =
        Arc::new(stemma_hub::Hub::new(stemma_hub::db::in_memory().await.unwrap(), config).unwrap());
    let app = stemma_hub::app(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Hub {
        url,
        state,
        _forge: forge,
        _data: data,
    }
}

/// Someone, with their personal token.
#[derive(Clone)]
struct Person {
    _id: i64,
    token: String,
}

/// Someone in `geometry`, with `role`; Ada, who administers the hub,
/// makes the group first.
async fn person(hub: &Hub, username: &str, role: &str) -> Person {
    let account = NewAccount {
        username: username.into(),
        display_name: capitalized(username),
        password: "password1".into(),
    };
    let id = stemma_hub::api::create_user(&hub.state, account, username == "ada")
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
    let person = Person {
        _id: id,
        token: created["secret"].as_str().unwrap().to_owned(),
    };
    if username == "ada" {
        let (status, body) = call(
            hub,
            &person,
            "POST",
            "/groups",
            Some(json!({"slug": "geometry", "name": "Geometry"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    } else {
        sqlx::query(
            "INSERT INTO memberships (group_id, user_id, role, joined_at)
             SELECT g.id, ?, ?, 0 FROM groups g WHERE g.slug = 'geometry'",
        )
        .bind(id)
        .bind(role)
        .execute(&hub.state.db)
        .await
        .unwrap();
    }
    person
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

async fn call(
    hub: &Hub,
    person: &Person,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = reqwest::Client::new()
        .request(method, format!("{}/api/v1{path}", hub.url))
        .bearer_auth(&person.token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn get(hub: &Hub, person: &Person, path: &str) -> Value {
    let (status, body) = call(hub, person, "GET", path, None).await;
    assert!(status.is_success(), "{path}: {status} {body}");
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn papers_keep_source_and_archived_results_without_new_jobs() {
    let hub = hub().await;
    let ada = person(&hub, "ada", "admin").await;
    let (status, paper) = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/papers",
        Some(json!({"slug": "notes", "title": "Notes"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{paper}");
    let page = get(&hub, &ada, "/groups/geometry/papers/notes").await;
    assert!(page["latest"].is_null());
    let source = get(&hub, &ada, "/groups/geometry/papers/notes/files").await;
    assert!(source["files"].is_array());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
