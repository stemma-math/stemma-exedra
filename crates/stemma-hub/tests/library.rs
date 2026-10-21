//! Library creation and git authorization, without scheduling builds.

use std::path::Path;
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

/// Runs git in `dir`, as nobody in particular.
fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Ada", "-c", "user.email=ada@example.org"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn libraries_are_made_cloned_and_pushed_without_building() {
    let hub = hub().await;
    let ada = person(&hub, "ada", true).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups",
        Some(json!({"slug": "geometry", "name": "Geometry"})),
    )
    .await;

    // The group has a library, waiting for a worker with Lean.
    let library = call(&hub, &ada, "GET", "/groups/geometry/library", None).await;
    assert_eq!(library["available"], true);
    assert_eq!(library["clone_path"], "/git/geometry/library.git");
    assert!(library["head"].is_string());
    assert!(library["latest"].is_null());
    assert!(library["current"].is_null());

    // Cloned with the token as the password; not without it.
    let work = tempfile::tempdir().unwrap();
    let host = hub.url.trim_start_matches("http://");
    let anonymous = git(
        work.path(),
        &[
            "clone",
            "-q",
            &format!("{}/git/geometry/library.git", hub.url),
            "nobody",
        ],
    );
    assert!(!anonymous.status.success());
    let url = format!("http://ada:{ada}@{host}/git/geometry/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "geometry"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let dir = work.path().join("geometry");
    let lakefile = std::fs::read_to_string(dir.join("lakefile.toml")).unwrap();
    assert!(lakefile.contains("name = \"geometry\""), "{lakefile}");
    assert!(lakefile.contains("rev = \"v4.35.0\""));
    let basic = std::fs::read_to_string(dir.join("Geometry/Basic.lean")).unwrap();
    assert!(basic.contains("namespace Geometry"));
    assert!(lakefile.contains("globs = [\"Geometry.+\"]"), "{lakefile}");
    assert!(!dir.join("Geometry.lean").exists());

    // A push builds the new commit, in place of the one still waiting.
    std::fs::write(
        dir.join("Geometry/Basic.lean"),
        format!("{basic}\n-- More.\n"),
    )
    .unwrap();
    git(&dir, &["commit", "-qam", "More"]);
    let pushed = git(&dir, &["push", "-q", "origin", "main"]);
    assert!(pushed.status.success(), "{pushed:?}");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // History cannot be rewritten.
    git(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
    let forced = git(&dir, &["push", "-q", "--force", "origin", "main"]);
    assert!(!forced.status.success());

    // Guests clone the library, but do not push to it.
    let gus = person(&hub, "gus", false).await;
    sqlx::query(
        "INSERT INTO memberships (group_id, user_id, role, joined_at)
         SELECT g.id, u.id, 'guest', 0 FROM groups g, users u WHERE u.username = 'gus'",
    )
    .execute(&hub.state.db)
    .await
    .unwrap();
    let url = format!("http://gus:{gus}@{host}/git/geometry/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "guest"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let guest = work.path().join("guest");
    std::fs::write(guest.join("Geometry/Basic.lean"), "-- Mine.\n").unwrap();
    git(&guest, &["commit", "-qam", "Mine"]);
    let pushed = git(&guest, &["push", "-q", "origin", "main"]);
    assert!(!pushed.status.success());

    // Nor does anyone outside the group clone it.
    let eve = person(&hub, "eve", false).await;
    let url = format!("http://eve:{eve}@{host}/git/geometry/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "outsider"]);
    assert!(!cloned.status.success());

    // A session of the web app does not reach git; only tokens do.
    let session = reqwest::Client::new()
        .post(format!("{}/api/v1/session", hub.url))
        .json(&json!({"username": "ada", "password": "password1"}))
        .send()
        .await
        .unwrap();
    let cookie = session.headers()["set-cookie"].to_str().unwrap().to_owned();
    let refs = reqwest::Client::new()
        .get(format!(
            "{}/git/geometry/library.git/info/refs?service=git-upload-pack",
            hub.url
        ))
        .header("cookie", cookie.split(';').next().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(refs.status(), 401);
    // Nor the dumb protocol, with a token.
    let dumb = reqwest::Client::new()
        .get(format!("{}/git/geometry/library.git/HEAD", hub.url))
        .bearer_auth(&ada)
        .send()
        .await
        .unwrap();
    assert_eq!(dumb.status(), 404);

    // A job's token clones, and never pushes.
    let (ada_id, group_id): (i64, i64) =
        sqlx::query_as("SELECT u.id, g.id FROM users u, groups g WHERE u.username = 'ada'")
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
    let (_token_id, job) = stemma_hub::auth::create_job_token(
        &hub.state.db,
        ada_id,
        group_id,
        "Lean",
        stemma_hub::db::now() + 60_000,
    )
    .await
    .unwrap();
    let url = format!("http://job:{job}@{host}/git/geometry/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "job"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let checkout = work.path().join("job");
    std::fs::write(checkout.join("NOTES.md"), "Notes.\n").unwrap();
    git(&checkout, &["add", "NOTES.md"]);
    git(&checkout, &["commit", "-qm", "Notes"]);
    let pushed = git(&checkout, &["push", "-q", "origin", "main"]);
    assert!(!pushed.status.success());

    // Nor does an agent's session token, which still clones.
    let (_, session, _) = stemma_hub::auth::create_agent_token(
        &hub.state.db,
        ada_id,
        group_id,
        "Claude",
        "a session",
    )
    .await
    .unwrap();
    let url = format!("http://agent:{session}@{host}/git/geometry/library.git");
    let cloned = git(work.path(), &["clone", "-q", &url, "agent"]);
    assert!(cloned.status.success(), "{cloned:?}");
    let checkout = work.path().join("agent");
    std::fs::write(checkout.join("NOTES.md"), "Notes.\n").unwrap();
    git(&checkout, &["add", "NOTES.md"]);
    git(&checkout, &["commit", "-qm", "Notes"]);
    let pushed = git(&checkout, &["push", "-q", "origin", "main"]);
    assert!(!pushed.status.success());
}
