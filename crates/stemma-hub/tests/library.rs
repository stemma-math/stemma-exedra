//! Library creation and git authorization, and build scheduling.

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
async fn libraries_are_made_cloned_and_pushed_with_roles() {
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
    assert_eq!(library["latest"]["state"], "queued");
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

    // Members can create and delete branches, but cannot update main or other refs.
    let bob = person(&hub, "bob", false).await;
    sqlx::query("INSERT INTO memberships (group_id, user_id, role, joined_at) SELECT g.id, u.id, 'member', 0 FROM groups g, users u WHERE u.username = 'bob'")
        .execute(&hub.state.db).await.unwrap();
    let url = format!("http://bob:{bob}@{host}/git/geometry/library.git");
    assert!(
        git(work.path(), &["clone", "-q", &url, "member"])
            .status
            .success()
    );
    let member = work.path().join("member");
    std::fs::write(member.join("NOTES.md"), "Notes.\n").unwrap();
    git(&member, &["add", "NOTES.md"]);
    git(&member, &["commit", "-qm", "Notes"]);
    let denied = git(&member, &["push", "origin", "HEAD:main"]);
    assert!(!denied.status.success());
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("Only the group's admins"),
        "{denied:?}"
    );
    assert!(
        git(&member, &["push", "origin", "HEAD:refs/heads/topic"])
            .status
            .success()
    );
    assert!(
        git(&member, &["push", "origin", ":refs/heads/topic"])
            .status
            .success()
    );
    assert!(
        !git(&member, &["push", "origin", "HEAD:refs/notes/test"])
            .status
            .success()
    );

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
        Default::default(),
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
        Default::default(),
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

/// Exercise the real WebSocket contract with a scripted build machine.
#[tokio::test(flavor = "multi_thread")]
async fn workshop_builds_latest_commits_with_fenced_leases_and_scoped_tokens() {
    use futures_util::{SinkExt, StreamExt};
    use stemma_core::protocol::{
        BuildResult, GroupWorkshop, Hello, HubMessage, PROTOCOL_VERSION, WorkerMessage,
    };
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let hub = hub().await;
    let ada = person(&hub, "ada", true).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups",
        Some(json!({"slug":"geometry", "name":"Geometry"})),
    )
    .await;
    let forge = hub.state.forge.as_ref().unwrap();
    // An obsolete queued commit is cancelled before any machine arrives.
    forge
        .commit_files(
            "geometry",
            "library",
            "main",
            "Second",
            &[("NOTES.md".into(), "Second".into())],
        )
        .await
        .unwrap();
    stemma_hub::library::refresh(&hub.state, 1, 1)
        .await
        .unwrap();
    let queued = call(&hub, &ada, "GET", "/groups/geometry/library/builds", None).await;
    assert_eq!(queued[0]["state"], "queued");
    assert_eq!(queued[1]["state"], "cancelled");
    let mut request = stemma_worker::connection::socket_url(&hub.url)
        .unwrap()
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {ada}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let hello = Hello {
        protocol: PROTOCOL_VERSION,
        version: "0.1.1".into(),
        runners: vec![],
        concurrency: 1,
        agents: stemma_core::protocol::GroupAgents {
            enabled: false,
            priority: 0,
            concurrency: Some(1),
            runners: None,
            attachments: Default::default(),
        },
        groups: Default::default(),
        workshop: GroupWorkshop {
            enabled: true,
            build: true,
            build_concurrency: 1,
            concurrency: 1,
        },
        workshop_groups: Default::default(),
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&WorkerMessage::Hello(hello))
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    // Skip ping/identity messages, with a bounded wait.
    async fn next(
        socket: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> HubMessage {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(Ok(Message::Text(text))) = socket.next().await {
                    return serde_json::from_str(&text).unwrap();
                }
            }
        })
        .await
        .unwrap()
    }
    assert!(matches!(next(&mut socket).await, HubMessage::Welcome(_)));
    let HubMessage::Build(first) = next(&mut socket).await else {
        panic!("a build");
    };
    let http = reqwest::Client::new();
    let get = |path: &str| {
        http.get(format!("{}{path}", hub.url))
            .bearer_auth(&first.token)
    };
    assert_eq!(
        get("/git/geometry/library.git/info/refs?service=git-upload-pack")
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    for path in [
        "/api/v1/me",
        "/api/v1/groups/geometry/library",
        "/git/geometry/other.git/info/refs?service=git-upload-pack",
        "/git/geometry/library.git/info/refs?service=git-receive-pack",
    ] {
        assert_eq!(get(path).send().await.unwrap().status(), 403, "{path}");
    }
    for path in [
        format!("/api/v1/builds/{}/result", first.lease.id + 1),
        format!("/api/v1/builds/{}/pdf", first.lease.id),
    ] {
        let response = http
            .post(format!("{}{path}", hub.url))
            .bearer_auth(&first.token)
            .header("content-type", "application/pdf")
            .body("%PDF-test")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403, "{path}");
    }
    socket
        .send(Message::Text(
            serde_json::to_string(&WorkerMessage::BuildHeartbeat { lease: first.lease })
                .unwrap()
                .into(),
        ))
        .await
        .unwrap();
    // Move main while running: cancellation, then a new lease for the latest commit.
    forge
        .commit_files(
            "geometry",
            "library",
            "main",
            "Third",
            &[("NOTES.md".into(), "Third".into())],
        )
        .await
        .unwrap();
    stemma_hub::library::refresh(&hub.state, 1, 1)
        .await
        .unwrap();
    assert!(
        matches!(next(&mut socket).await, HubMessage::CancelBuild { lease } if lease == first.lease)
    );
    let HubMessage::Build(second) = next(&mut socket).await else {
        panic!("latest build");
    };
    let passed = BuildResult {
        succeeded: true,
        report: Some(stemma_core::api::LibraryReport {
            lean: "4.35.0".into(),
            modules: vec!["Geometry.Basic".into()],
            declarations: vec![],
        }),
        error: None,
        log: format!(
            "{}\n{}",
            "x".repeat(210_000),
            "stb_abcdefghijklmnopqrstuvwxyz0123456789"
        ),
    };
    // A late result stays in history, but can never become current.
    socket
        .send(Message::Text(
            serde_json::to_string(&WorkerMessage::BuildFinished {
                lease: first.lease,
                result: Box::new(passed.clone()),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    for _ in 0..50 {
        let succeeded: Option<bool> =
            sqlx::query_scalar("SELECT succeeded FROM library_builds WHERE work_id = ?")
                .bind(first.lease.id)
                .fetch_one(&hub.state.db)
                .await
                .unwrap();
        if succeeded == Some(true) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        stemma_hub::library::current(&hub.state, 1)
            .await
            .unwrap()
            .is_none()
    );
    let (accepted, log): (bool, String) =
        sqlx::query_as("SELECT accepted, log FROM library_builds WHERE work_id = ?")
            .bind(first.lease.id)
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
    assert!(!accepted);
    assert!(
        log.len() <= 200_000
            && log.contains("[hidden]")
            && !log.contains("abcdefghijklmnopqrstuvwxyz")
    );
    // Expiry retries and fences old holders, up to three attempts.
    for attempt in 1..=3 {
        sqlx::query("UPDATE work_leases SET heartbeat = ? WHERE id = ?")
            .bind(stemma_hub::db::now() - 61_000)
            .bind(second.lease.id)
            .execute(&hub.state.db)
            .await
            .unwrap();
        stemma_hub::builds::dispatch(&hub.state).await.unwrap();
        assert!(
            matches!(next(&mut socket).await, HubMessage::CancelBuild { lease } if lease.attempt == attempt)
        );
        if attempt < 3 {
            let HubMessage::Build(retry) = next(&mut socket).await else {
                panic!("retry");
            };
            assert_eq!(retry.lease.attempt, attempt + 1);
            let old_heartbeat: Option<i64> =
                sqlx::query_scalar("SELECT heartbeat FROM work_leases WHERE id = ?")
                    .bind(second.lease.id)
                    .fetch_one(&hub.state.db)
                    .await
                    .unwrap();
            socket
                .send(Message::Text(
                    serde_json::to_string(&WorkerMessage::BuildHeartbeat {
                        lease: second.lease,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            if attempt > 1 {
                assert!(old_heartbeat.is_some());
            }
        }
    }
    let state: String = sqlx::query_scalar("SELECT state FROM work_leases WHERE id = ?")
        .bind(second.lease.id)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(state, "failed");
    let builds = call(&hub, &ada, "GET", "/groups/geometry/library/builds", None).await;
    assert_eq!(builds[0]["state"], "failed");
    assert!(builds[0]["error"].as_str().unwrap().contains("three times"));
    // Another commit succeeds through the token's report endpoint.
    forge
        .commit_files(
            "geometry",
            "library",
            "main",
            "Fourth",
            &[("NOTES.md".into(), "Fourth".into())],
        )
        .await
        .unwrap();
    stemma_hub::library::refresh(&hub.state, 1, 1)
        .await
        .unwrap();
    let HubMessage::Build(third) = next(&mut socket).await else {
        panic!("new build");
    };
    assert_eq!(
        http.post(format!(
            "{}/api/v1/builds/{}/result",
            hub.url, third.lease.id
        ))
        .bearer_auth(&third.token)
        .json(&passed)
        .send()
        .await
        .unwrap()
        .status(),
        204
    );
    assert_eq!(
        stemma_hub::library::current(&hub.state, 1)
            .await
            .unwrap()
            .unwrap()
            .commit,
        third.commit
    );
    assert_eq!(
        http.get(format!("{}/api/v1/me", hub.url))
            .bearer_auth(&third.token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    // A paper uploads its PDF with its own build token, then reports through the protocol.
    let paper = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/papers",
        Some(json!({"slug":"notes", "title":"Notes"})),
    )
    .await;
    assert_eq!(paper["latest"]["state"], "running");
    let HubMessage::Build(paper) = next(&mut socket).await else {
        panic!("paper build");
    };
    assert_eq!(
        http.post(format!("{}/api/v1/builds/{}/pdf", hub.url, paper.lease.id))
            .bearer_auth(&ada)
            .body("%PDF-test")
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(format!("{}/api/v1/builds/{}/pdf", hub.url, paper.lease.id))
            .bearer_auth(&paper.token)
            .body("%PDF-1.5\nTest")
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    socket
        .send(Message::Text(
            serde_json::to_string(&WorkerMessage::BuildFinished {
                lease: paper.lease,
                result: Box::new(BuildResult {
                    succeeded: true,
                    ..Default::default()
                }),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
    for _ in 0..50 {
        let page = call(&hub, &ada, "GET", "/groups/geometry/papers/notes", None).await;
        if page["pdf"].is_object() {
            assert_eq!(page["latest"]["state"], "succeeded");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let page = call(&hub, &ada, "GET", "/groups/geometry/papers/notes", None).await;
    assert!(page["pdf"].is_object());
    socket.close(None).await.unwrap();
}
