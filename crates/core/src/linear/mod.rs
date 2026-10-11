//! Linear access through Linear's official MCP server.
pub mod auth;
pub mod mcp;
pub mod parse;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::model::Issue;

/// The Linear user the connection is authorized as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Viewer {
    pub id: String,
    pub name: String,
}

/// A Linear project. `id` is the project's `uuid`, which is what an issue's `projectId`
/// refers to; Linear's own `P-<TEAM>-N` identifier is not used (M0 finding F1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
}

/// Tools IssueRelay calls; checked with `tools/list` on every connection.
pub const REQUIRED_TOOLS: [&str; 8] = [
    "list_issues",
    "get_issue",
    "save_issue",
    "get_user",
    "list_projects",
    "list_issue_labels",
    "list_teams",
    "list_issue_statuses",
];

/// The `fields` requested from `list_issues` and `get_issue`, so replies carry everything
/// `parse::parse_issue` reads.
pub const ISSUE_FIELDS: [&str; 13] = [
    "id",
    "uuid",
    "title",
    "url",
    "status",
    "labels",
    "project",
    "projectId",
    "assigneeId",
    "parentId",
    "priority",
    "createdAt",
    "updatedAt",
];

/// What the engine needs from Linear. `McpLinear` is the real implementation; tests use the
/// in-memory `FakeLinear`.
#[async_trait::async_trait]
pub trait LinearApi: Send + Sync {
    async fn viewer(&self) -> Result<Viewer>;
    /// Issues assigned to the authorized user with `label` in status `state`, all pages.
    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>>;
    /// `None` when Linear reports the issue does not exist.
    async fn issue(&self, id: &str) -> Result<Option<Issue>>;
    async fn set_state(&self, id: &str, state: &str) -> Result<()>;
    async fn projects(&self) -> Result<Vec<Project>>;
    async fn labels(&self) -> Result<Vec<String>>;
    async fn teams(&self) -> Result<Vec<Team>>;
    async fn statuses(&self, team_id: &str) -> Result<Vec<String>>;
}
