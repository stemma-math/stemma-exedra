//! Recipes are read only from one immutable main commit in Forgejo: each is a
//! folder at the root of the group's `recipes` repository.
use std::collections::BTreeMap;

use crate::{
    error::{ApiError, ApiResult},
    forge,
    state::AppState,
};
use sha2::{Digest, Sha256};
use stemma_core::{
    recipe::{self, Recipe, RecipeError},
    workshop::RecipeFile,
};
pub const REPOSITORY: &str = "recipes";

/// What a group's `recipes` repository starts with.
pub fn template() -> Vec<(String, String)> {
    [
        (
            recipe::MANIFEST,
            include_str!("../../../recipes/template/general/recipe.toml"),
        ),
        (
            recipe::ENTRY,
            include_str!("../../../recipes/template/general/run.sh"),
        ),
        (
            "files/prompt.md",
            include_str!("../../../recipes/template/general/files/prompt.md"),
        ),
    ]
    .into_iter()
    .map(|(path, text)| (format!("general/{path}"), text.to_owned()))
    .collect()
}

/// The name of the recipe every group has.
pub const GENERAL: &str = "general";

/// The hub's own `general`, from the template, for a group whose `recipes`
/// repository has no `general/` folder. Its version follows the template.
pub fn builtin_general() -> RecipeFile {
    let prefix = format!("{GENERAL}/");
    let mut digest = Sha256::new();
    let mut folder = BTreeMap::new();
    for (path, text) in template() {
        let Some(path) = path.strip_prefix(&prefix) else {
            continue;
        };
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(text.as_bytes());
        digest.update(b"\n");
        folder.insert(path.to_owned(), text.into_bytes());
    }
    let version: String = digest
        .finalize()
        .iter()
        .take(20)
        .map(|b| format!("{b:02x}"))
        .collect();
    let (recipe, error) = match Recipe::parse(GENERAL, folder) {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e)),
    };
    RecipeFile {
        name: GENERAL.to_owned(),
        version: format!("builtin-{version}"),
        commit: "builtin".to_owned(),
        url: String::new(),
        recipe,
        error,
    }
}

/// Whether `name` may name a recipe: letters, digits, `-` and `_`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

/// A group's recipes, sorted by name. Every group has `general`: the hub's,
/// unless the group's own `general/` folder, even an invalid one, replaces it.
pub async fn list(state: &AppState, slug: &str) -> ApiResult<Vec<RecipeFile>> {
    let mut recipes = list_forge(state, slug).await?;
    if !recipes.iter().any(|r| r.name == GENERAL) {
        recipes.push(builtin_general());
        recipes.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(recipes)
}

async fn list_forge(state: &AppState, slug: &str) -> ApiResult<Vec<RecipeFile>> {
    let forge = state
        .forge
        .as_ref()
        .ok_or_else(|| ApiError::not_found("Forge"))?;
    let Some(commit) = forge
        .branch_commit(slug, REPOSITORY, "main")
        .await
        .map_err(forge::api_error)?
    else {
        return Ok(vec![]);
    };
    let files = forge
        .tree(slug, REPOSITORY, &commit)
        .await
        .map_err(forge::api_error)?
        .unwrap_or_default();
    // Each folder at the root is a recipe; files at the root are not.
    let mut folders: BTreeMap<String, Vec<forge::client::TreeEntry>> = BTreeMap::new();
    for file in files {
        if let Some((name, _)) = file.path.split_once('/')
            && valid_name(name)
        {
            folders.entry(name.to_owned()).or_default().push(file);
        }
    }
    let mut recipes = Vec::new();
    for (name, entries) in folders {
        let mut digest = Sha256::new();
        for entry in &entries {
            digest.update(entry.path.as_bytes());
            digest.update([0]);
            digest.update(entry.sha.as_bytes());
            digest.update(b"\n");
        }
        let version: String = digest
            .finalize()
            .iter()
            .take(20)
            .map(|b| format!("{b:02x}"))
            .collect();
        let size: u64 = entries.iter().map(|e| e.size).sum();
        let parsed = if size > recipe::MAX_BYTES as u64 || entries.len() > recipe::MAX_FILES + 2 {
            Err(RecipeError {
                file: name.clone(),
                line: 1,
                message: format!(
                    "A recipe holds at most {} files and 1 MiB",
                    recipe::MAX_FILES
                ),
            })
        } else {
            let mut folder = BTreeMap::new();
            for entry in &entries {
                let raw = forge
                    .raw_file(slug, REPOSITORY, &commit, &entry.path)
                    .await
                    .map_err(forge::api_error)?
                    .unwrap_or_default();
                folder.insert(entry.path[name.len() + 1..].to_owned(), raw);
            }
            Recipe::parse(&name, folder)
        };
        let (recipe, error) = match parsed {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        };
        recipes.push(RecipeFile {
            url: forge.public_url(&format!("{slug}/recipes/src/commit/{commit}/{name}")),
            name,
            version,
            commit: commit.clone(),
            recipe,
            error,
        });
    }
    Ok(recipes)
}
pub async fn read(state: &AppState, slug: &str, name: &str) -> ApiResult<RecipeFile> {
    list(state, slug)
        .await?
        .into_iter()
        .find(|r| r.name == name)
        .ok_or_else(|| ApiError::not_found("Recipe"))
}
