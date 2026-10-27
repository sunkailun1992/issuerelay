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
    /// The ids of the teams the project belongs to; the workspace check looks only at these
    /// teams once projects are bound.
    #[serde(default)]
    pub team_ids: Vec<String>,
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
pub const ISSUE_FIELDS: [&str; 14] = [
    "id",
    "uuid",
    "title",
    "url",
    "status",
    "statusType",
    "labels",
    "project",
    "projectId",
    "assigneeId",
    "parentId",
    "priority",
    "createdAt",
    "updatedAt",
];

/// The `fields` requested from `get_issue`: `ISSUE_FIELDS` and the issue's `relations`, which only
/// `get_issue` offers (run-improvements design, section 4.1). A non-empty `fields` overrides
/// `includeRelations`, so relations are asked for as a field.
pub const ISSUE_DETAIL_FIELDS: [&str; ISSUE_FIELDS.len() + 1] = with_relations();

const fn with_relations() -> [&'static str; ISSUE_FIELDS.len() + 1] {
    let mut fields = [""; ISSUE_FIELDS.len() + 1];
    let mut i = 0;
    while i < ISSUE_FIELDS.len() {
        fields[i] = ISSUE_FIELDS[i];
        i += 1;
    }
    fields[ISSUE_FIELDS.len()] = "relations";
    fields
}

/// The `fields` requested from `list_projects`.
pub const PROJECT_FIELDS: [&str; 4] = ["id", "uuid", "name", "teams"];

/// Every `fields` value this module requests, per tool. The connect check finds each in the
/// tool's `fields` enum, so a field Linear stopped offering fails at connect time.
pub const REQUIRED_FIELDS: &[(&str, &[&str])] = &[
    ("list_issues", &ISSUE_FIELDS),
    ("get_issue", &ISSUE_DETAIL_FIELDS),
    ("list_projects", &PROJECT_FIELDS),
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
    /// The 50 most recently updated issues assigned to the authorized user with `label`, in any
    /// status: the panel's issue list. Fails if any issue cannot be read.
    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>>;
    /// One issue by identifier or uuid, with the issues that block it (`Issue::blocked_by`).
    /// `None` only when Linear reports that the issue does not exist; a reply that cannot be read
    /// is an error.
    async fn issue(&self, id: &str) -> Result<Option<Issue>>;
    /// Moves the issue to the status named `state`; an error when Linear rejects the change.
    async fn set_state(&self, id: &str, state: &str) -> Result<()>;
    /// All projects, each identified by its uuid and carrying the ids of its teams.
    async fn projects(&self) -> Result<Vec<Project>>;
    /// The names of all issue labels.
    async fn labels(&self) -> Result<Vec<String>>;
    /// All teams.
    async fn teams(&self) -> Result<Vec<Team>>;
    /// The names of the statuses of the team with id `team_id`.
    async fn statuses(&self, team_id: &str) -> Result<Vec<String>>;
}

/// Stands in for Linear while there is no usable connection (not logged in yet, or connecting
/// failed): every call fails with `reason`. The host keeps running and the engine's checks fail
/// visibly, instead of the host refusing to start.
pub struct Disconnected {
    reason: String,
}

impl Disconnected {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    fn fail<T>(&self) -> Result<T> {
        Err(anyhow::anyhow!("{}", self.reason))
    }
}

#[async_trait::async_trait]
impl LinearApi for Disconnected {
    async fn viewer(&self) -> Result<Viewer> {
        self.fail()
    }
    async fn candidate_issues(&self, _label: &str, _state: &str) -> Result<Vec<Issue>> {
        self.fail()
    }
    async fn labeled_issues(&self, _label: &str) -> Result<Vec<Issue>> {
        self.fail()
    }
    async fn issue(&self, _id: &str) -> Result<Option<Issue>> {
        self.fail()
    }
    async fn set_state(&self, _id: &str, _state: &str) -> Result<()> {
        self.fail()
    }
    async fn projects(&self) -> Result<Vec<Project>> {
        self.fail()
    }
    async fn labels(&self) -> Result<Vec<String>> {
        self.fail()
    }
    async fn teams(&self) -> Result<Vec<Team>> {
        self.fail()
    }
    async fn statuses(&self, _team_id: &str) -> Result<Vec<String>> {
        self.fail()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_stand_in_fails_every_call_with_its_reason() {
        let linear = Disconnected::new("not logged in to Linear");
        let errors = [
            linear.viewer().await.err(),
            linear.candidate_issues("ai-ready", "Todo").await.err(),
            linear.labeled_issues("ai-ready").await.err(),
            linear.issue("DEMO-1").await.err(),
            linear.set_state("DEMO-1", "Done").await.err(),
            linear.projects().await.err(),
            linear.labels().await.err(),
            linear.teams().await.err(),
            linear.statuses("team-1").await.err(),
        ];
        for error in errors {
            assert_eq!(
                error.expect("the call failed").to_string(),
                "not logged in to Linear"
            );
        }
    }
}
