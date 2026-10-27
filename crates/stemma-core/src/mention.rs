//! Mentions: `@claude` in a message summons the agent with that handle.

use crate::reference::{closing_run, prose_spans};

/// Whether `handle` can name an agent: a lowercase letter, then up to 31
/// lowercase letters, digits, `-` or `_`.
pub fn is_handle(handle: &str) -> bool {
    let mut bytes = handle.bytes();
    handle.len() <= 32
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(is_handle_byte)
}

fn is_handle_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'
}

/// The handles mentioned in a Markdown text, each once, in order.
///
/// A mention is `@` and a handle, after the start of a line or a character
/// that is not part of a word (so `ada@example.org` mentions no one), and
/// not in code. Handles are matched without case, and a trailing `-` or
/// `_` is punctuation, not part of the handle.
pub fn mentions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (_, handle) in mention_spans(text) {
        if is_handle(&handle) && !found.contains(&handle) {
            found.push(handle);
        }
    }
    found
}

/// Every mention with its byte span, including the broader account namespace.
pub fn mention_spans(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut found = Vec::new();
    for span in prose_spans(text) {
        scan_line(&text[span.clone()], span.start, &mut found);
    }
    found
}

fn scan_line(line: &str, base: usize, found: &mut Vec<(std::ops::Range<usize>, String)>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                i += run;
                if let Some(end) = closing_run(&bytes[i..], run) {
                    i += end + run;
                }
            }
            b'@' => {
                let after_word = line[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'));
                let len = bytes[i + 1..]
                    .iter()
                    .take_while(|&&b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    .count();
                let handle = line[i + 1..i + 1 + len]
                    .trim_end_matches(['.', '-', '_'])
                    .to_ascii_lowercase();
                if !after_word && !handle.is_empty() && handle.len() <= 32 {
                    found.push((base + i..base + i + 1 + handle.len(), handle));
                }
                i += 1 + len;
            }
            _ => i += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mentions_once_in_order() {
        assert_eq!(
            mentions("@claude @qwen what do you think? @Claude, and @qwen."),
            ["claude", "qwen"]
        );
        assert_eq!(mentions("(@codex) @gpt-4o-mini-"), ["codex", "gpt-4o-mini"]);
    }

    #[test]
    fn ignores_addresses_code_and_non_handles() {
        assert!(mentions("write to ada@example.org").is_empty());
        assert!(mentions("`@claude` and\n```\n@codex\n```").is_empty());
        assert!(mentions("@ @1bot @@x").is_empty());
    }

    #[test]
    fn handles() {
        assert!(is_handle("claude"));
        assert!(is_handle("qwen-3_b"));
        assert!(!is_handle("Claude"));
        assert!(!is_handle("1bot"));
        assert!(!is_handle(""));
        assert!(!is_handle(&"a".repeat(33)));
    }
}
