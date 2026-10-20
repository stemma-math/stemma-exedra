//! What an agent is given of a thread: all of it when it is short, and
//! when it is long, what matters most within a budget of characters.

use std::collections::BTreeSet;

use crate::address::Address;
use crate::api::Message;
use crate::protocol::ContextItem;
use crate::reference::references;

/// How many messages of a range referenced by the trigger are kept.
const MAX_REFERENCED_RANGE: u32 = 30;

/// What each message costs beyond its body: its header, roughly.
const OVERHEAD: usize = 80;

/// Trims the messages of thread `thread` (in order of number) to answer
/// message `trigger`, within about `budget` characters.
///
/// Always kept: the first message, the trigger, the message it replies to
/// and the messages of this thread it references with `[[…]]`. Then, as
/// the budget allows, the messages just before the trigger, going back.
/// Messages after the trigger are left out. A message longer than a
/// quarter of the budget is cut, saying so. What is left out is marked by
/// gaps, which the agent can read with its tools.
pub fn trim(messages: &[Message], thread: u32, trigger: u32, budget: usize) -> Vec<ContextItem> {
    let messages: Vec<&Message> = messages.iter().filter(|m| m.number <= trigger).collect();
    let Some(first) = messages.first().map(|m| m.number) else {
        return Vec::new();
    };
    let max_body = budget / 4;
    let cost = |m: &Message| m.body.chars().count().min(max_body) + OVERHEAD;

    let mut kept = BTreeSet::from([first, trigger]);
    if let Some(trigger) = messages.iter().find(|m| m.number == trigger) {
        kept.extend(trigger.reply_to);
        for reference in references(&trigger.body) {
            match reference.address {
                Ok(Address::Message { thread: t, message }) if t == thread => {
                    kept.insert(message);
                }
                Ok(Address::Messages {
                    thread: t,
                    from,
                    to,
                }) if t == thread => {
                    kept.extend(from..=to.min(from + MAX_REFERENCED_RANGE - 1));
                }
                _ => {}
            }
        }
    }
    kept.retain(|n| messages.iter().any(|m| m.number == *n));

    let mut used: usize = messages
        .iter()
        .filter(|m| kept.contains(&m.number))
        .map(|m| cost(m))
        .sum();
    for message in messages.iter().rev() {
        if kept.contains(&message.number) {
            continue;
        }
        let more = cost(message);
        if used + more > budget {
            break;
        }
        used += more;
        kept.insert(message.number);
    }

    let mut items = Vec::new();
    let mut gap: Option<(u32, u32)> = None;
    for message in &messages {
        if !kept.contains(&message.number) {
            gap = Some(match gap {
                Some((from, _)) => (from, message.number),
                None => (message.number, message.number),
            });
            continue;
        }
        if let Some((from, to)) = gap.take() {
            items.push(ContextItem::Gap { from, to });
        }
        let mut message = (*message).clone();
        if message.body.chars().count() > max_body {
            let cut: String = message.body.chars().take(max_body).collect();
            message.body = format!(
                "{cut}\n\n[… cut here; read t{thread}#{} for all of it]",
                message.number
            );
        }
        items.push(ContextItem::Message {
            message: Box::new(message),
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::User;

    fn message(number: u32, body: &str, reply_to: Option<u32>) -> Message {
        Message {
            number,
            author: User {
                id: 1,
                username: "ada".into(),
                display_name: "Ada".into(),
            },
            agent: None,
            agent_runner: None,
            body: body.into(),
            reply_to,
            attachments: Vec::new(),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            edited_at: None,
            deleted: false,
        }
    }

    fn shape(items: &[ContextItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                ContextItem::Message { message } => message.number.to_string(),
                ContextItem::Gap { from, to } => format!("{from}-{to}"),
            })
            .collect()
    }

    #[test]
    fn a_short_thread_is_whole() {
        let messages: Vec<Message> = (1..=5).map(|n| message(n, "hello", None)).collect();
        assert_eq!(
            shape(&trim(&messages, 1, 5, 10_000)),
            ["1", "2", "3", "4", "5"]
        );
        // What came after the trigger is left out.
        assert_eq!(shape(&trim(&messages, 1, 3, 10_000)), ["1", "2", "3"]);
    }

    #[test]
    fn a_long_thread_keeps_what_matters() {
        let long = "x".repeat(400);
        let mut messages: Vec<Message> = (1..=40).map(|n| message(n, &long, None)).collect();
        messages[39] = message(40, "@claude compare [[t7#10-11]] with [[t3#2]]", Some(20));
        let items = trim(&messages, 7, 40, 3_200);
        assert_eq!(
            shape(&items),
            [
                "1", "2-9", "10", "11", "12-19", "20", "21-37", "38", "39", "40"
            ]
        );
    }

    #[test]
    fn a_huge_message_is_cut() {
        let messages = vec![message(1, &"y".repeat(10_000), None)];
        let items = trim(&messages, 2, 1, 4_000);
        let ContextItem::Message { message } = &items[0] else {
            panic!("expected a message");
        };
        assert!(message.body.len() < 1_100);
        assert!(message.body.ends_with("read t2#1 for all of it]"));
    }
}
