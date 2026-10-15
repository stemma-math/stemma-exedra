//! The working papers' tools: list them, read one or a section, see what
//! changed, and write a new revision.

use serde::Deserialize;
use serde_json::{Value, json};
use stemma_core::Address;
use stemma_core::api::{CreatePaper, EditPaper};
use stemma_core::{diff, paper};

use super::{GROUP, ToolDef, ToolError, ToolResult, Toolbox, address_in, parse};
use crate::format;

/// Lines of context around each change.
const CONTEXT: usize = 2;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_papers",
            description: "List a group's working papers, most recently changed first: their \
                          addresses (p/compactification), titles, revisions, who changed them \
                          last, and whether they changed since you read them.",
            input_schema: json!({
                "type": "object",
                "properties": {"group": {"type": "string", "description": GROUP}}
            }),
            writes: false,
        },
        ToolDef {
            name: "read_paper",
            description: "Read a working paper, or one of its sections, by address: \
                          p/compactification or p/compactification#gluing. A paper is Markdown \
                          with $…$ math and [[…]] references; its sections are its headings. \
                          Give a revision to read an older one.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": {"type": "string",
                                "description": "p/slug, or p/slug#section."},
                    "revision": {"type": "integer", "minimum": 1,
                                 "description": "An older revision; the current one if absent."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["address"]
            }),
            writes: false,
        },
        ToolDef {
            name: "paper_changes",
            description: "What changed in a working paper since a revision: the reasons of the \
                          revisions after it and the changed lines. Without a revision, since \
                          the one you read last. Without a paper, which of the group's papers \
                          changed since you read them, and why.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "paper": {"type": "string", "description": "p/slug; every paper if absent."},
                    "since": {"type": "integer", "minimum": 0,
                              "description": "The revision to compare with; the one you read last if absent."},
                    "group": {"type": "string", "description": GROUP}
                }
            }),
            writes: false,
        },
        ToolDef {
            name: "propose_paper_edit",
            description: "Write a new revision of a working paper: its whole new text and the \
                          reason, for its history. It is stored as written by you, the agent, \
                          for the person you work for. Give the revision you read \
                          (base_revision): if the paper changed since, the edit is refused, and \
                          you should read it again. A paper that does not exist is started, \
                          with the title given.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "paper": {"type": "string", "description": "p/slug."},
                    "body": {"type": "string",
                             "description": "The whole new text, in Markdown, without the title."},
                    "reason": {"type": "string",
                               "description": "One line: what changed, and why."},
                    "title": {"type": "string", "description": "A new title, if it changes."},
                    "base_revision": {"type": "integer", "minimum": 1,
                                      "description": "The revision the edit is over; the current one if absent."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["paper", "body", "reason"]
            }),
            writes: true,
        },
    ]
}

/// Runs a paper's tool, if `name` is one.
pub async fn call(
    toolbox: &Toolbox,
    name: &str,
    args: Value,
    agent: Option<&str>,
) -> Option<ToolResult> {
    Some(match name {
        "list_papers" => list_papers(toolbox, args).await,
        "read_paper" => read_paper(toolbox, args).await,
        "paper_changes" => paper_changes(toolbox, args).await,
        "propose_paper_edit" => propose_edit(toolbox, args, agent).await,
        _ => return None,
    })
}

#[derive(Deserialize)]
struct GroupOnly {
    group: Option<String>,
}

async fn list_papers(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: GroupOnly = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let papers = toolbox.hub().papers(&group).await?;
    Ok(format::paper_list(&papers))
}

/// A paper's address in a group, as a call gives it: its slug and section.
async fn paper_in(
    toolbox: &Toolbox,
    written: &str,
    group: Option<&str>,
) -> Result<(String, String, Option<String>), ToolError> {
    match address_in(toolbox, written, group).await? {
        (group, Address::Paper { slug, section }) => Ok((group, slug, section)),
        (_, other) => Err(ToolError::invalid(format!(
            "{other} is not a working paper: a paper's address is p/slug"
        ))),
    }
}

#[derive(Deserialize)]
struct ReadPaper {
    address: String,
    revision: Option<u32>,
    group: Option<String>,
}

pub async fn read_paper(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: ReadPaper = parse(args)?;
    let (group, slug, section) = paper_in(toolbox, &args.address, args.group.as_deref()).await?;
    let hub = toolbox.hub();
    let page = hub.paper(&group, &slug).await?;
    let (body, heading) = match args.revision {
        Some(number) if number != page.paper.revision => {
            let old = hub.revision(&group, &slug, number).await?;
            let heading = format::revision_heading(&slug, &old.revision, page.paper.revision);
            (old.body, heading)
        }
        _ => (page.body.clone(), format::paper_heading(&page.paper)),
    };
    let mut out = heading;
    match section {
        Some(section) => match paper::section(&body, &section) {
            Some(text) => {
                out.push_str(&format!("\nSection p/{slug}#{section}:\n\n"));
                out.push_str(text.trim_end());
                out.push('\n');
            }
            None => {
                let sections = paper::sections(&body);
                return Err(ToolError::invalid(format!(
                    "p/{slug} has no section {section}; its sections are {}",
                    if sections.is_empty() {
                        "none".to_owned()
                    } else {
                        sections
                            .iter()
                            .map(|s| format!("#{}", s.slug))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )));
            }
        },
        None => {
            let sections = paper::sections(&body);
            if !sections.is_empty() {
                out.push_str("Sections: ");
                out.push_str(
                    &sections
                        .iter()
                        .map(|s| format!("#{}", s.slug))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                out.push('\n');
            }
            out.push('\n');
            out.push_str(if body.trim().is_empty() {
                "(The paper is empty.)\n"
            } else {
                &body
            });
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct Changes {
    paper: Option<String>,
    since: Option<u32>,
    group: Option<String>,
}

async fn paper_changes(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: Changes = parse(args)?;
    let hub = toolbox.hub();
    let Some(paper) = args.paper else {
        let group = toolbox.group(args.group.as_deref()).await?;
        return changed_papers(toolbox, &group, args.since).await;
    };
    let (group, slug, _) = paper_in(toolbox, &paper, args.group.as_deref()).await?;
    let page = hub.paper(&group, &slug).await?;
    let current = page.paper.revision;
    let since = match (args.since, page.paper.last_read) {
        (Some(since), _) => since,
        (None, Some(read)) => read,
        (None, None) => {
            return Ok(format!(
                "{}\nYou have not read p/{slug} yet: read it whole with read_paper.\n",
                format::paper_heading(&page.paper)
            ));
        }
    };
    if since >= current {
        return Ok(format!(
            "p/{slug} is at revision {current}: nothing changed since revision {since}.\n"
        ));
    }
    let revisions = hub.revisions(&group, &slug).await?;
    let mut out = format!(
        "# p/{slug} · {}\n\nFrom revision {since} to revision {current}:\n\n",
        page.paper.title
    );
    for revision in revisions.iter().filter(|r| r.number > since) {
        out.push_str(&format::revision_line(revision));
    }
    let old = if since == 0 {
        String::new()
    } else {
        hub.revision(&group, &slug, since).await?.body
    };
    let changes = diff::unified(&old, &page.body, CONTEXT);
    out.push_str("\nWhat changed in the text (- removed, + added):\n\n");
    out.push_str(if changes.is_empty() {
        "(The text is the same; only the title changed.)\n"
    } else {
        &changes
    });
    Ok(out)
}

/// The group's papers that changed since the person read them, with the
/// reasons of the revisions since. A revision means nothing across papers,
/// so `since` counts only as 0: every paper, from its start.
async fn changed_papers(toolbox: &Toolbox, group: &str, since: Option<u32>) -> ToolResult {
    let hub = toolbox.hub();
    let papers = hub.papers(group).await?;
    let mut out = String::new();
    for paper in &papers {
        let read = if since == Some(0) {
            Some(0)
        } else {
            paper.last_read
        };
        match read {
            Some(read) if read >= paper.revision => continue,
            None => {
                out.push_str(&format!(
                    "## p/{} · {}\n\nRevision {}; you have not read it.\n\n",
                    paper.slug, paper.title, paper.revision
                ));
            }
            Some(read) => {
                out.push_str(&format!(
                    "## p/{} · {}\n\nRevision {}; you read revision {read}. Since:\n\n",
                    paper.slug, paper.title, paper.revision
                ));
                for revision in hub
                    .revisions(group, &paper.slug)
                    .await?
                    .iter()
                    .filter(|r| r.number > read)
                {
                    out.push_str(&format::revision_line(revision));
                }
                out.push('\n');
            }
        }
    }
    if out.is_empty() {
        out.push_str(if papers.is_empty() {
            "The group has no papers yet.\n"
        } else {
            "Nothing changed since you read the papers.\n"
        });
    }
    Ok(out)
}

#[derive(Deserialize)]
struct ProposeEdit {
    paper: String,
    body: String,
    reason: String,
    title: Option<String>,
    base_revision: Option<u32>,
    group: Option<String>,
}

async fn propose_edit(toolbox: &Toolbox, args: Value, agent: Option<&str>) -> ToolResult {
    let args: ProposeEdit = parse(args)?;
    let (group, slug, _) = paper_in(toolbox, &args.paper, args.group.as_deref()).await?;
    let hub = toolbox.hub();
    let current = match hub.paper(&group, &slug).await {
        Ok(page) => Some(page.paper.revision),
        Err(error) if error.status() == Some(reqwest::StatusCode::NOT_FOUND) => None,
        Err(error) => return Err(error.into()),
    };
    let paper = match current {
        None => {
            let title = args.title.ok_or_else(|| {
                ToolError::invalid(format!("p/{slug} does not exist: give a title to start it"))
            })?;
            hub.create_paper(
                &group,
                &CreatePaper {
                    slug: slug.clone(),
                    title,
                    body: args.body,
                    reason: Some(args.reason),
                    agent: agent.map(str::to_owned),
                },
            )
            .await?
        }
        Some(current) => {
            hub.edit_paper(
                &group,
                &slug,
                &EditPaper {
                    title: args.title,
                    body: args.body,
                    reason: args.reason,
                    base_revision: args.base_revision.unwrap_or(current),
                    agent: agent.map(str::to_owned),
                },
            )
            .await?
        }
    };
    let address = Address::Paper {
        slug,
        section: None,
    };
    Ok(format!(
        "Stored {address} at revision {}: {}{}\n",
        paper.revision,
        hub.base(),
        address.web_path(&group)
    ))
}
