//! Stored prose consists of literal text and resolved identities. Its wire
//! representation is always rendered prose, never this internal JSON.
use crate::{Address, Located, locate};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    User(i64),
    Agent(String),
    Reference {
        group: i64,
        object: Object,
        qualified: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Object {
    Thread { id: i64, address: Address },
    Paper(i64),
    Declaration(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    Literal(String),
    Named { typed: String, target: Target },
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredText {
    pub parts: Vec<Part>,
}

impl StoredText {
    /// Resolve only at write time; unknown names stay literal forever.
    pub fn resolve(
        text: &str,
        mut mention: impl FnMut(&str) -> Option<Target>,
        mut reference: impl FnMut(&Located) -> Option<Target>,
    ) -> Self {
        let mut names = Vec::new();
        let refs = crate::references(text);
        for found in &refs {
            if let Ok(located) = locate(found.raw)
                && let Some(target) = reference(&located)
            {
                names.push((found.span.clone(), target));
            }
        }
        for (span, handle) in crate::mention::mention_spans(text) {
            if !refs
                .iter()
                .any(|r| r.span.start <= span.start && span.start < r.span.end)
                && let Some(target) = mention(&handle)
            {
                names.push((span, target));
            }
        }
        names.sort_by_key(|(span, _)| span.start);
        let mut parts = Vec::new();
        let mut cursor = 0;
        for (span, target) in names {
            if cursor < span.start {
                parts.push(Part::Literal(text[cursor..span.start].into()));
            }
            parts.push(Part::Named {
                typed: text[span.clone()].into(),
                target,
            });
            cursor = span.end;
        }
        if cursor < text.len() {
            parts.push(Part::Literal(text[cursor..].into()));
        }
        Self { parts }
    }
    /// A missing object retains its original spelling, separately identified
    /// so a newly created object with that name cannot take over the reference.
    pub fn render(
        &self,
        mut current: impl FnMut(&Target) -> Option<String>,
    ) -> (String, Vec<String>) {
        let mut text = String::new();
        let mut missing = Vec::new();
        for part in &self.parts {
            match part {
                Part::Literal(raw) => text.push_str(raw),
                Part::Named { typed, target } => match current(target) {
                    Some(name) => text.push_str(&name),
                    None => {
                        text.push_str(typed);
                        missing.push(typed.clone());
                    }
                },
            }
        }
        (text, missing)
    }
    pub fn typed(&self) -> String {
        self.parts
            .iter()
            .map(|p| match p {
                Part::Literal(text) | Part::Named { typed: text, .. } => text.as_str(),
            })
            .collect()
    }
    pub fn targets(&self) -> impl Iterator<Item = &Target> {
        self.parts.iter().filter_map(|p| match p {
            Part::Named { target, .. } => Some(target),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stored() -> StoredText {
        StoredText::resolve(
            "@ada @claude [[algebra/t12]] [[p/notes]] @unknown `@ada [[t1]]`\n```\n@ada [[p/notes]]\n```",
            |h| match h {
                "ada" => Some(Target::User(7)),
                "claude" => Some(Target::Agent("claude".into())),
                _ => None,
            },
            |r| {
                Some(Target::Reference {
                    group: 2,
                    qualified: r.group.is_some(),
                    object: match &r.address {
                        Address::Paper(_) => Object::Paper(9),
                        a => Object::Thread {
                            id: 4,
                            address: a.clone(),
                        },
                    },
                })
            },
        )
    }
    #[test]
    fn round_trip_and_rename_leave_code_and_unknown_names_alone() {
        let saved = stored();
        let saved: StoredText =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        let (text, missing) = saved.render(|t| {
            Some(match t {
                Target::User(_) => "@augusta".into(),
                Target::Agent(kind) => format!("@{kind}"),
                Target::Reference {
                    object: Object::Paper(_),
                    ..
                } => "[[p/new-notes]]".into(),
                _ => "[[geometry/t12]]".into(),
            })
        });
        assert_eq!(
            text,
            "@augusta @claude [[geometry/t12]] [[p/new-notes]] @unknown `@ada [[t1]]`\n```\n@ada [[p/notes]]\n```"
        );
        assert!(missing.is_empty());
    }
    #[test]
    fn deleted_account_uses_its_stored_name_and_deleted_objects_stay_typed() {
        let (text, missing) = stored().render(|t| match t {
            Target::User(_) => Some("A deleted account".into()),
            Target::Agent(kind) => Some(format!("@{kind}")),
            _ => None,
        });
        assert!(text.starts_with("A deleted account @claude [[algebra/t12]] [[p/notes]]"));
        assert_eq!(missing, ["[[algebra/t12]]", "[[p/notes]]"]);
    }
    #[test]
    fn multiline_code_spans_are_literal() {
        let text = "Before `@ada\n[[t12]]` after @ada";
        let stored =
            StoredText::resolve(text, |_| Some(Target::User(7)), |_| panic!("inside code"));
        assert_eq!(
            stored.render(|_| Some("@augusta".into())).0,
            "Before `@ada\n[[t12]]` after @augusta"
        );
    }
    #[test]
    fn literal_internal_looking_text_cannot_inject_an_identity() {
        let raw = r#"{"parts":[{"User":7}]} @nobody"#;
        assert_eq!(
            StoredText::resolve(raw, |_| None, |_| None)
                .render(|_| panic!())
                .0,
            raw
        );
    }
}
