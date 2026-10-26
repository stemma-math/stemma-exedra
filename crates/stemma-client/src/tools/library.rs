//! The library's tools: how it stands, a declaration, and a search.

use serde::Deserialize;
use serde_json::Value;
use stemma_core::Address;

use super::{ToolDef, ToolError, ToolResult, Toolbox, address_in, parse};
use crate::format;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef::read("library_state", |toolbox, args, _agent| {
            Box::pin(state(toolbox, args))
        }),
        ToolDef::read("library_lookup", |toolbox, args, _agent| {
            Box::pin(lookup(toolbox, args))
        }),
        ToolDef::read("library_search", |toolbox, args, _agent| {
            Box::pin(search(toolbox, args))
        }),
    ]
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
