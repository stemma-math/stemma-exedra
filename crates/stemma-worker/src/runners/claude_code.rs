//! Claude Code, headless (`claude -p`), with the person's own sign-in.
//!
//! Launched as [`crate::launch`] says, it gets the thread and the request
//! on standard input. To answer, its built-in tools are off, and only
//! Stemma's tools are allowed; to work on a task, it may also read, edit
//! and write files and run commands, in the task's worktree, for a limited
//! number of turns. Its output is a stream of JSON events: tool calls
//! become progress, and the final result is the answer.

use serde_json::Value;
use stemma_core::api::AgentProfile;
use stemma_core::protocol::JobSpec;

use super::{Work, failed, prepared, run_command};
use crate::config::CliRunner;
use crate::job::{Context, JobDir, Progress, Prompt};

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
    let prepared = prepared(context, config, job, agent, dir, work, prompt)?;
    let mut command = prepared.command();
    command.current_dir(work.dir);

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
        _ if !ran.status.success() => Err(failed(&prepared.program, &ran)),
        _ => anyhow::bail!("Claude Code gave no result"),
    }
}
