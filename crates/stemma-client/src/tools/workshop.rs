//! The board's tools, also used by people's sessions.
use super::{GROUP, ToolDef, ToolError, ToolResult, Toolbox};
use serde_json::{Value, json};

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_recipes",
            terminal: &["workshop", "recipes"],
            positional: &[],
            description: "List the group's recipes from main, with their blob versions and validation errors.",
            writes: false,
            input_schema: json!({"type":"object","properties":{"group":{"type":"string","description":GROUP}}}),
            handler: |t, a, _| Box::pin(read(t, a, "recipes")),
        },
        ToolDef {
            name: "read_recipe",
            terminal: &["workshop", "recipe"],
            positional: &["name"],
            description: "Read a recipe's metadata, inputs and prompt, with its forge link and version.",
            writes: false,
            input_schema: json!({"type":"object","properties":{"group":{"type":"string","description":GROUP},"name":{"type":"string"}},"required":["name"]}),
            handler: |t, a, _| Box::pin(read_one(t, a, true)),
        },
        ToolDef {
            name: "list_requests",
            terminal: &["workshop", "requests"],
            positional: &[],
            description: "List the group's workshop requests, newest first. Filter open, scheduled, recurring or finished.",
            writes: false,
            input_schema: json!({"type":"object","properties":{"group":{"type":"string","description":GROUP},"state":{"type":"string","enum":["open","scheduled","recurring","finished"]}}}),
            handler: |t, a, _| Box::pin(read(t, a, "requests")),
        },
        ToolDef {
            name: "read_request",
            terminal: &["workshop", "request"],
            positional: &["number"],
            description: "Read a request's recipe, inputs, assignment, schedule, state, processes, branches and usage. Read its thread with read_thread.",
            writes: false,
            input_schema: json!({"type":"object","properties":{"group":{"type":"string","description":GROUP},"number":{"type":"integer","minimum":1}},"required":["number"]}),
            handler: |t, a, _| Box::pin(read_one(t, a, false)),
        },
        ToolDef {
            name: "ask_workshop",
            terminal: &["workshop", "ask"],
            positional: &["recipe", "title"],
            description: "Ask for a recipe, or use an empty recipe name to ask in plain words. Inputs are a JSON object. Assignment is mine by default, member with a numeric user, or anyone. Schedule uses at/until Unix milliseconds, an IANA timezone, and optional every: {kind:day}, {kind:week}, or {kind:hours,hours:N}. Approvals remain on the machine.",
            writes: true,
            input_schema: json!({"type":"object","properties":{"group":{"type":"string","description":GROUP},"recipe":{"type":"string"},"title":{"type":"string"},"inputs":{"type":"object","additionalProperties":{"type":"string"}},"assignment":{"type":"object","properties":{"kind":{"type":"string","enum":["mine","member","anyone"]},"user":{"type":"integer"}},"required":["kind"]},"runner":{"type":"string","enum":["claude","codex","opencode"]},"model":{"type":"string"},"schedule":{"type":"object","properties":{"at":{"type":"integer"},"timezone":{"type":"string"},"every":{"type":"object"},"until":{"type":"integer"}},"required":["timezone"]},"wait_hours":{"type":"integer","minimum":1,"maximum":168}},"required":["recipe","title","runner"]}),
            handler: |t, a, _| Box::pin(ask(t, a)),
        },
        ToolDef {
            name: "cancel_request",
            terminal: &["workshop", "cancel"],
            positional: &["number"],
            description: "Cancel your workshop request and its processes. A recurring request stops permanently.",
            writes: true,
            input_schema: json!({"type":"object","properties":{"group":{"type":"string","description":GROUP},"number":{"type":"integer","minimum":1}},"required":["number"]}),
            handler: |t, a, _| Box::pin(cancel(t, a)),
        },
    ]
}
fn pretty(value: impl serde::Serialize) -> ToolResult {
    serde_json::to_string_pretty(&value).map_err(|e| ToolError::invalid(e.to_string()))
}
async fn read(t: &Toolbox, a: Value, resource: &str) -> ToolResult {
    let group = t.group(a["group"].as_str()).await?;
    let query = a["state"]
        .as_str()
        .map(|v| vec![("state", v.into())])
        .unwrap_or_default();
    pretty(t.hub().workshop_read(&group, resource, &query).await?)
}
async fn read_one(t: &Toolbox, a: Value, recipe: bool) -> ToolResult {
    let group = t.group(a["group"].as_str()).await?;
    let resource = if recipe {
        let name = a["name"]
            .as_str()
            .ok_or_else(|| ToolError::invalid("Expected recipe name"))?;
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(ToolError::invalid("Invalid recipe name"));
        }
        format!("recipes/{name}")
    } else {
        format!(
            "requests/{}",
            a["number"]
                .as_u64()
                .ok_or_else(|| ToolError::invalid("Expected request number"))?
        )
    };
    pretty(t.hub().workshop_read(&group, &resource, &[]).await?)
}
async fn ask(t: &Toolbox, mut a: Value) -> ToolResult {
    let group = t.group(a["group"].as_str()).await?;
    let obj = a
        .as_object_mut()
        .ok_or_else(|| ToolError::invalid("Expected ask arguments"))?;
    obj.remove("group");
    obj.entry("inputs").or_insert(json!({}));
    obj.entry("assignment").or_insert(json!({"kind":"mine"}));
    obj.entry("schedule").or_insert(json!({"timezone":"UTC"}));
    obj.entry("wait_hours").or_insert(json!(24));
    let ask = super::parse(a)?;
    pretty(t.hub().workshop_ask(&group, &ask).await?)
}
async fn cancel(t: &Toolbox, a: Value) -> ToolResult {
    let group = t.group(a["group"].as_str()).await?;
    let number = a["number"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| ToolError::invalid("Expected request number"))?;
    pretty(t.hub().workshop_cancel(&group, number).await?)
}
