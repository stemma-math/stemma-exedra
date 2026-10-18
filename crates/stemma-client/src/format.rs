//! Threads, messages and search results as text, the same for people at a
//! terminal and for agents: Markdown, with every message headed by its
//! address so that it can be cited.

use std::fmt::Write;

use stemma_core::Address;
use stemma_core::api::{
    Build, BuildState, DeclarationPage, DeclarationState, DeclarationSummary, LibraryState,
    Message, SearchHit, Task, TaskPage, Thread, ThreadKind,
};
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

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(10)]
}

fn build_line(build: &Build) -> String {
    let state = match build.state {
        BuildState::Queued => "queued",
        BuildState::Running => "running",
        BuildState::Succeeded => "passed",
        BuildState::Failed => "failed",
        BuildState::Cancelled => "cancelled",
    };
    let mut line = format!("commit {} · {state}", short(&build.commit));
    if let Some(finished) = build.finished_at {
        let _ = write!(line, " · {}", when(finished));
    }
    if let Some(lean) = &build.lean {
        let _ = write!(line, " · Lean {lean}");
    }
    if let Some(error) = &build.error {
        let _ = write!(line, "\n  {error}");
    }
    line
}

fn state_word(state: DeclarationState, kind: &str) -> &'static str {
    match state {
        DeclarationState::Proved if kind == "theorem" => "proved",
        DeclarationState::Proved => "complete",
        DeclarationState::Sorry => "sorry",
        DeclarationState::Missing => "missing",
    }
}

/// How the library stands.
pub fn library_state(hub: &str, library: &LibraryState, sorry: &[DeclarationSummary]) -> String {
    if !library.available {
        return "This hub keeps no library for the group (it needs a forge).\n".into();
    }
    let mut out = format!(
        "# The library\n\nClone it with git from {hub}{}, with your token as the password.\n",
        library.clone_path
    );
    if let Some(forge) = &library.forge {
        let _ = writeln!(out, "It is in the forge at {}.", forge.repository);
    }
    if let Some(head) = &library.head {
        let _ = writeln!(out, "main is at {}.", short(head));
    }
    match &library.current {
        Some(build) => {
            let _ = writeln!(out, "State from the build of {}.", build_line(build));
        }
        None => out.push_str("No build has passed yet: there is no state.\n"),
    }
    if let Some(latest) = &library.latest
        && library.current.as_ref().map(|c| c.id) != Some(latest.id)
    {
        let _ = writeln!(out, "Latest build: {}.", build_line(latest));
    }
    let _ = writeln!(
        out,
        "\n{} declarations, {} depending on sorry.",
        library.declarations, library.sorry
    );
    if !library.modules.is_empty() {
        out.push_str("\nModules:\n");
        for module in &library.modules {
            let _ = writeln!(
                out,
                "- {} · {} declarations, {} with sorry",
                module.name, module.declarations, module.sorry
            );
        }
    }
    if !sorry.is_empty() {
        out.push_str("\nDepending on sorry:\n");
        for declaration in sorry {
            let _ = writeln!(out, "- lean:{} ({})", declaration.name, declaration.module);
        }
    }
    out
}

/// A declaration, with all the hub knows of it.
pub fn declaration(page: &DeclarationPage) -> String {
    let d = &page.declaration;
    let mut out = format!(
        "# lean:{} · {} · {}\n\n{} in {}",
        d.name,
        d.kind,
        state_word(page.state, &d.kind),
        d.kind,
        d.module
    );
    if let Some(range) = &d.range {
        let _ = write!(out, ", lines {}–{}", range.start_line, range.end_line);
    }
    let _ = writeln!(out, " (commit {}).\n", short(&page.commit));
    let _ = writeln!(out, "```lean\n{} : {}\n```", d.name, d.type_);
    if let Some(doc) = &d.doc {
        let _ = writeln!(out, "\n{}", doc.trim());
    }
    if d.data_sorry {
        out.push_str("\nIts data, not only its proofs, depends on sorry.\n");
    }
    if !d.axioms.is_empty() {
        let _ = writeln!(out, "\nAxioms: {}.", d.axioms.join(", "));
    }
    let list = |names: &[String]| {
        names
            .iter()
            .map(|n| format!("lean:{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !d.uses.is_empty() {
        let _ = writeln!(out, "\nUses: {}.", list(&d.uses));
    }
    if !page.used_by.is_empty() {
        let _ = writeln!(out, "\nUsed by: {}.", list(&page.used_by));
    }
    if !page.references.is_empty() {
        out.push_str("\nReferenced in:\n");
        for site in &page.references {
            let _ = writeln!(out, "- {} ({})", site.address, site.title);
        }
    }
    out
}

/// One entry per declaration.
pub fn declaration_list(declarations: &[DeclarationSummary]) -> String {
    if declarations.is_empty() {
        return "Nothing found.\n".into();
    }
    let mut out = String::new();
    for d in declarations {
        let _ = writeln!(
            out,
            "- lean:{} · {} · {} · {}\n  {}",
            d.name,
            d.kind,
            state_word(d.state, &d.kind),
            d.module,
            d.type_.split_whitespace().collect::<Vec<_>>().join(" ")
        );
    }
    out
}

/// One line per task.
pub fn task_list(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "No tasks.\n".into();
    }
    let mut out = String::new();
    for task in tasks {
        let _ = writeln!(
            out,
            "- w{} · {} · {} · from {}, for {} · {}",
            task.number,
            task.title,
            task.state.as_str(),
            task.origin,
            task.requester.display_name,
            when(task.created_at)
        );
    }
    out
}

/// A task, with its report and the end of its log.
pub fn task(hub: &str, group: &str, page: &TaskPage) -> String {
    let task = &page.task;
    let mut out = format!(
        "# w{} · {} · {}\n\nAsked in {} by {}",
        task.number,
        task.title,
        task.state.as_str(),
        task.origin,
        task.requester.display_name
    );
    if let Some(agent) = &task.agent {
        let _ = write!(out, ", through {agent}");
    }
    let _ = writeln!(
        out,
        ", {}. {}{}",
        when(task.created_at),
        hub,
        Address::Task(task.number).web_path(group)
    );
    if let Some(runner) = &task.runner {
        let _ = write!(out, "\nWorked on with {runner}");
        if let Some(model) = &task.model {
            let _ = write!(out, " ({model})");
        }
        out.push_str(".\n");
    }
    if let Some(progress) = &task.progress {
        let _ = writeln!(out, "\nNow: {progress}.");
    }
    if let Some(reason) = &task.reason {
        let _ = writeln!(out, "\nWhy: {reason}");
    }
    if !task.commits.is_empty() {
        let commits: Vec<&str> = task.commits.iter().map(|c| short(c)).collect();
        let _ = writeln!(out, "\nCommits: {}.", commits.join(", "));
    }
    if let Some(message) = &task.report_message {
        let _ = writeln!(out, "\nReported in {message}.");
    }
    let _ = writeln!(out, "\n## Instructions\n\n{}", task.instructions.trim());
    if let Some(report) = page.report.as_deref().filter(|r| !r.trim().is_empty()) {
        let _ = writeln!(out, "\n## Report\n\n{}", report.trim());
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
