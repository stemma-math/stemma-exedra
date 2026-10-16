//! Claude Code, headless (`claude -p`), with the member's own login.
//!
//! It gets Stemma's instructions appended to its system prompt, the thread
//! and the request on standard input, and Stemma's tools through an MCP
//! configuration of its own: only `stemma mcp`, read-only, with the job's
//! token. To answer, Claude Code's built-in tools are off, and only
//! Stemma's tools are allowed; to work on a task, it may also read, edit
//! and write files and run commands, in the task's worktree, for a limited
//! number of turns. Its output is a stream of JSON events: tool calls
//! become progress, and the final result is the answer.

use serde_json::{Value, json};
use stemma_core::api::AgentProfile;
use stemma_core::protocol::JobSpec;
use tokio::process::Command;

use super::{Work, allowed_tools, failed, mcp_args, run_command};
use crate::config::CliRunner;
use crate::job::{Context, JobDir, Progress, Prompt};

/// Claude Code's own tools a task's run may use.
const WORK_TOOLS: &[&str] = &["Bash", "Read", "Edit", "MultiEdit", "Write", "Glob", "Grep"];

/// The tools an agent may use, as Claude Code names them: Stemma's, and to
/// work on a task, its own that edit files and run commands.
fn allowed(job: &JobSpec, edits: bool) -> String {
    let mut names: Vec<String> = allowed_tools(job)
        .iter()
        .map(|name| format!("mcp__stemma__{name}"))
        .collect();
    if edits {
        names.extend(WORK_TOOLS.iter().map(|name| (*name).to_owned()));
    }
    names.join(",")
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    context: &Context,
    config: &CliRunner,
    job: &JobSpec,
    agent: &AgentProfile,
    dir: &JobDir,
    work: &Work<'_>,
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
        .current_dir(work.dir)
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .args(["--no-session-persistence", "--strict-mcp-config"])
        .arg("--mcp-config")
        .arg(&mcp_file);
    if !work.edits {
        command.args(["--tools", ""]);
    }
    command
        .args(["--allowedTools", &allowed(job, work.edits)])
        .args(["--permission-mode", "dontAsk"])
        .arg("--append-system-prompt")
        .arg(&prompt.system);
    if let Some(turns) = work.max_turns {
        command.args(["--max-turns", &turns.to_string()]);
    }
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
