//! Stemma's tools: the operations agents use, the same over MCP and at the
//! command line.
//!
//! Each space of Stemma has a module with its tools: what each is called,
//! what it takes (a JSON Schema) and how it runs against the hub: the
//! forum, the library, the papers. [`Toolbox`] offers
//! them all.

mod attachments;
mod forum;
mod library;
mod papers;
mod workshop;

use serde::de::DeserializeOwned;
use serde_json::Value;
use stemma_core::{Address, locate};

use crate::hub::{ApiError, Hub};

const GROUP: &str = "The group's short name; the default group if absent.";

/// Every tool, whatever it does.
pub fn all_definitions() -> Vec<ToolDef> {
    let mut tools = forum::definitions();
    tools.extend(library::definitions());
    tools.extend(papers::definitions());
    tools.extend(workshop::definitions());
    tools.push(attachments::definition());
    tools.push(attachments::upload_definition());
    tools
}

/// The names of the tools that only read: what agents' jobs may use.
pub fn read_only_names() -> Vec<&'static str> {
    all_definitions()
        .into_iter()
        .filter(|tool| !tool.writes)
        .map(|tool| tool.name)
        .collect()
}

/// Both front ends invoke the handler carried by the catalog entry.
pub type ToolHandler =
    for<'a> fn(
        &'a Toolbox,
        Value,
        Option<&'a str>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolResult> + Send + 'a>>;

/// A tool, as agents see it.
#[derive(Clone, Debug)]
pub struct ToolDef {
    pub name: &'static str,
    /// Terminal command path and positional arguments, shared with MCP's schema.
    pub terminal: &'static [&'static str],
    pub positional: &'static [&'static str],
    pub description: &'static str,
    /// A JSON Schema for its arguments: an object.
    pub input_schema: Value,
    /// Whether it changes anything, so that a read-only toolbox leaves it out.
    pub writes: bool,
    pub handler: ToolHandler,
}

/// Why a tool could not do what it was asked, in words for the agent.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{0}")]
    Invalid(String),
    #[error("there is no tool {0}")]
    Unknown(String),
    #[error(transparent)]
    Hub(#[from] ApiError),
}

impl ToolError {
    pub fn invalid(message: impl Into<String>) -> ToolError {
        ToolError::Invalid(message.into())
    }
}

pub type ToolResult = Result<String, ToolError>;

/// The tools, as one person (or one job) uses them on one hub.
#[derive(Clone, Debug)]
pub struct Toolbox {
    hub: Hub,
    /// The group to use when a call names none.
    default_group: Option<String>,
    read_only: bool,
    attachment_dir: Option<std::path::PathBuf>,
}

impl Toolbox {
    pub fn new(hub: Hub, default_group: Option<String>) -> Toolbox {
        Toolbox {
            hub,
            default_group,
            read_only: false,
            attachment_dir: None,
        }
    }

    /// Leaves out the tools that change anything.
    pub fn read_only(mut self, read_only: bool) -> Toolbox {
        self.read_only = read_only;
        self
    }

    /// Where files too large or unsuitable for inline MCP content are saved.
    pub fn attachment_dir(mut self, path: std::path::PathBuf) -> Self {
        self.attachment_dir = Some(path);
        self
    }

    pub async fn call_content(
        &self,
        name: &str,
        args: Value,
        agent: Option<&str>,
    ) -> Result<Vec<rmcp::model::ContentBlock>, ToolError> {
        if name == "download_attachment" {
            return attachments::download(self, args, true).await;
        }
        Ok(vec![rmcp::model::ContentBlock::text(
            self.call(name, args, agent).await?,
        )])
    }

    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The tools on offer.
    pub fn definitions(&self) -> Vec<ToolDef> {
        all_definitions()
            .into_iter()
            .filter(|tool| !self.read_only || !tool.writes)
            .collect()
    }

    /// Runs the tool `name` with `args`. Whatever it posts is marked as
    /// written by `agent`, when given.
    pub async fn call(&self, name: &str, args: Value, agent: Option<&str>) -> ToolResult {
        let tool = self
            .definitions()
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| ToolError::Unknown(name.to_owned()))?;
        let args = if args.is_null() {
            Value::Object(Default::default())
        } else {
            args
        };
        (tool.handler)(self, args, agent).await
    }

    /// The group a call is about: the one it names, or the default, or the
    /// person's only group.
    pub async fn group(&self, named: Option<&str>) -> Result<String, ToolError> {
        if let Some(group) = named.map(str::trim).filter(|g| !g.is_empty()) {
            return Ok(self.hub.group(group).await?.slug);
        }
        if let Some(group) = &self.default_group {
            return Ok(self.hub.group(group).await?.slug);
        }
        let me = self.hub.me().await?;
        match me.groups.as_slice() {
            [only] => Ok(only.slug.clone()),
            [] => Err(ToolError::invalid("You are in no group on this hub")),
            groups => Err(ToolError::invalid(format!(
                "Say which group: {}",
                groups
                    .iter()
                    .map(|g| g.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }
}

/// Reads a tool's arguments.
fn parse<T: DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|e| ToolError::invalid(format!("Invalid arguments: {e}")))
}

/// An address in a group, as a call gives it: in any form people write
/// one, a link to the web app included.
async fn address_in(
    toolbox: &Toolbox,
    written: &str,
    group: Option<&str>,
) -> Result<(String, Address), ToolError> {
    let located = locate(written)
        .map_err(|e| ToolError::invalid(format!("{written:?} is not an address: {e}")))?;
    let group = toolbox.group(group.or(located.group.as_deref())).await?;
    Ok((group, located.address))
}
