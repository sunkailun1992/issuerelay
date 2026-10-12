//! MCP control server (`issuerelay mcp`, stdio) for Claude Code / Codex. It only reads status
//! and pauses / resumes monitoring; it can never mark an issue Done or retry a run.
use issuerelay_core::{ipc::client, paths::IpcEndpoint};
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, transport::stdio};
use serde_json::json;

/// Forwards each tool call to the running engine host over local IPC.
#[derive(Clone)]
pub struct ControlServer {
    endpoint: IpcEndpoint,
}

/// Arguments of the `list_jobs` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct JobsArgs {
    /// How many recent runs to return (default 20).
    pub limit: Option<u64>,
}

impl ControlServer {
    pub fn new(endpoint: IpcEndpoint) -> Self {
        Self { endpoint }
    }

    /// The engine's result as JSON text, or `error: <cause chain>` when the call failed (host
    /// not running, no answer in time, or an error the engine reported). The cause stays in the
    /// text so the AI client can tell a stopped app from a failed request.
    async fn forward(&self, method: &str, params: serde_json::Value) -> String {
        match client::call(&self.endpoint, method, params).await {
            Ok(value) => value.to_string(),
            Err(e) => format!("error: {e:#}"),
        }
    }
}

#[rmcp::tool_router(server_handler)]
impl ControlServer {
    #[rmcp::tool(description = "Show IssueRelay monitoring status")]
    async fn status(&self) -> String {
        self.forward("status", json!({})).await
    }

    #[rmcp::tool(description = "List recent IssueRelay runs")]
    async fn list_jobs(&self, Parameters(args): Parameters<JobsArgs>) -> String {
        self.forward("list_jobs", json!({"limit": args.limit.unwrap_or(20)}))
            .await
    }

    #[rmcp::tool(description = "Pause monitoring; runs already in progress continue")]
    async fn pause(&self) -> String {
        self.forward("pause", json!({})).await
    }

    #[rmcp::tool(description = "Resume monitoring")]
    async fn resume(&self) -> String {
        self.forward("resume", json!({})).await
    }

    #[rmcp::tool(description = "Check Linear for new issues now")]
    async fn poll_now(&self) -> String {
        self.forward("poll_now", json!({})).await
    }
}

/// Serves the control tools on stdin / stdout until the client disconnects.
pub async fn serve(endpoint: IpcEndpoint) -> anyhow::Result<()> {
    let service = ControlServer::new(endpoint).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
