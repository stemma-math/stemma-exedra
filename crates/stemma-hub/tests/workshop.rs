//! The board, process protocol, lease and token boundaries, against a real git proxy.

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
fn ask_body(recipe: &str, assignment: Value) -> Value {
    json!({"recipe":recipe,"title":"Prove the identity","inputs":if recipe=="prove"{json!({"statement":"True"})}else{json!({"paper_reference":"notes"})},"assignment":assignment,"runner":"codex","model":null,"schedule":{"timezone":"Europe/Madrid"},"wait_hours":24})
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
#[tokio::test(flavor = "multi_thread")]
async fn processes_push_only_their_branch_post_only_in_their_request_and_open_as_worker() {
    let hub = hub().await;
    let ada = person(&hub, "ada", true).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups",
        Some(json!({"slug":"geometry","name":"Geometry"})),
    )
    .await;
    let recipes = call(&hub, &ada, "GET", "/groups/geometry/workshop/recipes", None).await;
    assert_eq!(recipes.as_array().unwrap().len(), 2);
    assert_eq!(recipes[0]["version"].as_str().unwrap().len(), 40);
    let mut socket = machine(&hub, &ada).await;
    let asked = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body("prove", json!({"kind":"mine"}))),
    )
    .await;
    assert_eq!(asked["thread"]["kind"], "request");
    let HubMessage::ProcessOffer(offer) = next(&mut socket).await else {
        panic!("offer")
    };
    assert!(serde_json::to_value(&offer).unwrap().get("token").is_none());
    send(
        &mut socket,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec) = next(&mut socket).await else {
        panic!("lease")
    };
    let token = &spec.token;
    let (_, answer_token) = stemma_hub::auth::create_job_token(
        &hub.state.db,
        spec.owner.id,
        spec.offer.group.id,
        "Codex",
        stemma_hub::db::now() + 60_000,
        Default::default(),
    )
    .await
    .unwrap();
    assert!(
        status(
            &hub,
            &answer_token,
            "GET",
            "/api/v1/groups/geometry/workshop/recipes",
            Value::Null
        )
        .await
        .is_success()
    );
    assert_eq!(
        status(
            &hub,
            &answer_token,
            "POST",
            "/api/v1/groups/geometry/workshop/requests",
            ask_body("prove", json!({"kind":"mine"}))
        )
        .await,
        403
    );
    assert_eq!(
        status(
            &hub,
            &answer_token,
            "POST",
            "/api/v1/groups/geometry/workshop/requests/1",
            json!({"action":"cancel"})
        )
        .await,
        403
    );
    let guest = person(&hub, "guest", false).await;
    sqlx::query("INSERT INTO memberships(group_id,user_id,role,joined_at) SELECT g.id,u.id,'guest',0 FROM groups g,users u WHERE u.username='guest'").execute(&hub.state.db).await.unwrap();
    assert!(
        status(
            &hub,
            &guest,
            "GET",
            "/api/v1/groups/geometry/workshop/requests/1",
            Value::Null
        )
        .await
        .is_success()
    );
    assert_eq!(
        status(
            &hub,
            &guest,
            "POST",
            "/api/v1/groups/geometry/workshop/requests",
            ask_body("prove", json!({"kind":"mine"}))
        )
        .await,
        403
    );
    assert_eq!(
        status(
            &hub,
            &guest,
            "POST",
            "/api/v1/groups/geometry/threads/1/messages",
            json!({"body":"No"})
        )
        .await,
        403
    );
    assert_eq!(
        spec.repositories[0].branch,
        format!("workshop/1-{}", offer.process)
    );
    assert!(
        status(&hub, token, "GET", "/api/v1/groups/geometry", Value::Null)
            .await
            .is_success()
    );
    for (method, path) in [
        ("GET", "/api/v1/me"),
        ("POST", "/api/v1/groups/geometry/threads"),
        ("POST", "/api/v1/groups/geometry/notes"),
        ("POST", "/api/v1/groups/geometry/workshop/requests"),
    ] {
        assert_eq!(
            status(&hub, token, method, path, json!({})).await,
            403,
            "{path}"
        );
    }
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title":"Another thread"})),
    )
    .await;
    assert_eq!(
        status(
            &hub,
            token,
            "POST",
            "/api/v1/groups/geometry/threads/2/messages",
            json!({"body":"No"})
        )
        .await,
        403
    );
    let session_post = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body":"A session's answer","agent":"Codex"})),
    )
    .await;
    assert_eq!(session_post["workshop"], false);
    let posted = call(
        &hub,
        token,
        "POST",
        "/groups/geometry/threads/1/messages",
        Some(json!({"body":"Working on the proof"})),
    )
    .await;
    assert_eq!(posted["workshop"], true);
    assert_eq!(posted["author"]["username"], "ada");
    let temp = tempfile::tempdir().unwrap();
    let repo = &spec.repositories[0];
    assert!(
        git(
            temp.path(),
            token,
            &["clone", "-q", &format!("{}{}", hub.url, repo.path), "work"]
        )
        .status
        .success()
    );
    let dir = temp.path().join("work");
    assert!(
        git(&dir, token, &["checkout", "-qb", &repo.branch, &repo.base])
            .status
            .success()
    );
    std::fs::write(
        dir.join("Proof.lean"),
        "theorem workshop_test : True := by trivial\n",
    )
    .unwrap();
    assert!(git(&dir, token, &["add", "Proof.lean"]).status.success());
    assert!(
        git(&dir, token, &["commit", "-qm", "Prove True"])
            .status
            .success()
    );
    for reference in ["main", "other", "refs/tags/test", "refs/notes/test"] {
        assert!(
            !git(
                &dir,
                token,
                &["push", "origin", &format!("HEAD:{reference}")]
            )
            .status
            .success(),
            "{reference}"
        );
    }
    let pushed = git(
        &dir,
        token,
        &["push", "origin", &format!("HEAD:{}", repo.branch)],
    );
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    send(
        &mut socket,
        WorkerMessage::ProcessFinished {
            lease: spec.lease,
            result: Box::new(ProcessResult {
                summary: "Proved True and checked the proof.".into(),
                usage: Usage {
                    input_tokens: Some(100),
                    output_tokens: Some(20),
                    ..Default::default()
                },
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..100 {
        let found = call(
            &hub,
            &ada,
            "GET",
            "/groups/geometry/workshop/requests/1",
            None,
        )
        .await;
        if found["state"] == "done" {
            assert!(found["processes"][0]["branches"][0]["pull_request"].is_string());
            assert_eq!(found["processes"][0]["usage"]["input_tokens"], 100);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(hub._forge.pulls("geometry", "library").len(), 1);
    let pull: Value = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/repos/geometry/library/pulls/1",
            hub._forge.url
        ))
        .header(
            "authorization",
            format!("token {}", stemma_fake_forge::TOKEN),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pull["user"]["login"], "worker");
    assert_eq!(
        status(&hub, token, "GET", "/api/v1/groups/geometry", Value::Null).await,
        401
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_offers_first_acceptance_recurring_slots_and_expiry_are_durable() {
    let hub = hub().await;
    let ada = person(&hub, "ada", true).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups",
        Some(json!({"slug":"geometry","name":"Geometry"})),
    )
    .await;
    let bob = person(&hub, "bob", false).await;
    sqlx::query("INSERT INTO memberships(group_id,user_id,role,joined_at) SELECT g.id,u.id,'member',0 FROM groups g,users u WHERE u.username='bob'").execute(&hub.state.db).await.unwrap();
    let bob_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username='bob'")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let mut a = machine(&hub, &ada).await;
    let mut b = machine(&hub, &bob).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body(
            "review-paper",
            json!({"kind":"member","user":bob_id}),
        )),
    )
    .await;
    let HubMessage::ProcessOffer(offer) = next(&mut b).await else {
        panic!("Bob's offer")
    };
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: false,
            pending: true,
        },
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let waiting = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/1",
        None,
    )
    .await;
    assert_eq!(waiting["state"], "waiting");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind='process_approval' AND user_id=?",
    )
    .bind(bob_id)
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: offer.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec) = next(&mut b).await else {
        panic!("Bob's lease")
    };
    assert_eq!(spec.owner.id, bob_id);
    assert!(spec.repositories.is_empty());

    send(
        &mut b,
        WorkerMessage::ProcessFinished {
            lease: spec.lease,
            result: Box::new(ProcessResult {
                summary: "Reviewed the paper.".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body("review-paper", json!({"kind":"anyone"}))),
    )
    .await;
    let HubMessage::ProcessOffer(offer_a) = next(&mut a).await else {
        panic!("Ada's anyone offer")
    };
    let HubMessage::ProcessOffer(offer_b) = next(&mut b).await else {
        panic!("Bob's anyone offer")
    };
    assert_eq!(offer_a.process, offer_b.process);
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: offer_b.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec) = next(&mut b).await else {
        panic!("first accepted")
    };
    assert_eq!(spec.owner.id, bob_id);
    assert!(matches!(
        next(&mut a).await,
        HubMessage::ProcessTaken { .. }
    ));
    send(
        &mut a,
        WorkerMessage::ProcessDecision {
            process: offer_a.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    assert!(matches!(
        next(&mut a).await,
        HubMessage::ProcessTaken { .. }
    ));
    // A series' first slot is created once; two simultaneous hub tasks cannot replay it.
    let mut recurring = ask_body("review-paper", json!({"kind":"mine"}));
    recurring["schedule"] = json!({"timezone":"Europe/Madrid","every":{"kind":"hours","hours":1}});
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(recurring),
    )
    .await;
    let (x, y) = tokio::join!(
        stemma_hub::workshop::tick(&hub.state),
        stemma_hub::workshop::tick(&hub.state)
    );
    x.unwrap();
    y.unwrap();
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM workshop_processes WHERE thread_id=(SELECT id FROM threads WHERE number=3)").fetch_one(&hub.state.db).await.unwrap();
    assert_eq!(count, 1);
    sqlx::query("UPDATE workshop_requests SET next_at=? WHERE thread_id=(SELECT id FROM threads WHERE number=3)").bind(stemma_hub::db::now()-10_800_000).execute(&hub.state.db).await.unwrap();
    stemma_hub::workshop::tick(&hub.state).await.unwrap();
    stemma_hub::workshop::tick(&hub.state).await.unwrap();
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM workshop_processes WHERE thread_id=(SELECT id FROM threads WHERE number=3)").fetch_one(&hub.state.db).await.unwrap();
    assert_eq!(count, 2);
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests/3",
        Some(json!({"action":"pause"})),
    )
    .await;
    // Queue expiration posts and notifies, even when no eligible machine exists.
    let mut expires = ask_body("review-paper", json!({"kind":"mine"}));
    expires["model"] = json!("unavailable-model");
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(expires),
    )
    .await;
    sqlx::query("UPDATE workshop_processes SET expires_at=0 WHERE thread_id=(SELECT id FROM threads WHERE number=4)").execute(&hub.state.db).await.unwrap();
    stemma_hub::workshop::tick(&hub.state).await.unwrap();
    let expired = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/workshop/requests/4",
        None,
    )
    .await;
    assert_eq!(expired["state"], "expired");
    let thread = call(&hub, &ada, "GET", "/groups/geometry/threads/4", None).await;
    assert!(
        thread["messages"][0]["body"]
            .as_str()
            .unwrap()
            .contains("expired")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn retries_use_another_machine_once_and_a_possible_push_fences_them() {
    let hub = hub().await;
    let ada = person(&hub, "ada", true).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups",
        Some(json!({"slug":"geometry","name":"Geometry"})),
    )
    .await;
    let other = call(
        &hub,
        &ada,
        "POST",
        "/me/tokens",
        Some(json!({"name":"Ada's second machine"})),
    )
    .await;
    let mut a = machine(&hub, &ada).await;
    let mut b = machine(&hub, other["secret"].as_str().unwrap()).await;
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/workshop/requests",
        Some(ask_body("review-paper", json!({"kind":"mine"}))),
    )
    .await;
    let HubMessage::ProcessOffer(first) = next(&mut a).await else {
        panic!("first offer")
    };
    assert!(matches!(next(&mut b).await, HubMessage::ProcessOffer(_)));
    send(
        &mut a,
        WorkerMessage::ProcessDecision {
            process: first.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec1) = next(&mut a).await else {
        panic!("first lease")
    };
    assert!(matches!(
        next(&mut b).await,
        HubMessage::ProcessTaken { .. }
    ));
    send(
        &mut a,
        WorkerMessage::ProcessFinished {
            lease: spec1.lease,
            result: Box::new(ProcessResult {
                error: Some("Runner stopped before pushing".into()),
                ..Default::default()
            }),
        },
    )
    .await;
    let HubMessage::ProcessOffer(retry) = next(&mut b).await else {
        panic!("offer on another machine")
    };
    assert_ne!(retry.process, first.process);
    send(
        &mut b,
        WorkerMessage::ProcessDecision {
            process: retry.process,
            accept: true,
            pending: false,
        },
    )
    .await;
    let HubMessage::Process(spec2) = next(&mut b).await else {
        panic!("second lease")
    };
    assert_eq!(spec2.lease.attempt, 1);
    assert_ne!(spec2.lease.id, spec1.lease.id);
    // The old generation cannot finish or heartbeat the new run.
    send(
        &mut a,
        WorkerMessage::ProcessFinished {
            lease: spec1.lease,
            result: Box::new(ProcessResult {
                summary: "Late success".into(),
                ..Default::default()
            }),
        },
    )
    .await;
    send(
        &mut b,
        WorkerMessage::ProcessFinished {
            lease: spec2.lease,
            result: Box::new(ProcessResult {
                error: Some("Second failure".into()),
                ..Default::default()
            }),
        },
    )
    .await;
    for _ in 0..100 {
        let request = call(
            &hub,
            &ada,
            "GET",
            "/groups/geometry/workshop/requests/1",
            None,
        )
        .await;
        if request["state"] == "failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM work_leases WHERE id=?")
        .bind(spec2.lease.id)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM workshop_processes")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(count, 2);
    // Lost leases use the same limit, and a streamed push prevents even the first retry.
    sqlx::query("UPDATE work_leases SET state='running',attempts=1,heartbeat=? WHERE id=?")
        .bind(stemma_hub::db::now() - 61_000)
        .bind(spec2.lease.id)
        .execute(&hub.state.db)
        .await
        .unwrap();
    sqlx::query("UPDATE workshop_processes SET pushed=1 WHERE work_id=?")
        .bind(spec2.lease.id)
        .execute(&hub.state.db)
        .await
        .unwrap();
    stemma_hub::lease::release(&hub.state, None, stemma_hub::db::now())
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT state FROM work_leases WHERE id=?")
        .bind(spec2.lease.id)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(status, "failed");
}
