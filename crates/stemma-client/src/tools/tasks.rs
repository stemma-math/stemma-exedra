//! The workshop's tools: hand a task over, see how one is, and list them.

use serde::Deserialize;
use serde_json::{Value, json};
use stemma_core::Address;
use stemma_core::api::{CreateTask, TaskState};

use super::{GROUP, ToolDef, ToolError, ToolResult, Toolbox, address_in, parse};
use crate::format;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "create_task",
            description: "Hand a task to the group's workshop, which works on the group's Lean \
                          library in the background: formalize a statement or a proof, integrate \
                          a lemma, clean up a module. Use it only when someone asks for it, as in \
                          \"formalize this\". Give a short title, and instructions that say \
                          exactly what to do, citing by address what the thread decided \
                          ([[t12#45-48]], [[lean:Foo.bar]]). The workshop \
                          proposes its changes, which the person who asked approves, and \
                          reports in the thread of the message that asked. Returns the task's \
                          address, w7, to cite as [[w7]].",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "What to do, in a line."},
                    "instructions": {"type": "string",
                                     "description": "What to do, exactly, in Markdown."},
                    "origin": {"type": "string",
                               "description": "The message that asks for it, as t12#48; when \
                                               you answer in a thread, the message you answer \
                                               if absent."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["title", "instructions"]
            }),
            writes: true,
        },
        ToolDef {
            name: "task_status",
            description: "How a task of the workshop is, by address (w7): its state (open, \
                          running, review, done, failed, rejected, cancelled), what it is doing \
                          or why it waits, the changes it proposes and who approved them, and \
                          its report.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": {"type": "string", "description": "The task: w7."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["address"]
            }),
            writes: false,
        },
        ToolDef {
            name: "list_tasks",
            description: "List the workshop's tasks, newest first, with their addresses (w7), \
                          titles, states and where they were asked for.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "state": {"type": "string",
                              "enum": ["open", "running", "review", "done", "failed", "rejected",
                                       "cancelled"],
                              "description": "Only the tasks in this state."},
                    "group": {"type": "string", "description": GROUP},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200,
                              "description": "How many; 30 if absent."}
                }
            }),
            writes: false,
        },
    ]
}

/// Runs a workshop's tool, if `name` is one.
pub async fn call(
    toolbox: &Toolbox,
    name: &str,
    args: Value,
    agent: Option<&str>,
) -> Option<ToolResult> {
    Some(match name {
        "create_task" => create(toolbox, args, agent).await,
        "task_status" => status(toolbox, args).await,
        "list_tasks" => list(toolbox, args).await,
        _ => return None,
    })
}

#[derive(Deserialize)]
struct Create {
    title: String,
    instructions: String,
    origin: Option<String>,
    group: Option<String>,
}

async fn create(toolbox: &Toolbox, args: Value, agent: Option<&str>) -> ToolResult {
    let args: Create = parse(args)?;
    let group = match &args.origin {
        Some(origin) => address_in(toolbox, origin, args.group.as_deref()).await?.0,
        None => toolbox.group(args.group.as_deref()).await?,
    };
    let origin = match &args.origin {
        Some(origin) => Some(
            stemma_core::locate(origin)
                .map_err(|e| ToolError::invalid(format!("{origin:?} is not an address: {e}")))?
                .address
                .to_string(),
        ),
        None => None,
    };
    let task = toolbox
        .hub()
        .create_task(
            &group,
            &CreateTask {
                title: args.title,
                instructions: args.instructions,
                origin,
                agent: agent.map(str::to_owned),
                architecture: None,
            },
        )
        .await?;
    Ok(format!(
        "Handed w{} to the workshop: “{}”. It runs as soon as the workshop is free, and it \
         reports in {} when it is done. Cite it as [[w{}]]: {}{}\n",
        task.number,
        task.title,
        task.origin,
        task.number,
        toolbox.hub().base(),
        Address::Task(task.number).web_path(&group)
    ))
}

#[derive(Deserialize)]
struct Status {
    address: String,
    group: Option<String>,
}

pub async fn status(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: Status = parse(args)?;
    let written = args.address.trim();
    let written = if written.chars().all(|c| c.is_ascii_digit()) {
        format!("w{written}")
    } else {
        written.to_owned()
    };
    let (group, number) = match address_in(toolbox, &written, args.group.as_deref()).await? {
        (group, Address::Task(number)) => (group, number),
        (_, other) => {
            return Err(ToolError::invalid(format!(
                "{other} is not a task: a task's address is w7"
            )));
        }
    };
    let page = toolbox.hub().task(&group, number).await?;
    Ok(format::task(toolbox.hub().base(), &group, &page))
}

#[derive(Deserialize)]
struct List {
    state: Option<TaskState>,
    group: Option<String>,
    limit: Option<u32>,
}

async fn list(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: List = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let tasks = toolbox
        .hub()
        .tasks(&group, args.state, args.limit.unwrap_or(30).clamp(1, 200))
        .await?;
    Ok(format::task_list(&tasks))
}
