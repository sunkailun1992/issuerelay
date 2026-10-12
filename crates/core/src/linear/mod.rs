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

/// A Linear team. `id` is what `LinearApi::statuses` takes.
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

/// Every argument name this module sends, per tool. The same `tools/list` check that covers
/// `REQUIRED_TOOLS` also looks for these in each tool's input schema, so an argument Linear
/// renamed or dropped fails at connect time (design section 7) instead of as a rejected call
/// in the middle of polling. Keep it in step with the calls in `mcp.rs`.
pub const REQUIRED_ARGUMENTS: &[(&str, &[&str])] = &[
    (
        "list_issues",
        &[
            "assignee", "label", "state", "limit", "orderBy", "fields", "cursor",
        ],
    ),
    ("get_issue", &["id", "fields"]),
    ("save_issue", &["id", "state"]),
    ("get_user", &["query"]),
    ("list_projects", &["limit", "fields", "cursor"]),
    ("list_issue_labels", &["limit", "cursor"]),
    ("list_teams", &["limit", "cursor"]),
    ("list_issue_statuses", &["team"]),
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
/// in-memory `FakeLinear`. A reply that cannot be read is always an error, never an empty
/// result, so the engine cannot mistake a failure for "no candidates" or "the issue is gone".
#[async_trait::async_trait]
pub trait LinearApi: Send + Sync {
    /// The user the connection is authorized as; its `id` is the `me` of the trigger rules.
    async fn viewer(&self) -> Result<Viewer>;
    /// Issues assigned to the authorized user with `label` in status `state`, all pages.
    /// Fails if any issue cannot be read, so a poll never silently leaves one out.
    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>>;
    /// One issue by identifier or uuid. `None` only when Linear reports that the issue does
    /// not exist; a reply that cannot be read is an error.
    async fn issue(&self, id: &str) -> Result<Option<Issue>>;
    /// Moves the issue to the status named `state`; an error when Linear rejects the change.
    async fn set_state(&self, id: &str, state: &str) -> Result<()>;
    /// All projects, each identified by its uuid.
    async fn projects(&self) -> Result<Vec<Project>>;
    /// The names of all issue labels.
    async fn labels(&self) -> Result<Vec<String>>;
    /// All teams.
    async fn teams(&self) -> Result<Vec<Team>>;
    /// The names of the statuses of the team with id `team_id`.
    async fn statuses(&self, team_id: &str) -> Result<Vec<String>>;
}
