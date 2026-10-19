#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use stemma_client::{Hub, config::ClientConfig};
use stemma_core::api::{Machine, Me, WorkshopAgent};
use stemma_worker::{
    control, detect, image,
    isolation::{AgentSignIn, Runtime, SignInFile},
    machine, service,
};
use tauri::{Manager, State, WebviewWindow};
use tokio::sync::Mutex;

type Shared<'a> = State<'a, Mutex<()>>;
type Result<T> = std::result::Result<T, String>;

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

// Remote hub pages never receive native machine access, even if they try
// invoking an application command directly.
fn trusted_machine_url(url: &tauri::Url) -> bool {
    (cfg!(debug_assertions) && url.as_str() == "http://127.0.0.1:1420/")
        || (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
}

fn local(window: &WebviewWindow) -> Result<()> {
    let url = window.url().map_err(error)?;
    if window.label() != "machine" || !trusted_machine_url(&url) {
        return Err("Local machine access denied".into());
    }
    Ok(())
}
fn hub() -> anyhow::Result<Hub> {
    ClientConfig::load()?.with_env().hub()
}

#[derive(Serialize)]
struct Snapshot {
    hub: Option<String>,
    username: Option<String>,
    me: Option<Me>,
    hub_error: Option<String>,
    registered: bool,
    machine: Option<Machine>,
    name: String,
    work_dir: String,
    personal: bool,
    workshop: bool,
    service: service::Status,
    agents: Vec<detect::AgentStatus>,
    elan: Option<String>,
    latex: Option<String>,
    runtime: Runtime,
    image: String,
    credentials: BTreeMap<String, AgentSignIn>,
}

#[tauri::command]
async fn snapshot(window: WebviewWindow) -> Result<Snapshot> {
    local(&window)?;
    let config = ClientConfig::load().map_err(error)?.with_env();
    let machine =
        machine::MachineConfig::load_from(&machine::path().map_err(error)?).map_err(error)?;
    let settings = machine
        .as_ref()
        .and_then(|m| m.isolation.clone())
        .unwrap_or_default();
    let runners = machine
        .as_ref()
        .map(|m| m.runners.clone())
        .unwrap_or_default();
    let mut hub_error = None;
    let mut me = None;
    let mut remote = None;
    if config.token.is_some() {
        match config.hub() {
            Ok(hub) => match hub.me().await {
                Ok(person) => {
                    me = Some(person);
                    if let Some(machine) = &machine {
                        match control::check_hub(machine, &hub) {
                            Ok(()) => match hub.machines().await {
                                Ok(machines) => {
                                    remote = machines.into_iter().find(|m| m.id == machine.id)
                                }
                                Err(e) => hub_error = Some(error(e)),
                            },
                            Err(e) => hub_error = Some(error(e)),
                        }
                    }
                }
                Err(e) => hub_error = Some(error(e)),
            },
            Err(e) => hub_error = Some(error(e)),
        }
    }
    let (agents, elan, latex) = tokio::join!(
        detect::agents(&runners),
        detect::version("elan"),
        detect::version("latexmk")
    );
    let service = service::status().await.map_err(error)?;
    let personal = service.personal;
    let workshop = service.workshop;
    Ok(Snapshot {
        hub: config.hub,
        username: config.username,
        me,
        hub_error,
        registered: machine.is_some(),
        machine: remote,
        name: machine
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "This machine".into()),
        work_dir: machine
            .as_ref()
            .map(|m| m.worker_config(&[]).work_dir().display().to_string())
            .unwrap_or_default(),
        personal,
        workshop,
        service,
        agents,
        elan,
        latex,
        runtime: settings.runtime(),
        image: settings.image(),
        credentials: settings.agents,
    })
}

#[tauri::command]
async fn login(
    window: WebviewWindow,
    url: String,
    username: String,
    password: String,
) -> Result<()> {
    local(&window)?;
    if let Ok(machine) = control::load() {
        let target = Hub::new(&url, None).map_err(error)?;
        control::check_hub(&machine, &target).map_err(error)?;
        // A different account must not accidentally keep serving the old owner.
        if ClientConfig::load().map_err(error)?.username.as_deref() != Some(&username) {
            return Err(
                "Remove this machine from its current account before switching accounts".into(),
            );
        }
    }
    stemma_client::sign_in::password(&url, &username, &password, "desktop", None)
        .await
        .map_err(error)?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
    Register {
        name: String,
        preferred: bool,
    },
    Prefer {
        preferred: bool,
    },
    Remove,
    Folder {
        path: PathBuf,
    },
    Serving {
        enabled: bool,
    },
    Service {
        personal: bool,
        workshop: bool,
    },
    Containers {
        runtime: Runtime,
        image: String,
    },
    Build {
        source: String,
    },
    Lend {
        group: String,
        enabled: bool,
    },
    Agents {
        agents: Vec<WorkshopAgent>,
    },
    Credentials {
        runner: String,
        env: Vec<String>,
        files: Vec<SignInFile>,
    },
}

#[tauri::command]
async fn action(window: WebviewWindow, state: Shared<'_>, action: Action) -> Result<()> {
    local(&window)?;
    perform(state, action).await.map_err(|e| format!("{e:#}"))
}

async fn perform(state: Shared<'_>, action: Action) -> anyhow::Result<()> {
    // Image builds can take minutes; keep the service available for shutdown.
    if let Action::Build { source } = &action {
        let settings = control::load()?.isolation.unwrap_or_default();
        let build = image::Build {
            tag: settings.image(),
            source: image::Source::parse(source).context("Choose release or git")?,
            toolchain: image::TOOLCHAIN.trim().into(),
            version: env!("CARGO_PKG_VERSION").into(),
        };
        return control::build_image(&settings, &build).await;
    }
    let _guard = state.lock().await;
    match action {
        Action::Register { name, preferred } => {
            control::register(&hub()?, &name, preferred).await?;
        }
        Action::Prefer { preferred } => {
            let hub = hub()?;
            let machine = control::this(&hub).await?;
            hub.prefer_machine(machine.id, preferred).await?;
        }
        Action::Remove => {
            let hub = hub()?;
            let machine = control::this(&hub).await?;
            service::remove_machine()?;
            hub.revoke_machine(machine.id).await?;
            std::fs::remove_file(machine::path()?)?;
        }
        Action::Folder { path } => {
            control::configure(Some(path), None, None)?;
        }
        Action::Service { personal, workshop } => {
            control::configure(None, Some(personal), Some(workshop))?;
            let config = control::load()?;
            control::check_hub(&config, &hub()?)?;
            if personal || workshop {
                ensure_serving()?;
            } else {
                service::stop()?;
            }
        }
        Action::Serving { enabled } => {
            if enabled {
                ensure_serving()?;
            } else {
                service::stop()?;
            }
        }

        Action::Containers { runtime, image } => {
            control::containers(runtime, image)?;
        }
        Action::Build { .. } => unreachable!("handled before locking the service"),
        Action::Lend { group, enabled } => {
            let hub = hub()?;
            let machine = control::this(&hub).await?;
            if enabled {
                hub.add_workshop_machine(&group, machine.id).await?;
            } else {
                hub.remove_workshop_machine(&group, machine.id).await?;
            }
        }
        Action::Agents { agents } => {
            let hub = hub()?;
            let machine = control::this(&hub).await?;
            hub.set_workshop_agents(machine.id, &agents).await?;
        }
        Action::Credentials { runner, env, files } => {
            control::credentials(runner, env, files)?;
        }
    }
    Ok(())
}

fn ensure_serving() -> anyhow::Result<()> {
    // Installation starts the service too; start is also safe for an existing one.
    let stemma = control::stemma()?;
    service::install(&stemma)
}

#[tauri::command]
async fn open_hub(window: WebviewWindow, app: tauri::AppHandle) -> Result<()> {
    local(&window)?;
    let hub = hub().map_err(error)?;
    if let Some(window) = app.get_webview_window("hub") {
        window.close().map_err(error)?;
    }
    let handoff = hub.login_handoff().await.map_err(error)?;
    let mut url: tauri::Url = format!("{}/login/handoff", hub.base())
        .parse()
        .map_err(error)?;
    url.query_pairs_mut()
        .append_pair("code", &handoff.code)
        .append_pair("next", "/");
    // No capabilities are granted to this remote window. Its browser session
    // belongs to the hub, and the native device token never enters JavaScript.
    tauri::WebviewWindowBuilder::new(&app, "hub", tauri::WebviewUrl::External(url))
        .title("Stemma")
        .build()
        .map_err(error)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(Mutex::new(()))
        .setup(|app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_navigation(trusted_machine_url)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![snapshot, login, action, open_hub])
        .run(tauri::generate_context!())
        .expect("running Stemma");
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions_match_the_workspace() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace: toml::Value = toml::from_str(
            &std::fs::read_to_string(root.join("../../Cargo.toml"))
                .expect("read workspace Cargo.toml"),
        )
        .expect("parse workspace Cargo.toml");
        let expected = workspace["workspace"]["package"]["version"]
            .as_str()
            .unwrap();
        assert_eq!(
            env!("CARGO_PKG_VERSION"),
            expected,
            "desktop Cargo.toml version must match workspace.package.version in the root Cargo.toml"
        );
        let tauri: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(root.join("tauri.conf.json")).expect("read tauri.conf.json"),
        )
        .expect("parse tauri.conf.json");
        if let Some(version) = tauri.get("version") {
            assert_eq!(
                version.as_str(),
                Some(expected),
                "tauri.conf.json version must match workspace.package.version in the root Cargo.toml"
            );
        }
    }

    #[test]
    fn remote_content_cannot_be_a_machine_page() {
        for url in [
            "https://stemma.example.org",
            "https://tauri.localhost",
            "http://localhost:8080",
            "http://tauri.localhost.evil.test",
            "file:///tmp/page.html",
        ] {
            assert!(!super::trusted_machine_url(&url.parse().unwrap()), "{url}");
        }
        assert!(super::trusted_machine_url(
            &"tauri://localhost/".parse().unwrap()
        ));
    }
}
