//! A group worker's configuration: a TOML file. (A person's machine has
//! its own, which `stemma machine add` writes: see [`crate::machine`].)
//!
//! ```toml
//! hub = "https://stemma.example.org"
//! token_env = "STEMMA_WORKER_TOKEN"   # or token = "stw_…"
//! work_dir = "/var/lib/stemma-worker" # where jobs run; a default if absent
//! concurrency = 2                     # jobs at once
//!
//! [runners.claude-code]               # each runner listed is offered
//! [runners.codex]
//! model = "gpt-5-codex"
//! [runners.opencode]                  # OpenCode, with a model of any provider
//! model = "openrouter/qwen/qwen3-coder"
//!
//! [lean]                              # library builds, with Lean installed
//! timeout_minutes = 90
//! [workshop]                          # tasks of the workshop, with [lean]
//! timeout_minutes = 120
//! ```
//!
//! Keys are never written in the file: each runner uses its own login, or
//! its providers' keys from the environment.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use stemma_core::api::runners;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// The hub's address.
    pub hub: String,
    /// The worker's token, from the hub's settings.
    pub token: Option<String>,
    /// The environment variable that holds the token, instead.
    pub token_env: Option<String>,
    /// Where jobs run, each in a folder of its own.
    pub work_dir: Option<PathBuf>,
    /// How many jobs run at once.
    #[serde(default = "one")]
    pub concurrency: u32,
    /// The `stemma` binary the agents' MCP server runs; this one if absent.
    pub stemma: Option<PathBuf>,
    #[serde(default)]
    pub runners: Runners,
    /// How a group's worker with Lean builds its library.
    pub lean: Option<LeanConfig>,
    /// How it works on the workshop's tasks, if it does: it needs [`lean`]
    /// and a runner that edits files (`opencode`, `claude-code`, `codex`).
    ///
    /// [`lean`]: WorkerConfig::lean
    pub workshop: Option<WorkshopConfig>,
}

/// How a worker works on the workshop's tasks.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopConfig {
    /// How long a task may take, all in all; 120 minutes if absent.
    pub timeout_minutes: Option<u64>,
    /// How many times the runner may work on a task: the first, and then
    /// to fix what the build or the inspection found; 2 if absent.
    pub rounds: Option<u32>,
    /// How many steps (turns) a runner that counts them may take in a
    /// round; 80 if absent.
    pub max_turns: Option<u32>,
}

/// How libraries are built. A worker builds them if this is configured, it
/// has Lean (`elan` or `lake` on its path) and it is a group's.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LeanConfig {
    /// How long a build may take; 90 minutes if absent.
    pub timeout_minutes: Option<u64>,
    /// Whether to get Mathlib's cache before building; yes if absent.
    pub cache: Option<bool>,
}

fn one() -> u32 {
    1
}

/// The runners this worker offers: those listed. For a person's machine,
/// how to run those it finds installed.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Runners {
    #[serde(rename = "claude-code", skip_serializing_if = "Option::is_none")]
    pub claude_code: Option<CliRunner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CliRunner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opencode: Option<CliRunner>,
}

impl Runners {
    /// How to run `runner`, if it is listed.
    pub fn get(&self, runner: &str) -> Option<&CliRunner> {
        match runner {
            runners::CLAUDE_CODE => self.claude_code.as_ref(),
            runners::CODEX => self.codex.as_ref(),
            runners::OPENCODE => self.opencode.as_ref(),
            _ => None,
        }
    }

    /// Lists `runner`, as `config` says to run it.
    pub fn set(&mut self, runner: &str, config: CliRunner) {
        match runner {
            runners::CLAUDE_CODE => self.claude_code = Some(config),
            runners::CODEX => self.codex = Some(config),
            runners::OPENCODE => self.opencode = Some(config),
            _ => {}
        }
    }
}

/// A runner that is a command-line harness.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CliRunner {
    /// The command; the harness's usual name if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// More arguments to give it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// The model, unless an agent's profile names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How long a job may take; 15 minutes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u64>,
}

impl WorkerConfig {
    pub fn load(path: &Path) -> anyhow::Result<WorkerConfig> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: WorkerConfig =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.check()?;
        Ok(config)
    }

    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.runner_names().is_empty() || self.lean.is_some(),
            "the worker offers nothing: add a runner, such as [runners.claude-code], or [lean]"
        );
        anyhow::ensure!(self.concurrency >= 1, "concurrency is at least 1");
        if self.workshop.is_some() {
            anyhow::ensure!(
                self.lean.is_some(),
                "the workshop needs Lean: add [lean] beside [workshop]"
            );
            anyhow::ensure!(
                !self.workshop_runners().is_empty(),
                "the workshop needs a runner that edits files: add [runners.opencode], \
                 [runners.claude-code] or [runners.codex]"
            );
        }
        Ok(())
    }

    /// The worker's token.
    pub fn token(&self) -> anyhow::Result<String> {
        match (&self.token, &self.token_env) {
            (_, Some(var)) => std::env::var(var)
                .ok()
                .filter(|t| !t.trim().is_empty())
                .map(|t| t.trim().to_owned())
                .with_context(|| format!("the environment variable {var} holds no token")),
            (Some(token), None) => Ok(token.trim().to_owned()),
            (None, None) => anyhow::bail!("the configuration has no token, nor token_env"),
        }
    }

    /// The runners offered, by name.
    pub fn runner_names(&self) -> Vec<String> {
        runners::ALL
            .iter()
            .filter(|name| self.runners.get(name).is_some())
            .map(|name| (*name).to_owned())
            .collect()
    }

    /// The runners offered that can work on the workshop's tasks.
    pub fn workshop_runners(&self) -> Vec<String> {
        self.runner_names()
            .into_iter()
            .filter(|name| runners::WORKSHOP.contains(&name.as_str()))
            .collect()
    }

    pub fn work_dir(&self) -> PathBuf {
        self.work_dir.clone().unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("stemma")
                .join("worker")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_configuration() {
        let config: WorkerConfig = toml::from_str(
            r#"
            hub = "https://stemma.example.org"
            token = "stw_x"
            concurrency = 2

            [runners.claude-code]
            [runners.opencode]
            model = "openrouter/qwen/qwen3-coder"
            "#,
        )
        .unwrap();
        config.check().unwrap();
        assert_eq!(config.runner_names(), ["claude-code", "opencode"]);
        assert_eq!(config.token().unwrap(), "stw_x");
    }

    #[test]
    fn rejects_what_cannot_work() {
        let none: WorkerConfig = toml::from_str("hub = \"x\"\ntoken = \"t\"").unwrap();
        assert!(none.check().is_err());
        // There is no `api` runner.
        assert!(
            toml::from_str::<WorkerConfig>("hub = \"x\"\ntoken = \"t\"\n[runners.api]").is_err()
        );
        assert!(toml::from_str::<WorkerConfig>("hub = \"x\"\ntokn = \"t\"").is_err());
        let workshop_without_lean: WorkerConfig =
            toml::from_str("hub = \"x\"\ntoken = \"t\"\n[runners.opencode]\n[workshop]").unwrap();
        assert!(workshop_without_lean.check().is_err());
        let workshop_without_runner: WorkerConfig =
            toml::from_str("hub = \"x\"\ntoken = \"t\"\n[lean]\n[workshop]").unwrap();
        assert!(workshop_without_runner.check().is_err());
        let workshop: WorkerConfig =
            toml::from_str("hub = \"x\"\ntoken = \"t\"\n[runners.opencode]\n[lean]\n[workshop]")
                .unwrap();
        workshop.check().unwrap();
        assert_eq!(workshop.workshop_runners(), ["opencode"]);
    }
}
