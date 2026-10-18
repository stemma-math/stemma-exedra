//! Stemma's tools: the operations agents use, the same over MCP and at the
//! command line.
//!
//! Each space of Stemma has a module with its tools: what each is called,
//! what it takes (a JSON Schema) and how it runs against the hub: the
//! forum, the library and the workshop. [`Toolbox`]
//! offers them all.

mod forum;
mod library;
mod tasks;

use serde::de::DeserializeOwned;
use serde_json::Value;
use stemma_core::{Address, locate};

use crate::hub::{ApiError, Hub};

const GROUP: &str = "The group's short name; the default group if absent.";

/// Every tool, whatever it does.
fn all_definitions() -> Vec<ToolDef> {
    let mut tools = forum::definitions();
    tools.extend(library::definitions());
    tools.extend(tasks::definitions());
    tools
}

/// The tools that write which an agent answering in a thread may still
/// use, when its toolbox hands over: handing a task to the workshop.
pub const HANDS_OVER: &[&str] = &["create_task"];

/// The tools that write which the workshop's agent may still use, when its
/// toolbox opens threads: asking the group for a decision.
pub const OPENS_THREADS: &[&str] = &["open_thread"];

/// The names of the tools that only read: what agents' jobs may use.
pub fn read_only_names() -> Vec<&'static str> {
    all_definitions()
        .into_iter()
        .filter(|tool| !tool.writes)
        .map(|tool| tool.name)
        .collect()
}

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
    /// With `read_only`, still offer [`HANDS_OVER`].
    hands_over: bool,
    /// With `read_only`, still offer [`OPENS_THREADS`].
    opens_threads: bool,
}

impl Toolbox {
    pub fn new(hub: Hub, default_group: Option<String>) -> Toolbox {
        Toolbox {
            hub,
            default_group,
            read_only: false,
            hands_over: false,
            opens_threads: false,
        }
    }

    /// Leaves out the tools that change anything.
    pub fn read_only(mut self, read_only: bool) -> Toolbox {
        self.read_only = read_only;
        self
    }

    /// With [`Toolbox::read_only`], still offers what hands work over:
    /// [`HANDS_OVER`], for agents answering in a thread.
    pub fn hands_over(mut self, hands_over: bool) -> Toolbox {
        self.hands_over = hands_over;
        self
    }

    /// With [`Toolbox::read_only`], still offers what opens a thread:
    /// [`OPENS_THREADS`], for the workshop's agent.
    pub fn opens_threads(mut self, opens_threads: bool) -> Toolbox {
        self.opens_threads = opens_threads;
        self
    }

    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The tools on offer.
    pub fn definitions(&self) -> Vec<ToolDef> {
        all_definitions()
            .into_iter()
            .filter(|tool| {
                !(self.read_only
                    && tool.writes
                    && !(self.hands_over && HANDS_OVER.contains(&tool.name))
                    && !(self.opens_threads && OPENS_THREADS.contains(&tool.name)))
            })
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
        if let Some(result) = forum::call(self, tool.name, args.clone(), agent).await {
            return result;
        }
        if let Some(result) = tasks::call(self, tool.name, args.clone(), agent).await {
            return result;
        }
        match library::call(self, tool.name, args).await {
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
