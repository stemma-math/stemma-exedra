//! Claude Code, headless (`claude -p`), with the person's own sign-in.
//!
//! Launched as [`crate::launch`] says, it gets the thread and the request
//! on standard input. To answer, its built-in tools are off, and only
//! Stemma's tools are allowed; to work on a task, it may also read, edit
//! and write files and run commands, in the task's worktree, for a limited
//! number of turns. Its output is a stream of JSON events: tool calls
//! become progress, and the final result is the answer, with what the run
//! consumed: its tokens (`usage`) and its cost (`total_cost_usd`).

use serde_json::Value;
use stemma_core::api::AgentProfile;
use stemma_core::protocol::{JobSpec, Usage};

use super::{Answer, Work, failed, prepared, run_command, tokens};
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
) -> anyhow::Result<Answer> {
    let prepared = prepared(context, config, job, agent, dir, work, prompt)?;
    let mut command = prepared.command();
    command.current_dir(work.dir);

    let mut result: Option<(String, bool)> = None;
    let mut usage = Usage::default();
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
                usage = usage_of(&event);
            }
            _ => {}
        }
    })
    .await?;

    match result {
        Some((text, false)) => Ok(Answer { text, usage }),
        Some((text, true)) if !text.trim().is_empty() => {
            anyhow::bail!("Claude Code stopped: {}", text.trim())
        }
        _ if !ran.status.success() => Err(failed(&prepared.program, &ran)),
        _ => anyhow::bail!("Claude Code gave no result"),
    }
}

/// What a run consumed, from its whole output: its result event's.
pub fn usage_of_output(output: &str) -> Usage {
    output
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["type"] == "result")
        .map(|event| usage_of(&event))
        .unwrap_or_default()
}

/// What a run consumed, from its result event.
pub fn usage_of(result: &Value) -> Usage {
    let usage = &result["usage"];
    Usage {
        input_tokens: tokens(&usage["input_tokens"]),
        output_tokens: tokens(&usage["output_tokens"]),
        cache_read_tokens: tokens(&usage["cache_read_input_tokens"]),
        cache_write_tokens: tokens(&usage["cache_creation_input_tokens"]),
        cost_usd: result["total_cost_usd"].as_f64(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_run_consumed() {
        let result: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/claude-code-result.json"))
                .unwrap();
        assert_eq!(
            usage_of(&result),
            Usage {
                input_tokens: Some(1_204),
                output_tokens: Some(3_512),
                cache_read_tokens: Some(48_210),
                cache_write_tokens: Some(9_870),
                cost_usd: Some(0.274_381),
            }
        );
        assert_eq!(usage_of(&serde_json::json!({})), Usage::default());
    }
}
