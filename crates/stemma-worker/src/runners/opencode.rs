//! OpenCode, headless (`opencode run`), with a model of any provider it
//! supports: OpenRouter's open models among them, with the keys of its own
//! configuration.
//!
//! Launched as [`crate::launch`] says, with the request as its message: to
//! answer, with none of its own tools that edit or run commands; to work
//! on a task, with its tools that edit files and run commands, allowed
//! without asking, in the task's worktree. Its output, with
//! `--format json`, is a stream of JSON events: tool calls become
//! progress, and the text after the last of them is the answer. Each step's
//! end (`step_finish`) says its cost, in dollars, and its tokens: `input`,
//! `output`, `reasoning` (counted here as output) and `cache` (`read`,
//! `write`).

use serde_json::Value;
use stemma_core::api::AgentProfile;
use stemma_core::protocol::{JobSpec, Usage};

use super::{Answer, Work, failed, prepared, run_command, tokens};
use crate::config::CliRunner;
use crate::job::{Context, JobDir, Progress, Prompt};

/// What OpenCode said, from its events.
#[derive(Default)]
struct Said {
    /// The text since its last tool call.
    answer: String,
    error: Option<String>,
    events: usize,
    usage: Usage,
}

impl Said {
    fn read(&mut self, line: &str, progress: &Progress) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        self.events += 1;
        let part = &event["part"];
        match event["type"].as_str() {
            Some("tool_use") => {
                let tool = part["tool"].as_str().unwrap_or_default();
                let tool = tool.strip_prefix("stemma_").unwrap_or(tool);
                progress.tool(tool, &part["state"]["input"]);
                self.answer.clear();
            }
            Some("text") => {
                if let Some(text) = part["text"].as_str() {
                    if !self.answer.is_empty() {
                        self.answer.push('\n');
                    }
                    self.answer.push_str(text);
                }
            }
            Some("step_finish") => {
                let tokens_of = &part["tokens"];
                let output = match (
                    tokens(&tokens_of["output"]),
                    tokens(&tokens_of["reasoning"]),
                ) {
                    (Some(o), Some(r)) => Some(o + r),
                    (o, r) => o.or(r),
                };
                self.usage.add(&Usage {
                    input_tokens: tokens(&tokens_of["input"]),
                    output_tokens: output,
                    cache_read_tokens: tokens(&tokens_of["cache"]["read"]),
                    cache_write_tokens: tokens(&tokens_of["cache"]["write"]),
                    cost_usd: part["cost"].as_f64(),
                });
            }
            Some("error") => {
                let error = &event["error"];
                let message = error["data"]["message"]
                    .as_str()
                    .or_else(|| error["message"].as_str())
                    .or_else(|| error.as_str())
                    .unwrap_or("an error");
                self.error = Some(message.to_owned());
            }
            _ => {}
        }
    }
}

/// What a run consumed, from its whole output: all its steps.
pub fn usage_of_output(output: &str) -> Usage {
    let (sender, _) = tokio::sync::mpsc::unbounded_channel();
    let progress = Progress::new(0, sender);
    let mut said = Said::default();
    for line in output.lines() {
        said.read(line, &progress);
    }
    said.usage
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
) -> anyhow::Result<Answer> {
    let mut prepared = prepared(context, config, job, agent, dir, work, prompt)?;
    // The request is its message, last.
    prepared.args.push(prompt.user.clone());
    let command = super::command(&prepared, work);

    let mut said = Said::default();
    let mut plain = String::new();
    let ran = run_command(command, "", |line| {
        said.read(line, progress);
        if said.events == 0 {
            plain.push_str(line);
            plain.push('\n');
        }
    })
    .await?;
    if let Some(error) = said.error {
        anyhow::bail!("OpenCode stopped: {error}");
    }
    if !ran.status.success() {
        return Err(failed(&prepared.program, &ran));
    }
    // Without events, its output is the answer.
    let answer = if said.events == 0 { plain } else { said.answer };
    anyhow::ensure!(!answer.trim().is_empty(), "OpenCode gave no answer");
    Ok(Answer {
        text: answer.trim().to_owned(),
        usage: said.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn reads_its_events() {
        let (sender, mut seen) = mpsc::unbounded_channel();
        let progress = Progress::new(1, sender);
        let mut said = Said::default();
        for line in [
            r#"{"type":"step_start","part":{}}"#,
            r#"{"type":"text","part":{"text":"Let me look."}}"#,
            r#"{"type":"tool_use","part":{"tool":"stemma_read_thread","state":{"input":{"address":"t1#2"}}}}"#,
            r#"{"type":"text","part":{"text":"It is proper."}}"#,
            "not json",
        ] {
            said.read(line, &progress);
        }
        assert_eq!(said.answer, "It is proper.");
        assert_eq!(said.events, 4);
        assert_eq!(seen.try_recv().unwrap().1, "Reading t1#2");
    }

    #[test]
    fn reads_what_a_run_consumed() {
        let (sender, _seen) = mpsc::unbounded_channel();
        let progress = Progress::new(1, sender);
        let mut said = Said::default();
        for line in include_str!("../../tests/fixtures/opencode-events.jsonl").lines() {
            said.read(line, &progress);
        }
        assert_eq!(said.answer, "Done.");
        let cost = said.usage.cost_usd.unwrap();
        assert!((cost - 0.0123).abs() < 1e-9, "{cost}");
        assert_eq!(
            Usage {
                cost_usd: None,
                ..said.usage
            },
            Usage {
                input_tokens: Some(3_000),
                output_tokens: Some(700),
                cache_read_tokens: Some(12_000),
                cache_write_tokens: Some(500),
                cost_usd: None,
            }
        );
    }
}
