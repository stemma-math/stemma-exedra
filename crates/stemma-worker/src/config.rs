//! How a machine runs: what its configuration (see [`crate::machine`])
//! makes of it, with the runners it found installed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use stemma_core::api::runners;

use crate::isolation::IsolationConfig;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    /// The hub's address.
    pub hub: String,
    /// Where jobs run, each in a folder of its own.
    pub work_dir: Option<PathBuf>,
    /// How many jobs run at once.
    #[serde(default = "one")]
    pub concurrency: u32,
    #[serde(default)]
    pub runners: Runners,
    /// How the workshop builds the library, with Lean.
    #[serde(default)]
    pub lean: LeanConfig,
    /// How the workshop compiles papers, with LaTeX.
    #[serde(default)]
    pub latex: LatexConfig,
    /// How it works on the workshop's tasks.
    #[serde(default)]
    pub workshop: WorkshopConfig,
    /// How it isolates the workshop's jobs.
    #[serde(default)]
    pub isolation: IsolationConfig,
}

/// How a workshop machine works on the workshop's tasks.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkshopConfig {
    /// How long a task may take, all in all; 120 minutes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u64>,
    /// How many times the agent may work on a task: the first, and then
    /// to fix what the check found; 2 if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rounds: Option<u32>,
    /// How many steps (turns) an agent whose runner counts them may take
    /// in a round; 80 if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

/// How a workshop machine builds libraries, with Lean (`elan` or `lake`
/// on its path).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LeanConfig {
    /// How long a build may take; 90 minutes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u64>,
    /// Whether to get Mathlib's cache before building; yes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
}

/// How a workshop machine compiles papers, with LaTeX (`latexmk` on its
/// path, or in the workshop's image).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatexConfig {
    /// How long a build may take; 15 minutes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u64>,
}

fn one() -> u32 {
    1
}

/// How to run the runners a machine has.
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
    /// The runners offered, by name.
    pub fn runner_names(&self) -> Vec<String> {
        runners::ALL
            .iter()
            .filter(|name| self.runners.get(name).is_some())
            .map(|name| (*name).to_owned())
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
            concurrency = 2

            [runners.claude-code]
            [runners.opencode]
            model = "openrouter/qwen/qwen3-coder"
            [workshop]
            rounds = 3
            "#,
        )
        .unwrap();
        assert_eq!(config.runner_names(), ["claude-code", "opencode"]);
        assert_eq!(config.workshop.rounds, Some(3));
        // There is no `api` runner.
        assert!(toml::from_str::<WorkerConfig>("hub = \"x\"\n[runners.api]").is_err());
    }
}
