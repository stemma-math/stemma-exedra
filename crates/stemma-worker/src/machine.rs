//! A person's machine: where their own agents run, with their own sign-in
//! to each runner, and only for them.
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
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use stemma_core::api::runners;

use crate::config::{Runners, WorkerConfig};
use crate::{Context, connection, detect};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineConfig {
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
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        Ok(())
    }

    /// The worker it is, offering `found`, the runners found installed.
    pub fn worker_config(&self, found: &[String]) -> WorkerConfig {
        let mut offered = Runners::default();
        for runner in found {
            let config = self.runners.get(runner).cloned().unwrap_or_default();
            offered.set(runner, config);
        }
        WorkerConfig {
            hub: self.hub.clone(),
            token: Some(self.token.clone()),
            token_env: None,
            work_dir: self.work_dir.clone(),
            concurrency: self.concurrency.unwrap_or(2).max(1),
            stemma: None,
            runners: offered,
            lean: None,
            workshop: None,
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
    let found = installed(&config.runners).await;
    if found.is_empty() {
        tracing::warn!(
            "found none of claude, codex and opencode installed: install one, sign in to it, \
             and start this machine again"
        );
    } else {
        tracing::info!("found {}", names(&found));
    }
    let worker = config.worker_config(&found);
    let stemma = std::env::current_exe().context("finding the stemma binary")?;
    let context = Context::new(worker, stemma)?;
    tokio::select! {
        served = connection::serve(Arc::new(context), config.token.clone()) => served,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Stopping");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_privately_and_offers_what_it_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stemma/machine.toml");
        assert!(MachineConfig::load_from(&path).unwrap().is_none());
        let config = MachineConfig {
            hub: "https://stemma.example.org".into(),
            token: "stw_x".into(),
            id: 3,
            name: "laptop".into(),
            concurrency: None,
            work_dir: None,
            runners: toml::from_str("[codex]\nmodel = \"gpt-5\"\n").unwrap(),
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
        assert_eq!(worker.token().unwrap(), "stw_x");
        assert_eq!(names(&worker.runner_names()), "Claude Code, Codex");
    }
}
