//! Stemma's client: the hub's API over HTTP ([`Hub`]), the settings of a
//! machine signed in to a hub ([`config`]), Stemma's tools ([`tools`]),
//! the same for the command line, our own harness and MCP, and the MCP
//! server that offers them to any agent ([`mcp`]).

pub mod config;
pub mod format;
pub mod hub;
pub mod mcp;
pub mod tools;

pub use hub::{ApiError, Hub};
pub use tools::Toolbox;
