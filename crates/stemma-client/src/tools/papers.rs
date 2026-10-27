//! The papers' tools: list them, and read one's LaTeX source.

use serde::Deserialize;
use serde_json::{Value, json};
use stemma_core::Address;
use stemma_core::api::PAPER_MAIN;

use super::{GROUP, ToolDef, ToolError, ToolResult, Toolbox, address_in, parse};
use crate::format;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "create_paper",
            handler: |toolbox, args, _agent| Box::pin(create(toolbox, args)),
            terminal: &["paper", "new"],
            positional: &["slug", "title"],
            description: "Create a working paper from the group's template.",
            input_schema: json!({"type": "object", "properties": {
                "group": {"type": "string", "description": GROUP},
                "slug": {"type": "string", "description": "The paper's short name."},
                "title": {"type": "string", "description": "The paper's title."}
            }, "required": ["slug", "title"]}),
            writes: true,
        },
        ToolDef::read("list_papers", |toolbox, args, _agent| {
            Box::pin(list(toolbox, args))
        }),
        ToolDef::read("read_paper", |toolbox, args, _agent| {
            Box::pin(read(toolbox, args))
        }),
    ]
}

#[derive(Deserialize)]
struct GroupOnly {
    group: Option<String>,
}

async fn list(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: GroupOnly = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let papers = toolbox.hub().papers(&group).await?;
    Ok(format::paper_list(&papers))
}

#[derive(Deserialize)]
struct Read {
    address: String,
    file: Option<String>,
    group: Option<String>,
}

pub async fn read(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: Read = parse(args)?;
    let written = args.address.trim();
    let written = if written.starts_with("p/") || written.contains("[[") || written.contains("/g/")
    {
        written.to_owned()
    } else {
        format!("p/{written}")
    };
    let (group, slug) = match address_in(toolbox, &written, args.group.as_deref()).await? {
        (group, Address::Paper(slug)) => (group, slug),
        (_, other) => {
            return Err(ToolError::invalid(format!(
                "{other} is not a paper: a paper's address is p/notes"
            )));
        }
    };
    let hub = toolbox.hub();
    let page = hub.paper(&group, &slug).await?;
    let source = hub.paper_files(&group, &slug).await?;
    let file = args
        .file
        .as_deref()
        .map(|f| f.trim().trim_start_matches("./"))
        .filter(|f| !f.is_empty())
        .unwrap_or(PAPER_MAIN);
    let text = match hub.paper_file(&group, &slug, file).await {
        Ok(found) => found.text,
        Err(error) if error.status() == Some(reqwest::StatusCode::NOT_FOUND) => {
            let files: Vec<&str> = source.files.iter().map(|f| f.path.as_str()).collect();
            return Err(ToolError::invalid(format!(
                "p/{slug} has no file {file}: it has {}",
                files.join(", ")
            )));
        }
        Err(error) => return Err(error.into()),
    };
    Ok(format::paper(
        hub.base(),
        &group,
        &page,
        &source,
        file,
        &text,
    ))
}

#[derive(Deserialize)]
struct Create {
    group: Option<String>,
    slug: String,
    title: String,
}
async fn create(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: Create = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let paper = toolbox
        .hub()
        .create_paper(&group, args.slug.trim_start_matches("p/"), &args.title)
        .await?;
    Ok(format!(
        "Created p/{}, “{}”: {}{}\nClone it with a personal token as the password: git clone {}{}\n",
        paper.slug,
        paper.title,
        toolbox.hub().base(),
        Address::Paper(paper.slug.clone()).web_path(&group),
        toolbox.hub().base(),
        paper.clone_path
    ))
}
