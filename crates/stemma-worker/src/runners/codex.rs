//! Codex, headless (`codex exec`), with the person's own sign-in.
//!
//! Launched as [`crate::launch`] says, with the request on standard input.
//! To answer, it runs in a read-only sandbox in the job's folder; to work
//! on a task, in a sandbox where it writes in the task's worktree. Its
//! last message is the answer.

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
    let answer_file = prepared
        .answer
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Codex was launched without an answer's file"))?;
    let mut command = prepared.command();
    command.current_dir(work.dir);

    let ran = run_command(command, &prompt.user, |line| {
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
        if event["type"] == "item.started" && item["type"] == "command_execution" {
            progress.tool("shell", &serde_json::json!({"command": item["command"]}));
        }
    })
    .await?;
    if !ran.status.success() {
        return Err(failed(&prepared.program, &ran));
    }
    std::fs::read_to_string(&answer_file).map_err(|_| anyhow::anyhow!("Codex gave no answer"))
}
