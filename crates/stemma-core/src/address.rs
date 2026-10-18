//! Addresses: the stable short names of everything in a group.
//!
//! | Object                  | Address                               |
//! |-------------------------|---------------------------------------|
//! | Thread                  | `t12`                                 |
//! | Message                 | `t12#45`                              |
//! | Messages                | `t12#45-50`                           |
//! | Attachment              | `t12#45/photo.jpg`                    |
//! | Library declaration     | `lean:Foo.bar`                        |
//! | Task                    | `w7`                                  |
//!
//! Every address has one canonical spelling, which [`Address`]'s `Display`
//! writes and [`Address::from_str`](std::str::FromStr) reads back.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The address of an object in a group.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Address {
    /// A thread: `t12`.
    Thread(u32),
    /// One message of a thread: `t12#45`.
    Message { thread: u32, message: u32 },
    /// A range of messages of a thread, both ends included: `t12#45-50`.
    Messages { thread: u32, from: u32, to: u32 },
    /// A file attached to a message: `t12#45/photo.jpg`.
    Attachment {
        thread: u32,
        message: u32,
        name: String,
    },
    /// A declaration of the library: `lean:Foo.bar`.
    Declaration(String),
    /// A workshop task: `w7`.
    Task(u32),
}

/// Why a string is not an address.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    #[error("an address cannot be empty")]
    Empty,
    #[error("unknown kind of address")]
    UnknownKind,
    #[error("expected a number from 1 on, without leading zeros: {0:?}")]
    InvalidNumber(String),
    #[error("a range of messages must go forwards: {from}-{to}")]
    BackwardRange { from: u32, to: u32 },
    #[error("invalid attachment name: {0:?}")]
    InvalidAttachmentName(String),
    #[error("invalid Lean name: {0:?}")]
    InvalidLeanName(String),
}

/// The kind of object an address names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressKind {
    Thread,
    Message,
    Messages,
    Attachment,
    Declaration,
    Task,
}

impl Address {
    /// The kind of object this address names.
    pub fn kind(&self) -> AddressKind {
        match self {
            Address::Thread(_) => AddressKind::Thread,
            Address::Message { .. } => AddressKind::Message,
            Address::Messages { .. } => AddressKind::Messages,
            Address::Attachment { .. } => AddressKind::Attachment,
            Address::Declaration(_) => AddressKind::Declaration,
            Address::Task(_) => AddressKind::Task,
        }
    }

    /// The thread this address is in, if it is in one.
    pub fn thread(&self) -> Option<u32> {
        match *self {
            Address::Thread(thread)
            | Address::Message { thread, .. }
            | Address::Messages { thread, .. }
            | Address::Attachment { thread, .. } => Some(thread),
            _ => None,
        }
    }

    /// The path of this address in the web app, for the group with slug
    /// `group`: `/g/<group>/t/12#45`.
    pub fn web_path(&self, group: &str) -> String {
        match self {
            Address::Thread(t) => format!("/g/{group}/t/{t}"),
            Address::Message { thread, message }
            | Address::Attachment {
                thread, message, ..
            } => format!("/g/{group}/t/{thread}#{message}"),
            Address::Messages { thread, from, .. } => format!("/g/{group}/t/{thread}#{from}"),
            Address::Declaration(name) => format!("/g/{group}/lib/{name}"),
            Address::Task(n) => format!("/g/{group}/w/{n}"),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Address::Thread(t) => write!(f, "t{t}"),
            Address::Message { thread, message } => write!(f, "t{thread}#{message}"),
            Address::Messages { thread, from, to } => write!(f, "t{thread}#{from}-{to}"),
            Address::Attachment {
                thread,
                message,
                name,
            } => write!(f, "t{thread}#{message}/{name}"),
            Address::Declaration(name) => write!(f, "lean:{name}"),
            Address::Task(n) => write!(f, "w{n}"),
        }
    }
}

impl FromStr for Address {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(AddressError::Empty);
        }
        if let Some(name) = s.strip_prefix("lean:") {
            return parse_lean_name(name).map(Address::Declaration);
        }
        if let Some(rest) = s.strip_prefix('w') {
            return parse_number(rest).map(Address::Task);
        }
        if let Some(rest) = s.strip_prefix('t') {
            return parse_thread_address(rest);
        }
        Err(AddressError::UnknownKind)
    }
}

/// What follows the `t` of a thread, message or attachment address.
fn parse_thread_address(rest: &str) -> Result<Address, AddressError> {
    let Some((thread, fragment)) = rest.split_once('#') else {
        return parse_number(rest).map(Address::Thread);
    };
    let thread = parse_number(thread)?;

    if let Some((message, name)) = fragment.split_once('/') {
        return Ok(Address::Attachment {
            thread,
            message: parse_number(message)?,
            name: parse_attachment_name(name)?,
        });
    }
    match fragment.split_once('-') {
        Some((from, to)) => {
            let (from, to) = (parse_number(from)?, parse_number(to)?);
            match from.cmp(&to) {
                std::cmp::Ordering::Less => Ok(Address::Messages { thread, from, to }),
                // A range of one message is that message.
                std::cmp::Ordering::Equal => Ok(Address::Message {
                    thread,
                    message: from,
                }),
                std::cmp::Ordering::Greater => Err(AddressError::BackwardRange { from, to }),
            }
        }
        None => Ok(Address::Message {
            thread,
            message: parse_number(fragment)?,
        }),
    }
}

/// A number from 1 on, in decimal, without sign or leading zeros.
fn parse_number(s: &str) -> Result<u32, AddressError> {
    let invalid = || AddressError::InvalidNumber(s.to_owned());
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    s.parse().map_err(|_| invalid())
}

/// Whether `s` is a slug: lowercase ASCII letters, digits and dashes, not
/// at either end and never two in a row.
pub fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A file name: anything but slashes, brackets and control characters,
/// not blank, and not `.` or `..`.
fn parse_attachment_name(s: &str) -> Result<String, AddressError> {
    let valid = !s.trim().is_empty()
        && s != "."
        && s != ".."
        && !s
            .chars()
            .any(|c| c == '/' || c == '\\' || c == '[' || c == ']' || c.is_control());
    if valid {
        Ok(s.to_owned())
    } else {
        Err(AddressError::InvalidAttachmentName(s.to_owned()))
    }
}

/// A Lean name: dot-separated components, none empty, with no whitespace
/// or brackets. Components may use any Unicode, as Lean allows.
fn parse_lean_name(s: &str) -> Result<String, AddressError> {
    let valid = !s.is_empty()
        && s.split('.').all(|part| !part.is_empty())
        && !s
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '[' | ']' | '#' | '/'));
    if valid {
        Ok(s.to_owned())
    } else {
        Err(AddressError::InvalidLeanName(s.to_owned()))
    }
}

impl Serialize for Address {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Address {
        s.parse()
            .unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
    }

    #[test]
    fn parses_every_kind() {
        assert_eq!(parse("t12"), Address::Thread(12));
        assert_eq!(
            parse("t12#45"),
            Address::Message {
                thread: 12,
                message: 45
            }
        );
        assert_eq!(
            parse("t12#45-50"),
            Address::Messages {
                thread: 12,
                from: 45,
                to: 50
            }
        );
        assert_eq!(
            parse("t12#45/photo of the board.jpg"),
            Address::Attachment {
                thread: 12,
                message: 45,
                name: "photo of the board.jpg".into()
            }
        );
        assert_eq!(
            parse("lean:Foo.bar"),
            Address::Declaration("Foo.bar".into())
        );
        assert_eq!(
            parse("lean:CategoryTheory.Functor.map_id'"),
            Address::Declaration("CategoryTheory.Functor.map_id'".into())
        );
        assert_eq!(parse("lean:α.β₁"), Address::Declaration("α.β₁".into()));
        assert_eq!(parse("w7"), Address::Task(7));
    }

    #[test]
    fn a_range_of_one_is_a_message() {
        assert_eq!(
            parse("t3#4-4"),
            Address::Message {
                thread: 3,
                message: 4
            }
        );
    }

    #[test]
    fn rejects_malformed_addresses() {
        use AddressError::*;
        let cases: &[(&str, AddressError)] = &[
            ("", Empty),
            ("x12", UnknownKind),
            ("T12", UnknownKind),
            ("t", InvalidNumber("".into())),
            ("t0", InvalidNumber("0".into())),
            ("t012", InvalidNumber("012".into())),
            ("t-1", InvalidNumber("-1".into())),
            ("t+1", InvalidNumber("+1".into())),
            ("t1.5", InvalidNumber("1.5".into())),
            ("t 12", InvalidNumber(" 12".into())),
            ("t99999999999", InvalidNumber("99999999999".into())),
            ("t12#", InvalidNumber("".into())),
            ("t12#a", InvalidNumber("a".into())),
            ("t12#45-", InvalidNumber("".into())),
            ("t12#-45", InvalidNumber("".into())),
            ("t12#45-50-60", InvalidNumber("50-60".into())),
            ("t12#50-45", BackwardRange { from: 50, to: 45 }),
            ("t12#45/", InvalidAttachmentName("".into())),
            ("t12#45/..", InvalidAttachmentName("..".into())),
            ("t12#45/a/b", InvalidAttachmentName("a/b".into())),
            ("p/notes", UnknownKind),
            ("lean:", InvalidLeanName("".into())),
            ("lean:Foo..bar", InvalidLeanName("Foo..bar".into())),
            ("lean:.Foo", InvalidLeanName(".Foo".into())),
            ("lean:Foo bar", InvalidLeanName("Foo bar".into())),
            ("w", InvalidNumber("".into())),
            ("w0", InvalidNumber("0".into())),
        ];
        for (input, error) in cases {
            assert_eq!(
                input.parse::<Address>().as_ref(),
                Err(error),
                "parsing {input:?}"
            );
        }
    }

    #[test]
    fn formats_canonically_and_round_trips() {
        for s in [
            "t1",
            "t12#45",
            "t12#45-50",
            "t12#45/notes.tex",
            "lean:Nat.add_comm",
            "w4294967295",
        ] {
            assert_eq!(parse(s).to_string(), s);
        }
    }

    #[test]
    fn knows_its_thread_and_kind() {
        assert_eq!(parse("t12#45-50").thread(), Some(12));
        assert_eq!(parse("t12#45/a.png").thread(), Some(12));
        assert_eq!(parse("w12").thread(), None);
        assert_eq!(parse("t12#45-50").kind(), AddressKind::Messages);
        assert_eq!(parse("lean:A").kind(), AddressKind::Declaration);
    }

    #[test]
    fn has_a_web_path() {
        assert_eq!(parse("t12").web_path("g"), "/g/g/t/12");
        assert_eq!(parse("t12#45").web_path("g"), "/g/g/t/12#45");
        assert_eq!(parse("t12#45-50").web_path("g"), "/g/g/t/12#45");
        assert_eq!(parse("t12#45/x.pdf").web_path("g"), "/g/g/t/12#45");
        assert_eq!(parse("lean:A.b").web_path("g"), "/g/g/lib/A.b");
        assert_eq!(parse("w7").web_path("g"), "/g/g/w/7");
    }

    #[test]
    fn serializes_as_a_string() {
        let address = parse("t12#45-50");
        let json = serde_json::to_string(&address).unwrap();
        assert_eq!(json, r#""t12#45-50""#);
        assert_eq!(serde_json::from_str::<Address>(&json).unwrap(), address);
        assert!(serde_json::from_str::<Address>(r#""t0""#).is_err());
    }

    #[test]
    fn slugs() {
        assert!(is_slug("a"));
        assert!(is_slug("number-theory-2"));
        assert!(!is_slug(""));
        assert!(!is_slug("a_b"));
        assert!(!is_slug("ñ"));
        assert!(!is_slug("a-"));
    }
}
