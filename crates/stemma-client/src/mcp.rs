//! Stemma's tools over MCP, for any agent: Claude Code, Codex or another.
//!
//! What an agent posts through it is marked as written by that agent, for
//! the person whose token it uses. The agent is named by `--agent`, or
//! else by what the MCP client says it is, or else just "Agent".

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use crate::tools::Toolbox;

const INSTRUCTIONS: &str = "Stemma is where a research group does mathematics by talking. \
These tools reach the group's forum, threads of numbered messages in Markdown with $…$ math; its \
Lean library; its working papers, in LaTeX; and its workshop, which works on the library and the \
papers in the background. Everything has an address: t12 is thread 12, t12#45 its message 45, \
t12#45-50 messages 45 to 50; lean:Foo.bar is a declaration of the library; p/notes is a paper; w7 \
is a task of the workshop. Cite what you use by address, written as a reference: [[t12#45]], \
[[lean:Foo.bar]], [[p/notes]], [[w7]].";

/// The MCP server.
#[derive(Clone)]
pub struct StemmaServer {
    toolbox: Arc<Toolbox>,
    /// The agent's name, when given; otherwise the client's.
    agent: Option<String>,
}

impl StemmaServer {
    pub fn new(toolbox: Toolbox, agent: Option<String>) -> StemmaServer {
        StemmaServer {
            toolbox: Arc::new(toolbox),
            agent,
        }
    }

    /// Serves over standard input and output until the client leaves.
    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let running = self.serve(rmcp::transport::stdio()).await?;
        running.waiting().await?;
        Ok(())
    }
}

/// The name an MCP client goes by in what it posts.
pub fn agent_name(client: Option<&Implementation>) -> String {
    let Some(client) = client else {
        return "Agent".into();
    };
    let name = client.name.to_lowercase();
    if name.starts_with("claude") {
        "Claude Code".into()
    } else if name.starts_with("codex") {
        "Codex".into()
    } else if name.starts_with("opencode") {
        "OpenCode".into()
    } else if let Some(title) = client.title.as_deref().filter(|t| !t.trim().is_empty()) {
        title.trim().to_owned()
    } else if client.name.trim().is_empty() {
        "Agent".into()
    } else {
        client.name.trim().to_owned()
    }
}

impl ServerHandler for StemmaServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("stemma", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = self
            .toolbox
            .definitions()
            .into_iter()
            .map(|def| {
                let Value::Object(schema) = def.input_schema else {
                    unreachable!("a tool's schema is an object")
                };
                let mut annotations = ToolAnnotations::default();
                annotations.read_only_hint = Some(!def.writes);
                Tool::new(def.name, def.description, Arc::new(schema)).annotate(annotations)
            })
            .collect();
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let agent = match &self.agent {
            Some(agent) => agent.clone(),
            None => agent_name(context.client_info().as_ref()),
        };
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = match self.toolbox.call(&request.name, args, Some(&agent)).await {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        };
        Ok(result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_agents_after_their_clients() {
        let client = |name: &str| Implementation::new(name, "1.0");
        assert_eq!(agent_name(Some(&client("claude-code"))), "Claude Code");
        assert_eq!(agent_name(Some(&client("codex-mcp-client"))), "Codex");
        assert_eq!(agent_name(Some(&client("zed"))), "zed");
        assert_eq!(agent_name(None), "Agent");
    }
}
