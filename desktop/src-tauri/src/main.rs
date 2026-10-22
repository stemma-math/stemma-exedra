#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use stemma_client::config::{Explained, Settings, Source};
use stemma_client::{Hub, config::Credentials};
use stemma_core::api::Me;
use stemma_worker::{doctor, service};
use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{Emitter, Manager, State, WebviewWindow};
use transport::{subscribe, unsubscribe};

mod page;
mod transport;
use tokio::sync::Mutex;

type Shared<'a> = State<'a, Mutex<()>>;
type Result<T> = std::result::Result<T, String>;

fn error(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
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
    service: service::Status,
    settings: Vec<Explained>,
    /// The names of the containers' sign-ins kept, never their values.
    secrets: Vec<String>,
    checks: Vec<doctor::Check>,
}

#[tauri::command]
async fn snapshot(window: WebviewWindow, group: Option<String>) -> Result<Snapshot> {
    local(&window)?;
    let credentials = Credentials::load().map_err(error)?.with_env();
    let (me, hub_error) = match hub() {
        Ok(hub) => match hub.me().await {
            Ok(me) => (Some(me), None),
            Err(e) => (None, Some(error(e))),
        },
        Err(e) => (None, Some(error(e))),
    };
    let loaded = Settings::load().map_err(error)?;
    let settings = loaded
        .explain(group.as_deref(), &Default::default())
        .map_err(error)?;
    let secrets = credentials.secrets.keys().cloned().collect();
    Ok(Snapshot {
        hub: credentials.hub,
        username: credentials.username,
        me,
        hub_error,
        service: service::status().await.map_err(error)?,
        settings,
        secrets,
        checks: doctor::run(group.as_deref()).await,
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
    let host = std::env::var("HOSTNAME")
        .or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .map_err(|_| std::env::VarError::NotPresent)
        })
        .unwrap_or_else(|_| "This computer".into());
    stemma_client::sign_in::password(&url, &username, &password, &host, None)
        .await
        .map_err(error)?;
    transport::stop(window.app_handle());
    Ok(())
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
    Set {
        key: String,
        value: String,
        group: Option<String>,
    },
    Unset {
        key: String,
        group: Option<String>,
    },
    Service {
        enabled: bool,
    },
    Image {
        group: Option<String>,
    },
    ShowWorkspace {
        group: Option<String>,
    },
    /// Keeps, or forgets, the value of a variable a container is given.
    Secret {
        name: String,
        value: Option<String>,
    },
}

#[tauri::command]
async fn action(window: WebviewWindow, state: Shared<'_>, action: Action) -> Result<()> {
    local(&window)?;
    let _guard = state.lock().await;
    perform(action).await.map_err(error)
}
async fn perform(action: Action) -> anyhow::Result<()> {
    match action {
        Action::Set { key, value, group } => {
            let settings = Settings::load()?;
            anyhow::ensure!(
                !matches!(
                    settings.get(&key, group.as_deref())?.source,
                    Source::Environment(_)
                ),
                "This field is set by an environment variable"
            );
            settings.write(&key, Some(&value), group.as_deref())?;
        }
        Action::Unset { key, group } => {
            Settings::load()?.write(&key, None, group.as_deref())?;
        }
        Action::Service { enabled } => {
            if enabled {
                hub()?.me().await?;
                let stemma = stemma_worker::detect::find_program(
                    "stemma",
                    std::env::var_os("PATH").as_deref(),
                )
                .context("Install the stemma CLI before starting the service")?;
                service::install(&stemma)?;
            } else {
                service::uninstall()?;
            }
        }
        Action::Image { group } => stemma_worker::image::build_for(group.as_deref()).await?,
        Action::Secret { name, value } => {
            stemma_client::config::check_secret_name(&name)?;
            let path = stemma_client::config::path()?;
            let mut credentials = Credentials::load_from(&path)?;
            match value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) {
                Some(value) => credentials.secrets.insert(name, value),
                None => credentials.secrets.remove(&name),
            };
            credentials.save_to(&path)?;
        }
        Action::ShowWorkspace { group } => {
            let settings = Settings::load()?;
            let folder = if let Some(group) = group {
                settings.workspace(&group)?
            } else {
                stemma_client::config::expand_home(
                    settings
                        .get("agents.workspace", None)?
                        .value
                        .as_str()
                        .unwrap(),
                )?
            };
            #[cfg(target_os = "macos")]
            let mut command = std::process::Command::new("open");
            #[cfg(target_os = "linux")]
            let mut command = std::process::Command::new("xdg-open");
            #[cfg(target_os = "windows")]
            let mut command = std::process::Command::new("explorer");
            command.arg(folder).spawn()?;
        }
    }
    Ok(())
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
            let mut builder = tauri::WebviewWindowBuilder::from_config(app, &window)?;
            if let Some(directory) = std::env::var_os("STEMMA_WEBVIEW_DIR") {
                builder = builder.data_directory(directory.into());
                #[cfg(target_os = "macos")]
                {
                    builder = builder.incognito(true);
                }
            }
            builder
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
    fn action_errors_keep_the_cause() {
        let failure = anyhow::anyhow!("permission denied").context("Could not start the service");
        assert_eq!(
            super::error(failure),
            "Could not start the service: permission denied"
        );
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
