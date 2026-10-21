//! The CLI edits only its explicitly isolated settings and state folders.
use std::path::Path;
fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_stemma"))
        .args(args)
        .env("STEMMA_CONFIG_DIR", dir.join("config"))
        .env("STEMMA_STATE_DIR", dir.join("state"))
        .env("STEMMA_DATA_DIR", dir.join("data"))
        .env("STEMMA_SERVICE_DIR", dir.join("service"))
        .env_remove("STEMMA_AGENTS_CLAUDE_MODEL")
        .output()
        .unwrap()
}
#[test]
fn settings_round_trip_and_removed_commands_fail() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        cli(
            dir.path(),
            &["config", "set", "agents.claude.model", "opus"]
        )
        .status
        .success()
    );
    assert!(
        cli(
            dir.path(),
            &[
                "config",
                "set",
                "agents.claude.model",
                "sonnet",
                "--group",
                "algebra"
            ]
        )
        .status
        .success()
    );
    let output = cli(
        dir.path(),
        &["config", "get", "agents.claude.model", "--group", "algebra"],
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains("sonnet"));
    assert!(!dir.path().join("config/credentials.toml").exists());
    for command in [
        ["machine", "add"],
        ["machine", "status"],
        ["machine", "prefer"],
        ["task", "work"],
    ] {
        assert!(!cli(dir.path(), &command).status.success());
    }
}

#[test]
fn containers_sign_ins_are_kept_among_the_credentials() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let set = |name: &str, value: &str| {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_stemma"))
            .args(["config", "secret", "set", name])
            .env("STEMMA_CONFIG_DIR", dir.path().join("config"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(value.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    assert!(
        set("CLAUDE_CODE_OAUTH_TOKEN", "sk-ant-oat01-x\n")
            .status
            .success()
    );
    assert!(!set("STEMMA_TOKEN", "stm_x").status.success());
    let listed = cli(dir.path(), &["config", "secret", "list"]);
    assert_eq!(
        String::from_utf8(listed.stdout).unwrap(),
        "CLAUDE_CODE_OAUTH_TOKEN\n"
    );
    let file = dir.path().join("config/credentials.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("sk-ant-oat01-x"), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert!(
        cli(
            dir.path(),
            &["config", "secret", "unset", "CLAUDE_CODE_OAUTH_TOKEN"]
        )
        .status
        .success()
    );
    let listed = cli(dir.path(), &["config", "secret", "list"]);
    assert!(listed.stdout.is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn foreground_service_answers_from_the_workspace_reading_only() {
    use serde_json::json;
    use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
    let temp = tempfile::tempdir().unwrap();
    let forge = stemma_fake_forge::FakeForge::start().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(
        stemma_hub::Hub::new(
            stemma_hub::db::in_memory().await.unwrap(),
            stemma_hub::Config {
                data_dir: temp.path().join("hub"),
                public_url: Some(url.clone()),
                forge: Some(stemma_hub::config::ForgeConfig::new(
                    &forge.url,
                    "http://forge.test/callback",
                    stemma_hub::config::ForgeSecrets {
                        token: stemma_fake_forge::TOKEN.into(),
                        webhook_secret: "test hook".into(),
                        client_secret: "test client".into(),
                    },
                )),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    stemma_hub::api::create_user(
        &state,
        stemma_core::api::NewAccount {
            username: "ada".into(),
            display_name: "Ada".into(),
            password: "test password".into(),
        },
        true,
    )
    .await
    .unwrap();
    let app = stemma_hub::app(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut hub = stemma_client::Hub::new(&url, None).unwrap();
    hub.sign_in("ada", "test password").await.unwrap();
    let token = hub.create_token("Test computer").await.unwrap();
    let http = reqwest::Client::new();
    let group = http
        .post(format!("{url}/api/v1/groups"))
        .bearer_auth(&token.secret)
        .json(&json!({"slug":"algebra", "name":"Algebra"}))
        .send()
        .await
        .unwrap();
    assert!(
        group.status().is_success(),
        "{}",
        group.text().await.unwrap()
    );
    let runner = temp.path().join("fake-codex");
    std::fs::write(
        &runner,
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = --version ]; then echo 'codex test'; exit 0; fi
answer=''
sandbox=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-last-message) shift; answer="$1" ;;
    --sandbox) shift; sandbox="$1" ;;
  esac
  shift
done
cat >/dev/null
test -z "${STEMMA_TOKEN:-}"
test -z "${STEMMA_HUB:-}"
test ! -e "$STEMMA_CONFIG_DIR/credentials.toml"
test "$sandbox" = read-only
case "$PWD" in */groups/algebra/scratch/*) ;; *) exit 3 ;; esac
test -d ../../library/.git
test -n "$answer"
printf 'Read in the workspace.\n' > "$answer"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        format!(
            "[agents.codex]\ncommand = {}\n",
            serde_json::to_string(&runner.display().to_string()).unwrap()
        ),
    )
    .unwrap();
    let mut process = tokio::process::Command::new(env!("CARGO_BIN_EXE_stemma"));
    process
        .args(["machine", "run"])
        .env("STEMMA_CONFIG_DIR", &config_dir)
        .env("STEMMA_STATE_DIR", temp.path().join("state"))
        .env("STEMMA_DATA_DIR", temp.path().join("data"))
        .env("STEMMA_SERVICE_DIR", temp.path().join("service"))
        .env("STEMMA_HUB", &url)
        .env("STEMMA_TOKEN", &token.secret)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(
            std::fs::File::create(temp.path().join("service.log")).unwrap(),
        ));
    let mut child = process.spawn().unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if state.workers.lock().await.is_online(token.token.id) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("service connects with the personal token");
    let requester = stemma_client::Hub::new(&url, Some(token.secret.clone())).unwrap();
    let thread = requester
        .create_thread(
            "algebra",
            &stemma_core::api::CreateThread {
                title: "Question".into(),
                kind: stemma_core::api::ThreadKind::Discussion,
                resident_agent: None,
                anchor: None,
                body: Some("@codex check it".into()),
                attachments: vec![],
            },
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let page = requester
                .thread("algebra", thread.thread.number)
                .await
                .unwrap();
            if page.messages.len() == 2 {
                assert_eq!(page.messages[1].agent.as_deref(), Some("Codex"));
                assert_eq!(page.messages[1].body, "Read in the workspace.");
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "reply timed out: {error}; logs: {}",
            std::fs::read_to_string(temp.path().join("service.log")).unwrap()
        )
    });
    assert!(
        temp.path()
            .join("data/groups/algebra/library/.git")
            .is_dir()
    );
    assert_eq!(
        std::fs::read_dir(temp.path().join("state/jobs"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        std::fs::read_dir(temp.path().join("data/groups/algebra/scratch"))
            .unwrap()
            .count(),
        0
    );
    // A runner disappearing changes Hello on the same connection, and queued work
    // starts when a valid edit makes it available again.
    let connections = state.connections.load(std::sync::atomic::Ordering::Relaxed);
    std::fs::write(
        config_dir.join("config.toml"),
        "[agents.codex]\ncommand = '/missing/codex'\n",
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while state
            .workers
            .lock()
            .await
            .runners(token.token.id)
            .contains(&"codex".into())
        {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .expect("capabilities reload without reconnecting");
    requester
        .post_message(
            "algebra",
            thread.thread.number,
            &stemma_core::api::PostMessage {
                body: "@codex once more".into(),
                reply_to: None,
                agent: None,
                attachments: vec![],
            },
        )
        .await
        .unwrap();
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE state = 'queued'")
        .fetch_one(&state.db)
        .await
        .unwrap();
    assert_eq!(queued, 1);
    std::fs::write(
        config_dir.join("config.toml"),
        format!(
            "[agents.codex]\ncommand = {}\n[groups.algebra.agents]\npriority = 'high'\n",
            serde_json::to_string(&runner.display().to_string()).unwrap()
        ),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if requester
                .thread("algebra", thread.thread.number)
                .await
                .unwrap()
                .messages
                .len()
                == 4
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("restoring a runner dispatches waiting work");
    assert_eq!(
        state.connections.load(std::sync::atomic::Ordering::Relaxed),
        connections
    );
    requester.revoke_token(token.token.id).await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(8), child.wait())
        .await
        .expect("revocation signs the computer out")
        .unwrap();
    assert!(status.success());
    server.abort();
}
