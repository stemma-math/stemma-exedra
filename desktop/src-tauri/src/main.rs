#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use stemma_client::config::{Explained, Settings, Source};
use stemma_client::{Hub, config::Credentials};
use stemma_core::api::Me;
use stemma_worker::{doctor, service, sign_in};
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
    settings: Vec<PanelSetting>,
    /// The names of the containers' sign-ins kept, never their values.
    secrets: Vec<String>,
    checks: Vec<doctor::Check>,
    /// The name of Stemma's own agents' image; another is the person's own.
    stemma_image: String,
    /// The agents signed in for their containers.
    signed_in: Vec<String>,
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
        .map_err(error)?
        .into_iter()
        .map(|setting| panel_setting(&loaded, setting, group.as_deref()))
        .collect::<anyhow::Result<_>>()
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
        checks: Vec::new(),
        stemma_image: stemma_worker::isolation::default_image(),
        signed_in: signed_in(),
    })
}

#[derive(Serialize)]
struct PanelSetting {
    #[serde(flatten)]
    setting: Explained,
    reset: Explained,
}
fn panel_setting(
    settings: &Settings,
    setting: Explained,
    group: Option<&str>,
) -> anyhow::Result<PanelSetting> {
    let reset = settings.inherited(&setting.key, group)?;
    Ok(PanelSetting { setting, reset })
}

#[derive(Serialize)]
struct Diagnostics {
    checks: Vec<doctor::Check>,
    service: service::Status,
    secrets: Vec<String>,
    /// The agents signed in for their containers.
    signed_in: Vec<String>,
}

/// The agents signed in for their containers.
fn signed_in() -> Vec<String> {
    stemma_client::config::Paths::discover()
        .map(|paths| {
            sign_in::AGENTS
                .into_iter()
                .filter(|handle| sign_in::signed_in(&paths, handle))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
#[tauri::command]
async fn workshop_state(window: WebviewWindow) -> Result<serde_json::Value> {
    local(&window)?;
    let paths = stemma_client::config::Paths::discover().map_err(error)?;
    let store = stemma_worker::approvals::read(&paths.state).map_err(error)?;
    let log = std::fs::read_to_string(paths.state.join("logs/workshop.jsonl")).unwrap_or_default();
    Ok(
        serde_json::json!({"store":store,"log":log.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")}),
    )
}
#[tauri::command]
async fn workshop_decision(
    window: WebviewWindow,
    process: Option<i64>,
    approval: Option<u64>,
    mode: String,
    limits: stemma_worker::approvals::Limits,
) -> Result<()> {
    local(&window)?;
    let paths = stemma_client::config::Paths::discover().map_err(error)?;
    let credentials = Credentials::load().map_err(error)?.with_env();
    let hub = credentials
        .hub
        .ok_or_else(|| "Sign in to a hub first".to_owned())?;
    match mode.as_str() {
        "revoke" => stemma_worker::approvals::revoke(
            &paths.state,
            &hub,
            approval.ok_or("Missing approval")?,
        )
        .map_err(error),
        "decline" => {
            stemma_worker::approvals::decline(&paths.state, &hub, process.ok_or("Missing process")?)
                .map_err(error)
        }
        "once" | "series" | "recipe" => stemma_worker::approvals::approve(
            &paths.state,
            &hub,
            process.ok_or("Missing process")?,
            &mode,
            limits,
        )
        .map_err(error),
        _ => Err("Unknown workshop decision".into()),
    }
}
#[tauri::command]
async fn diagnostics(window: WebviewWindow, group: Option<String>) -> Result<Diagnostics> {
    local(&window)?;
    Ok(Diagnostics {
        checks: doctor::run(group.as_deref()).await,
        service: service::status().await.map_err(error)?,
        secrets: Credentials::load()
            .map_err(error)?
            .secrets
            .keys()
            .cloned()
            .collect(),
        signed_in: signed_in(),
    })
}

/// Asks an agent, in a container as an answer's, for one word.
#[tauri::command]
async fn test_sign_in(
    window: WebviewWindow,
    agent: String,
    group: Option<String>,
) -> Result<String> {
    local(&window)?;
    sign_in::test(&agent, group.as_deref()).await.map_err(error)
}

#[tauri::command]
async fn open_external(window: WebviewWindow, url: String) -> Result<()> {
    local(&window)?;
    launch_browser(&url)
}
fn launch_browser(url: &str) -> Result<()> {
    let parsed = tauri::Url::parse(url).map_err(error)?;
    if !matches!(parsed.scheme(), "https" | "http" | "mailto") {
        return Err("This address cannot be opened in the browser".into());
    }
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer");
    command.arg(url).spawn().map_err(error)?;
    Ok(())
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
    /// Signs an agent in for its containers, apart from the person's own
    /// sign-in: Codex, whose sign-in opens the browser.
    SignIn {
        agent: String,
    },
    /// Forgets an agent's sign-in for its containers.
    SignOut {
        agent: String,
    },
}

#[tauri::command]
async fn action(
    window: WebviewWindow,
    state: Shared<'_>,
    action: Action,
) -> Result<Option<PanelSetting>> {
    local(&window)?;
    // Only file writes share the lock; a long image build or service action
    // must not hold up an unrelated setting.
    let _guard = if matches!(
        action,
        Action::Set { .. } | Action::Unset { .. } | Action::Secret { .. } | Action::SignOut { .. }
    ) {
        Some(state.lock().await)
    } else {
        None
    };
    perform(action).await.map_err(error)
}
async fn perform(action: Action) -> anyhow::Result<Option<PanelSetting>> {
    match action {
        Action::SignIn { agent } => {
            anyhow::ensure!(
                agent == "codex",
                "Sign {agent} in from a terminal: stemma machine sign-in {agent}"
            );
            let paths = stemma_client::config::Paths::discover()?;
            let program = Settings::load()?
                .get("agents.codex.command", None)?
                .value
                .as_str()
                .unwrap_or("codex")
                .to_owned();
            let mut command =
                tokio::process::Command::from(sign_in::command(&paths, &agent, &program)?);
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            // Codex opens the browser and waits for the person there.
            let output =
                tokio::time::timeout(std::time::Duration::from_secs(600), command.output())
                    .await
                    .context("Codex's sign-in did not finish within ten minutes")??;
            anyhow::ensure!(
                output.status.success() && sign_in::signed_in(&paths, &agent),
                "Codex did not sign in: {}",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            );
        }
        Action::SignOut { agent } => {
            sign_in::runner_of(&agent)?;
            sign_in::forget(&stemma_client::config::Paths::discover()?, &agent)?;
        }
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
            let loaded = Settings::load()?;
            return Ok(Some(panel_setting(
                &loaded,
                loaded.get(&key, group.as_deref())?,
                group.as_deref(),
            )?));
        }
        Action::Unset { key, group } => {
            let settings = Settings::load()?;
            anyhow::ensure!(
                !matches!(
                    settings.get(&key, group.as_deref())?.source,
                    Source::Environment(_)
                ),
                "This field is set by an environment variable"
            );
            settings.write(&key, None, group.as_deref())?;
            let loaded = Settings::load()?;
            return Ok(Some(panel_setting(
                &loaded,
                loaded.get(&key, group.as_deref())?,
                group.as_deref(),
            )?));
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
    Ok(None)
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
                .on_new_window(|url, _| {
                    let _ = launch_browser(url.as_str());
                    tauri::webview::NewWindowResponse::Deny
                })
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
            workshop_state,
            workshop_decision,
            snapshot,
            diagnostics,
            open_external,
            login,
            action,
            test_sign_in,
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
