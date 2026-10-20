//! The desktop's shared local controls, through the CLI, against a real hub.

use std::path::Path;
use std::sync::Arc;

use stemma_core::api::NewAccount;
use stemma_worker::machine::MachineConfig;

async fn cli(dir: &Path, args: &[&str]) {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_stemma"))
        .args(args)
        .env_remove("STEMMA_HUB")
        .env_remove("STEMMA_TOKEN")
        .env_remove("STEMMA_GROUP")
        .env("STEMMA_SERVICE_DIR", dir.join("service"))
        .env(
            "PATH",
            std::env::join_paths([dir.join("bin"), "/usr/bin".into(), "/bin".into()]).unwrap(),
        )
        .env("STEMMA_CLIENT_CONFIG", dir.join("client.toml"))
        .env("STEMMA_MACHINE_CONFIG", dir.join("machine.toml"))
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn registers_configures_prefers_and_removes_this_machine() {
    let dir = tempfile::tempdir().unwrap();
    // Never install a real login service or overwrite the developer's service.
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for manager in ["launchctl", "systemctl"] {
        let script = bin.join(manager);
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$*\" in *print*|*is-active*) exit 1;; *) exit 0;; esac\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let state = Arc::new(
        stemma_hub::Hub::new(
            stemma_hub::db::in_memory().await.unwrap(),
            stemma_hub::Config {
                data_dir: dir.path().join("hub"),
                ..Default::default()
            },
        )
        .unwrap(),
    );
    stemma_hub::api::create_user(
        &state,
        NewAccount {
            username: "ada".into(),
            display_name: "Ada".into(),
            password: "analytical".into(),
        },
        true,
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, stemma_hub::app(state)).await });
    let mut hub = stemma_client::Hub::new(&url, None).unwrap();
    hub.sign_in("ada", "analytical").await.unwrap();
    let created = hub.create_token("tests").await.unwrap();
    cli(dir.path(), &["login", &url, "--token", &created.secret]).await;
    let worker_path = dir.path().join("machine.toml");
    let worker = tokio::spawn(async move {
        while !worker_path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        stemma_worker::machine::run(&worker_path).await.unwrap();
    });
    cli(dir.path(), &["machine", "add", "test machine"]).await;
    assert!(hub.machines().await.unwrap()[0].online);
    worker.abort();
    let _ = worker.await;
    let path = dir.path().join("machine.toml");
    let config = MachineConfig::load_from(&path).unwrap().unwrap();
    assert!(config.personal);
    assert!(!config.workshop_enabled);
    let copies = dir.path().join("copies");
    cli(
        dir.path(),
        &[
            "machine",
            "configure",
            "--work-dir",
            copies.to_str().unwrap(),
            "--personal",
            "false",
            "--workshop",
            "false",
        ],
    )
    .await;
    cli(
        dir.path(),
        &[
            "machine",
            "workshop",
            "isolation",
            "podman",
            "--image",
            "stemma-workshop:test",
        ],
    )
    .await;
    cli(
        dir.path(),
        &[
            "machine",
            "workshop",
            "credentials",
            "codex",
            "--env",
            "OPENAI_API_KEY",
            "--file",
            "~/.codex/auth.json",
        ],
    )
    .await;
    let config = MachineConfig::load_from(&path).unwrap().unwrap();
    assert_eq!(config.work_dir.as_deref(), Some(copies.as_path()));
    assert!(!config.personal && !config.workshop_enabled);
    let settings = config.isolation.unwrap();
    assert_eq!(settings.image(), "stemma-workshop:test");
    assert_eq!(settings.agents["codex"].env, ["OPENAI_API_KEY"]);
    cli(dir.path(), &["machine", "prefer"]).await;
    assert!(hub.machines().await.unwrap()[0].preferred);
    // Both services off exits without detecting tools or contacting a runtime.
    cli(dir.path(), &["machine", "run"]).await;
    cli(dir.path(), &["machine", "remove"]).await;
    assert!(!path.exists());
    assert!(hub.machines().await.unwrap().is_empty());
    server.abort();
}
