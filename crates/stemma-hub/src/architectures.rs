//! The workshop's architectures: the kinds of work it knows, each a folder
//! of `workshop/architectures/` in the repository, embedded in the binary.
//!
//! A folder has a manifest, `architecture.toml` (its name, what it does in
//! a line, the repository it works on, its steps, what makes a change big,
//! and its instructions' files), and the instructions, in Markdown. Adding
//! an architecture is adding a folder. The hub lists them, and gives a
//! workshop machine the one a task runs with, its instructions included.

use std::collections::HashSet;

use rust_embed::Embed;
use serde::Deserialize;
use stemma_core::api::{Architecture, ArchitectureStep};
use stemma_core::protocol::ArchitectureRun;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../workshop/architectures"]
struct Folders;

/// The architecture a task runs with unless it says otherwise.
pub const DEFAULT: &str = "formalize";

/// A manifest, as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    description: String,
    works_on: String,
    instructions: Vec<String>,
    #[serde(default)]
    big: Vec<String>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    name: String,
    description: String,
}

fn text(path: &str) -> anyhow::Result<String> {
    let file = Folders::get(path).ok_or_else(|| anyhow::anyhow!("{path} is missing"))?;
    Ok(String::from_utf8(file.data.into_owned())?)
}

fn load(folder: &str) -> anyhow::Result<Architecture> {
    let manifest: Manifest = toml::from_str(&text(&format!("{folder}/architecture.toml"))?)
        .map_err(|e| anyhow::anyhow!("{folder}/architecture.toml: {e}"))?;
    anyhow::ensure!(
        manifest.name == folder,
        "the architecture in {folder}/ is called {}",
        manifest.name
    );
    anyhow::ensure!(!manifest.steps.is_empty(), "{folder} has no steps");
    for file in &manifest.instructions {
        text(&format!("{folder}/{file}"))?;
    }
    Ok(Architecture {
        name: manifest.name,
        description: manifest.description.trim().to_owned(),
        works_on: manifest.works_on,
        steps: manifest
            .steps
            .into_iter()
            .map(|s| ArchitectureStep {
                name: s.name,
                description: s.description.trim().to_owned(),
            })
            .collect(),
        big: manifest.big,
        instructions: manifest.instructions,
    })
}

/// Every architecture, by name. One whose folder cannot be read is left
/// out, and said so in the log.
pub fn all() -> Vec<Architecture> {
    let folders: HashSet<String> = Folders::iter()
        .filter_map(|path| {
            let (folder, file) = path.split_once('/')?;
            (file == "architecture.toml").then(|| folder.to_owned())
        })
        .collect();
    let mut found: Vec<Architecture> = folders
        .iter()
        .filter_map(|folder| match load(folder) {
            Ok(architecture) => Some(architecture),
            Err(error) => {
                tracing::error!(folder, %error, "an architecture of the workshop cannot be read");
                None
            }
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The architecture called `name`, if there is one.
pub fn find(name: &str) -> Option<Architecture> {
    all().into_iter().find(|a| a.name == name)
}

/// The architecture as a workshop machine runs it: its steps and its
/// instructions, joined.
pub fn run_of(architecture: &Architecture) -> anyhow::Result<ArchitectureRun> {
    let mut instructions = String::new();
    for file in &architecture.instructions {
        if !instructions.is_empty() {
            instructions.push('\n');
        }
        instructions.push_str(&text(&format!("{}/{file}", architecture.name))?);
    }
    Ok(ArchitectureRun {
        name: architecture.name.clone(),
        steps: architecture.steps.iter().map(|s| s.name.clone()).collect(),
        instructions,
    })
}

/// A file a change touches, as the forge lists it.
pub struct Touched<'a> {
    pub path: &'a str,
    /// Its path before, if it was renamed.
    pub previous: Option<&'a str>,
    /// `added`, `modified`, `deleted`, `renamed`…
    pub status: &'a str,
}

/// Why a change is big, if it is: it deletes or renames a file, or touches
/// one of the architecture's `big` files. Nothing, if it is not.
pub fn why_big(architecture: &Architecture, files: &[Touched]) -> Vec<String> {
    let mut why = Vec::new();
    for file in files {
        match file.status {
            "deleted" | "removed" => why.push(format!("deletes {}", file.path)),
            "renamed" => why.push(format!(
                "renames {} to {}",
                file.previous.unwrap_or("a file"),
                file.path
            )),
            _ => {}
        }
        for path in [Some(file.path), file.previous].into_iter().flatten() {
            if architecture.big.iter().any(|big| big == path) {
                let what = format!("changes {path}");
                if !why.contains(&what) {
                    why.push(what);
                }
            }
        }
    }
    why
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formalize_is_there_and_readable() {
        let all = all();
        let formalize = all.iter().find(|a| a.name == DEFAULT).expect("formalize");
        assert_eq!(formalize.works_on, "library");
        let steps: Vec<&str> = formalize.steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(steps, ["work", "check", "propose"]);
        let run = run_of(formalize).unwrap();
        assert!(
            run.instructions
                .starts_with("You are {agent}, the workshop")
        );
        assert!(run.instructions.contains("open_thread"));
    }

    #[test]
    fn big_changes() {
        let formalize = find(DEFAULT).unwrap();
        let touched = |path, previous, status| Touched {
            path,
            previous,
            status,
        };
        assert!(why_big(&formalize, &[touched("Library/A.lean", None, "added")]).is_empty());
        assert!(why_big(&formalize, &[touched("Library/A.lean", None, "modified")]).is_empty());
        assert_eq!(
            why_big(&formalize, &[touched("lean-toolchain", None, "modified")]),
            ["changes lean-toolchain"]
        );
        assert_eq!(
            why_big(&formalize, &[touched("Library/A.lean", None, "deleted")]),
            ["deletes Library/A.lean"]
        );
        assert_eq!(
            why_big(
                &formalize,
                &[touched("Library/B.lean", Some("Library/A.lean"), "renamed")]
            ),
            ["renames Library/A.lean to Library/B.lean"]
        );
    }
}
