//! Claude Code, headless (`claude -p`), with the member's own login.
//!
//! It gets Stemma's instructions appended to its system prompt, the thread
//! and the request on standard input, and Stemma's tools through an MCP
//! configuration of its own: only `stemma mcp`, read-only, with the job's
//! token. Claude Code's built-in tools are off, and only Stemma's tools are
//! allowed. Its output is a stream of JSON events: tool calls become
//! progress, and the final result is the answer.

use serde_json::{Value, json};
use stemma_core::api::AgentProfile;
use stemma_core::protocol::JobSpec;
use tokio::process::Command;

use super::{allowed_tools, failed, mcp_args, run_command};
use crate::config::CliRunner;
use crate::job::{Context, JobDir, Progress, Prompt};

/// The Stemma tools an agent may use, as Claude Code names them.
fn allowed() -> String {
    allowed_tools()
        .iter()
        .map(|name| format!("mcp__stemma__{name}"))
        .collect::<Vec<_>>()
        .join(",")
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
    let mcp = json!({
        "mcpServers": {
            "stemma": {
                "type": "stdio",
                "command": context.stemma,
                "args": mcp_args(context, job, agent, dir),
            }
        }
    });
    let mcp_file = dir.path.join("mcp.json");
    std::fs::write(&mcp_file, serde_json::to_vec_pretty(&mcp)?)?;

    let program = config.command.as_deref().unwrap_or("claude");
    let mut command = Command::new(program);
    command
        .current_dir(&dir.path)
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .args(["--no-session-persistence", "--strict-mcp-config"])
        .arg("--mcp-config")
        .arg(&mcp_file)
        .args(["--tools", ""])
        .args(["--allowedTools", &allowed()])
        .args(["--permission-mode", "dontAsk"])
        .arg("--append-system-prompt")
        .arg(&prompt.system);
    if let Some(model) = agent.model.as_ref().or(config.model.as_ref()) {
        command.args(["--model", model]);
    }
    command.args(&config.args);

    let mut result: Option<(String, bool)> = None;
    let ran = run_command(command, &prompt.user, |line| {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match event["type"].as_str() {
            Some("assistant") => {
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "tool_use" {
                        progress.tool(block["name"].as_str().unwrap_or(""), &block["input"]);
                    }
                }
            }
            Some("result") => {
                let text = event["result"].as_str().unwrap_or_default().to_owned();
                let is_error = event["is_error"].as_bool().unwrap_or(false)
                    || event["subtype"].as_str().is_some_and(|s| s != "success");
                result = Some((text, is_error));
            }
            _ => {}
        }
    })
    .await?;

    match result {
        Some((text, false)) => Ok(text),
        Some((text, true)) if !text.trim().is_empty() => {
            anyhow::bail!("Claude Code stopped: {}", text.trim())
        }
        _ if !ran.status.success() => Err(failed(program, &ran)),
        _ => anyhow::bail!("Claude Code gave no result"),
    }
}
