//! The forum's tools: list threads, read a thread or some of its messages,
//! post a message, open a thread and search.

use serde::Deserialize;
use serde_json::{Value, json};
use stemma_core::Address;
use stemma_core::api::{CreateThread, PostMessage, ThreadKind};

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
        ToolDef {
            name: "list_threads",
            handler: |toolbox, args, _agent| Box::pin(list_threads(toolbox, args)),
            terminal: &["threads"],
            positional: &[],
            description: "List the threads of a group, most recently active first, with their \
                          addresses (t12), titles and number of messages.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "group": {"type": "string", "description": GROUP},
                    "kind": {"type": "string", "enum": ["discussion", "chat"],
                             "description": "Only discussions, or only chats."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 200,
                              "description": "How many; 30 if absent."}
                }
            }),
            writes: false,
        },
        ToolDef {
            name: "read_thread",
            handler: |toolbox, args, _agent| Box::pin(read_thread(toolbox, args)),
            terminal: &["read"],
            positional: &["address"],
            description: "Read a thread, a message or a range of messages, by address: t12 \
                          (the thread), t12#45 (a message) or t12#45-50 (messages 45 to 50). \
                          Links to the web app work too. Each message comes headed by its \
                          address, its author and its time. Declarations (lean:Name) and the \
                          and papers are read too.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "address": {"type": "string", "description": "t12, t12#45, t12#45-50, lean:Name or p/notes."},
                    "file": {"type": "string", "description": "For a paper, the source file to read."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["address"]
            }),
            writes: false,
        },
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
                    "reply_to": {"type": "integer", "minimum": 1,
                                 "description": "The number of the message it replies to."},
                    "group": {"type": "string", "description": GROUP}
                },
                "required": ["thread", "body"]
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
        ToolDef {
            name: "search",
            handler: |toolbox, args, _agent| Box::pin(search(toolbox, args)),
            terminal: &["search"],
            positional: &["query"],
            description: "Search a group's thread titles and messages, regardless of case and \
                          accents: every word (or prefix), and \"phrases\" in quotes, narrowed by \
                          author:<username>, since:/until: (2026-03-01, 7d, 2w), \
                          in:forum|chats|papers|library|p/<slug>|t<n> and \
                          kind:thread|message|agent|person. Results come with their addresses, \
                          to read them with read_thread.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "The words to find, all of them, and operators."},
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
                attachments: Vec::new(),
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
