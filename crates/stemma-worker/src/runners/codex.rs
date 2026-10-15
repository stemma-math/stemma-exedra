//! Codex, headless (`codex exec`), with the member's own login.
//!
//! It runs in a read-only sandbox in the job's folder, with Stemma's tools
//! through `stemma mcp`, read-only, with the job's token, configured on the
//! command line so that the member's own configuration stays as it is.
//! Codex has no separate system prompt here: Stemma's instructions come
//! first in its input. Its last message is the answer.

use serde_json::Value;
use stemma_core::api::AgentProfile;
use stemma_core::protocol::JobSpec;
use tokio::process::Command;

use super::{failed, mcp_args, run_command};
use crate::config::CliRunner;
use crate::job::{Context, JobDir, Progress, Prompt};

/// A TOML value, as `codex -c key=value` reads it.
fn toml_value(value: toml::Value) -> String {
    value.to_string()
}

pub async fn run(
    context: &Context,
    config: &CliRunner,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
    prompt: &Prompt,
    progress: &Progress,
) -> anyhow::Result<String> {
    let answer_file = dir.path.join("answer.md");
    let command_value = toml_value(toml::Value::String(context.stemma.display().to_string()));
    let args_value = toml_value(toml::Value::Array(
        mcp_args(context, job, agent, dir)
            .into_iter()
            .map(toml::Value::String)
            .collect(),
    ));

    let program = config.command.as_deref().unwrap_or("codex");
    let mut command = Command::new(program);
    command
        .current_dir(&dir.path)
        .arg("exec")
        .args(["--skip-git-repo-check", "--ephemeral", "--json"])
        .args(["--sandbox", "read-only", "--color", "never"])
        .arg("--cd")
        .arg(&dir.path)
        .arg("--output-last-message")
        .arg(&answer_file)
        .args(["-c", "approval_policy=\"never\""])
        .arg("-c")
        .arg(format!("mcp_servers.stemma.command={command_value}"))
        .arg("-c")
        .arg(format!("mcp_servers.stemma.args={args_value}"));
    if let Some(model) = agent.model.as_ref().or(config.model.as_ref()) {
        command.args(["--model", model]);
    }
    command.args(&config.args).arg("-");

    let input = format!("{}\n---\n\n{}", prompt.system, prompt.user);
    let ran = run_command(command, &input, |line| {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let item = &event["item"];
        if event["type"] == "item.started" && item["type"] == "mcp_tool_call" {
            let args = match &item["arguments"] {
                Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                other => other.clone(),
            };
            progress.tool(item["tool"].as_str().unwrap_or(""), &args);
        }
    })
    .await?;
    if !ran.status.success() {
        return Err(failed(program, &ran));
    }
    std::fs::read_to_string(&answer_file).map_err(|_| anyhow::anyhow!("Codex gave no answer"))
}
