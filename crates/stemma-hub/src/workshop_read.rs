//! Adapters for the answers' read-only catalog, calling hub handlers in-process.
use crate::{
    api::{forum, library, papers},
    auth::Viewer,
    error::{ApiError, ApiResult},
    model::Tool,
    state::AppState,
};
use axum::extract::{Path, Query, State};
use serde_json::{Value, json};

const NAMES: &[&str] = &[
    "list_threads",
    "read_thread",
    "search",
    "library_state",
    "library_lookup",
    "library_search",
    "list_papers",
    "read_paper",
];
pub fn definitions() -> Vec<Tool> {
    stemma_core::catalog::read_tools()
        .into_iter()
        .filter(|t| NAMES.contains(&t.name))
        .map(|t| Tool {
            name: t.name.into(),
            description: t.description.into(),
            parameters: t.input_schema,
        })
        .collect()
}
fn text<'a>(args: &'a Value, name: &str) -> ApiResult<&'a str> {
    args[name]
        .as_str()
        .ok_or_else(|| ApiError::invalid(format!("Missing {name}")))
}
fn query<T: serde::de::DeserializeOwned>(value: Value) -> ApiResult<Query<T>> {
    Ok(Query(
        serde_json::from_value(value).map_err(|e| ApiError::invalid(format!("{e}")))?,
    ))
}
pub async fn call(
    state: &AppState,
    viewer: &Viewer,
    slug: &str,
    name: &str,
    mut args: Value,
) -> ApiResult<Value> {
    if args["group"].as_str().is_some_and(|g| g != slug) {
        return Err(ApiError::forbidden("Read only this request's group"));
    }
    if matches!(name, "search" | "library_search") {
        args["q"] = json!(text(&args, "query")?);
    }
    let s = State(state.clone());
    let v = viewer.clone();
    let p = Path(slug.to_owned());
    match name {
        "list_threads" => Ok(json!(forum::list_threads(s, v, p, query(args)?).await?.0)),
        "search" => Ok(json!(forum::search(s, v, p, query(args)?).await?.0)),
        "library_state" => {
            let state = library::state(s.clone(), v.clone(), Path(slug.to_owned()))
                .await?
                .0;
            let sorry =
                library::declarations(s, v, p, query(json!({"state": "sorry", "limit": 30}))?)
                    .await?
                    .0;
            Ok(json!({"state": state, "sorry": sorry}))
        }
        "library_lookup" => Ok(json!(
            library::declaration(
                s,
                v,
                Path((
                    slug.into(),
                    text(&args, "name")?.trim_start_matches("lean:").into()
                ))
            )
            .await?
            .0
        )),
        "library_search" => Ok(json!(library::declarations(s, v, p, query(args)?).await?.0)),
        "list_papers" => Ok(json!(papers::list(s, v, p).await?.0)),
        "read_thread" | "read_paper" => {
            let raw = text(&args, "address")?;
            let normalized = if name == "read_paper"
                && !raw.starts_with("p/")
                && !raw.contains("[[")
                && !raw.contains("/g/")
            {
                format!("p/{raw}")
            } else {
                raw.into()
            };
            let located =
                stemma_core::locate(&normalized).map_err(|e| ApiError::invalid(format!("{e}")))?;
            if located.group.as_deref().is_some_and(|g| g != slug) {
                return Err(ApiError::forbidden("Read only this request's group"));
            }
            use stemma_core::Address;
            match located.address {
                Address::Thread(n) => Ok(json!(
                    forum::get_thread(s, v, Path((slug.into(), n))).await?.0
                )),
                Address::Message { thread, message }
                | Address::Attachment {
                    thread, message, ..
                } => Ok(json!(
                    forum::get_message(s, v, Path((slug.into(), thread, message)))
                        .await?
                        .0
                )),
                Address::Messages { thread, from, to } => Ok(json!(
                    forum::list_messages(
                        s,
                        v,
                        Path((slug.into(), thread)),
                        query(json!({"from":from,"to":to.min(from.saturating_add(199))}))?
                    )
                    .await?
                    .0
                )),
                Address::Declaration(name) => Ok(json!(
                    library::declaration(s, v, Path((slug.into(), name)))
                        .await?
                        .0
                )),
                Address::Paper(paper) => {
                    let file = args["file"].as_str().unwrap_or("main.tex");
                    Ok(
                        json!({"paper":papers::get(s.clone(), v.clone(), Path((slug.into(), paper.clone()))).await?.0,
                        "source":papers::file(s, v, Path((slug.into(), paper, file.into()))).await?.0}),
                    )
                }
            }
        }
        _ => Err(ApiError::invalid("Unknown read tool")),
    }
}
