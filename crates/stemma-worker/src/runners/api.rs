//! Stemma's own harness: a loop of tool calls over an OpenAI-compatible
//! Chat Completions API (OpenRouter, OpenAI, a local server…), with
//! Stemma's tools, read-only, as the job's requester.
//!
//! Each step asks the model; if it calls tools, they run and the loop goes
//! on, and if it answers, that is the answer. The last step, or the first
//! past the job's budget of tokens, asks for an answer without tools.

use std::time::Duration;

use anyhow::Context as _;
use serde_json::{Value, json};
use stemma_client::{Hub, Toolbox};
use stemma_core::protocol::JobSpec;

use crate::config::{ApiRunner, Endpoint};
use crate::job::{Context, Progress, Prompt};

const DEFAULT_STEPS: u32 = 8;
const DEFAULT_TOKENS: u64 = 200_000;

/// The endpoint, model and key a job uses.
struct Target<'a> {
    endpoint: &'a Endpoint,
    model: String,
    key: Option<String>,
}

fn target<'a>(
    context: &'a Context,
    config: &ApiRunner,
    job: &JobSpec,
) -> anyhow::Result<Target<'a>> {
    let endpoints = &context.config.endpoints;
    let (name, endpoint) = match job.agent.endpoint.as_ref().or(config.endpoint.as_ref()) {
        Some(name) => (
            name.as_str(),
            endpoints
                .get(name)
                .with_context(|| format!("this worker has no endpoint {name}"))?,
        ),
        None if endpoints.len() == 1 => {
            let (name, endpoint) = endpoints.iter().next().expect("one");
            (name.as_str(), endpoint)
        }
        None => anyhow::bail!("say which endpoint {} uses", job.agent.display_name),
    };
    let model = job
        .agent
        .model
        .clone()
        .or_else(|| endpoint.model.clone())
        .with_context(|| {
            format!(
                "no model for {} on the endpoint {name}",
                job.agent.display_name
            )
        })?;
    let key = match &endpoint.key_env {
        Some(var) => Some(
            std::env::var(var)
                .ok()
                .filter(|k| !k.trim().is_empty())
                .with_context(|| format!("the environment variable {var} holds no key"))?,
        ),
        None => None,
    };
    Ok(Target {
        endpoint,
        model,
        key,
    })
}

pub async fn run(
    context: &Context,
    config: &ApiRunner,
    job: &JobSpec,
    prompt: &Prompt,
    progress: &Progress,
) -> anyhow::Result<String> {
    let target = target(context, config, job)?;
    let toolbox = Toolbox::new(
        Hub::new(&context.hub, Some(job.token.clone()))?,
        Some(job.group.slug.clone()),
    )
    .read_only(true);
    let tools: Vec<Value> = toolbox
        .definitions()
        .into_iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                }
            })
        })
        .collect();
    let http = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(300))
        .build()?;
    let url = format!(
        "{}/chat/completions",
        target.endpoint.url.trim_end_matches('/')
    );

    let max_steps = config.max_steps.unwrap_or(DEFAULT_STEPS).max(1);
    let max_tokens = config.max_tokens.unwrap_or(DEFAULT_TOKENS);
    let mut spent: u64 = 0;
    let mut messages = vec![
        json!({"role": "system", "content": prompt.system}),
        json!({"role": "user", "content": prompt.user}),
    ];
    for step in 1..=max_steps {
        let last = step == max_steps || spent >= max_tokens;
        let mut body = json!({"model": target.model, "messages": messages});
        if last {
            if step > 1 {
                messages.push(json!({
                    "role": "user",
                    "content": "Answer now, with what you have found.",
                }));
                body["messages"] = json!(messages);
            }
        } else {
            body["tools"] = json!(tools);
        }

        let mut request = http.post(&url).json(&body);
        if let Some(key) = &target.key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.context("reaching the model's API")?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            let excerpt: String = text.chars().take(500).collect();
            anyhow::bail!("the model's API answered {status}: {excerpt}");
        }
        let reply: Value = serde_json::from_str(&text).context("reading the model's answer")?;
        spent += reply["usage"]["total_tokens"].as_u64().unwrap_or(0);
        let message = reply["choices"][0]["message"].clone();
        let calls = message["tool_calls"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if calls.is_empty() || last {
            let answer = message["content"].as_str().unwrap_or_default();
            return Ok(without_thinking(answer));
        }
        messages.push(json!({
            "role": "assistant",
            "content": message["content"].clone(),
            "tool_calls": calls,
        }));
        for call in &calls {
            let name = call["function"]["name"].as_str().unwrap_or_default();
            let args: Value = match &call["function"]["arguments"] {
                Value::String(text) if text.trim().is_empty() => json!({}),
                Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
                other => other.clone(),
            };
            progress.tool(name, &args);
            let content = match toolbox.call(name, args, None).await {
                Ok(text) => text,
                Err(error) => format!("Error: {error}"),
            };
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call["id"].clone(),
                "content": content,
            }));
        }
        progress.say("Thinking");
    }
    anyhow::bail!("no answer after {max_steps} steps")
}

/// An answer without the reasoning some models write in `<think>` tags.
fn without_thinking(answer: &str) -> String {
    let mut text = answer.to_owned();
    while let (Some(start), Some(end)) = (text.find("<think>"), text.find("</think>")) {
        if end < start {
            break;
        }
        text.replace_range(start..end + "</think>".len(), "");
    }
    text.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::without_thinking;

    #[test]
    fn leaves_out_thinking() {
        assert_eq!(
            without_thinking("<think>hmm</think>\nThe map is proper."),
            "The map is proper."
        );
        assert_eq!(without_thinking("Plain."), "Plain.");
    }
}
