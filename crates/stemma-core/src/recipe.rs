//! The recipe format, shared by the hub, the machine that approves a run and
//! the one that runs it.
//!
//! A recipe is a folder of the group's `recipes` repository:
//!
//! - `recipe.toml`, its manifest: what it is, what it takes, what it may
//!   deliver and what else it may do;
//! - `run.sh`, which a process runs, and nothing else: starting the recipe
//!   is running it;
//! - `files/`, anything else it needs, such as an agent's prompt.
//!
//! Each `{{input}}` in `run.sh` and in the text files of `files/` becomes
//! the input's value, as given: nothing is quoted or escaped.
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The manifest's file.
pub const MANIFEST: &str = "recipe.toml";
/// The script a process runs.
pub const ENTRY: &str = "run.sh";
/// The folder of everything else.
pub const FILES: &str = "files/";
/// The most a recipe's folder may hold.
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_FILES: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub title: String,
    /// One sentence: what it does and what it leaves behind. People and
    /// the orchestrator choose by it.
    pub summary: String,
    /// More for the orchestrator, when the summary does not say enough of
    /// when to use the recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub timeout_minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// What a run may deliver; every output Stemma knows if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<Output>>,
    /// What else a run may do in the group, beyond its outputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub inputs: BTreeMap<String, Input>,
}

/// What a run may deliver, chosen by its script or its agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    /// A pull request from the run's branch, in one of the group's
    /// repositories.
    PullRequest,
    /// A message in the request's thread.
    Message,
}

impl Output {
    pub const ALL: [Output; 2] = [Output::PullRequest, Output::Message];

    pub fn as_str(self) -> &'static str {
        match self {
            Output::PullRequest => "pull_request",
            Output::Message => "message",
        }
    }
}

/// Something a run may do in the group beyond its outputs, named after
/// Stemma's tool that does it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Post in any of the group's threads, not only the request's.
    PostMessage,
    OpenThread,
    Notify,
    CreatePaper,
}

impl Permission {
    /// The tool it allows.
    pub fn tool(self) -> &'static str {
        match self {
            Permission::PostMessage => "post_message",
            Permission::OpenThread => "open_thread",
            Permission::Notify => "notify",
            Permission::CreatePaper => "create_paper",
        }
    }
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
    /// One of the group's papers, by its slug: the run gets a checkout.
    Paper,
    Declaration,
    Thread,
}

/// A file of the recipe: text, where inputs go, or anything else, kept as
/// it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Binary { base64: String },
}

impl Content {
    fn of(bytes: Vec<u8>) -> Content {
        match String::from_utf8(bytes) {
            Ok(text) => Content::Text(text),
            Err(error) => Content::Binary {
                base64: base64::engine::general_purpose::STANDARD.encode(error.into_bytes()),
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recipe {
    pub manifest: Manifest,
    /// `run.sh` and the files under `files/`, by their path in the folder.
    pub files: BTreeMap<String, Content>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{file}:{line}: {message}")]
pub struct RecipeError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

/// Each `{{name}}` of a declared input in `text`, replaced once by `value`:
/// what a value holds is never replaced again, and other braces stay.
fn substitute(text: &str, mut value: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(close) = rest[start + 2..].find("}}") else {
            break;
        };
        let name = rest[start + 2..start + 2 + close].trim();
        out.push_str(&rest[..start]);
        match value(name) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&rest[start..start + close + 4]),
        }
        rest = &rest[start + close + 4..];
    }
    out.push_str(rest);
    out
}

impl Recipe {
    /// Reads the recipe `name` from its folder's files, by their path in
    /// the folder.
    pub fn parse(name: &str, folder: BTreeMap<String, Vec<u8>>) -> Result<Self, RecipeError> {
        let at = |file: &str, line, message: String| RecipeError {
            file: format!("{name}/{file}"),
            line,
            message,
        };
        if folder.len() > MAX_FILES + 2 {
            return Err(at(
                "",
                1,
                format!("A recipe holds at most {MAX_FILES} files"),
            ));
        }
        if folder.values().map(Vec::len).sum::<usize>() > MAX_BYTES {
            return Err(at("", 1, "A recipe holds at most 1 MiB".into()));
        }
        let mut folder = folder;
        let manifest = folder
            .remove(MANIFEST)
            .ok_or_else(|| at(MANIFEST, 1, "Every recipe needs a recipe.toml".into()))?;
        let manifest = String::from_utf8(manifest)
            .map_err(|_| at(MANIFEST, 1, "recipe.toml is not UTF-8".into()))?;
        let entry = folder
            .get(ENTRY)
            .ok_or_else(|| at(ENTRY, 1, "Every recipe needs a run.sh".into()))?;
        if std::str::from_utf8(entry).is_err() {
            return Err(at(ENTRY, 1, "run.sh is not UTF-8".into()));
        }
        if let Some(stray) = folder
            .keys()
            .find(|path| path.as_str() != ENTRY && !path.starts_with(FILES))
        {
            return Err(at(
                stray,
                1,
                "A recipe holds recipe.toml, run.sh and files/: put this under files/".into(),
            ));
        }
        let manifest: Manifest = toml::from_str(&manifest).map_err(|e: toml::de::Error| {
            at(
                MANIFEST,
                e.span()
                    .map(|s| 1 + manifest[..s.start].bytes().filter(|b| *b == b'\n').count())
                    .unwrap_or(1),
                e.message().into(),
            )
        })?;
        let bad = |message: &str| at(MANIFEST, 1, message.into());
        if manifest.title.trim().is_empty() || manifest.summary.trim().is_empty() {
            return Err(bad("Title and summary must not be empty"));
        }
        if manifest.hint.as_ref().is_some_and(|h| h.trim().is_empty()) {
            return Err(bad("The hint is empty: leave it out instead"));
        }
        if manifest.timeout_minutes == 0 || manifest.timeout_minutes > 24 * 60 {
            return Err(bad("Timeout must be between 1 and 1440 minutes"));
        }
        if let Some(version) = &manifest.min_version {
            semver::Version::parse(version)
                .map_err(|_| bad("min_version must be a semantic version, such as 0.3.0"))?;
        }
        if let Some(outputs) = &manifest.outputs
            && outputs.iter().collect::<BTreeSet<_>>().len() != outputs.len()
        {
            return Err(bad("An output is listed twice"));
        }
        if manifest.permissions.iter().collect::<BTreeSet<_>>().len() != manifest.permissions.len()
        {
            return Err(bad("A permission is listed twice"));
        }
        let files: BTreeMap<String, Content> = folder
            .into_iter()
            .map(|(path, bytes)| (path, Content::of(bytes)))
            .collect();
        let mut used = BTreeSet::new();
        for content in files.values() {
            if let Content::Text(text) = content {
                substitute(text, |name| {
                    used.insert(name.to_owned());
                    None
                });
            }
        }
        for (input, spec) in &manifest.inputs {
            if input.is_empty() || !input.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(bad("Input names use letters, digits and underscores"));
            }
            if spec.description.trim().is_empty() {
                return Err(bad(&format!("Input {input} has no description")));
            }
            if !used.contains(input) {
                return Err(bad(&format!(
                    "Input {input} is never used: put {{{{{input}}}}} in run.sh or a file"
                )));
            }
        }
        Ok(Self { manifest, files })
    }

    /// The outputs a run may deliver.
    pub fn outputs(&self) -> Vec<Output> {
        self.manifest
            .outputs
            .clone()
            .unwrap_or_else(|| Output::ALL.to_vec())
    }

    pub fn admits(&self, output: Output) -> bool {
        self.outputs().contains(&output)
    }

    pub fn accepts_version(&self, version: &str) -> bool {
        self.manifest.min_version.as_ref().is_none_or(|min| {
            semver::Version::parse(version)
                .ok()
                .zip(semver::Version::parse(min).ok())
                .is_some_and(|(v, m)| v >= m)
        })
    }

    /// Checks `inputs` against the manifest: no unknown input, every
    /// required one given.
    pub fn check(&self, name: &str, inputs: &BTreeMap<String, String>) -> Result<(), RecipeError> {
        let bad = |message| RecipeError {
            file: format!("{name}/{MANIFEST}"),
            line: 1,
            message,
        };
        for input in inputs.keys() {
            if !self.manifest.inputs.contains_key(input) {
                return Err(bad(format!("Unknown input {input}")));
            }
        }
        for (input, spec) in &self.manifest.inputs {
            if spec.required && inputs.get(input).is_none_or(|v| v.trim().is_empty()) {
                return Err(bad(format!("Missing required input {input}")));
            }
        }
        Ok(())
    }

    /// The folder a run starts with: `run.sh` and `files/`, with the
    /// inputs in place, once; an input left out becomes nothing.
    pub fn render(
        &self,
        name: &str,
        inputs: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, Vec<u8>>, RecipeError> {
        self.check(name, inputs)?;
        let value = |input: &str| {
            self.manifest
                .inputs
                .contains_key(input)
                .then(|| inputs.get(input).cloned().unwrap_or_default())
        };
        Ok(self
            .files
            .iter()
            .map(|(path, content)| {
                let bytes = match content {
                    Content::Text(text) => substitute(text, value).into_bytes(),
                    Content::Binary { base64 } => base64::engine::general_purpose::STANDARD
                        .decode(base64)
                        .unwrap_or_default(),
                };
                (path.clone(), bytes)
            })
            .collect())
    }

    /// `run.sh` with the inputs in place, as a machine's owner sees it
    /// before approving.
    pub fn script(&self, inputs: &BTreeMap<String, String>) -> String {
        match self.files.get(ENTRY) {
            Some(Content::Text(text)) => substitute(text, |input| {
                self.manifest
                    .inputs
                    .contains_key(input)
                    .then(|| inputs.get(input).cloned().unwrap_or_default())
            }),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(manifest: &str, run: &str, prompt: &str) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::from([
            (MANIFEST.into(), manifest.as_bytes().to_vec()),
            (ENTRY.into(), run.as_bytes().to_vec()),
            ("files/prompt.md".into(), prompt.as_bytes().to_vec()),
        ])
    }
    const MANIFEST_TEXT: &str = r#"
title = "Prove a statement"
summary = "Proves a statement in the library."
timeout_minutes = 60

[inputs.statement]
description = "What to prove"
kind = "text"
required = true
"#;
    const RUN: &str = "#!/bin/sh\nstemma agent run files/prompt.md\n";
    const PROMPT: &str = "Prove {{statement}} in the group's library. Keep {{ other }}.";

    #[test]
    fn the_template_is_valid() {
        let recipe = Recipe::parse(
            "general",
            BTreeMap::from([
                (
                    MANIFEST.into(),
                    include_bytes!("../../../recipes/template/general/recipe.toml").to_vec(),
                ),
                (
                    ENTRY.into(),
                    include_bytes!("../../../recipes/template/general/run.sh").to_vec(),
                ),
                (
                    "files/prompt.md".into(),
                    include_bytes!("../../../recipes/template/general/files/prompt.md").to_vec(),
                ),
            ]),
        )
        .unwrap();
        assert_eq!(recipe.outputs(), Output::ALL);
    }

    #[test]
    fn validation_names_file_and_line() {
        let err = Recipe::parse(
            "prove",
            folder(
                &MANIFEST_TEXT.replace("timeout_minutes = 60", "surprise = 60"),
                RUN,
                PROMPT,
            ),
        )
        .unwrap_err();
        assert_eq!(err.file, "prove/recipe.toml");
        assert!(err.line > 1);
        let never = Recipe::parse(
            "prove",
            folder(MANIFEST_TEXT, RUN, &PROMPT.replace("{{statement}}", "it")),
        )
        .unwrap_err();
        assert!(never.message.contains("never used"), "{never}");
        let mut stray = folder(MANIFEST_TEXT, RUN, PROMPT);
        stray.insert("notes.md".into(), b"x".to_vec());
        assert_eq!(
            Recipe::parse("prove", stray).unwrap_err().file,
            "prove/notes.md"
        );
        let mut missing = folder(MANIFEST_TEXT, RUN, PROMPT);
        missing.remove(ENTRY);
        assert!(Recipe::parse("prove", missing).is_err());
        assert!(
            Recipe::parse(
                "prove",
                folder(
                    &format!("outputs = [\"message\", \"message\"]\n{MANIFEST_TEXT}"),
                    RUN,
                    PROMPT
                )
            )
            .is_err()
        );
    }

    #[test]
    fn rendering_puts_inputs_in_every_text_file_once_and_as_given() {
        let mut files = folder(MANIFEST_TEXT, "#!/bin/sh\necho {{statement}}\n", PROMPT);
        files.insert("files/logo.bin".into(), vec![0xff, 0x00, 0xfe]);
        let recipe = Recipe::parse("prove", files).unwrap();
        assert!(recipe.render("prove", &BTreeMap::new()).is_err());
        let inputs = BTreeMap::from([("statement".into(), "{{statement}}; rm -rf x".into())]);
        let rendered = recipe.render("prove", &inputs).unwrap();
        assert_eq!(
            rendered[ENTRY],
            b"#!/bin/sh\necho {{statement}}; rm -rf x\n"
        );
        assert_eq!(
            String::from_utf8(rendered["files/prompt.md"].clone()).unwrap(),
            "Prove {{statement}}; rm -rf x in the group's library. Keep {{ other }}."
        );
        assert_eq!(rendered["files/logo.bin"], [0xff, 0x00, 0xfe]);
        assert_eq!(
            recipe.script(&inputs),
            "#!/bin/sh\necho {{statement}}; rm -rf x\n"
        );
        assert!(
            recipe
                .render("prove", &BTreeMap::from([("surprise".into(), "x".into())]))
                .is_err()
        );
    }

    #[test]
    fn outputs_are_all_of_stemmas_unless_narrowed() {
        let recipe = Recipe::parse("prove", folder(MANIFEST_TEXT, RUN, PROMPT)).unwrap();
        assert!(recipe.admits(Output::PullRequest) && recipe.admits(Output::Message));
        let narrowed = Recipe::parse(
            "report",
            folder(
                &format!(
                    "outputs = [\"message\"]\npermissions = [\"open_thread\"]\n{MANIFEST_TEXT}"
                ),
                RUN,
                PROMPT,
            ),
        )
        .unwrap();
        assert!(!narrowed.admits(Output::PullRequest));
        assert_eq!(narrowed.manifest.permissions, [Permission::OpenThread]);
        let nothing = Recipe::parse(
            "quiet",
            folder(&format!("outputs = []\n{MANIFEST_TEXT}"), RUN, PROMPT),
        )
        .unwrap();
        assert!(nothing.outputs().is_empty());
        assert!(recipe.accepts_version("0.1.1"));
    }
}
