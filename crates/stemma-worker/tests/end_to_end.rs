//! The hub and people's machines, end to end, in the test's process: a hub
//! on a local port, machines connected to it, and runners that are fakes
//! of `claude`, `codex` and `opencode`, so that no model is ever called.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use stemma_core::api::NewAccount;
use stemma_fake_forge::FakeForge;
use stemma_hub::config::{ForgeConfig, ForgeSecrets};
use stemma_worker::session::Session;
use stemma_worker::{Context, WorkerConfig, connection};
use tokio::task::JoinHandle;

/// A hub serving on a local port, over a database in a folder, beside a
/// stand-in for its forge.
struct TestHub {
    url: String,
    state: stemma_hub::AppState,
    server: JoinHandle<()>,
    forge: FakeForge,
    data: tempfile::TempDir,
}

impl TestHub {
    async fn start() -> TestHub {
        let forge = FakeForge::start().await;
        let data = tempfile::tempdir().unwrap();
        let (url, state, server) = serve(data.path(), &forge).await;
        TestHub {
            url,
            state,
            server,
            forge,
            data,
        }
    }

    /// Stops the hub and starts it again on the same data, as after a
    /// restart: what was running is queued again.
    async fn restart(&mut self) {
        self.server.abort();
        let (url, state, server) = serve(self.data.path(), &self.forge).await;
        stemma_hub::jobs::recover(&state).await.unwrap();
        self.url = url;
        self.state = state;
        self.server = server;
    }
}

async fn serve(dir: &Path, forge: &FakeForge) -> (String, stemma_hub::AppState, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let config = stemma_hub::Config {
        data_dir: dir.to_owned(),
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
    let db = stemma_hub::db::open(&config.database_path()).await.unwrap();
    let state = Arc::new(stemma_hub::Hub::new(db, config).unwrap());
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

/// A runner's output, as it ran for real once.
fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

/// A fake `claude -p --output-format stream-json`, which answers after
/// `delay` seconds, with a result as Claude Code's.
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
cat {result:?}
"#,
            result = fixture("claude-code-result.json")
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

/// A machine with `config` (TOML, without its hub), running until the
/// handle is aborted.
fn worker(hub: &TestHub, token: &str, work: &Path, config: &str) -> JoinHandle<anyhow::Result<()>> {
    let config: WorkerConfig = toml::from_str(&format!(
        "hub = {:?}\nwork_dir = {:?}\n{config}",
        hub.url,
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

/// One of Ada's machines, which works for the workshop of `geometry` with
/// `agents`: its token.
async fn workshop_machine(hub: &TestHub, ada: &Person, agents: Value) -> String {
    let created = ada
        .post(hub, "/me/machines", json!({"name": "workshop"}))
        .await;
    let id = created["id"].as_i64().unwrap();
    ada.post(
        hub,
        "/groups/geometry/workshop/machines",
        json!({"machine": id}),
    )
    .await;
    ada.call(
        hub,
        "PUT",
        &format!("/me/machines/{id}/workshop-agents"),
        Some(agents),
    )
    .await;
    created["secret"].as_str().unwrap().to_owned()
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
    let token = worker_token(&hub, &ada, "/me/machines").await;
    let running = worker(
        &hub,
        &token,
        work.path(),
        &format!(
            "[runners.claude-code]\ncommand = {:?}\n",
            claude.display().to_string()
        ),
    );
    wait_online(&hub, &ada, "/me/machines").await;
    let workers = ada.get(&hub, "/me/machines").await;
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
    // What Claude Code said it consumed is kept with the job's run.
    let (input, cost): (Option<i64>, Option<f64>) =
        sqlx::query_as("SELECT input_tokens, cost_usd FROM job_runs WHERE kind = 'reply'")
            .fetch_one(&hub.state.db)
            .await
            .unwrap();
    assert_eq!(input, Some(1_204));
    assert_eq!(cost, Some(0.274_381));
    running.abort();
}

#[tokio::test]
async fn a_persons_agent_waits_for_one_of_their_machines() {
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
        &worker_token(&hub, &ada, "/me/machines").await,
        work.path(),
        &config,
    );
    wait_online(&hub, &ada, "/me/machines").await;

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
        "Waiting for a machine of Bob's with Claude Code"
    );

    let bobs = worker(
        &hub,
        &worker_token(&hub, &bob, "/me/machines").await,
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
async fn a_persons_own_agent_runs_as_they_defined_it() {
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    ada.post(
        &hub,
        "/me/agents",
        json!({"handle": "claude-sonnet", "display_name": "Claude Sonnet",
               "runner": "claude-code", "model": "sonnet", "args": ["--effort", "high"]}),
    )
    .await;
    let tools = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let claude = fake_claude(tools.path(), 0);
    // A machine, as `stemma machine add` configures it, offering what it
    // found installed.
    let created = ada
        .post(&hub, "/me/machines", json!({"name": "laptop"}))
        .await;
    let machine: stemma_worker::machine::MachineConfig = toml::from_str(&format!(
        "hub = {:?}\ntoken = {:?}\nid = {}\nname = \"laptop\"\nwork_dir = {:?}\n\
         [runners.claude-code]\ncommand = {:?}\n",
        hub.url,
        created["secret"].as_str().unwrap(),
        created["id"],
        work.path().display().to_string(),
        claude.display().to_string()
    ))
    .unwrap();
    let found = stemma_worker::detect::installed(
        &machine.runners,
        Some(std::ffi::OsStr::new("/nonexistent")),
    )
    .await;
    assert_eq!(found, ["claude-code"]);
    let context = Context::new(machine.worker_config(&found), "stemma".into()).unwrap();
    let running = tokio::spawn(connection::serve(Arc::new(context), machine.token.clone()));
    wait_online(&hub, &ada, "/me/machines").await;
    assert_eq!(
        ada.get(&hub, "/me/machines").await[0]["runners"],
        json!(["claude-code"])
    );

    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "@claude-sonnet is it proper?"}),
    )
    .await;
    let message = answer(&hub, &ada, 1, 2).await;
    assert_eq!(message["agent"], "Claude Sonnet");
    assert_eq!(message["author"]["username"], "ada");
    let args = std::fs::read_to_string(tools.path().join("claude.args")).unwrap();
    assert!(args.contains("--model\nsonnet\n--effort\nhigh\n"), "{args}");
    running.abort();
}

/// What `stemma claude` and the others prepare, without launching an
/// agent: a token for the session, revoked when it ends, and a private
/// folder with what the agent reads.
#[tokio::test]
async fn a_session_prepares_the_agent_and_revokes_its_token() {
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    ada.post(
        &hub,
        "/me/agents",
        json!({"handle": "codex-high", "display_name": "Codex High", "runner": "codex",
               "model": "gpt-5", "args": ["-c", "model_reasoning_effort=\"high\""]}),
    )
    .await;
    let client = stemma_client::Hub::new(&hub.url, Some(ada.token.clone())).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let options = |runner: &str, agent: Option<&str>| stemma_worker::session::Options {
        runner: runner.into(),
        agent: agent.map(str::to_owned),
        group: None,
        args: vec!["--search".into()],
        stemma: "/opt/stemma".into(),
        program: None,
        temp_dir: Some(temp.path().to_owned()),
        host: "laptop".into(),
    };

    // Ada's Claude Code, in her only group.
    let session = Session::open(&client, options("claude-code", None))
        .await
        .unwrap();
    assert_eq!(session.agent.handle, "claude");
    assert_eq!(session.group.slug, "geometry");
    let prepared = &session.prepared;
    assert_eq!(prepared.program, "claude");
    let args = &prepared.args;
    assert_eq!(args[0], "--mcp-config");
    assert_eq!(args.last().unwrap(), "--search");
    assert!(!args.contains(&"-p".to_owned()), "{args:?}");
    let instructions = &args[args
        .iter()
        .position(|a| a == "--append-system-prompt")
        .unwrap()
        + 1];
    assert!(
        instructions.starts_with("You are Claude, Ada's agent in Stemma"),
        "{instructions}"
    );
    assert!(instructions.contains("the group Geometry"));
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let server = &mcp["mcpServers"]["stemma"];
    assert_eq!(server["command"], "/opt/stemma");
    let mcp_args: Vec<String> = serde_json::from_value(server["args"].clone()).unwrap();
    assert_eq!(
        &mcp_args[..5],
        ["mcp", "--hub", &hub.url, "--group", "geometry"]
    );
    assert!(!mcp_args.contains(&"--read-only".to_owned()));
    let token_file = &mcp_args[mcp_args.iter().position(|a| a == "--token-file").unwrap() + 1];
    assert!(token_file.starts_with(&session.dir.display().to_string()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&session.dir)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    // The session's token acts as Ada, until the session ends.
    let token = std::fs::read_to_string(token_file).unwrap();
    let as_session = Person { token };
    assert_eq!(as_session.get(&hub, "/me").await["username"], "ada");
    let tokens = ada.get(&hub, "/me/tokens").await;
    assert!(
        tokens.as_array().unwrap().iter().any(|t| t["name"]
            == "Claude Code, a session on laptop"
            && t["expires_at"].is_string()),
        "{tokens}"
    );
    let dir = session.dir.clone();
    session.close(&client).await.unwrap();
    assert!(!dir.exists());
    let refused = reqwest::Client::new()
        .get(format!("{}/api/v1/me", hub.url))
        .bearer_auth(&as_session.token)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Another of her agents, with its model and arguments, then the
    // person's.
    let session = Session::open(&client, options("codex", Some("@codex-high")))
        .await
        .unwrap();
    assert_eq!(session.prepared.program, "codex");
    let args = session.prepared.args.join(" ");
    assert!(
        args.ends_with("--model gpt-5 -c model_reasoning_effort=\"high\" --search"),
        "{args}"
    );
    assert!(args.contains("developer_instructions="), "{args}");
    session.close(&client).await.unwrap();

    // An agent of another runner, or none, is refused, and leaves no token.
    for (runner, agent) in [("claude-code", Some("codex-high")), ("codex", Some("nope"))] {
        assert!(
            Session::open(&client, options(runner, agent))
                .await
                .is_err()
        );
    }
    let tokens = ada.get(&hub, "/me/tokens").await;
    assert_eq!(tokens.as_array().unwrap().len(), 1, "{tokens}");
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
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
    let token = worker_token(&hub, &ada, "/me/machines").await;

    // The worker goes away while it runs a job: the job is queued again.
    let first = worker(&hub, &token, work.path(), &slow);
    wait_online(&hub, &ada, "/me/machines").await;
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
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM jobs WHERE kind = 'reply'")
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
        &worker_token(&hub, &ada, "/me/machines").await,
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
    // Its instructions, as configuration: a string of TOML.
    assert!(args.contains("developer_instructions=\"\"\"\nYou are Codex, an agent in Stemma"));
    let input = std::fs::read_to_string(tools.path().join("codex.stdin")).unwrap();
    assert!(input.starts_with("Group: Geometry (geometry)."), "{input}");
    running.abort();
}

/// Runs git in `dir`, with nobody's configuration.
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

/// Copies a folder, leaving out what Lake built.
fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let name = entry.file_name();
        if name == ".lake" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(&name));
        } else {
            std::fs::copy(&path, to.join(&name)).unwrap();
        }
    }
}

/// Whether Lean can be used here: if the tests must use it, it must be.
fn lean_here() -> bool {
    let here = stemma_worker::connection::has_lean();
    if !here && std::env::var_os("STEMMA_TEST_LEAN").is_some() {
        panic!("STEMMA_TEST_LEAN is set, and Lean is not installed");
    }
    here
}

/// The build of `commit`, once it ends: builds take a while.
async fn build_of(hub: &TestHub, person: &Person, commit: &str) -> Value {
    let mut builds = Value::Null;
    for _ in 0..600 {
        builds = person.get(hub, "/groups/geometry/library/builds").await;
        let ended = builds.as_array().unwrap().iter().find(|b| {
            b["commit"] == commit && (b["state"] == "succeeded" || b["state"] == "failed")
        });
        if let Some(build) = ended {
            return build.clone();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the build of {commit} did not end: {builds}");
}

/// Makes the group's library the fixture, without Mathlib, as Ada pushes
/// it. Returns the folder of her clone, the clone, and the commit.
async fn fixture_library(hub: &TestHub, ada: &Person) -> (tempfile::TempDir, PathBuf, String) {
    let work = tempfile::tempdir().unwrap();
    let url = clone_url(hub, ada);
    assert!(
        git(work.path(), &["clone", "-q", &url, "library"])
            .status
            .success()
    );
    let dir = work.path().join("library");
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        if entry.file_name() != ".git" {
            let path = entry.path();
            if path.is_dir() {
                std::fs::remove_dir_all(path).unwrap();
            } else {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lean/fixture");
    copy_dir(&fixture, &dir);
    std::fs::write(dir.join(".gitignore"), "/.lake\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-qm", "The fixture"]);
    assert!(
        git(&dir, &["push", "-q", "origin", "main"])
            .status
            .success()
    );
    let commit = String::from_utf8(git(&dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    let commit = commit.trim().to_owned();
    // The forge tells the hub of the push; until it has, the template's
    // build, with Mathlib, is the one waiting.
    eventually("the hub to see the push", || async {
        let library = ada.get(hub, "/groups/geometry/library").await;
        (library["head"] == commit.as_str()).then_some(())
    })
    .await;
    (work, dir, commit)
}

/// Where Ada clones the library from, with her token.
fn clone_url(hub: &TestHub, ada: &Person) -> String {
    let host = hub.url.trim_start_matches("http://");
    format!("http://ada:{}@{host}/git/geometry/library.git", ada.token)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workshop_machine_with_lean_builds_and_inspects_the_library() {
    if !lean_here() {
        eprintln!("Lean is not installed: skipping");
        return;
    }
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let (_clone, dir, commit) = fixture_library(&hub, &ada).await;

    let worker_dir = tempfile::tempdir().unwrap();
    let running = worker(
        &hub,
        &workshop_machine(&hub, &ada, json!([])).await,
        worker_dir.path(),
        "[lean]\ncache = false\n",
    );
    let built = build_of(&hub, &ada, &commit).await;
    assert_eq!(built["state"], "succeeded", "{built}");
    assert_eq!(built["lean"], "4.35.0");

    let library = ada.get(&hub, "/groups/geometry/library").await;
    assert_eq!(library["current"]["commit"], commit.as_str());
    let hard = ada
        .get(&hub, "/groups/geometry/library/declarations/Fixture.hard")
        .await;
    assert_eq!(hard["state"], "sorry");
    assert_eq!(hard["kind"], "theorem");
    assert_eq!(hard["doc"], "Not proved yet.");
    assert_eq!(hard["used_by"], json!(["Fixture.uses_hard"]));
    let unknown = ada
        .get(
            &hub,
            "/groups/geometry/library/declarations/Fixture.unknown",
        )
        .await;
    assert_eq!(unknown["data_sorry"], true);
    let half = ada
        .get(&hub, "/groups/geometry/library/declarations/Fixture.half")
        .await;
    assert_eq!(half["sorry"], true);
    assert_eq!(half["data_sorry"], false);
    let sum = ada
        .get(
            &hub,
            "/groups/geometry/library/declarations/Fixture.Pair.sum",
        )
        .await;
    assert_eq!(sum["state"], "proved");
    assert_eq!(sum["uses"], json!(["Fixture.Pair", "Fixture.Pair.mk"]));
    assert_eq!(sum["range"]["start_line"], 31);
    let axiom = ada
        .get(
            &hub,
            "/groups/geometry/library/declarations/Fixture.uses_axiom",
        )
        .await;
    assert_eq!(axiom["axioms"], json!(["Fixture.choice_like"]));

    // A commit that does not build is shown with its log; the state stays.
    std::fs::write(
        dir.join("Fixture/More.lean"),
        "import Fixture.Basic\n\ntheorem broken : 1 = 2 := rfl\n",
    )
    .unwrap();
    git(&dir, &["commit", "-qam", "Broken"]);
    assert!(
        git(&dir, &["push", "-q", "origin", "main"])
            .status
            .success()
    );
    let broken = String::from_utf8(git(&dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    let failed = build_of(&hub, &ada, broken.trim()).await;
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"], "lake build failed: see the log");
    let log = ada
        .get(
            &hub,
            &format!("/groups/geometry/library/builds/{}", failed["id"]),
        )
        .await;
    assert!(
        log["log"].as_str().unwrap().contains("$ lake build"),
        "{log}"
    );
    assert!(log["log"].as_str().unwrap().contains("More.lean"));
    assert!(
        !log["log"].as_str().unwrap().contains("stj_"),
        "the token is not in the log"
    );
    let library = ada.get(&hub, "/groups/geometry/library").await;
    assert_eq!(library["current"]["commit"], commit.as_str());
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

/// Ada, an admin, and Gus, a guest, each summon their own Claude Code:
/// only Ada's may hand tasks to the workshop.
#[tokio::test]
async fn only_agents_for_those_who_ask_for_work_hand_tasks_over() {
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let gus = Person::new(&hub, "gus", false).await;
    let invitation = ada
        .post(
            &hub,
            "/groups/geometry/invitations",
            json!({"role": "guest"}),
        )
        .await;
    gus.post(
        &hub,
        &format!(
            "/invitations/{}/accept",
            invitation["token"].as_str().unwrap()
        ),
        json!({}),
    )
    .await;
    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Gluing", "body": "The charts glue."}),
    )
    .await;

    for (person, asked, hands_over) in [(&ada, 2, true), (&gus, 4, false)] {
        let tools = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let claude = fake_claude(tools.path(), 0);
        let running = worker(
            &hub,
            &worker_token(&hub, person, "/me/machines").await,
            work.path(),
            &format!(
                "[runners.claude-code]\ncommand = {:?}\n",
                claude.display().to_string()
            ),
        );
        wait_online(&hub, person, "/me/machines").await;
        person
            .post(
                &hub,
                "/groups/geometry/threads/1/messages",
                json!({"body": "@claude have the workshop formalize the gluing"}),
            )
            .await;
        let message = answer(&hub, person, 1, asked + 1).await;
        assert_eq!(message["reply_to"], asked);

        let args = std::fs::read_to_string(tools.path().join("claude.args")).unwrap();
        assert_eq!(
            args.contains("mcp__stemma__create_task"),
            hands_over,
            "{args}"
        );
        let mcp: Value =
            serde_json::from_str(&std::fs::read_to_string(tools.path().join("mcp.json")).unwrap())
                .unwrap();
        let mcp_args = mcp["mcpServers"]["stemma"]["args"].as_array().unwrap();
        assert!(mcp_args.contains(&json!("--read-only")));
        assert_eq!(mcp_args.contains(&json!("--hand-over")), hands_over);
        running.abort();
    }
}

/// A fake `opencode run`, working on the fixture in its worktree: in its
/// first round it adds a theorem and a definition whose data is `sorry`,
/// and Ada meanwhile pushes another change to `main`; told that the
/// definition does not pass, it completes it. Each round costs what the
/// fixture's steps do.
fn fake_opencode(dir: &Path, clone_url: &str) -> PathBuf {
    script(
        dir,
        "opencode",
        &format!(
            r#"printf '%s\n' "$@" > "$HERE/opencode.args"
echo call >> "$HERE/opencode.calls"
cp "$OPENCODE_CONFIG" "$HERE/opencode.json"
cp "$(dirname "$OPENCODE_CONFIG")/instructions.md" "$HERE/opencode.instructions"
case "$*" in
  *"does not pass yet"*)
    sed -i.bak 's/def three : Nat := sorry/def three : Nat := 3/' Fixture/More.lean
    rm -f Fixture/More.lean.bak
    ;;
  *)
    cat >> Fixture/More.lean <<'LEAN'

namespace Fixture

/-- Twice two is four, as t1#1 says. -/
theorem double_two : double 2 = 4 := rfl

/-- Three. -/
def three : Nat := sorry

end Fixture
LEAN
    # Meanwhile, Ada changes main elsewhere.
    rm -rf "$HERE/ada" && git clone -q {clone_url} "$HERE/ada"
    cd "$HERE/ada" && echo "Notes." > NOTES.md && git add NOTES.md \
      && git -c user.name=Ada -c user.email=ada@example.org commit -qm Notes \
      && git push -q origin main && cd - > /dev/null
    ;;
esac
mkdir -p .stemma
printf 'Added [[lean:Fixture.double_two]] and [[lean:Fixture.three]].\n\nNothing is pending.\n' > .stemma/report.md
echo '{{"type":"tool_use","part":{{"tool":"bash","state":{{"input":{{"command":"lake build"}}}}}}}}'
echo '{{"type":"text","part":{{"text":"Done: see the report."}}}}'
grep step_finish {events:?}
"#,
            events = fixture("opencode-events.jsonl")
        ),
    )
}

/// The task, once its run is over.
async fn task_over(hub: &TestHub, person: &Person, number: u32) -> Value {
    let mut task = Value::Null;
    for _ in 0..600 {
        task = person
            .get(hub, &format!("/groups/geometry/tasks/{number}"))
            .await;
        let state = task["state"].as_str().unwrap_or_default();
        if !matches!(state, "open" | "running") {
            return task;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("task w{number} did not end: {task}");
}

/// A workshop machine of Ada's that works with a fake OpenCode.
async fn workshop_worker(
    hub: &TestHub,
    ada: &Person,
    work: &Path,
    opencode: &Path,
) -> JoinHandle<anyhow::Result<()>> {
    let token = workshop_machine(
        hub,
        ada,
        json!([{"runner": "opencode", "model": "fake/model", "daily_jobs": 5}]),
    )
    .await;
    worker(
        hub,
        &token,
        work,
        &format!(
            "[runners.opencode]\ncommand = {:?}\n[lean]\ncache = false\n",
            opencode.display().to_string()
        ),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn with_lean_the_workshop_formalizes_and_proposes_a_task_from_a_thread() {
    if !lean_here() {
        eprintln!("Lean is not installed: skipping");
        return;
    }
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let (_clone, _dir, commit) = fixture_library(&hub, &ada).await;
    let tools = tempfile::tempdir().unwrap();
    let opencode = fake_opencode(tools.path(), &clone_url(&hub, &ada));
    let worker_dir = tempfile::tempdir().unwrap();
    let running = workshop_worker(&hub, &ada, worker_dir.path(), &opencode).await;
    assert_eq!(build_of(&hub, &ada, &commit).await["state"], "succeeded");

    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Doubling", "body": "Twice two is four."}),
    )
    .await;
    ada.post(
        &hub,
        "/groups/geometry/threads/1/messages",
        json!({"body": "@workshop formalize t1#1."}),
    )
    .await;
    let task = task_over(&hub, &ada, 1).await;
    assert_eq!(task["state"], "review", "{task}");
    assert_eq!(task["runner"], "opencode");
    assert_eq!(task["model"], "fake/model");
    assert_eq!(task["architecture"], "formalize");
    assert_eq!(task["report_message"], "t1#3");
    let changes = &task["changes"];
    assert_eq!(changes["branch"], "workshop/w1");
    assert_eq!(changes["big"], false);
    assert_eq!(changes["needs"], json!(["asker"]));
    let proposed = changes["commit"].as_str().unwrap().to_owned();
    let page = ada.get(&hub, "/groups/geometry/tasks/1").await;
    let log = page["log"].as_str().unwrap();
    assert!(
        log.contains("Not accepted: The data of Fixture.three depends on sorry"),
        "{log}"
    );
    assert!(log.contains("Round 2"), "{log}");
    assert!(log.contains("Pushed"), "{log}");
    assert!(!log.contains("stj_"), "the token is not in the log");
    assert_eq!(
        std::fs::read_to_string(tools.path().join("opencode.calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );

    // One message in the thread reports on it, for Ada, who approves it.
    let report = answer(&hub, &ada, 1, 3).await;
    assert_eq!(report["agent"], "Workshop");
    assert_eq!(report["author"]["username"], "ada");
    assert_eq!(report["reply_to"], 2);
    let body = report["body"].as_str().unwrap();
    assert!(
        body.starts_with("**Ready for review:** [[w1]], “Formalize t1#1”."),
        "{body}"
    );
    assert!(body.contains("Added [[lean:Fixture.double_two]]"), "{body}");
    assert!(
        body.contains("they change [[lean:Fixture.double_two]], [[lean:Fixture.three]]"),
        "{body}"
    );
    // Nothing is on main yet: it is on the task's branch, with its
    // provenance, signed by the workshop.
    let library = ada.get(&hub, "/groups/geometry/library").await;
    assert_ne!(library["head"], proposed.as_str());
    let repository = hub.forge.repository_path("geometry", "library");
    let shown = String::from_utf8(
        git(
            &repository,
            &["log", "-1", "--format=%an <%ae>%n%B", "workshop/w1"],
        )
        .stdout,
    )
    .unwrap();
    assert!(
        shown.starts_with(
            "Workshop, for Ada <ada@stemma>\nFormalize t1#1\n\nTask: w1\nOrigin: t1#2\nRunner: \
             opencode\nModel: fake/model\n"
        ),
        "{shown}"
    );

    // Approved, it is merged, beside Ada's change meanwhile, and built.
    ada.post(&hub, "/groups/geometry/tasks/1/approve", json!({}))
        .await;
    let task = ada.get(&hub, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "done", "{task}");
    let merged = task["changes"]["merged"].as_str().unwrap().to_owned();
    let built = build_of(&hub, &ada, &merged).await;
    assert_eq!(built["state"], "succeeded", "{built}");
    let double_two = ada
        .get(
            &hub,
            "/groups/geometry/library/declarations/Fixture.double_two",
        )
        .await;
    assert_eq!(double_two["state"], "proved");
    assert_eq!(double_two["doc"], "Twice two is four, as t1#1 says.");
    let three = ada
        .get(&hub, "/groups/geometry/library/declarations/Fixture.three")
        .await;
    assert_eq!(three["data_sorry"], false);
    let notes = git(&repository, &["show", "main:NOTES.md"]);
    assert!(notes.status.success(), "Ada's change is on main too");

    // How OpenCode was run: headless, with its own configuration and the
    // architecture's instructions; it may open a thread, and nothing more.
    let args = std::fs::read_to_string(tools.path().join("opencode.args")).unwrap();
    assert!(
        args.starts_with("run\n--format\njson\n--model\nfake/model\n"),
        "{args}"
    );
    let instructions = std::fs::read_to_string(tools.path().join("opencode.instructions")).unwrap();
    assert!(
        instructions.starts_with("You are Workshop, the workshop of Stemma"),
        "{instructions}"
    );
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(tools.path().join("opencode.json")).unwrap())
            .unwrap();
    assert_eq!(config["permission"]["edit"], "allow");
    let command = config["mcp"]["stemma"]["command"].as_array().unwrap();
    assert!(command.contains(&json!("--read-only")));
    assert!(command.contains(&json!("--open-threads")));
    assert!(!command.contains(&json!("--hand-over")));
    // Its worktree is gone; what it consumed is measured.
    assert_eq!(
        std::fs::read_dir(worker_dir.path().join("tasks"))
            .unwrap()
            .count(),
        0
    );
    let days = ada.get(&hub, "/groups/geometry/workshop/consumption").await;
    let cost = days[0]["cost_usd"].as_f64().unwrap();
    assert!((cost - 2.0 * 0.0123).abs() < 1e-9, "{days}");
    running.abort();
}

/// A fake `opencode run` that changes a line of the fixture which Ada
/// changes too, meanwhile, on `main`.
fn fake_conflicting_opencode(dir: &Path, clone_url: &str) -> PathBuf {
    script(
        dir,
        "opencode",
        &format!(
            r#"sed -i.bak 's/theorem classical/theorem classical_em/' Fixture/More.lean
rm -f Fixture/More.lean.bak
rm -rf "$HERE/ada" && git clone -q {clone_url} "$HERE/ada"
cd "$HERE/ada" && sed -i.bak 's/theorem classical/theorem excluded_middle/' Fixture/More.lean \
  && rm -f Fixture/More.lean.bak && git add -A \
  && git -c user.name=Ada -c user.email=ada@example.org commit -qm Rename \
  && git push -q origin main && cd - > /dev/null
echo 'Renamed it.'
"#
        ),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn with_lean_changes_that_no_longer_apply_are_not_merged() {
    if !lean_here() {
        eprintln!("Lean is not installed: skipping");
        return;
    }
    let hub = TestHub::start().await;
    let (ada, _) = group(&hub).await;
    let (_clone, _dir, _commit) = fixture_library(&hub, &ada).await;
    let tools = tempfile::tempdir().unwrap();
    let opencode = fake_conflicting_opencode(tools.path(), &clone_url(&hub, &ada));
    let worker_dir = tempfile::tempdir().unwrap();
    let running = workshop_worker(&hub, &ada, worker_dir.path(), &opencode).await;
    ada.post(
        &hub,
        "/groups/geometry/threads",
        json!({"title": "Names", "body": "Call it classical_em."}),
    )
    .await;
    ada.post(
        &hub,
        "/groups/geometry/tasks",
        json!({"title": "Rename classical", "instructions": "As t1#1 says.", "origin": "t1#1"}),
    )
    .await;
    let task = task_over(&hub, &ada, 1).await;
    assert_eq!(task["state"], "review", "{task}");
    let report = answer(&hub, &ada, 1, 2).await;
    let body = report["body"].as_str().unwrap();
    assert!(body.contains("Renamed it."), "{body}");

    // Main changed where it worked: the forge cannot merge it.
    let refused = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/groups/geometry/tasks/1/approve",
            hub.url
        ))
        .bearer_auth(&ada.token)
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 409);
    let task = ada.get(&hub, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "review");
    ada.post(&hub, "/groups/geometry/tasks/1/reject", json!({}))
        .await;
    let task = ada.get(&hub, "/groups/geometry/tasks/1").await;
    assert_eq!(task["state"], "rejected");
    running.abort();
}
