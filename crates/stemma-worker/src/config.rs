//! Resolved runtime settings, derived from the shared configuration module.
use crate::isolation::{IsolationConfig, Runtime};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use stemma_client::config::{Credentials, Paths, Settings};
use stemma_core::api::runners;
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    pub hub: String,
    pub settings: Settings,
    pub paths: Paths,
    pub concurrency: u32,
    pub runners: Runners,
    pub isolation: IsolationConfig,
    pub work_dir: PathBuf,
}
impl WorkerConfig {
    pub fn load(group: Option<&str>) -> anyhow::Result<Self> {
        Self::resolve(
            &Settings::load()?,
            &Paths::discover()?,
            &Credentials::load()?.with_env(),
            group,
        )
    }
    pub fn resolve(
        settings: &Settings,
        paths: &Paths,
        credentials: &Credentials,
        group: Option<&str>,
    ) -> anyhow::Result<Self> {
        let get = |key: &str| settings.get(key, group).map(|v| v.value);
        let mut runners = Runners::default();
        let mut isolation = IsolationConfig {
            runtime: Some(match get("agents.isolation")?.as_str().unwrap() {
                "docker" => Runtime::Docker,
                "podman" => Runtime::Podman,
                _ => Runtime::Host,
            }),
            image: Some(get("agents.image")?.as_str().unwrap().into()),
            ..Default::default()
        };
        for (handle, runner) in [
            ("claude", runners::CLAUDE_CODE),
            ("codex", runners::CODEX),
            ("opencode", runners::OPENCODE),
        ] {
            let value = |key: &str| get(&format!("agents.{handle}.{key}"));
            let model = value("model")?.as_str().unwrap().to_owned();
            runners.set(
                runner,
                CliRunner {
                    command: Some(value("command")?.as_str().unwrap().into()),
                    args: value("args")?.try_into()?,
                    model: (!model.is_empty()).then_some(model),
                    timeout_minutes: Some(value("timeout_minutes")?.as_integer().unwrap() as u64),
                },
            );
            isolation.agents.insert(
                runner.into(),
                crate::isolation::AgentSignIn {
                    env: value("container.env")?.try_into()?,
                    files: value("container.files")?.try_into()?,
                    command: Some(value("container.command")?.as_str().unwrap().into()),
                },
            );
        }
        Ok(Self {
            hub: credentials.hub.clone().unwrap_or_default(),
            settings: settings.clone(),
            paths: paths.clone(),
            concurrency: get("agents.concurrency")?.as_integer().unwrap() as u32,
            runners,
            isolation,
            work_dir: paths.state.clone(),
        })
    }
    pub fn runner_names(&self) -> Vec<String> {
        runners::ALL
            .iter()
            .filter(|r| self.runners.get(r).is_some())
            .map(|r| (*r).into())
            .collect()
    }
    pub fn work_dir(&self) -> PathBuf {
        self.work_dir.clone()
    }
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
