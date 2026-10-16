//! Libraries, end to end over HTTP: made from the template, cloned and
//! pushed with git, built, and their reports kept and queried.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use stemma_core::api::{Declaration, LibraryReport, NewAccount};
use stemma_core::protocol::Built;

struct Hub {
    url: String,
    state: stemma_hub::AppState,
    _data: tempfile::TempDir,
}

async fn hub() -> Hub {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let data = tempfile::tempdir().unwrap();
    let config = stemma_hub::Config {
        data_dir: data.path().to_owned(),
        ..Default::default()
    };
    let state = Arc::new(stemma_hub::Hub::new(
        stemma_hub::db::in_memory().await.unwrap(),
        config,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = stemma_hub::app(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });
    Hub {
        url,
        state,
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

fn declaration(name: &str, sorry: bool, uses: &[&str]) -> Declaration {
    Declaration {
        name: name.into(),
        module: "Geometry.Basic".into(),
        kind: "theorem".into(),
        type_: "1 + 1 = 2".into(),
        doc: Some("A fact. ".into()),
        range: None,
        sorry,
        data_sorry: false,
        axioms: if sorry {
            vec!["sorryAx".into()]
        } else {
            Vec::new()
        },
        uses: uses.iter().map(|u| u.to_string()).collect(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn libraries_are_made_cloned_pushed_and_built() {
    let hub = hub().await;
    if !hub.state.git {
        eprintln!("git is not installed: skipping");
        return;
    }
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
    assert_eq!(library["clone_path"], "/git/geometry.git");
    let first = library["head"].as_str().unwrap().to_owned();
    assert_eq!(library["latest"]["state"], "queued");
    assert_eq!(library["latest"]["commit"], first.as_str());
    assert_eq!(
        library["latest"]["progress"],
        "Waiting for a group worker with Lean to come online"
    );
    assert!(library["current"].is_null());

    // Cloned with the token as the password; not without it.
    let work = tempfile::tempdir().unwrap();
    let host = hub.url.trim_start_matches("http://");
    let anonymous = git(
        work.path(),
        &[
            "clone",
            "-q",
            &format!("{}/git/geometry.git", hub.url),
            "nobody",
        ],
    );
    assert!(!anonymous.status.success());
    let url = format!("http://ada:{ada}@{host}/git/geometry.git");
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
    let second = String::from_utf8(git(&dir, &["rev-parse", "HEAD"]).stdout).unwrap();
    let second = second.trim();
    let mut builds = Value::Null;
    for _ in 0..100 {
        builds = call(&hub, &ada, "GET", "/groups/geometry/library/builds", None).await;
        if builds.as_array().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(builds[0]["commit"], second);
    assert_eq!(builds[0]["state"], "queued");
    assert_eq!(builds[1]["commit"], first.as_str());
    assert_eq!(builds[1]["state"], "cancelled");

    // History cannot be rewritten.
    git(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
    let forced = git(&dir, &["push", "-q", "--force", "origin", "main"]);
    assert!(!forced.status.success());

    // A worker's report becomes the library's state.
    let job_id: i64 = sqlx::query_scalar("SELECT job_id FROM library_builds WHERE commit_sha = ?")
        .bind(second)
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let group_id: i64 = sqlx::query_scalar("SELECT id FROM groups")
        .fetch_one(&hub.state.db)
        .await
        .unwrap();
    let report = LibraryReport {
        lean: "4.35.0".into(),
        modules: vec!["Geometry".into(), "Geometry.Basic".into()],
        declarations: vec![
            declaration("Geometry.two", false, &[]),
            declaration("Geometry.hard", true, &["Geometry.two"]),
        ],
    };
    stemma_hub::library::build_finished(
        &hub.state,
        group_id,
        job_id,
        Built {
            succeeded: true,
            error: None,
            log: "✔ Built Geometry".into(),
            report: Some(report),
        },
    )
    .await
    .unwrap();

    let library = call(&hub, &ada, "GET", "/groups/geometry/library", None).await;
    assert_eq!(library["current"]["commit"], second);
    assert_eq!(library["current"]["state"], "succeeded");
    assert_eq!(library["current"]["lean"], "4.35.0");
    assert_eq!(library["declarations"], 2);
    assert_eq!(library["sorry"], 1);
    assert_eq!(library["modules"][0]["name"], "Geometry.Basic");

    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title": "Hard", "body": "Is [[lean:Geometry.hard]] true?"})),
    )
    .await;
    let page = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/library/declarations/Geometry.two",
        None,
    )
    .await;
    assert_eq!(page["state"], "proved");
    assert_eq!(page["doc"], "A fact.");
    assert_eq!(page["used_by"], json!(["Geometry.hard"]));
    let page = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/library/declarations/Geometry.hard",
        None,
    )
    .await;
    assert_eq!(page["state"], "sorry");
    assert_eq!(page["references"][0]["address"], "t1#1");
    // A thread about it, as "Discuss it" starts one, is listed too.
    call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/threads",
        Some(json!({"title": "Proving it", "anchor": "lean:Geometry.hard"})),
    )
    .await;
    let page = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/library/declarations/Geometry.hard",
        None,
    )
    .await;
    assert_eq!(page["references"][0]["address"], "t2");
    assert_eq!(page["references"][0]["title"], "Proving it");
    assert_eq!(page["references"][1]["address"], "t1#1");

    let found = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/library/declarations?q=HARD",
        None,
    )
    .await;
    assert_eq!(found.as_array().unwrap().len(), 1);
    let sorry = call(
        &hub,
        &ada,
        "GET",
        "/groups/geometry/library/declarations?state=sorry",
        None,
    )
    .await;
    assert_eq!(sorry[0]["name"], "Geometry.hard");

    let resolved = call(
        &hub,
        &ada,
        "POST",
        "/groups/geometry/resolve",
        Some(json!({"addresses": ["lean:Geometry.two", "lean:Geometry.hard", "lean:Nope"]})),
    )
    .await;
    let states: Vec<&str> = resolved
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["proved", "sorry", "missing"]);
    assert_eq!(resolved[0]["exists"], true);
    assert_eq!(resolved[2]["exists"], false);

    let log = call(
        &hub,
        &ada,
        "GET",
        &format!(
            "/groups/geometry/library/builds/{}",
            library["current"]["id"]
        ),
        None,
    )
    .await;
    assert_eq!(log["log"], "✔ Built Geometry");
}
