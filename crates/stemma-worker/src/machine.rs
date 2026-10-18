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
    let mut service = crate::control::Service::default();
    let stemma = std::env::current_exe().context("finding the stemma binary")?;
    service.start(config, stemma).await?;
    tokio::select! {
        served = service.wait() => served,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Stopping");
            service.stop().await;
            Ok(())
        }
    }
}

fn enabled() -> bool {
    true
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
