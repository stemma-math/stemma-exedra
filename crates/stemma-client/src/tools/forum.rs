//! The forum's tools: list threads, read a thread or some of its messages,
//! post a message, open a thread and search.

use serde::Deserialize;
use serde_json::{Value, json};
use stemma_core::Address;
use stemma_core::api::{CreateThread, PostMessage, SendNote, ThreadKind};

use super::{GROUP, ToolDef, ToolError, ToolResult, Toolbox, address_in, parse};
use crate::format;

/// A thread longer than this is read as its first message and its last
/// [`RECENT`] ones.
const LONG_THREAD: usize = 80;
const RECENT: usize = 60;
/// The most messages read at once.
const MAX_RANGE: u32 = 200;

pub fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef::read("list_threads", |toolbox, args, _agent| {
            Box::pin(list_threads(toolbox, args))
        }),
        ToolDef::read("read_thread", |toolbox, args, _agent| {
            Box::pin(read_thread(toolbox, args))
        }),
        ToolDef {
            name: "post_message",
            handler: |toolbox, args, agent| Box::pin(post_message(toolbox, args, agent)),
            terminal: &["post"],
            positional: &["thread", "body"],
            description: "Post a message in a thread, in Markdown, with $…$ math and [[t12#45]] \
                          references. It is shown as written by you, the agent, for the person \
                          you work for. Give a message's address (t12#45) to reply to it.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "thread": {"type": "string",
                               "description": "The thread (t12), or the message to reply to (t12#45)."},
                    "body": {"type": "string", "description": "The message, in Markdown."},
                    "attachments": {"type":"array","items":{"type":"integer"},"description":"Ids returned by upload_attachment."},
                    "reply_to": {"type": "integer", "minimum": 1,
                                 "description": "The number of the message it replies to."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["thread", "body"]
            }),
            writes: true,
        },
        ToolDef {
            name: "notify",
            handler: |toolbox, args, agent| Box::pin(notify(toolbox, args, agent)),
            terminal: &["notify"],
            positional: &["to", "body"],
            description: "Leave a note for some of the group's members: it reaches them as a \
                          notification, shown as written by you, the agent, for the person you \
                          work for. Use it when someone must know of something and no thread \
                          is the place to say it, and keep it short: cite what it is about by \
                          address ([[t12#45]], [[lean:Foo.bar]]).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "to": {"type": "string",
                           "description": "Their usernames, separated by commas or spaces: @ada, @bob."},
                    "body": {"type": "string", "description": "The note, in Markdown."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["to", "body"]
            }),
            writes: true,
        },
        ToolDef {
            name: "open_thread",
            handler: |toolbox, args, _agent| Box::pin(open_thread(toolbox, args)),
            terminal: &["threads", "new"],
            positional: &["title", "body"],
            description: "Open a new thread in the group, to ask it something: a short title, \
                          and a first message in Markdown that cites by address what it is \
                          about ([[t12#45]], [[lean:Foo.bar]]). Use it sparingly, only \
                          when a decision is needed and no thread fits. Returns the thread's \
                          address.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "What it asks, in a line."},
                    "body": {"type": "string", "description": "The question, in Markdown."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["title", "body"]
            }),
            writes: true,
        },
        ToolDef::read("search", |toolbox, args, _agent| {
            Box::pin(search(toolbox, args))
        }),
    ]
}

#[derive(Deserialize)]
struct OpenThread {
    title: String,
    body: String,
    group: Option<String>,
}

async fn open_thread(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: OpenThread = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let page = toolbox
        .hub()
        .create_thread(
            &group,
            &CreateThread {
                title: args.title,
                kind: ThreadKind::Discussion,
                resident_agent: None,
                anchor: None,
                body: Some(args.body),
                attachments: Vec::new(),
            },
        )
        .await?;
    let number = page.thread.number;
    Ok(format!(
        "Opened t{number}, “{}”: cite it as [[t{number}]]. {}{}\n",
        page.thread.title,
        toolbox.hub().base(),
        Address::Thread(number).web_path(&group)
    ))
}

#[derive(Deserialize)]
struct ListThreads {
    group: Option<String>,
    kind: Option<ThreadKind>,
    limit: Option<u32>,
}

async fn list_threads(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: ListThreads = parse(args)?;
    let group = toolbox.group(args.group.as_deref()).await?;
    let limit = args.limit.unwrap_or(30).clamp(1, 200);
    let threads = toolbox.hub().threads(&group, args.kind, limit).await?;
    Ok(format::thread_list(&threads))
}

#[derive(Deserialize)]
struct ReadThread {
    file: Option<String>,
    address: String,
    group: Option<String>,
}

async fn read_thread(toolbox: &Toolbox, args: Value) -> ToolResult {
    let args: ReadThread = parse(args)?;
    let (group, address) = address_in(toolbox, &args.address, args.group.as_deref()).await?;
    let hub = toolbox.hub();
    let (thread, from, to) = match address {
        Address::Thread(number) => {
            let page = hub.thread(&group, number).await?;
            let mut out = format::thread_heading(&group, &page.thread);
            let messages = &page.messages;
            let long = messages.len() > LONG_THREAD;
            for (i, message) in messages.iter().enumerate() {
                if long && i > 0 && i < messages.len() - RECENT {
                    if i == 1 {
                        let last_left_out = messages[messages.len() - RECENT - 1].number;
                        out.push('\n');
                        out.push_str(&format::gap(number, message.number, last_left_out));
                    }
                    continue;
                }
                out.push('\n');
                out.push_str(&format::message(number, message));
            }
            return Ok(out);
        }
        Address::Message { thread, message }
        | Address::Attachment {
            thread, message, ..
        } => (thread, message, message),
        Address::Messages { thread, from, to } => (thread, from, to.min(from + MAX_RANGE - 1)),
        Address::Declaration(name) => {
            let args = json!({"name": name, "group": group});
            return super::library::lookup(toolbox, args).await;
        }

        Address::Paper(slug) => {
            let args = json!({"address": format!("p/{slug}"), "file": args.file, "group": group});
            return super::papers::read(toolbox, args).await;
        }
    };
    let messages = hub.messages(&group, thread, from, to).await?;
    if messages.is_empty() {
        return Err(ToolError::invalid(format!(
            "t{thread} has no message #{from}{}",
            if to > from {
                format!(" to #{to}")
            } else {
                String::new()
            }
        )));
    }
    let mut out = String::new();
    for (i, message) in messages.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format::message(thread, message));
    }
    Ok(out)
}

#[derive(Deserialize)]
struct Post {
    #[serde(default)]
    attachments: Vec<i64>,
    thread: String,
    body: String,
    reply_to: Option<u32>,
    group: Option<String>,
}

async fn post_message(toolbox: &Toolbox, args: Value, agent: Option<&str>) -> ToolResult {
    let args: Post = parse(args)?;
    let (group, address) = address_in(toolbox, &args.thread, args.group.as_deref()).await?;
    let (thread, reply_to) = match address {
        Address::Thread(thread) => (thread, args.reply_to),
        Address::Message { thread, message } => (thread, args.reply_to.or(Some(message))),
        _ => {
            return Err(ToolError::invalid(
                "Post in a thread (t12), or in reply to a message (t12#45)",
            ));
        }
    };
    let message = toolbox
        .hub()
        .post_message(
            &group,
            thread,
            &PostMessage {
                body: args.body,
                reply_to,
                attachments: args.attachments,
                agent: agent.map(str::to_owned),
            },
        )
        .await?;
    let address = Address::Message {
        thread,
        message: message.number,
    };
    Ok(format!(
        "Posted {address}: {}{}\n",
        toolbox.hub().base(),
        address.web_path(&group)
    ))
}

#[derive(Deserialize)]
struct Notify {
    to: String,
    body: String,
    group: Option<String>,
}

async fn notify(toolbox: &Toolbox, args: Value, agent: Option<&str>) -> ToolResult {
    let args: Notify = parse(args)?;
    let to: Vec<String> = args
        .to
        .split(|c: char| c == ',' || c.is_whitespace())
        .map(|name| name.trim().trim_start_matches('@'))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if to.is_empty() {
        return Err(ToolError::invalid("Say who the note is for"));
    }
    let group = toolbox.group(args.group.as_deref()).await?;
    toolbox
        .hub()
        .send_note(
            &group,
            &SendNote {
                to: to.clone(),
                body: args.body,
                agent: agent.map(str::to_owned),
            },
        )
        .await?;
    let to: Vec<String> = to.iter().map(|name| format!("@{name}")).collect();
    Ok(format!("Left a note for {}.\n", to.join(", ")))
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
    let hits = toolbox
        .hub()
        .search(&group, &args.query, args.limit.unwrap_or(20).clamp(1, 100))
        .await?;
    Ok(format::search_hits(&hits))
}
