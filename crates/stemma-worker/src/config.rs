//! A worker's configuration: a TOML file.
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
//! [runners.api]
//! endpoint = "openrouter"
//!
//! [endpoints.openrouter]              # for the `api` runner
//! url = "https://openrouter.ai/api/v1"
//! key_env = "OPENROUTER_API_KEY"
//! model = "qwen/qwen3-235b-a22b"
//!
//! [lean]                              # library builds, with Lean installed
//! timeout_minutes = 90
//! ```
//!
//! Keys are never written in the file: an endpoint names the environment
//! variable that holds its key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;
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
    /// OpenAI-compatible endpoints, by name.
    #[serde(default)]
    pub endpoints: BTreeMap<String, Endpoint>,
    /// How a group's worker with Lean builds its library.
    pub lean: Option<LeanConfig>,
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

/// The runners this worker offers: those listed.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runners {
    #[serde(rename = "claude-code")]
    pub claude_code: Option<CliRunner>,
    pub codex: Option<CliRunner>,
    pub api: Option<ApiRunner>,
}

/// A runner that is a command-line harness.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliRunner {
    /// The command; the harness's usual name if absent.
    pub command: Option<String>,
    /// More arguments to give it.
    #[serde(default)]
    pub args: Vec<String>,
    /// The model, unless an agent's profile names one.
    pub model: Option<String>,
    /// How long a job may take; 15 minutes if absent.
    pub timeout_minutes: Option<u64>,
}

/// Stemma's own harness, over an OpenAI-compatible chat API.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRunner {
    /// The endpoint, unless an agent's profile names one; the only one
    /// configured if absent.
    pub endpoint: Option<String>,
    /// How many requests to the model a job may make; 8 if absent.
    pub max_steps: Option<u32>,
    /// How many tokens a job may spend, in all; 200 000 if absent.
    pub max_tokens: Option<u64>,
    /// How long a job may take; 10 minutes if absent.
    pub timeout_minutes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    /// The API's base, such as `https://openrouter.ai/api/v1`; requests go
    /// to its `/chat/completions`.
    pub url: String,
    /// The environment variable that holds the key, if it needs one.
    pub key_env: Option<String>,
    /// The model, unless an agent's profile names one.
    pub model: Option<String>,
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
        if let Some(api) = &self.runners.api {
            anyhow::ensure!(
                !self.endpoints.is_empty(),
                "the api runner needs an endpoint: add one under [endpoints.<name>]"
            );
            if let Some(name) = &api.endpoint {
                anyhow::ensure!(
                    self.endpoints.contains_key(name),
                    "there is no endpoint {name}"
                );
            }
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
        let r = &self.runners;
        [
            (r.claude_code.is_some(), runners::CLAUDE_CODE),
            (r.codex.is_some(), runners::CODEX),
            (r.api.is_some(), runners::API),
        ]
        .into_iter()
        .filter(|(offered, _)| *offered)
        .map(|(_, name)| name.to_owned())
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
            [runners.api]
            endpoint = "local"

            [endpoints.local]
            url = "http://127.0.0.1:11434/v1"
            model = "qwen3"
            "#,
        )
        .unwrap();
        config.check().unwrap();
        assert_eq!(config.runner_names(), ["claude-code", "api"]);
        assert_eq!(config.token().unwrap(), "stw_x");
    }

    #[test]
    fn rejects_what_cannot_work() {
        let none: WorkerConfig = toml::from_str("hub = \"x\"\ntoken = \"t\"").unwrap();
        assert!(none.check().is_err());
        let no_endpoint: WorkerConfig =
            toml::from_str("hub = \"x\"\ntoken = \"t\"\n[runners.api]").unwrap();
        assert!(no_endpoint.check().is_err());
        assert!(toml::from_str::<WorkerConfig>("hub = \"x\"\ntokn = \"t\"").is_err());
    }
}
