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
    machine,
};
use tauri::{Manager, State, WebviewWindow};
use tokio::sync::Mutex;

#[derive(Default)]
struct Local {
    service: control::Service,
}
type Shared<'a> = State<'a, Mutex<Local>>;
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
    serving: bool,
    agents: Vec<detect::AgentStatus>,
    elan: Option<String>,
    latex: Option<String>,
    runtime: Runtime,
    image: String,
    credentials: BTreeMap<String, AgentSignIn>,
}

#[tauri::command]
async fn snapshot(window: WebviewWindow, state: Shared<'_>) -> Result<Snapshot> {
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
    let state = state.lock().await;
    let serving = state.service.running();
    let personal = state.service.personal();
    let workshop = state.service.workshop();
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
        serving,
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
    state: Shared<'_>,
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
    state.lock().await.service.stop().await;
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
    let mut state = state.lock().await;
    let was_running = state.service.running();
    let restart = matches!(
        &action,
        Action::Folder { .. } | Action::Containers { .. } | Action::Credentials { .. }
    );
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
            state.service.stop().await;
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
            let stemma = if personal {
                control::stemma()?
            } else {
                PathBuf::from("stemma")
            };
            state.service.start(config, stemma).await?;
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
    if was_running
        && restart
        && let Ok(config) = control::load()
    {
        let stemma = if config.personal {
            control::stemma()?
        } else {
            PathBuf::from("stemma")
        };
        state.service.start(config, stemma).await?;
    }
    Ok(())
}

#[tauri::command]
async fn open_hub(window: WebviewWindow, app: tauri::AppHandle) -> Result<()> {
    local(&window)?;
    let hub = hub().map_err(error)?;
    if let Some(window) = app.get_webview_window("hub") {
        window.close().map_err(error)?;
    }
    // No capabilities are granted to this remote window. Its browser session
    // belongs to the hub, and the native device token never enters JavaScript.
    tauri::WebviewWindowBuilder::new(
        &app,
        "hub",
        tauri::WebviewUrl::External(hub.base().parse().map_err(error)?),
    )
    .title("Stemma")
    .build()
    .map_err(error)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(Mutex::new(Local::default()))
        .setup(|app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_navigation(trusted_machine_url)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![snapshot, login, action, open_hub])
        .build(tauri::generate_context!())
        .expect("building Stemma")
        .run(|app, event| {
            let closing = match event {
                tauri::RunEvent::ExitRequested {
                    code: None, api, ..
                } => {
                    api.prevent_exit();
                    true
                }
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::CloseRequested { api, .. },
                    ..
                } if label == "machine" => {
                    api.prevent_close();
                    true
                }
                _ => false,
            };
            if closing {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    app.state::<Mutex<Local>>()
                        .lock()
                        .await
                        .service
                        .stop()
                        .await;
                    app.exit(0);
                });
            }
        });
}

#[cfg(test)]
mod tests {
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
