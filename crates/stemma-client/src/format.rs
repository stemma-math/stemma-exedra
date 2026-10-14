//! Threads, messages and search results as text, the same for people at a
//! terminal and for agents: Markdown, with every message headed by its
//! address so that it can be cited.

use std::fmt::Write;

use stemma_core::api::{Message, SearchHit, Thread, ThreadKind};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A time, to the minute, in UTC: `2026-10-14 21:03 UTC`.
pub fn when(t: OffsetDateTime) -> String {
    let t = t.to_offset(time::UtcOffset::UTC);
    match t.format(&Rfc3339) {
        Ok(s) => format!("{} {} UTC", &s[..10], &s[11..16]),
        Err(_) => t.to_string(),
    }
}

/// Who wrote a message: `Ada` or `Claude, for Ada`.
pub fn author(message: &Message) -> String {
    match &message.agent {
        Some(agent) => format!("{agent}, for {}", message.author.display_name),
        None => message.author.display_name.clone(),
    }
}

/// A message, headed by its address.
pub fn message(thread: u32, message: &Message) -> String {
    let mut out = format!(
        "### t{thread}#{} · {} · {}",
        message.number,
        author(message),
        when(message.created_at)
    );
    if let Some(to) = message.reply_to {
        let _ = write!(out, " · reply to #{to}");
    }
    if message.edited_at.is_some() && !message.deleted {
        out.push_str(" · edited");
    }
    out.push('\n');
    if message.deleted {
        out.push_str("(deleted)\n");
        return out;
    }
    if !message.body.is_empty() {
        out.push('\n');
        out.push_str(message.body.trim_end());
        out.push('\n');
    }
    if !message.attachments.is_empty() {
        out.push_str("\nFiles:\n");
        for file in &message.attachments {
            let _ = writeln!(
                out,
                "- t{thread}#{}/{} ({}, {} bytes)",
                message.number, file.name, file.content_type, file.size
            );
        }
    }
    out
}

/// A note for messages left out: `[#2–#9 not shown: read t12#2-9]`.
pub fn gap(thread: u32, from: u32, to: u32) -> String {
    if from == to {
        format!("[#{from} not shown: read t{thread}#{from}]\n")
    } else {
        format!("[#{from}–#{to} not shown: read t{thread}#{from}-{to}]\n")
    }
}

/// A thread's heading.
pub fn thread_heading(group: &str, thread: &Thread) -> String {
    let kind = match (thread.kind, &thread.resident_agent) {
        (ThreadKind::Chat, Some(agent)) => format!("A chat with @{agent}"),
        (ThreadKind::Chat, None) => "A chat".into(),
        (ThreadKind::Discussion, _) => "A discussion".into(),
    };
    format!(
        "# t{} · {}\n\n{kind} in {group}, started by {} on {}; {} messages.\n",
        thread.number,
        thread.title,
        thread.created_by.display_name,
        when(thread.created_at),
        thread.last_message
    )
}

/// One line per thread.
pub fn thread_list(threads: &[Thread]) -> String {
    if threads.is_empty() {
        return "No threads.\n".into();
    }
    let mut out = String::new();
    for thread in threads {
        let chat = match (&thread.kind, &thread.resident_agent) {
            (ThreadKind::Chat, Some(agent)) => format!(" (chat with @{agent})"),
            (ThreadKind::Chat, None) => " (chat)".into(),
            _ => String::new(),
        };
        let _ = writeln!(
            out,
            "t{} · {}{chat} · {} messages · last active {}",
            thread.number,
            thread.title,
            thread.last_message,
            when(thread.last_activity_at)
        );
    }
    out
}

/// One entry per search result.
pub fn search_hits(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "Nothing found.\n".into();
    }
    let mut out = String::new();
    for hit in hits {
        let who = match (&hit.author, &hit.agent) {
            (Some(user), Some(agent)) => format!(" · {agent}, for {}", user.display_name),
            (Some(user), None) => format!(" · {}", user.display_name),
            _ => String::new(),
        };
        let _ = writeln!(
            out,
            "- {} ({}){who} · {}\n  {}",
            hit.address,
            hit.thread_title,
            when(hit.created_at),
            hit.snippet
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use stemma_core::api::User;

    #[test]
    fn messages_are_headed_by_their_address() {
        let message = Message {
            number: 45,
            author: User {
                id: 1,
                username: "ada".into(),
                display_name: "Ada".into(),
            },
            agent: Some("Claude".into()),
            body: "The map is proper.\n".into(),
            reply_to: Some(44),
            attachments: Vec::new(),
            created_at: OffsetDateTime::from_unix_timestamp(1_760_475_780).unwrap(),
            edited_at: None,
            deleted: false,
        };
        assert_eq!(
            super::message(12, &message),
            "### t12#45 · Claude, for Ada · 2025-10-14 21:03 UTC · reply to #44\n\nThe map is proper.\n"
        );
        assert_eq!(gap(12, 2, 9), "[#2–#9 not shown: read t12#2-9]\n");
    }
}
