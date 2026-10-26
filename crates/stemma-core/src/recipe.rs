//! The recipe format, shared by the hub and the machine that approves a run.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatter {
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub runners: Vec<String>,
    #[serde(default)]
    pub writes: Vec<Repository>,
    pub timeout_minutes: u32,
    pub min_version: Option<String>,
    #[serde(default)]
    pub merge: Merge,
    #[serde(default)]
    pub accept_sorry: bool,
    #[serde(default)]
    pub inputs: BTreeMap<String, Input>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repository {
    Library,
    Paper,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Merge {
    #[default]
    None,
    Review,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub description: String,
    pub kind: InputKind,
    #[serde(default)]
    pub required: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Text,
    Paper,
    Declaration,
    Thread,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipe {
    pub front: FrontMatter,
    pub prompt: String,
    /// The first line of the Markdown prompt in the source file.
    pub prompt_line: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{file}:{line}: {message}")]
pub struct RecipeError {
    pub file: String,
    pub line: usize,
    pub message: String,
}
impl Recipe {
    pub fn parse(file: &str, source: &str) -> Result<Self, RecipeError> {
        let error = |line, message: String| RecipeError {
            file: file.into(),
            line,
            message,
        };
        let lines: Vec<_> = source.lines().collect();
        if lines.first() != Some(&"+++") {
            return Err(error(
                1,
                "Expected TOML front matter between +++ lines".into(),
            ));
        }
        let end = lines
            .iter()
            .skip(1)
            .position(|l| *l == "+++")
            .map(|n| n + 1)
            .ok_or_else(|| error(1, "Missing closing +++".into()))?;
        let header = lines[1..end].join("\n");
        let front: FrontMatter = toml::from_str(&header).map_err(|e: toml::de::Error| {
            error(
                e.span()
                    .map(|s| 2 + header[..s.start].bytes().filter(|b| *b == b'\n').count())
                    .unwrap_or(2),
                e.message().into(),
            )
        })?;
        let line_of = |key: &str| {
            lines[..end]
                .iter()
                .position(|l| l.trim().starts_with(key))
                .map(|n| n + 1)
                .unwrap_or(2)
        };
        let bad = |key: &str, message: &str| error(line_of(key), message.into());
        if front.title.trim().is_empty() || front.summary.trim().is_empty() {
            return Err(bad("title", "Title and summary must not be empty"));
        }
        if front.timeout_minutes == 0 || front.timeout_minutes > 24 * 60 {
            return Err(bad(
                "timeout_minutes",
                "Timeout must be between 1 and 1440 minutes",
            ));
        }
        if front
            .runners
            .iter()
            .any(|r| !["claude", "codex", "opencode"].contains(&r.as_str()))
        {
            return Err(bad("runners", "Unknown runner"));
        }
        if let Some(version) = &front.min_version {
            semver::Version::parse(version)
                .map_err(|_| bad("min_version", "Expected a semantic version"))?;
        }
        let paper_inputs = front
            .inputs
            .values()
            .filter(|i| i.kind == InputKind::Paper)
            .count();
        if front.writes.contains(&Repository::Paper) && paper_inputs != 1 {
            return Err(bad(
                "writes",
                "Writing a paper requires exactly one paper input",
            ));
        }
        if !front.writes.contains(&Repository::Paper) && paper_inputs != 0 {
            return Err(bad("inputs", "A paper input requires writes = [\"paper\"]"));
        }
        if front.writes.iter().collect::<BTreeSet<_>>().len() != front.writes.len() {
            return Err(bad("writes", "Duplicate repository"));
        }
        let prompt = lines[end + 1..].join("\n");
        if prompt.trim().is_empty() {
            return Err(error(end + 2, "The prompt is empty".into()));
        }
        let mut used = BTreeSet::new();
        let mut remaining = prompt.as_str();
        while let Some(start) = remaining.find("{{") {
            let offset = prompt.len() - remaining.len() + start;
            let Some(close) = remaining[start + 2..].find("}}") else {
                return Err(error(
                    end + 2 + prompt[..offset].lines().count().saturating_sub(1),
                    "Unclosed input placeholder".into(),
                ));
            };
            let name = remaining[start + 2..start + 2 + close].trim();
            if !front.inputs.contains_key(name) {
                return Err(error(
                    end + 2 + prompt[..offset].bytes().filter(|b| *b == b'\n').count(),
                    format!("Unknown input {name}"),
                ));
            }
            used.insert(name.to_owned());
            remaining = &remaining[start + close + 4..];
        }
        for (name, input) in &front.inputs {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(bad(
                    "inputs",
                    "Input names use letters, digits and underscores",
                ));
            }
            if input.description.trim().is_empty() {
                return Err(bad(
                    &format!("[inputs.{name}]"),
                    "Input description is empty",
                ));
            }
            if !used.contains(name) {
                return Err(bad(
                    &format!("[inputs.{name}]"),
                    &format!("The prompt never uses input {name}"),
                ));
            }
        }
        Ok(Self {
            front,
            prompt,
            prompt_line: end + 2,
        })
    }
    pub fn accepts_version(&self, version: &str) -> bool {
        self.front.min_version.as_ref().is_none_or(|min| {
            semver::Version::parse(version)
                .ok()
                .zip(semver::Version::parse(min).ok())
                .is_some_and(|(v, m)| v >= m)
        })
    }
    /// Substitutes once: input values cannot introduce new placeholders.
    pub fn render(
        &self,
        file: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<String, RecipeError> {
        let bad = |message| RecipeError {
            file: file.into(),
            line: self.prompt_line,
            message,
        };
        for name in inputs.keys() {
            if !self.front.inputs.contains_key(name) {
                return Err(bad(format!("Unknown input {name}")));
            }
        }
        for (name, input) in &self.front.inputs {
            if (input.required || input.kind == InputKind::Paper)
                && inputs.get(name).is_none_or(|v| v.trim().is_empty())
            {
                return Err(bad(format!("Missing required input {name}")));
            }
        }
        let mut out = String::new();
        let mut rest = self.prompt.as_str();
        while let Some(start) = rest.find("{{") {
            let end = rest[start + 2..].find("}}").expect("validated placeholder") + start + 2;
            out.push_str(&rest[..start]);
            out.push_str(
                inputs
                    .get(rest[start + 2..end].trim())
                    .map(String::as_str)
                    .unwrap_or(""),
            );
            rest = &rest[end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROVE: &str = include_str!("../../../recipes/template/prove.md");
    #[test]
    fn templates_are_valid() {
        for (name, text) in [
            ("prove.md", PROVE),
            (
                "review-paper.md",
                include_str!("../../../recipes/template/review-paper.md"),
            ),
        ] {
            Recipe::parse(name, text).unwrap();
        }
    }
    #[test]
    fn validation_names_file_and_line() {
        let err = Recipe::parse(
            "prove.md",
            &PROVE.replace("timeout_minutes = 60", "surprise = 60"),
        )
        .unwrap_err();
        assert_eq!(err.file, "prove.md");
        assert!(err.line > 1);
        assert!(
            Recipe::parse("prove.md", &PROVE.replace("{{statement}}", "{{typo}}"))
                .unwrap_err()
                .message
                .contains("Unknown input")
        );
        assert!(
            Recipe::parse("prove.md", &PROVE.replace("{{statement}}", "nothing"))
                .unwrap_err()
                .message
                .contains("never uses")
        );
        assert!(
            Recipe::parse(
                "prove.md",
                &PROVE.replace("kind = \"text\"", "kind = \"paper\"")
            )
            .is_err()
        );
        assert!(
            Recipe::parse(
                "prove.md",
                &PROVE.replace("writes = [\"library\"]", "writes = [\"paper\"]")
            )
            .is_err()
        );
    }
    #[test]
    fn a_paper_target_is_always_needed_to_write_a_paper() {
        let text = PROVE
            .replace("writes = [\"library\"]", "writes = [\"paper\"]")
            .replace("kind = \"text\"", "kind = \"paper\"");
        let text = text.replace("required = true", "required = false");
        let recipe = Recipe::parse("paper.md", &text).unwrap();
        assert!(recipe.render("paper.md", &BTreeMap::new()).is_err());
        recipe
            .render(
                "paper.md",
                &BTreeMap::from([("statement".into(), "notes".into())]),
            )
            .unwrap();
    }
    #[test]
    fn rendering_checks_inputs_and_does_not_interpret_values() {
        let recipe = Recipe::parse("prove.md", PROVE).unwrap();
        assert!(recipe.render("prove.md", &BTreeMap::new()).is_err());
        let inputs = BTreeMap::from([("statement".into(), "{{unknown}}".into())]);
        assert!(
            recipe
                .render("prove.md", &inputs)
                .unwrap()
                .contains("{{unknown}}")
        );
        assert!(recipe.accepts_version("0.1.1"));
    }
}
