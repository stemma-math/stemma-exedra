#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use stemma_client::{Hub, config::Credentials};
use stemma_core::api::{Machine, Me, WorkshopAgent};
use stemma_worker::{
    control, detect, image,
    isolation::{AgentSignIn, Runtime, SignInFile},
    machine, service,
};
use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{Emitter, Manager, State, WebviewWindow};
use transport::{subscribe, unsubscribe};

mod page;
mod transport;
use tokio::sync::Mutex;

type Shared<'a> = State<'a, Mutex<()>>;
type Result<T> = std::result::Result<T, String>;

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn local(window: &WebviewWindow) -> Result<()> {
    let url = window.url().map_err(error)?;
    let origin = window.app_handle().state::<page::Origin>();
    if window.label() != "main" || !page::same_origin(&url, &origin.0) {
        return Err("Local machine access denied".into());
    }
    Ok(())
}
fn hub() -> anyhow::Result<Hub> {
    Credentials::load()?.with_env().hub()
}

#[derive(Serialize)]
struct Snapshot {
    hub: Option<String>,
    username: Option<String>,
    me: Option<Me>,
    hub_error: Option<String>,
    registered: bool,
    offers: Vec<WorkshopAgent>,
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
    image_ready: bool,
    credentials: BTreeMap<String, AgentSignIn>,
}

#[tauri::command]
async fn snapshot(window: WebviewWindow) -> Result<Snapshot> {
    local(&window)?;
    let config = Credentials::load().map_err(error)?.with_env();
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
                            Ok(()) => match control::this(&hub).await {
                                Ok(machine) => remote = Some(machine),
                                Err(e)
                                    if e.downcast_ref::<machine::Stopped>()
                                        == Some(&machine::Stopped::NotKnown) => {}
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
    let image_ready = if machine.is_some() && settings.runtime() != Runtime::Host {
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            stemma_worker::isolation::resolve(
                &settings,
                config.hub.as_deref().unwrap_or(""),
                std::env::var_os("PATH").as_deref(),
            ),
        )
        .await
        .is_ok_and(|result| result.is_ok())
    } else {
        false
    };
    let mut service = service::status().await.map_err(error)?;
    if machine.is_none() {
        service.running = false;
        service.connected = None;
    }
    let personal = service.personal;
    let workshop = service.workshop;
    Ok(Snapshot {
        hub: config.hub,
        username: config.username,
        me,
        hub_error,
        registered: machine.is_some(),
        offers: machine
            .as_ref()
            .map(|c| c.offers.clone())
            .unwrap_or_default(),
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
        image_ready,
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
        if Credentials::load().map_err(error)?.username.as_deref() != Some(&username) {
            return Err(
                "Remove this machine from its current account before switching accounts".into(),
            );
        }
    }
    stemma_client::sign_in::password(&url, &username, &password, "desktop", None)
        .await
        .map_err(|e| format!("{e:#}"))?;
    transport::stop(window.app_handle());
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
    Register {
        name: String,
        preferred: bool,
    },
    RegisterAgain,
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
        control::build_image(&settings, &build).await.map_err(|_| anyhow::anyhow!(
            "The image could not be prepared. Make sure Docker or Podman is running, then try again."
        ))?;
        let config = control::load()?;
        if config.personal || config.workshop_enabled {
            ensure_serving()?;
        }
        return Ok(());
    }
    let _guard = state.lock().await;
    match action {
        Action::Register { name, preferred } => {
            let config = control::register(&hub()?, &name, preferred).await?;
            ensure_serving()?;
            control::wait_online(&config).await?;
        }
        Action::RegisterAgain => {
            let config = control::register_again(&hub()?).await?;
            ensure_serving()?;
            control::wait_online(&config).await?;
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
        Action::Service { personal } => {
            let workshop = !control::this(&hub()?).await?.workshop.is_empty();
            control::configure(None, Some(personal), Some(workshop))?;
            let config = control::load()?;
            control::check_hub(&config, &hub()?)?;
            if personal || workshop {
                ensure_serving()?;
                control::wait_online(&control::load()?).await?;
            } else {
                service::stop()?;
            }
        }
        Action::Serving { enabled } => {
            if enabled {
                ensure_serving()?;
                control::wait_online(&control::load()?).await?;
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
            if enabled {
                return control::lend(&hub, &group, &control::stemma()?).await;
            }
            let machine = control::this(&hub).await?;
            hub.remove_workshop_machine(&group, machine.id).await?;
            let workshop = !control::this(&hub).await?.workshop.is_empty();
            let config = control::load()?;
            control::configure(None, None, Some(workshop))?;
            if config.personal || workshop {
                ensure_serving()?;
                control::wait_online(&control::load()?).await?;
            } else {
                service::stop()?;
            }
        }
        Action::Agents { agents } => {
            let hub = hub()?;
            let machine = control::this(&hub).await?;
            control::offers(&hub, &machine, &agents).await?;
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
async fn sign_out(window: WebviewWindow, state: Shared<'_>) -> Result<Option<String>> {
    local(&window)?;
    let _guard = state.lock().await;
    transport::stop(window.app_handle());
    stemma_client::sign_in::logout().await.map_err(error)
}

/// An app opened from the Finder or the Dock gets a bare `PATH`, without
/// the folders where the agents and the tools they need are installed: it
/// takes the login shell's, for finding them, for what it runs, and for the
/// service it installs.
#[allow(unsafe_code)]
fn use_login_path() {
    if let Some(path) = stemma_worker::detect::login_path() {
        // SAFETY: called first thing in `main`, before any other thread
        // exists to read the environment.
        unsafe { std::env::set_var("PATH", path) };
    }
}

fn main() {
    use_login_path();
    tauri::Builder::default()
        .manage(Mutex::new(()))
        .manage(transport::Streams::default())
        .register_asynchronous_uri_scheme_protocol("hub", |context, request, responder| {
            let origin = context.app_handle().state::<page::Origin>().0.clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(transport::proxy(request, &origin).await);
            });
        })
        .setup(|app| {
            let origin = if cfg!(debug_assertions) {
                // A different host from devUrl: it must be Remote to Tauri's ACL too.
                "http://localhost:1420/".parse()?
            } else {
                tauri::async_runtime::block_on(page::serve(app.handle().clone()))?
            };
            app.add_capability(page::capability(&origin).to_string())?;
            app.manage(page::Origin(origin.clone()));
            let allowed_origin = origin.clone();
            let mut window = app.config().app.windows[0].clone();
            window.url = tauri::WebviewUrl::External(origin);
            tauri::WebviewWindowBuilder::from_config(app, &window)?
                .on_navigation(move |url| page::same_origin(url, &allowed_origin))
                .build()?;
            let menu = Menu::default(app.handle())?;
            let item = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
            if let Some(MenuItemKind::Submenu(submenu)) = menu.items()?.first() {
                submenu.insert_items(&[&PredefinedMenuItem::separator(app)?, &item], 1)?;
            }
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "settings" {
                let _ = app.emit_to("main", "open-settings", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            login,
            action,
            sign_out,
            subscribe,
            unsubscribe
        ])
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
    fn capabilities_and_csp_are_local_only() {
        let cap: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/local.json")).unwrap();
        assert_eq!(cap["local"], false);
        assert_eq!(cap["windows"], serde_json::json!(["main"]));
        assert!(cap.get("remote").is_none());
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["app"]["windows"].as_array().unwrap().len(), 1);
        for key in ["csp", "devCsp"] {
            let csp = config["app"]["security"][key].as_str().unwrap();
            assert!(csp.contains("script-src 'self';"));
            assert!(!csp.contains("unsafe-eval"));
            assert!(csp.contains("frame-src hub: http://hub.localhost;"));
            assert_eq!(
                csp.split(';')
                    .map(str::trim)
                    .find(|directive| directive.starts_with("object-src ")),
                Some("object-src hub: http://hub.localhost"),
            );
        }
    }
}
