//! In-process MCP server that mimics Linear's tools, connected over an in-memory duplex.
use std::sync::{Arc, Mutex};

use issuerelay_core::linear::mcp::LinearClient;
use rmcp::{ServiceExt, handler::server::wrapper::Parameters, model::ClientConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Default)]
pub struct FakeLinearServer {
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListIssuesArgs {
    pub assignee: Option<String>,
    pub label: Option<String>,
    pub state: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetIssueArgs {
    pub id: String,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SaveIssueArgs {
    pub id: String,
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GetUserArgs {
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PageArgs {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TeamArgs {
    pub team: String,
}

pub fn issue_json(n: u32) -> Value {
    json!({
        "id": format!("DEMO-{n}"),
        "uuid": format!("00000000-0000-0000-0000-00000000000{n}"),
        "title": format!("Demo task {n}"),
        "url": format!("https://linear.app/demo/issue/DEMO-{n}"),
        "status": "Todo",
        "labels": ["ai-ready"],
        "project": "Demo Project",
        "projectId": "proj-1",
        "assigneeId": "user-1",
        "parentId": null,
        "priority": {"value": 2, "name": "High"},
        "createdAt": "2026-10-01T08:00:00.000Z",
        "updatedAt": "2026-10-01T09:00:00.000Z"
    })
}

impl FakeLinearServer {
    fn record(&self, tool: &str, args: &impl Serialize) {
        self.calls
            .lock()
            .unwrap()
            .push((tool.into(), serde_json::to_value(args).unwrap()));
    }

    pub fn calls_to(&self, tool: &str) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(t, _)| t == tool)
            .map(|(_, a)| a.clone())
            .collect()
    }
}

#[rmcp::tool_router(server_handler)]
impl FakeLinearServer {
    #[rmcp::tool(description = "List issues")]
    async fn list_issues(&self, Parameters(args): Parameters<ListIssuesArgs>) -> String {
        self.record("list_issues", &args);
        if args.cursor.as_deref() == Some("c2") {
            json!({"issues": [issue_json(2)], "hasNextPage": false}).to_string()
        } else {
            json!({"issues": [issue_json(1)], "hasNextPage": true, "cursor": "c2"}).to_string()
        }
    }

    #[rmcp::tool(description = "Get one issue")]
    async fn get_issue(
        &self,
        Parameters(args): Parameters<GetIssueArgs>,
    ) -> Result<String, String> {
        self.record("get_issue", &args);
        if args.id == "DEMO-1" || args.id.ends_with("0001") {
            Ok(issue_json(1).to_string())
        } else {
            // Linear's real reply for an unknown issue (M0 sample get_issue_missing.json).
            Err(r#"{"error":"invalid_request","message":"Could not find referenced Issue.","status":400}"#.to_string())
        }
    }

    #[rmcp::tool(description = "Create or update an issue")]
    async fn save_issue(&self, Parameters(args): Parameters<SaveIssueArgs>) -> String {
        self.record("save_issue", &args);
        json!({"id": args.id, "status": args.state}).to_string()
    }

    #[rmcp::tool(description = "Get a user")]
    async fn get_user(&self, Parameters(args): Parameters<GetUserArgs>) -> String {
        self.record("get_user", &args);
        json!({"id": "user-1", "name": "Jane Doe", "email": "jane@example.com"}).to_string()
    }

    #[rmcp::tool(description = "List projects")]
    async fn list_projects(&self, Parameters(args): Parameters<PageArgs>) -> String {
        self.record("list_projects", &args);
        json!({"projects": [{"id": "P-DEMO-1", "uuid": "proj-1", "name": "Demo Project"}], "hasNextPage": false}).to_string()
    }

    #[rmcp::tool(description = "List labels")]
    async fn list_issue_labels(&self, Parameters(args): Parameters<PageArgs>) -> String {
        self.record("list_issue_labels", &args);
        json!({"labels": [{"id": "l1", "name": "ai-ready"}, {"id": "l2", "name": "Bug"}], "hasNextPage": false}).to_string()
    }

    #[rmcp::tool(description = "List teams")]
    async fn list_teams(&self, Parameters(args): Parameters<PageArgs>) -> String {
        self.record("list_teams", &args);
        json!({"teams": [{"id": "team-1", "name": "Demo"}], "hasNextPage": false}).to_string()
    }

    #[rmcp::tool(description = "List statuses of a team")]
    async fn list_issue_statuses(&self, Parameters(args): Parameters<TeamArgs>) -> String {
        self.record("list_issue_statuses", &args);
        json!([
            {"id": "s1", "type": "unstarted", "name": "Todo"},
            {"id": "s2", "type": "started", "name": "In Progress"},
            {"id": "s3", "type": "started", "name": "In Testing"},
            {"id": "s4", "type": "completed", "name": "Done"}
        ])
        .to_string()
    }
}

/// A server that only offers `list_issues`, to exercise the missing-tools check.
#[derive(Clone, Default)]
pub struct PartialServer;

#[rmcp::tool_router(server_handler)]
impl PartialServer {
    #[rmcp::tool(description = "List issues")]
    async fn list_issues(&self, Parameters(_args): Parameters<ListIssuesArgs>) -> String {
        json!({"issues": [], "hasNextPage": false}).to_string()
    }
}

pub async fn connect_full(server: FakeLinearServer) -> LinearClient {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ClientConfig::default()
        .serve(client_io)
        .await
        .expect("client handshake")
}

pub async fn connect_partial() -> LinearClient {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = PartialServer.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ClientConfig::default()
        .serve(client_io)
        .await
        .expect("client handshake")
}
