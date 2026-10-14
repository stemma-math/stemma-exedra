//! The hub and its workers, end to end, in the test's process: a hub on a
//! local port, workers connected to it, and runners that are fakes of
//! `claude` and `codex` and a mock of an OpenAI-compatible API, so that no
//! model is ever called.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use stemma_core::api::NewAccount;
use stemma_worker::{Context, WorkerConfig, connection};
use tokio::task::JoinHandle;

/// A hub serving on a local port, over a database in a folder.
struct TestHub {
    url: String,
    state: stemma_hub::AppState,
    server: JoinHandle<()>,
    data: tempfile::TempDir,
}

impl TestHub {
    async fn start() -> TestHub {
        let data = tempfile::tempdir().unwrap();
        let (url, state, server) = serve(data.path()).await;
        TestHub {
            url,
            state,
            server,
            data,
        }
    }

    /// Stops the hub and starts it again on the same data, as after a
    /// restart: what was running is queued again.
    async fn restart(&mut self) {
        self.server.abort();
        let (url, state, server) = serve(self.data.path()).await;
        stemma_hub::jobs::recover(&state).await.unwrap();
        self.url = url;
        self.state = state;
        self.server = server;
    }
}

async fn serve(dir: &Path) -> (String, stemma_hub::AppState, JoinHandle<()>) {
    let config = stemma_hub::Config {
        data_dir: dir.to_owned(),
        ..Default::default()
    };
    let db = stemma_hub::db::open(&config.database_path()).await.unwrap();
    let state = Arc::new(stemma_hub::Hub::new(db, config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = stemma_hub::app(state.clone());
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (url, state, server)
}

/// A person, with an API token.
#[derive(Clone)]
struct Person {
    token: String,
}

impl Person {
    async fn new(hub: &TestHub, username: &str, admin: bool) -> Person {
        let account = NewAccount {
            username: username.into(),
            display_name: capitalized(username),
            password: "password1".into(),
        };
        stemma_hub::api::create_user(&hub.state, account, admin)
            .await
            .unwrap();
        let mut client = stemma_client::Hub::new(&hub.url, None).unwrap();
        client.sign_in(username, "password1").await.unwrap();
        let token = client.create_token("tests").await.unwrap().secret;
        Person { token }
    }

    async fn call(&self, hub: &TestHub, method: &str, path: &str, body: Option<Value>) -> Value {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap();
        let mut request = reqwest::Client::new()
            .request(method, format!("{}/api/v1{path}", hub.url))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let value = response.json().await.unwrap_or(Value::Null);
        assert!(status.is_success(), "{path}: {status} {value}");
        value
    }

    async fn get(&self, hub: &TestHub, path: &str) -> Value {
        self.call(hub, "GET", path, None).await
    }

    async fn post(&self, hub: &TestHub, path: &str, body: Value) -> Value {
        self.call(hub, "POST", path, Some(body)).await
    }
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Ada, who administers the hub and the group `geometry`, and Bob, a member.
async fn group(hub: &TestHub) -> (Person, Person) {
    let ada = Person::new(hub, "ada", true).await;
    ada.post(
        hub,
        "/groups",
        json!({"slug": "geometry", "name": "Geometry"}),
    )
    .await;
    let bob = Person::new(hub, "bob", false).await;
    let invitation = ada
        .post(hub, "/groups/geometry/invitations", json!({}))
        .await;
    bob.post(
        hub,
        &format!(
            "/invitations/{}/accept",
            invitation["token"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    (ada, bob)
}

/// Waits, up to a while, for `check` to give something.
async fn eventually<T, F: Future<Output = Option<T>>>(
    what: &str,
    mut check: impl FnMut() -> F,
) -> T {
    for _ in 0..200 {
        if let Some(found) = check().await {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("waited too long for {what}");
}

/// A fake harness: a shell script that records how it was called.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!("#!/bin/sh\nHERE=\"{}\"\n{body}", dir.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

/// A fake `claude -p --output-format stream-json`, which answers after
/// `delay` seconds.
fn fake_claude(dir: &Path, delay: u32) -> PathBuf {
    script(
        dir,
        "claude",
        &format!(
            r#"printf '%s\n' "$@" > "$HERE/claude.args"
cat > "$HERE/claude.stdin"
cp mcp.json "$HERE/mcp.json"
echo '{{"type":"system","subtype":"init"}}'
echo '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"mcp__stemma__read_thread","input":{{"address":"t1#1"}}}}]}}}}'
sleep {delay}
echo '{{"type":"result","subtype":"success","is_error":false,"result":"It is proper: see [[t1#1]]."}}'
"#
        ),
    )
}

/// A fake `codex exec`, which writes its answer where it is told.
fn fake_codex(dir: &Path) -> PathBuf {
    script(
        dir,
        "codex",
        r#"printf '%s\n' "$@" > "$HERE/codex.args"
cat > "$HERE/codex.stdin"
out=""; previous=""
for arg in "$@"; do
  if [ "$previous" = "--output-last-message" ]; then out="$arg"; fi
  previous="$arg"
done
echo '{"type":"item.started","item":{"type":"mcp_tool_call","server":"stemma","tool":"search","arguments":{"query":"gluing"}}}'
printf 'Codex agrees.' > "$out"
"#,
    )
}

/// A worker with `config` (TOML, without hub and token), running until
/// the handle is aborted.
fn worker(hub: &TestHub, token: &str, work: &Path, config: &str) -> JoinHandle<anyhow::Result<()>> {
    let config: WorkerConfig = toml::from_str(&format!(
        "hub = {:?}\ntoken = {:?}\nwork_dir = {:?}\n{config}",
        hub.url,
        token,
        work.display().to_string()
    ))
    .unwrap();
    let context = Context::new(config, "stemma".into()).unwrap();
    tokio::spawn(connection::serve(Arc::new(context), token.to_owned()))
}

async fn worker_token(hub: &TestHub, person: &Person, path: &str) -> String {
    person.post(hub, path, json!({"name": "test"})).await["secret"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn wait_online(hub: &TestHub, person: &Person, path: &str) {
    eventually("the worker to be online", || async {
        let workers = person.get(hub, path).await;
        (workers[0]["online"] == true).then_some(())
    })
    .await;
}

async fn answer(hub: &TestHub, person: &Person, thread: u32, number: u32) -> Value {
    eventually(&format!("message t{thread}#{number}"), || async {
        let messages = person
            .get(
                hub,
                &format!("/groups/geometry/threads/{thread}/messages?from={number}&to={number}"),
            )
            .await;
        messages.as_array().unwrap().first().cloned()
    })
    .await
}

async fn jobs(hub: &TestHub, person: &Person, thread: u32) -> Value {
    person
        .get(hub, &format!("/groups/geometry/threads/{thread}/jobs"))
        .await
}

#[tokio::test]
async fn a_members_claude_answers_a_mention() {
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let tools = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let claude = fake_claude(tools.path(), 0);
    let token = worker_token(&hub, &ada, "/me/workers").await;
    let running = worker(
        &hub,
        &token,
        work.path(),
        &format!(
            "[runners.claude-code]\ncommand = {:?}\n",
            claude.display().to_string()
        ),
    );
    wait_online(&hub, &ada, "/me/workers").await;
    let workers = ada.get(&hub, "/me/workers").await;
    assert_eq!(workers[0]["runners"], json!(["claude-code"]));

    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@claude is the gluing map proper?"}),
    )
    .await;
    let message = answer(&hub, &ada, 1, 2).await;
    assert_eq!(message["agent"], "Claude");
    assert_eq!(message["author"]["username"], "ada");
    assert_eq!(message["reply_to"], 1);
    assert_eq!(message["body"], "It is proper: see [[t1#1]].");
    eventually("the job to be over", || async {
        jobs(&hub, &ada, 1)
            .await
            .as_array()
            .unwrap()
            .is_empty()
            .then_some(())
    })
    .await;

    // How Claude Code was run: headless, with only Stemma's tools, read-only.
    let args = std::fs::read_to_string(tools.path().join("claude.args")).unwrap();
    for expected in ["-p", "--strict-mcp-config", "--permission-mode", "dontAsk"] {
        assert!(args.lines().any(|l| l == expected), "{expected} in {args}");
    }
    assert!(args.contains("mcp__stemma__read_thread"));
    assert!(args.contains("You are Claude, an agent in Stemma"));
    let input = std::fs::read_to_string(tools.path().join("claude.stdin")).unwrap();
    assert!(input.contains("### t1#1 · Ada · "), "{input}");
    assert!(
        input.contains("Answer message t1#1, by Ada (@ada)"),
        "{input}"
    );
    let mcp: Value =
        serde_json::from_str(&std::fs::read_to_string(tools.path().join("mcp.json")).unwrap())
            .unwrap();
    let mcp_args = mcp["mcpServers"]["stemma"]["args"].as_array().unwrap();
    assert!(mcp_args.contains(&json!("--read-only")));
    assert!(mcp_args.contains(&json!("geometry")));
    // The job's folder, with its token, is gone.
    assert_eq!(
        std::fs::read_dir(work.path().join("jobs")).unwrap().count(),
        0
    );
    running.abort();
}

#[tokio::test]
async fn a_personal_agent_waits_for_its_requesters_worker() {
    let hub = TestHub::start().await;
    let (ada, bob) = group(&hub).await;
    let tools = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let config = format!(
        "[runners.claude-code]\ncommand = {:?}\n",
        fake_claude(tools.path(), 0).display().to_string()
    );
    let adas = worker(
        &hub,
        &worker_token(&hub, &ada, "/me/workers").await,
        work.path(),
        &config,
    );
    wait_online(&hub, &ada, "/me/workers").await;

    bob.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Bob asks", "body": "@claude, what is a scheme?"}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let waiting = jobs(&hub, &bob, 1).await;
    assert_eq!(waiting[0]["state"], "queued", "{waiting}");
    assert_eq!(
        waiting[0]["note"],
        "Waiting for Bob's worker, with Claude Code, to come online"
    );

    let bobs = worker(
        &hub,
        &worker_token(&hub, &bob, "/me/workers").await,
        work.path(),
        &config,
    );
    let message = answer(&hub, &bob, 1, 2).await;
    assert_eq!(message["author"]["username"], "bob");
    assert_eq!(message["agent"], "Claude");
    adas.abort();
    bobs.abort();
}

#[tokio::test]
async fn jobs_outlive_their_workers_and_the_hub() {
    let mut hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let tools = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let slow = format!(
        "[runners.claude-code]\ncommand = {:?}\n",
        fake_claude(tools.path(), 30).display().to_string()
    );
    let token = worker_token(&hub, &ada, "/me/workers").await;

    // The worker goes away while it runs a job: the job is queued again.
    let first = worker(&hub, &token, work.path(), &slow);
    wait_online(&hub, &ada, "/me/workers").await;
    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@claude, a long question"}),
    )
    .await;
    eventually("the job to run", || async {
        let jobs = jobs(&hub, &ada, 1).await;
        (jobs[0]["state"] == "running").then_some(())
    })
    .await;
    first.abort();
    eventually("the job to be queued again", || async {
        let jobs = jobs(&hub, &ada, 1).await;
        (jobs[0]["state"] == "queued").then_some(())
    })
    .await;

    // It runs again on the next worker, and then the hub restarts.
    let second = worker(&hub, &token, work.path(), &slow);
    eventually("the job to run again", || async {
        let jobs = jobs(&hub, &ada, 1).await;
        (jobs[0]["state"] == "running").then_some(())
    })
    .await;
    hub.restart().await;
    second.abort();
    let jobs_now = jobs(&hub, &ada, 1).await;
    assert_eq!(jobs_now[0]["state"], "queued", "{jobs_now}");

    // A worker of the restarted hub finishes it.
    let fast = format!(
        "[runners.claude-code]\ncommand = {:?}\n",
        fake_claude(tools.path(), 0).display().to_string()
    );
    let third = worker(&hub, &token, work.path(), &fast);
    let message = answer(&hub, &ada, 1, 2).await;
    assert_eq!(message["agent"], "Claude");
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM jobs")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    assert_eq!(attempts, 3);
    third.abort();
}

#[tokio::test]
async fn a_chat_with_codex_answers_every_message() {
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let tools = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let codex = fake_codex(tools.path());
    let running = worker(
        &hub,
        &worker_token(&hub, &ada, "/me/workers").await,
        work.path(),
        &format!(
            "[runners.codex]\ncommand = {:?}\n",
            codex.display().to_string()
        ),
    );
    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "With Codex", "kind": "chat", "resident_agent": "codex", "body": "Hello"}),
    )
    .await;
    let first = answer(&hub, &ada, 1, 2).await;
    assert_eq!(first["agent"], "Codex");
    assert_eq!(first["body"], "Codex agrees.");
    ada.post(
        &hub,
        "/groups/geometry/threads/1/messages",
        json!({"body": "And now?"}),
    )
    .await;
    let second = answer(&hub, &ada, 1, 4).await;
    assert_eq!(second["reply_to"], 3);

    let args = std::fs::read_to_string(tools.path().join("codex.args")).unwrap();
    assert!(args.starts_with("exec\n"), "{args}");
    assert!(args.contains("read-only"));
    assert!(args.contains("mcp_servers.stemma.args=["));
    let input = std::fs::read_to_string(tools.path().join("codex.stdin")).unwrap();
    assert!(input.starts_with("You are Codex, an agent in Stemma"));
    running.abort();
}

/// What the mock model saw.
type Seen = Arc<Mutex<Vec<Value>>>;

/// A mock of an OpenAI-compatible chat API: it first reads the thread with
/// a tool, then answers with what it read.
async fn mock_model(State(seen): State<Seen>, Json(body): Json<Value>) -> Json<Value> {
    seen.lock().unwrap().push(body.clone());
    let messages = body["messages"].as_array().unwrap();
    let read = messages.iter().find(|m| m["role"] == "tool");
    match read {
        None => Json(json!({
            "choices": [{"message": {"role": "assistant", "content": null, "tool_calls": [{
                "id": "call_1", "type": "function",
                "function": {"name": "read_thread", "arguments": "{\"address\":\"t1#1\"}"}
            }]}}],
            "usage": {"total_tokens": 100}
        })),
        Some(tool) => {
            let first_line = tool["content"].as_str().unwrap().lines().next().unwrap();
            Json(json!({
                "choices": [{"message": {"role": "assistant",
                    "content": format!("<think>reading</think>I read {first_line}.")}}],
                "usage": {"total_tokens": 100}
            }))
        }
    }
}

#[tokio::test]
async fn the_api_harness_uses_the_tools() {
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let seen: Seen = Arc::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/v1/chat/completions", axum::routing::post(mock_model))
        .with_state(seen.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    ada.post(
        &hub,
        "/groups/geometry/agents",
        json!({"handle": "qwen", "display_name": "Qwen", "runner": "api",
               "endpoint": "mock", "scope": "group"}),
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let token = worker_token(&hub, &ada, "/groups/geometry/workers").await;
    let running = worker(
        &hub,
        &token,
        work.path(),
        &format!(
            "[runners.api]\nmax_steps = 4\n[endpoints.mock]\nurl = {model_url:?}\nmodel = \"mock-model\"\n"
        ),
    );
    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@qwen, what does this say?"}),
    )
    .await;
    let message = answer(&hub, &ada, 1, 2).await;
    assert_eq!(message["agent"], "Qwen");
    assert!(
        message["body"]
            .as_str()
            .unwrap()
            .starts_with("I read ### t1#1 · Ada · "),
        "{message}"
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0]["model"], "mock-model");
    let tools: Vec<&str> = seen[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["list_threads", "read_thread", "search"]);
    running.abort();
}

#[tokio::test]
async fn a_revoked_worker_stops() {
    let hub = TestHub::start().await;
    let work = tempfile::tempdir().unwrap();
    let running = worker(
        &hub,
        "stw_not-a-token",
        work.path(),
        "[runners.claude-code]\n",
    );
    let ended = tokio::time::timeout(Duration::from_secs(10), running)
        .await
        .expect("the worker stops")
        .unwrap();
    let error = ended.unwrap_err().to_string();
    assert!(
        error.contains("does not accept this worker's token"),
        "{error}"
    );
}
