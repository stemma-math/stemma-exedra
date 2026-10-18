//! The workshop over the API: its machines and their agents, routing by
//! preference and daily limit, consumption, architectures, and approving
//! its changes, which the stand-in forge merges or closes.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use stemma_core::protocol::{HubMessage, JobSpec, Outcome, TaskEnd, TaskOutcome, Usage, Work};
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};
use stemma_hub::{Config, Hub, app, db};
use tokio::sync::mpsc::UnboundedReceiver;
use tower::ServiceExt;

struct TestHub {
    app: Router,
    state: stemma_hub::AppState,
    forge: FakeForge,
    _data: tempfile::TempDir,
}

/// A person, with their session.
#[derive(Clone)]
struct Client {
    cookie: Option<String>,
    token: Option<String>,
    id: i64,
}

struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

impl TestHub {
    async fn new() -> TestHub {
        let forge = FakeForge::start().await;
        let data = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let config = Config {
            data_dir: data.path().to_owned(),
            public_url: Some(url),
            forge: Some(ForgeConfig::new(
                &forge.url,
                "http://forge.test/user/oauth2/stemma/callback",
                ForgeSecrets {
                    token: stemma_fake_forge::TOKEN.into(),
                    webhook_secret: "the webhook's secret".into(),
                    client_secret: "the client's secret".into(),
                },
            )),
            ..Config::default()
        };
        let hub = Arc::new(Hub::new(db::in_memory().await.unwrap(), config).unwrap());
        *hub.setup_token.lock().unwrap() = Some("setup-secret".into());
        // Served too, for the forge's webhooks.
        let served = app(hub.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, served).await;
        });
        TestHub {
            app: app(hub.clone()),
            state: hub,
            forge,
            _data: data,
        }
    }

    async fn call(
        &self,
        client: &Client,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Reply {
        let builder = Request::builder()
            .method(method)
            .uri(format!("/api/v1{path}"));
        let mut request = match body {
            Some(body) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => builder.body(Body::empty()),
        }
        .unwrap();
        if let Some(cookie) = &client.cookie {
            request
                .headers_mut()
                .insert(header::COOKIE, cookie.parse().unwrap());
        }
        if let Some(token) = &client.token {
            request.headers_mut().insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            body: body.to_vec(),
        }
    }

    async fn get(&self, client: &Client, path: &str) -> Value {
        let reply = self.call(client, Method::GET, path, None).await;
        assert!(reply.status.is_success(), "{path}: {}", reply.status);
        reply.json()
    }

    async fn post(&self, client: &Client, path: &str, body: Value) -> Reply {
        self.call(client, Method::POST, path, Some(body)).await
    }

    async fn signed_in(&self, reply: Reply, cookie: &str) -> Client {
        let client = Client {
            cookie: Some(cookie.to_owned()),
            token: None,
            id: 0,
        };
        let _ = reply;
        let id = self.get(&client, "/me").await["id"].as_i64().unwrap();
        Client { id, ..client }
    }

    /// Ada, who administers the hub and `geometry`.
    async fn ada(&self) -> Client {
        let request = Request::post("/api/v1/setup")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"token": "setup-secret", "username": "ada", "display_name": "Ada",
                       "password": "analytical"})
                .to_string(),
            ))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let cookie = cookie_of(&response);
        let ada = self
            .signed_in(
                Reply {
                    status: response.status(),
                    body: Vec::new(),
                },
                &cookie,
            )
            .await;
        let reply = self
            .post(
                &ada,
                "/groups",
                json!({"slug": "geometry", "name": "Geometry"}),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED);
        ada
    }

    /// Someone new in `geometry` with `role`.
    async fn join(&self, admin: &Client, username: &str, role: &str) -> Client {
        let reply = self
            .post(
                admin,
                "/groups/geometry/invitations",
                json!({"max_uses": 1, "role": role}),
            )
            .await;
        let token = reply.json()["token"].as_str().unwrap().to_owned();
        let request = Request::post(format!("/api/v1/invitations/{token}/accept"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"account": {"username": username, "display_name": username,
                                   "password": "password1"}})
                .to_string(),
            ))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = cookie_of(&response);
        self.signed_in(
            Reply {
                status: StatusCode::OK,
                body: Vec::new(),
            },
            &cookie,
        )
        .await
    }

    /// A machine of `owner`'s, which connected once and said it isolates
    /// the workshop's jobs in containers.
    async fn machine(&self, owner: &Client, name: &str) -> i64 {
        let id = self.unseen_machine(owner, name).await;
        self.reported(id, Some("docker")).await;
        id
    }

    /// A machine of `owner`'s, which never connected.
    async fn unseen_machine(&self, owner: &Client, name: &str) -> i64 {
        let reply = self
            .post(owner, "/me/machines", json!({"name": name}))
            .await;
        assert_eq!(reply.status, StatusCode::CREATED);
        reply.json()["id"].as_i64().unwrap()
    }

    /// What a machine said, as if it connected: Lean, and `isolation`.
    async fn reported(&self, id: i64, isolation: Option<&str>) {
        sqlx::query("UPDATE workers SET last_seen_at = 1, lean = 1, isolation = ? WHERE id = ?")
            .bind(isolation)
            .bind(id)
            .execute(&self.state.db)
            .await
            .unwrap();
    }

    /// Connects a machine, as if over its WebSocket, with Lean and
    /// `runners`: what the hub sends it arrives on the receiver.
    async fn connect(
        &self,
        id: i64,
        owner_id: i64,
        runners: &[&str],
    ) -> UnboundedReceiver<HubMessage> {
        self.connect_isolated(
            id,
            owner_id,
            runners,
            Some(stemma_core::api::Isolation::Docker),
        )
        .await
    }

    /// Connects a machine, as [`TestHub::connect`], isolating as said.
    async fn connect_isolated(
        &self,
        id: i64,
        owner_id: i64,
        runners: &[&str],
        isolation: Option<stemma_core::api::Isolation>,
    ) -> UnboundedReceiver<HubMessage> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let workshop = stemma_hub::workshop::machines::groups_of(&self.state, id)
            .await
            .unwrap();
        stemma_hub::jobs::connected(
            &self.state,
            id,
            stemma_hub::jobs::Connected {
                connection: id as u64,
                owner_id,
                runners: runners.iter().map(|r| (*r).to_owned()).collect(),
                lean: true,
                latex: true,
                isolation,
                workshop,
                concurrency: 4,
                preferred: false,
                running: Default::default(),
                sender,
            },
        )
        .await
        .unwrap();
        receiver
    }
}

fn cookie_of(response: &axum::response::Response) -> String {
    response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

/// The jobs a machine was sent, in order, its builds aside.
fn tasks_sent(receiver: &mut UnboundedReceiver<HubMessage>) -> Vec<JobSpec> {
    let mut sent = Vec::new();
    while let Ok(message) = receiver.try_recv() {
        if let HubMessage::Job(spec) = message
            && matches!(spec.work, Work::TaskRun(_))
        {
            sent.push(*spec);
        }
    }
    sent
}

fn usage(input: u64, cost: Option<f64>) -> Usage {
    Usage {
        input_tokens: Some(input),
        output_tokens: Some(10),
        cache_read_tokens: None,
        cache_write_tokens: None,
        cost_usd: cost,
    }
}

/// A task's run ends as a machine says.
async fn finish(hub: &TestHub, worker: i64, job: i64, commit: Option<String>, usage: Usage) {
    let outcome = Outcome::Task(Box::new(TaskOutcome {
        end: TaskEnd::Done,
        reason: None,
        report: "Added [[lean:Geometry.new]].".into(),
        commit,
        log: "Round 1".into(),
        built: None,
        runner: "claude-code".into(),
        model: None,
    }));
    stemma_hub::jobs::finished(&hub.state, worker, job, outcome, usage)
        .await
        .unwrap();
}

/// Bob asks the workshop for a task in thread 1.
async fn ask(hub: &TestHub, bob: &Client, title: &str) -> u32 {
    let reply = hub
        .post(
            bob,
            "/groups/geometry/tasks",
            json!({"title": title, "instructions": "As t1#1 says.", "origin": "t1#1"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    reply.json()["number"].as_u64().unwrap() as u32
}

#[tokio::test]
async fn admins_add_their_own_machines_to_the_workshop() {
    let hub = TestHub::new().await;
    let ada = hub.ada().await;
    let bob = hub.join(&ada, "bob", "member").await;
    let adas = hub.machine(&ada, "server").await;
    let bobs = hub.machine(&bob, "laptop").await;

    // Admins only, their own machines only.
    let reply = hub
        .post(
            &bob,
            "/groups/geometry/workshop/machines",
            json!({"machine": bobs}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/workshop/machines",
            json!({"machine": bobs}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/workshop/machines",
            json!({"machine": adas}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/workshop/machines",
            json!({"machine": adas}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);

    // Everyone in the group sees them.
    let machines = hub.get(&bob, "/groups/geometry/workshop/machines").await;
    assert_eq!(machines[0]["name"], "server");
    assert_eq!(machines[0]["owner"]["username"], "ada");
    assert_eq!(machines[0]["online"], false);
    assert_eq!(machines[0]["isolation"], "docker");
    let mine = hub.get(&ada, "/me/machines").await;
    assert_eq!(mine[0]["workshop"], json!(["geometry"]));

    // Only the owner chooses its agents, in order.
    let agents = json!([
        {"runner": "claude-code", "model": "opus", "daily_jobs": 3},
        {"runner": "opencode", "daily_minutes": 60, "args": ["--variant", "high"]}
    ]);
    let path = format!("/me/machines/{adas}/workshop-agents");
    let reply = hub
        .call(&bob, Method::PUT, &path, Some(agents.clone()))
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    let reply = hub.call(&ada, Method::PUT, &path, Some(agents)).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    let kept = reply.json();
    assert_eq!(kept[0]["runner"], "claude-code");
    assert_eq!(kept[0]["today"]["jobs"], 0);
    assert_eq!(kept[1]["args"], json!(["--variant", "high"]));
    // Reordered, by id, they are the same agents.
    let first = kept[0]["id"].as_i64().unwrap();
    let reordered = json!([kept[1].clone(), kept[0].clone()]);
    let reply = hub.call(&ada, Method::PUT, &path, Some(reordered)).await;
    assert_eq!(reply.json()[1]["id"], first);
    for wrong in [
        json!([{"runner": "nothing"}]),
        json!([{"runner": "codex", "daily_jobs": 0}]),
        json!([{"id": 999, "runner": "codex"}]),
    ] {
        let reply = hub
            .call(&ada, Method::PUT, &path, Some(wrong.clone()))
            .await;
        assert!(reply.status.is_client_error(), "{wrong}");
    }
    let machines = hub.get(&bob, "/groups/geometry/workshop/machines").await;
    assert_eq!(machines[0]["agents"][0]["runner"], "opencode");

    // An admin, or its owner, takes it away.
    let path = format!("/groups/geometry/workshop/machines/{adas}");
    let reply = hub.call(&bob, Method::DELETE, &path, None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub.call(&ada, Method::DELETE, &path, None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let machines = hub.get(&bob, "/groups/geometry/workshop/machines").await;
    assert!(machines.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn machines_that_do_not_isolate_are_not_lent_unless_their_owner_chose_so() {
    let hub = TestHub::new().await;
    let ada = hub.ada().await;
    let lend = |id: i64| {
        let hub = &hub;
        let ada = &ada;
        async move {
            hub.post(
                ada,
                "/groups/geometry/workshop/machines",
                json!({"machine": id}),
            )
            .await
        }
    };

    // Never connected: how it isolates is not known.
    let server = hub.unseen_machine(&ada, "server").await;
    let reply = lend(server).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(
        reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("has not connected yet"),
        "{:?}",
        reply.json()
    );

    // Connected without a container runtime, and no choice of its owner's.
    hub.reported(server, None).await;
    let reply = lend(server).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert!(
        reply.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("cannot isolate"),
        "{:?}",
        reply.json()
    );

    // Its owner chose to run them on it, unisolated: the group sees so.
    hub.reported(server, Some("host")).await;
    assert_eq!(lend(server).await.status, StatusCode::CREATED);
    let machines = hub.get(&ada, "/groups/geometry/workshop/machines").await;
    assert_eq!(machines[0]["isolation"], "host");

    // A machine that stops isolating, lent already, gets no task.
    hub.reported(server, None).await;
    let mut sent = hub
        .connect_isolated(server, ada.id, &["claude-code"], None)
        .await;
    hub.call(
        &ada,
        Method::PUT,
        &format!("/me/machines/{server}/workshop-agents"),
        Some(json!([{"runner": "claude-code"}])),
    )
    .await;
    hub.post(
        &ada,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "The charts glue."}),
    )
    .await;
    ask(&hub, &ada, "First").await;
    assert!(tasks_sent(&mut sent).is_empty());
}

#[tokio::test]
async fn the_architectures_are_listed() {
    let hub = TestHub::new().await;
    let ada = hub.ada().await;
    let architectures = hub.get(&ada, "/workshop/architectures").await;
    let formalize = architectures
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "formalize")
        .unwrap();
    assert_eq!(formalize["works_on"], "library");
    assert_eq!(formalize["steps"][0]["name"], "work");
    assert!(
        formalize["big"]
            .as_array()
            .unwrap()
            .contains(&json!("lean-toolchain"))
    );
    // A task names one the workshop knows.
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/threads",
            json!({"title": "Gluing", "body": "The charts glue."}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let reply = hub
        .post(
            &ada,
            "/groups/geometry/tasks",
            json!({"title": "X", "instructions": "Y", "origin": "t1#1",
                   "architecture": "paint"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn tasks_go_to_the_first_agent_under_its_daily_limit_and_are_measured() {
    let hub = TestHub::new().await;
    let ada = hub.ada().await;
    let bob = hub.join(&ada, "bob", "member").await;
    hub.post(
        &bob,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "The charts glue."}),
    )
    .await;
    let server = hub.machine(&ada, "server").await;
    hub.post(
        &ada,
        "/groups/geometry/workshop/machines",
        json!({"machine": server}),
    )
    .await;
    hub.call(
        &ada,
        Method::PUT,
        &format!("/me/machines/{server}/workshop-agents"),
        Some(json!([
            {"runner": "codex"},
            {"runner": "claude-code", "model": "opus", "daily_jobs": 1},
            {"runner": "opencode", "model": "qwen"}
        ])),
    )
    .await;

    // Without a machine, a task waits, and says why.
    ask(&hub, &bob, "First").await;
    let task = hub.get(&bob, "/groups/geometry/tasks/1").await;
    assert_eq!(
        task["progress"],
        "Waiting for a workshop machine with Lean to come online"
    );

    // The machine has no Codex: its first agent it can run is Claude Code.
    let mut sent = hub
        .connect(server, ada.id, &["claude-code", "opencode"])
        .await;
    let first = tasks_sent(&mut sent);
    assert_eq!(first.len(), 1);
    let Work::TaskRun(run) = &first[0].work else {
        unreachable!()
    };
    assert_eq!(run.conversation.agent.runner, "claude-code");
    assert_eq!(run.conversation.agent.model.as_deref(), Some("opus"));
    assert_eq!(run.architecture.name, "formalize");
    assert!(run.architecture.instructions.contains("open_thread"));
    assert_eq!(run.branch, "workshop/w1");
    assert!(first[0].opens_threads);
    let task = hub.get(&bob, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "running");
    assert_eq!(task["runner"], "claude-code");

    // What Claude Code said it consumed is kept.
    let claude = stemma_worker::runners::claude_code::usage_of_output(include_str!(
        "../../stemma-worker/tests/fixtures/claude-code-result.json"
    ));
    assert_eq!(claude.cost_usd, Some(0.274_381));
    finish(&hub, server, first[0].id, None, claude).await;
    let task = hub.get(&bob, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "done");

    // Claude Code ran its one job today: the next goes to OpenCode.
    ask(&hub, &bob, "Second").await;
    let second = tasks_sent(&mut sent);
    let Work::TaskRun(run) = &second[0].work else {
        unreachable!()
    };
    assert_eq!(run.conversation.agent.runner, "opencode");
    let opencode = stemma_worker::runners::opencode::usage_of_output(include_str!(
        "../../stemma-worker/tests/fixtures/opencode-events.jsonl"
    ));
    finish(&hub, server, second[0].id, None, opencode).await;

    // With no agent under its limit, a task waits, and says why. An agent
    // keeps what it did today when its owner changes it.
    let machines = hub.get(&ada, "/me/machines").await;
    let claude = machines[0]["workshop_agents"][1]["id"].clone();
    hub.call(
        &ada,
        Method::PUT,
        &format!("/me/machines/{server}/workshop-agents"),
        Some(json!([{"id": claude, "runner": "claude-code", "daily_jobs": 1}])),
    )
    .await;
    ask(&hub, &bob, "Third").await;
    assert!(tasks_sent(&mut sent).is_empty());
    let task = hub.get(&bob, "/groups/geometry/tasks/3").await;
    assert_eq!(
        task["progress"],
        "Waiting for a workshop agent under its daily limit"
    );

    // Codex's tokens, from its events, are kept too.
    let codex = stemma_worker::runners::codex::usage_of_output(include_str!(
        "../../stemma-worker/tests/fixtures/codex-events.jsonl"
    ));
    assert_eq!(codex.input_tokens, Some(4_100));
    assert_eq!(codex.cost_usd, None);

    // The panel's days add it up, per machine, runner and model: the
    // tasks' runs and the library's build, which the machine started too.
    let days = hub.get(&bob, "/groups/geometry/workshop/consumption").await;
    let today = &days[0];
    assert_eq!(today["jobs"], 3);
    assert_eq!(today["input_tokens"], 1_204 + 3_000);
    assert_eq!(today["output_tokens"], 3_512 + 700);
    let cost = today["cost_usd"].as_f64().unwrap();
    assert!((cost - (0.274_381 + 0.0123)).abs() < 1e-9, "{cost}");
    let rows = today["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1]["runner"], "lean");
    assert_eq!(rows[0]["machine"], "server");
    assert_eq!(rows[0]["runner"], "claude-code");
    assert_eq!(rows[0]["model"], "opus");
    let machines = hub.get(&ada, "/me/machines").await;
    assert_eq!(machines[0]["workshop_agents"][0]["today"]["jobs"], 1);
    let runs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM job_runs WHERE kind = 'task_run' AND finished_at IS NOT NULL",
    )
    .fetch_one(&hub.state.db)
    .await
    .unwrap();
    assert_eq!(runs, 2);
}

#[tokio::test]
async fn changes_go_in_once_approved() {
    let hub = TestHub::new().await;
    let ada = hub.ada().await;
    let bob = hub.join(&ada, "bob", "member").await;
    let cleo = hub.join(&ada, "cleo", "member").await;
    let gus = hub.join(&ada, "gus", "guest").await;
    hub.post(
        &bob,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "The charts glue."}),
    )
    .await;
    let server = hub.machine(&ada, "server").await;
    hub.post(
        &ada,
        "/groups/geometry/workshop/machines",
        json!({"machine": server}),
    )
    .await;
    hub.call(
        &ada,
        Method::PUT,
        &format!("/me/machines/{server}/workshop-agents"),
        Some(json!([{"runner": "claude-code"}])),
    )
    .await;
    let mut sent = hub.connect(server, ada.id, &["claude-code"]).await;

    // A small change: Bob, who asked, approves it alone.
    let number = ask(&hub, &bob, "Add a lemma").await;
    let job = tasks_sent(&mut sent).remove(0);
    let commit = hub
        .forge
        .push_branch(
            "geometry",
            "library",
            "workshop/w1",
            &[("Geometry/New.lean", "theorem new : True := trivial\n")],
            &[],
        )
        .await;
    finish(&hub, server, job.id, Some(commit.clone()), usage(5, None)).await;
    let task = hub
        .get(&bob, &format!("/groups/geometry/tasks/{number}"))
        .await;
    assert_eq!(task["state"], "review", "{task}");
    let changes = &task["changes"];
    assert_eq!(changes["branch"], "workshop/w1");
    assert_eq!(changes["commit"], commit.as_str());
    assert_eq!(changes["big"], false);
    assert_eq!(changes["needs"], json!(["asker"]));
    assert!(
        changes["url"]
            .as_str()
            .unwrap()
            .ends_with("/geometry/library/pulls/1/files")
    );
    let pulls = hub.forge.pulls("geometry", "library");
    assert_eq!(pulls[0].head, "workshop/w1");
    assert!(pulls[0].open);
    // The thread says it is ready, and who approves it.
    let messages = hub.get(&bob, "/groups/geometry/threads/1/messages").await;
    let report = messages.as_array().unwrap().last().unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        report.starts_with("**Ready for review:** [[w1]]"),
        "{report}"
    );
    assert!(report.contains("once bob approves them"), "{report}");

    // Guests never approve; nor others, for a small change; nor a job.
    let path = format!("/groups/geometry/tasks/{number}/approve");
    for (who, status) in [
        (&gus, StatusCode::FORBIDDEN),
        (&cleo, StatusCode::FORBIDDEN),
        (&ada, StatusCode::FORBIDDEN),
    ] {
        let reply = hub.call(who, Method::POST, &path, None).await;
        assert_eq!(reply.status, status);
    }
    let job_token = Client {
        cookie: None,
        token: Some(job.token.clone()),
        id: bob.id,
    };
    let reply = hub.call(&job_token, Method::POST, &path, None).await;
    assert!(reply.status.is_client_error(), "{}", reply.status);
    let reply = hub.call(&bob, Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.json());
    let task = hub
        .get(&bob, &format!("/groups/geometry/tasks/{number}"))
        .await;
    assert_eq!(task["state"], "done");
    assert_eq!(task["changes"]["approvals"][0]["approver"], "asker");
    let merged = task["changes"]["merged"].as_str().unwrap().to_owned();
    // The forge merged it into main, and deleted the branch.
    let pulls = hub.forge.pulls("geometry", "library");
    assert_eq!(pulls[0].merged.as_deref(), Some(merged.as_str()));
    assert_eq!(
        hub.forge.branch_commit("geometry", "library", "main").await,
        Some(merged.clone())
    );
    assert!(
        hub.forge
            .file("geometry", "library", "main", "Geometry/New.lean")
            .await
            .is_some()
    );
    assert!(
        hub.forge
            .branch_commit("geometry", "library", "workshop/w1")
            .await
            .is_none()
    );
    // The push to main is built.
    let mut built = false;
    for _ in 0..100 {
        let builds = hub.get(&bob, "/groups/geometry/library/builds").await;
        if builds[0]["commit"] == merged.as_str() {
            built = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(built, "main's new commit is built");
    let reply = hub.call(&bob, Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::CONFLICT);

    // A big change, one that changes the toolchain and deletes a file,
    // also needs an admin.
    let number = ask(&hub, &bob, "Update Lean").await;
    let job = tasks_sent(&mut sent).remove(0);
    let commit = hub
        .forge
        .push_branch(
            "geometry",
            "library",
            "workshop/w2",
            &[("lean-toolchain", "leanprover/lean4:v4.36.0\n")],
            &["Geometry/New.lean"],
        )
        .await;
    finish(&hub, server, job.id, Some(commit), usage(5, None)).await;
    let task = hub
        .get(&bob, &format!("/groups/geometry/tasks/{number}"))
        .await;
    assert_eq!(task["changes"]["big"], true);
    assert_eq!(
        task["changes"]["why_big"],
        json!(["deletes Geometry/New.lean", "changes lean-toolchain"])
    );
    assert_eq!(task["changes"]["needs"], json!(["asker", "admin"]));
    let path = format!("/groups/geometry/tasks/{number}/approve");
    let reply = hub.call(&bob, Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let task = hub
        .get(&bob, &format!("/groups/geometry/tasks/{number}"))
        .await;
    assert_eq!(task["state"], "review");
    assert_eq!(task["changes"]["needs"], json!(["admin"]));
    assert!(hub.forge.pulls("geometry", "library")[1].open);
    let reply = hub.call(&cleo, Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let reply = hub.call(&ada, Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let task = hub
        .get(&bob, &format!("/groups/geometry/tasks/{number}"))
        .await;
    assert_eq!(task["state"], "done");
    assert_eq!(task["changes"]["approvals"].as_array().unwrap().len(), 2);

    // Rejected, by an admin: the pull request is closed, its branch gone.
    let number = ask(&hub, &bob, "Another lemma").await;
    let job = tasks_sent(&mut sent).remove(0);
    let commit = hub
        .forge
        .push_branch(
            "geometry",
            "library",
            "workshop/w3",
            &[("Geometry/Other.lean", "theorem other : True := trivial\n")],
            &[],
        )
        .await;
    finish(&hub, server, job.id, Some(commit), usage(5, None)).await;
    let path = format!("/groups/geometry/tasks/{number}/reject");
    for who in [&gus, &cleo] {
        let reply = hub.call(who, Method::POST, &path, None).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }
    let reply = hub.call(&ada, Method::POST, &path, None).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let task = hub
        .get(&bob, &format!("/groups/geometry/tasks/{number}"))
        .await;
    assert_eq!(task["state"], "rejected");
    assert_eq!(task["changes"]["rejected_by"]["username"], "ada");
    let pulls = hub.forge.pulls("geometry", "library");
    assert!(!pulls[2].open);
    assert!(pulls[2].merged.is_none());
    assert!(
        hub.forge
            .branch_commit("geometry", "library", "workshop/w3")
            .await
            .is_none()
    );
    let messages = hub.get(&bob, "/groups/geometry/threads/1/messages").await;
    let last = messages.as_array().unwrap().last().unwrap()["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(last.starts_with("**Rejected:** [[w3]], by Ada."), "{last}");
}

#[tokio::test]
async fn the_workshop_opens_threads_to_ask() {
    let hub = TestHub::new().await;
    let ada = hub.ada().await;
    let bob = hub.join(&ada, "bob", "member").await;
    hub.post(
        &bob,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "The charts glue."}),
    )
    .await;
    let server = hub.machine(&ada, "server").await;
    hub.post(
        &ada,
        "/groups/geometry/workshop/machines",
        json!({"machine": server}),
    )
    .await;
    hub.call(
        &ada,
        Method::PUT,
        &format!("/me/machines/{server}/workshop-agents"),
        Some(json!([{"runner": "opencode"}])),
    )
    .await;
    let mut sent = hub.connect(server, ada.id, &["opencode"]).await;
    ask(&hub, &bob, "Formalize gluing").await;
    let job = tasks_sent(&mut sent).remove(0);
    let workshop = Client {
        cookie: None,
        token: Some(job.token.clone()),
        id: bob.id,
    };
    let question = json!({"title": "Which charts?", "body": "Do [[t1#1]]'s charts overlap?"});
    let reply = hub
        .post(&workshop, "/groups/geometry/threads", question.clone())
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.json());
    let page = reply.json();
    assert_eq!(page["thread"]["number"], 2);
    assert_eq!(page["messages"][0]["agent"], "Workshop");
    assert_eq!(page["messages"][0]["author"]["username"], "bob");
    // A chat, or an empty one, it does not open.
    let reply = hub
        .post(
            &workshop,
            "/groups/geometry/threads",
            json!({"title": "Chat", "kind": "chat", "body": "Hi"}),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    // Sparingly: two at most.
    let reply = hub
        .post(&workshop, "/groups/geometry/threads", question.clone())
        .await;
    assert_eq!(reply.status, StatusCode::CREATED);
    let reply = hub
        .post(&workshop, "/groups/geometry/threads", question)
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    // The token pushes only to its task's branch, which the git tests try.
    let push_ref: Option<String> =
        sqlx::query_scalar("SELECT push_ref FROM api_tokens WHERE push_ref IS NOT NULL")
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
    assert_eq!(push_ref.as_deref(), Some("refs/heads/workshop/w1"));
}
