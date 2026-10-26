//! Recipes are read only from one immutable main commit in Forgejo.
use crate::{
    error::{ApiError, ApiResult},
    forge,
    state::AppState,
};
use stemma_core::{
    recipe::{Recipe, RecipeError},
    workshop::RecipeFile,
};
pub const REPOSITORY: &str = "recipes";
/// What a group's `recipes` repository starts with.
pub fn template() -> Vec<(String, String)> {
    vec![(
        "general.md".into(),
        include_str!("../../../recipes/template/general.md").into(),
    )]
}
pub async fn list(state: &AppState, slug: &str) -> ApiResult<Vec<RecipeFile>> {
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
    let mut recipes = Vec::new();
    for file in files
        .into_iter()
        .filter(|f| !f.path.contains('/') && f.path.ends_with(".md"))
    {
        let name = file.path.trim_end_matches(".md").to_owned();
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            continue;
        }
        let parsed = if file.size > 128 * 1024 {
            Err(RecipeError {
                file: file.path.clone(),
                line: 1,
                message: "Recipe is larger than 128 KiB".into(),
            })
        } else {
            let raw = forge
                .raw_file(slug, REPOSITORY, &commit, &file.path)
                .await
                .map_err(forge::api_error)?
                .unwrap_or_default();
            String::from_utf8(raw)
                .map_err(|_| RecipeError {
                    file: file.path.clone(),
                    line: 1,
                    message: "Recipe is not UTF-8".into(),
                })
                .and_then(|s| Recipe::parse(&file.path, &s))
        };
        let (recipe, error) = match parsed {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        };
        recipes.push(RecipeFile {
            name,
            version: file.sha,
            commit: commit.clone(),
            url: forge.public_url(&format!("{slug}/recipes/src/commit/{commit}/{}", file.path)),
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

/// The group's own `rebase` recipe, or else the hub's.
pub async fn rebase(state: &AppState, slug: &str) -> ApiResult<RecipeFile> {
    if let Some(file) = list(state, slug)
        .await?
        .into_iter()
        .find(|r| r.name == "rebase")
    {
        return Ok(file);
    }
    use sha2::{Digest, Sha256};
    let source = include_str!("../recipes/rebase.md");
    Ok(RecipeFile {
        name: "rebase".into(),
        version: format!(
            "builtin-{}",
            Sha256::digest(source)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ),
        commit: "builtin".into(),
        url: String::new(),
        recipe: Some(Recipe::parse("rebase.md", source).map_err(ApiError::internal)?),
        error: None,
    })
}
