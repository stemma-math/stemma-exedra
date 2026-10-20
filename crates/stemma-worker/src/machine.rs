//! A person's machine: where their own agents run, with their own sign-in
//! to each runner, and only for them; and, if its owner makes it work for a
//! group's workshop, where the workshop's tasks and builds run, with the
//! agents its owner offers the workshop.
//!
//! `stemma machine add` registers it with the hub and writes its
//! configuration, with its token, in the user's configuration directory
//! (`stemma/machine.toml`, or where `STEMMA_MACHINE_CONFIG` says),
//! readable only by them; `stemma machine run` serves it. When it starts,
//! it offers the runners it finds installed and usable: `claude`, `codex`
//! and `opencode` on its `PATH`, each answering `--version`.
//!
//! The configuration is written for it, and may be changed:
//!
//! ```toml
//! hub = "https://stemma.example.org"
//! token = "stw_…"
//! id = 12                     # the machine, on the hub
//! name = "laptop"
//! # concurrency = 2           # jobs at once; 2 if absent
//! # work_dir = "…"            # where jobs run; a folder of the user's data if absent
//!
//! # How to run a runner, if not as usual:
//! # [runners.claude-code]
//! # command = "…"; args = […]; model = "…"; timeout_minutes = 15
//!
//! # For the workshop, with Lean (elan) installed:
//! # [lean]
//! # timeout_minutes = 90; cache = true   # whether to get Mathlib's cache
//! # [latex]                              # the papers' builds
//! # timeout_minutes = 15
//! # [workshop]
//! # timeout_minutes = 120; rounds = 2; max_turns = 80
//!
//! # How the workshop's jobs are isolated (see `crate::isolation`):
//! # [isolation]
//! # runtime = "auto"          # docker, podman, or host: unisolated
//! # image = "stemma-workshop:<version>"
//! # memory = "8g"; cpus = 4; pids = 4096; network = "…"; hub_url = "…"
//! # [isolation.agents.claude-code]
//! # env = ["CLAUDE_CODE_OAUTH_TOKEN"]   # passed by name
//! # [isolation.agents.codex]
//! # files = [{ from = "~/.codex/auth.json" }]   # mounted read-only
//! ```

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use stemma_core::api::runners;

use crate::config::{LatexConfig, LeanConfig, Runners, WorkerConfig, WorkshopConfig};
use crate::detect;
use crate::isolation::IsolationConfig;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineConfig {
    #[serde(default = "enabled")]
    pub personal: bool,
    #[serde(default)]
    pub workshop_enabled: bool,
    /// The hub's address.
    pub hub: String,
    /// The machine's token.
    pub token: String,
    /// The machine, on the hub.
    pub id: i64,
    pub name: String,
    /// How many jobs run at once; 2 if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concurrency: Option<u32>,
    /// Where jobs run; a folder of the user's data if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<PathBuf>,
    /// How to run the runners it finds, if not as usual.
    #[serde(default)]
    pub runners: Runners,
    /// How it builds libraries for the workshop, with Lean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lean: Option<LeanConfig>,
    /// How it compiles papers for the workshop, with LaTeX.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<LatexConfig>,
    /// How it works on the workshop's tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workshop: Option<WorkshopConfig>,
    /// How it isolates the workshop's jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<IsolationConfig>,
    /// Keep the offers here so replacing a lost registration can restore them.
    #[serde(default)]
    pub offers: Vec<stemma_core::api::WorkshopAgent>,
    /// A replacement token was saved, but its offers still need restoring.
    #[serde(default)]
    pub restore_offers: bool,
}

/// Where a machine's configuration is.
pub fn path() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("STEMMA_MACHINE_CONFIG") {
        return Ok(path.into());
    }
    let dir = dirs::config_dir().context("this system has no configuration directory")?;
    Ok(dir.join("stemma").join("machine.toml"))
}

impl MachineConfig {
    /// The machine's configuration, if it has one.
    pub fn load_from(path: &Path) -> anyhow::Result<Option<MachineConfig>> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .map(Some)
                .with_context(|| format!("reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Writes the configuration, readable only by its owner.
    pub fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = format!(
            "# This machine, where your own agents run: `stemma machine run` serves it.\n{}",
            toml::to_string(self)?
        );
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let temporary = path.with_extension(format!("toml.{}.tmp", rand::random::<u64>()));
        let result = (|| {
            let mut file = options
                .open(&temporary)
                .with_context(|| format!("writing {}", path.display()))?;
            std::io::Write::write_all(&mut file, text.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }

    /// The worker it is, offering `found`, the runners found installed.
    pub fn worker_config(&self, found: &[String]) -> WorkerConfig {
        let mut offered = Runners::default();
        for runner in found {
            let config = self.runners.get(runner).cloned().unwrap_or_default();
            offered.set(runner, config);
        }
        WorkerConfig {
            personal: self.personal,
            workshop_enabled: self.workshop_enabled,
            hub: self.hub.clone(),
            work_dir: self.work_dir.clone(),
            concurrency: self.concurrency.unwrap_or(2).max(1),
            runners: offered,
            lean: self.lean.clone().unwrap_or_default(),
            latex: self.latex.clone().unwrap_or_default(),
            workshop: self.workshop.clone().unwrap_or_default(),
            isolation: self.isolation.clone().unwrap_or_default(),
        }
    }
}

/// The runners installed and usable here, as a machine's configuration
/// says to run them.
pub async fn installed(runners: &Runners) -> Vec<String> {
    detect::installed(runners, std::env::var_os("PATH").as_deref()).await
}

/// The runners, as people read them: "Claude Code, Codex".
pub fn names(found: &[String]) -> String {
    if found.is_empty() {
        return "none".into();
    }
    found
        .iter()
        .map(|r| runners::name(r))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Serves the machine configured at `path` until it is stopped.
pub async fn run(path: &Path) -> anyhow::Result<()> {
    let config = MachineConfig::load_from(path)?.with_context(|| {
        format!(
            "this machine is not set up ({} does not exist): run `stemma machine add`",
            path.display()
        )
    })?;
    // One process serves a machine: the lock sits beside its configuration,
    // not in its configurable checkout folder.
    let _lock = lock(&path.with_extension("lock"))?;
    if !config.personal && !config.workshop_enabled {
        tracing::info!("Both machine modes are off");
        return Ok(());
    }
    if let Some(reason @ Stopped::NotKnown) = stopped(path, &config)? {
        tracing::warn!("{reason}");
        return Ok(());
    }
    if let Ok(Err(error)) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        crate::control::identity(&config),
    )
    .await
        && error.downcast_ref::<Stopped>() == Some(&Stopped::NotKnown)
    {
        record_stopped(path, &config, &Stopped::NotKnown)?;
        tracing::warn!("{}", Stopped::NotKnown);
        return Ok(());
    }
    let found = installed(&config.runners).await;
    tracing::info!("found {}", names(&found));
    let stemma = std::env::current_exe().context("finding the stemma binary")?;
    let context = crate::Context::start(config.worker_config(&found), stemma).await?;
    anyhow::ensure!(
        !config.workshop_enabled || context.isolator.is_some(),
        "The workshop has no usable isolation. Choose Docker or Podman and build the configured image."
    );
    tokio::select! {
        served = crate::connection::serve(std::sync::Arc::new(context), config.token.clone()) => {
            match served {
                Err(error) if error.downcast_ref::<Stopped>().is_some() => {
                    let reason = error.downcast_ref::<Stopped>().expect("checked above");
                    record_stopped(path, &config, reason)?;
                    tracing::warn!("{reason}");
                    // Both launchd and systemd restart failures, never a successful exit.
                    Ok(())
                }
                result => result,
            }
        },
        _ = shutdown() => Ok(()),
    }
}

/// A terminal service state, readable even when the hub is unreachable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "reason", rename_all = "snake_case")]
pub enum Stopped {
    NotKnown,
    Refused(String),
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotKnown => f.write_str("The hub no longer knows this machine."),
            Self::Refused(reason) => write!(f, "The hub refused this machine: {reason}"),
        }
    }
}
impl std::error::Error for Stopped {}

#[derive(Serialize, Deserialize)]
struct ServiceState {
    registration: String,
    stopped: Stopped,
}

fn registration(config: &MachineConfig) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(format!("{}\0{}", config.hub, config.token))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn stopped(path: &Path, config: &MachineConfig) -> anyhow::Result<Option<Stopped>> {
    let text = match std::fs::read(path.with_extension("state.json")) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let state: ServiceState = serde_json::from_slice(&text)?;
    Ok((state.registration == registration(config)).then_some(state.stopped))
}

pub fn record_stopped(
    path: &Path,
    config: &MachineConfig,
    stopped: &Stopped,
) -> anyhow::Result<()> {
    let state = ServiceState {
        registration: registration(config),
        stopped: stopped.clone(),
    };
    let path = path.with_extension("state.json");
    let temporary = path.with_extension(format!("json.{}.tmp", rand::random::<u64>()));
    std::fs::write(&temporary, serde_json::to_vec(&state)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn lock(path: &Path) -> anyhow::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock().context("Another process is already serving this machine; stop it before running `stemma machine run`")?;
    Ok(file)
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("listen for SIGTERM");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Stopping");
}

fn enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_process_can_serve_and_exit_releases_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.lock");
        let first = lock(&path).unwrap();
        let error = lock(&path).unwrap_err();
        assert!(error.to_string().contains("already serving this machine"));
        drop(first);
        assert!(lock(&path).is_ok());
    }

    #[test]
    fn saves_privately_and_offers_what_it_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stemma/machine.toml");
        assert!(MachineConfig::load_from(&path).unwrap().is_none());
        let config = MachineConfig {
            personal: true,
            workshop_enabled: true,
            hub: "https://stemma.example.org".into(),
            token: "stw_x".into(),
            id: 3,
            name: "laptop".into(),
            concurrency: None,
            work_dir: None,
            runners: toml::from_str("[codex]\nmodel = \"gpt-5\"\n").unwrap(),
            lean: None,
            latex: None,
            workshop: None,
            isolation: None,
            offers: Vec::new(),
            restore_offers: false,
        };
        config.save_to(&path).unwrap();
        let read = MachineConfig::load_from(&path).unwrap().unwrap();
        assert_eq!(read.token, "stw_x");
        assert_eq!(read.name, "laptop");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let worker = read.worker_config(&["claude-code".into(), "codex".into()]);
        assert_eq!(worker.runner_names(), ["claude-code", "codex"]);
        assert_eq!(
            worker.runners.get("codex").unwrap().model.as_deref(),
            Some("gpt-5")
        );
        assert_eq!(worker.concurrency, 2);
        assert_eq!(names(&worker.runner_names()), "Claude Code, Codex");
    }
}
