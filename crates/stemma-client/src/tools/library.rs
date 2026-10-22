//! The library's tools: how it stands, a declaration, and a search.

use serde::Deserialize;
use serde_json::{Value, json};
use stemma_core::Address;

use super::{GROUP, ToolDef, ToolError, ToolResult, Toolbox, address_in, parse};
use crate::format;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "library_state",
            terminal: &["library"],
            positional: &[],
            description: "How the group's Lean library stands: its last build, how many \
                          declarations it has, which depend on sorry, and its modules.",
            input_schema: json!({
                "type": "object",
                "properties": {"group": {"type": "string", "description": GROUP}}
            }),
            writes: false,
        },
        ToolDef {
            name: "library_lookup",
            terminal: &["library", "lookup"],
            positional: &["name"],
            description: "A declaration of the group's Lean library, by name (Foo.bar, or \
                          lean:Foo.bar): its kind, type, docstring, module and place, whether \
                          it is proved or depends on sorry, its axioms, the declarations it uses \
                          and those that use it, and the messages and threads that reference it.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The declaration's full name."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["name"]
            }),
            writes: false,
        },
        ToolDef {
            name: "library_search",
            terminal: &["library", "search"],
            positional: &["query"],
            description: "Search the group's Lean library for declarations whose name, type or \
                          docstring contain every word of the query, regardless of case.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "The words to find."},
                    "group": {"type": "string", "description": GROUP},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 100,
                              "description": "How many results; 20 if absent."}
                },
                "required": ["query"]
            }),
            writes: false,
        },
    ]
}

/// Runs a library's tool, if `name` is one.
pub async fn call(toolbox: &Toolbox, name: &str, args: Value) -> Option<ToolResult> {
    Some(match name {
        "library_state" => state(toolbox, args).await,
        "library_lookup" => lookup(toolbox, args).await,
        "library_search" => search(toolbox, args).await,
        _ => return None,
    })
}

#[derive(Deserialize)]
struct GroupOnly {
    group: Option<String>,
}

async fn state(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: GroupOnly = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let hub = toolbox.hub();
    let library = hub.library(&group).await?;
    let sorry = if library.sorry > 0 {
        hub.declarations(
            &group,
            &[("state", "sorry".to_owned()), ("limit", "100".to_owned())],
        )
        .await?
    } else {
        Vec::new()
    };
    Ok(format::library_state(hub.base(), &library, &sorry))
}

#[derive(Deserialize)]
struct Lookup {
    name: String,
    group: Option<String>,
}

pub async fn lookup(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: Lookup = parse(args)?;
    let written = args.name.trim();
    let written = if written.starts_with("lean:") || written.contains("/lib/") {
        written.to_owned()
    } else {
        format!("lean:{written}")
    };
    let (group, name) = match address_in(toolbox, &written, args.group.as_deref()).await? {
        (group, Address::Declaration(name)) => (group, name),
        (_, other) => {
            return Err(ToolError::invalid(format!(
                "{other} is not a declaration: a declaration's address is lean:Name"
            )));
        }
    };
    match toolbox.hub().declaration(&group, &name).await {
        Ok(page) => Ok(format::declaration(&page)),
        Err(error) if error.status() == Some(reqwest::StatusCode::NOT_FOUND) => Ok(format!(
            "lean:{name} is missing: the library's last successful build has no such \
             declaration. Search for it with library_search.\n"
        )),
        Err(error) => Err(error.into()),
    }
}

#[derive(Deserialize)]
struct Search {
    query: String,
    group: Option<String>,
    limit: Option<u32>,
}

async fn search(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: Search = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let limit = args.limit.unwrap_or(20).clamp(1, 100);
    let found = toolbox
        .hub()
        .declarations(
            &group,
            &[("q", args.query.clone()), ("limit", limit.to_string())],
        )
        .await?;
    Ok(format::declaration_list(&found))
}
