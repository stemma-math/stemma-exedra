//! References: addresses written in Markdown as `[[t12#45-50]]`.

use std::ops::Range;

use crate::address::{Address, AddressError};

/// A `[[…]]` reference found in a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference<'a> {
    /// Where the whole reference, brackets included, is in the text.
    pub span: Range<usize>,
    /// What is between the brackets.
    pub raw: &'a str,
    /// The address it names, or why it names none.
    pub address: Result<Address, AddressError>,
}

/// Finds the `[[…]]` references in a Markdown text, in order.
///
/// References in code (inline spans and fenced blocks) are not references.
/// A reference does not span lines, and its brackets hold no brackets.
pub fn references(text: &str) -> Vec<Reference<'_>> {
    let mut found = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;

    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();

        if let Some(marker) = fence_marker(line) {
            match fence {
                None => fence = Some(marker),
                Some((ch, len)) if marker.0 == ch && marker.1 >= len && is_closing(line) => {
                    fence = None
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_none() {
            scan_line(line, start, &mut found);
        }
    }
    found
}

/// The fence a line opens or closes, if it is one: its character and length.
pub(crate) fn fence_marker(line: &str) -> Option<(u8, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line.as_bytes()[indent..];
    let ch = *rest.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = rest.iter().take_while(|&&b| b == ch).count();
    (len >= 3).then_some((ch, len))
}

/// A closing fence has nothing after its marker but spaces.
pub(crate) fn is_closing(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.bytes().all(|b| b == trimmed.as_bytes()[0])
}

fn scan_line<'a>(line: &'a str, base: usize, found: &mut Vec<Reference<'a>>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                // Skip an inline code span: a run of backticks up to the
                // next run of the same length, or the run alone if unclosed.
                let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
                i += run;
                if let Some(end) = closing_run(&bytes[i..], run) {
                    i += end + run;
                }
            }
            b'[' if bytes.get(i + 1) == Some(&b'[') => {
                let inner_start = i + 2;
                let close = line[inner_start..]
                    .find(['[', ']', '\n'])
                    .map(|n| inner_start + n);
                match close {
                    Some(end) if line[end..].starts_with("]]") => {
                        let raw = &line[inner_start..end];
                        found.push(Reference {
                            span: base + i..base + end + 2,
                            raw,
                            address: raw.trim().parse(),
                        });
                        i = end + 2;
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
}

/// Where a run of exactly `run` backticks starts in `bytes`.
pub(crate) fn closing_run(bytes: &[u8], run: usize) -> Option<usize> {
    let mut j = 0;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let len = bytes[j..].iter().take_while(|&&b| b == b'`').count();
            if len == run {
                return Some(j);
            }
            j += len;
        } else {
            j += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raws(text: &str) -> Vec<&str> {
        references(text).into_iter().map(|r| r.raw).collect()
    }

    #[test]
    fn finds_references_with_their_spans() {
        let text = "See [[t12#45-50]] and [[lean:Foo.bar]].";
        let found = references(text);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].span.clone()], "[[t12#45-50]]");
        assert_eq!(
            found[0].address,
            Ok(Address::Messages {
                thread: 12,
                from: 45,
                to: 50
            })
        );
        assert_eq!(&text[found[1].span.clone()], "[[lean:Foo.bar]]");
    }

    #[test]
    fn keeps_invalid_references_with_their_error() {
        let found = references("[[t0]] [[nothing]] [[]]");
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|r| r.address.is_err()));
    }

    #[test]
    fn trims_spaces_inside_the_brackets() {
        assert_eq!(references("[[ w7 ]]")[0].address, Ok(Address::Task(7)));
    }

    #[test]
    fn ignores_code() {
        let text =
            "`[[t1]]` ``a ` [[t2]]`` [[t3]]\n```\n[[t4]]\n```\n~~~~\n[[t5]]\n~~~\n~~~~\n[[t6]]";
        assert_eq!(raws(text), ["t3", "t6"]);
    }

    #[test]
    fn an_unclosed_backtick_is_text() {
        assert_eq!(raws("a ` b [[t1]]"), ["t1"]);
    }

    #[test]
    fn ignores_unclosed_and_nested_brackets() {
        assert_eq!(raws("[[t1] [t2]] [[t3"), Vec::<&str>::new());
        assert_eq!(raws("[[[t1]]]"), ["t1"]);
        assert_eq!(raws("[[t1\n]]"), Vec::<&str>::new());
    }

    #[test]
    fn spans_are_byte_offsets_in_the_whole_text() {
        let text = "ñ\n→ [[w1]]";
        let found = references(text);
        assert_eq!(&text[found[0].span.clone()], "[[w1]]");
    }
}
