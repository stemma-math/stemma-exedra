//! Codex, headless (`codex exec`), with the person's own sign-in.
//!
//! Launched as [`crate::launch`] says, with the request on standard input.
//! To answer, it runs in a read-only sandbox in the job's folder; to work
//! on a task, in a sandbox where it writes in the task's worktree. Its
//! last message is the answer. Its `--json` events say, at the end of each
//! turn (`turn.completed`), the tokens it took, but not their cost: its
//! `input_tokens` include the cached ones, which are counted apart here.

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
    let mut usage = Usage::default();
    let answer_file = prepared
        .answer
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Codex was launched without an answer's file"))?;
    let command = super::command(&prepared, work);

    let ran = run_command(command, &prompt.user, |line| {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        add_turn(&event, &mut usage);
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
    let text = std::fs::read_to_string(&answer_file)
        .map_err(|_| anyhow::anyhow!("Codex gave no answer"))?;
    Ok(Answer { text, usage })
}

/// What a run consumed, from its whole output: all its turns.
pub fn usage_of_output(output: &str) -> Usage {
    let mut usage = Usage::default();
    for event in output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        add_turn(&event, &mut usage);
    }
    usage
}

/// Adds what a turn took, if `event` ends one.
pub fn add_turn(event: &Value, usage: &mut Usage) {
    if event["type"] != "turn.completed" {
        return;
    }
    let turn = &event["usage"];
    let cached = tokens(&turn["cached_input_tokens"]);
    let input = tokens(&turn["input_tokens"]).map(|i| i.saturating_sub(cached.unwrap_or(0)));
    usage.add(&Usage {
        input_tokens: input,
        output_tokens: tokens(&turn["output_tokens"]),
        cache_read_tokens: cached,
        cache_write_tokens: None,
        cost_usd: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_a_run_consumed() {
        let mut usage = Usage::default();
        for line in include_str!("../../tests/fixtures/codex-events.jsonl").lines() {
            add_turn(&serde_json::from_str(line).unwrap(), &mut usage);
        }
        assert_eq!(
            usage,
            Usage {
                input_tokens: Some(4_100),
                output_tokens: Some(1_830),
                cache_read_tokens: Some(20_480),
                cache_write_tokens: None,
                cost_usd: None,
            }
        );
    }
}
