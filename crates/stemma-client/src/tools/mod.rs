//! Stemma's tools: the operations agents use, the same over MCP, in our
//! own harness and at the command line.
//!
//! Each space of Stemma has a module with its tools: what each is called,
//! what it takes (a JSON Schema) and how it runs against the hub. Today
//! that is the forum; working papers, the library and the workshop add
//! their modules beside it, and [`Toolbox`] offers them all.

mod forum;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::hub::{ApiError, Hub};

/// A tool, as agents see it.
#[derive(Clone, Debug)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    /// A JSON Schema for its arguments: an object.
    pub input_schema: Value,
    /// Whether it changes anything, so that a read-only toolbox leaves it out.
    pub writes: bool,
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
}

impl Toolbox {
    pub fn new(hub: Hub, default_group: Option<String>) -> Toolbox {
        Toolbox {
            hub,
            default_group,
            read_only: false,
        }
    }

    /// Leaves out the tools that change anything.
    pub fn read_only(mut self, read_only: bool) -> Toolbox {
        self.read_only = read_only;
        self
    }

    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The tools on offer.
    pub fn definitions(&self) -> Vec<ToolDef> {
        forum::definitions()
            .into_iter()
            .filter(|tool| !(self.read_only && tool.writes))
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
        match forum::call(self, tool.name, args, agent).await {
            Some(result) => result,
            None => Err(ToolError::Unknown(name.to_owned())),
        }
    }

    /// The group a call is about: the one it names, or the default, or the
    /// person's only group.
    pub async fn group(&self, named: Option<&str>) -> Result<String, ToolError> {
        if let Some(group) = named.map(str::trim).filter(|g| !g.is_empty()) {
            return Ok(group.to_owned());
        }
        if let Some(group) = &self.default_group {
            return Ok(group.clone());
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
