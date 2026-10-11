//! LinearApi backed by Linear's MCP server through rmcp.
use anyhow::{Result, anyhow};
use rmcp::{
    RoleClient,
    model::{CallToolRequestParams, ClientConfig},
    service::RunningService,
};
use serde_json::{Value, json};

use super::{ISSUE_FIELDS, LinearApi, Project, REQUIRED_TOOLS, Team, Viewer, parse};
use crate::model::Issue;

/// A connected, initialized rmcp client; the transport and its authorization are the caller's.
pub type LinearClient = RunningService<RoleClient, ClientConfig>;

const MAX_PAGES: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum LinearError {
    #[error("Linear MCP is missing required tools: {0:?}")]
    MissingTools(Vec<String>),
    #[error("Linear tool `{tool}` failed: {message}")]
    ToolFailed { tool: String, message: String },
}

/// `LinearApi` over one rmcp connection to Linear's MCP server.
pub struct McpLinear {
    client: LinearClient,
}

fn is_not_found(error: &anyhow::Error) -> bool {
    // Linear answers an unknown issue with "Could not find referenced Issue." (M0 sample).
    match error.downcast_ref::<LinearError>() {
        Some(LinearError::ToolFailed { message, .. }) => {
            let message = message.to_lowercase();
            message.contains("could not find") || message.contains("not found")
        }
        _ => false,
    }
}

impl McpLinear {
    /// Fails with `LinearError::MissingTools` when Linear no longer offers a tool we rely on.
    pub async fn new(client: LinearClient) -> Result<Self> {
        let offered: Vec<String> = client
            .list_all_tools()
            .await?
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let missing: Vec<String> = REQUIRED_TOOLS
            .iter()
            .filter(|tool| !offered.iter().any(|name| name == *tool))
            .map(|tool| tool.to_string())
            .collect();
        if !missing.is_empty() {
            return Err(LinearError::MissingTools(missing).into());
        }
        Ok(Self { client })
    }

    /// Ends the MCP session and waits for the connection task to stop.
    pub async fn close(self) -> Result<()> {
        self.client.cancel().await?;
        Ok(())
    }

    async fn call(&self, tool: &'static str, args: Value) -> Result<String> {
        let Value::Object(arguments) = args else {
            return Err(anyhow!("tool arguments must be a JSON object"));
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
            .await?;
        let text: String = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect();
        if result.is_error.unwrap_or(false) {
            return Err(LinearError::ToolFailed {
                tool: tool.into(),
                message: text,
            }
            .into());
        }
        Ok(text)
    }

    async fn all_pages<T: Send>(
        &self,
        tool: &'static str,
        mut args: Value,
        parse_page: fn(&str) -> Result<parse::Page<T>>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut last: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = parse_page(&self.call(tool, args.clone()).await?)?;
            let empty = page.items.is_empty();
            items.extend(page.items);
            match follow(empty, page.next_cursor, last.as_deref()) {
                Some(cursor) => {
                    args["cursor"] = json!(cursor);
                    last = Some(cursor);
                }
                None => return Ok(items),
            }
        }
        Err(anyhow!("{tool} returned more than {MAX_PAGES} pages"))
    }
}

/// The cursor to request next. M0 never observed the last page of `list_issues`, so besides a
/// missing cursor, an empty page or a cursor that did not advance also ends the walk.
fn follow(page_was_empty: bool, next: Option<String>, last: Option<&str>) -> Option<String> {
    next.filter(|cursor| !page_was_empty && Some(cursor.as_str()) != last)
}

#[async_trait::async_trait]
impl LinearApi for McpLinear {
    async fn viewer(&self) -> Result<Viewer> {
        parse::parse_viewer(&self.call("get_user", json!({"query": "me"})).await?)
    }

    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        let args = json!({"assignee": "me", "label": label, "state": state, "limit": 50, "fields": ISSUE_FIELDS});
        self.all_pages("list_issues", args, parse::parse_issue_page)
            .await
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        match self
            .call("get_issue", json!({"id": id, "fields": ISSUE_FIELDS}))
            .await
        {
            Ok(text) => parse::parse_single_issue(&text),
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        self.call("save_issue", json!({"id": id, "state": state}))
            .await
            .map(|_| ())
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        self.all_pages(
            "list_projects",
            json!({"limit": 100}),
            parse::parse_projects,
        )
        .await
    }

    async fn labels(&self) -> Result<Vec<String>> {
        self.all_pages(
            "list_issue_labels",
            json!({"limit": 100}),
            parse::parse_labels,
        )
        .await
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        self.all_pages("list_teams", json!({"limit": 100}), parse::parse_teams)
            .await
    }

    async fn statuses(&self, team_id: &str) -> Result<Vec<String>> {
        parse::parse_statuses(
            &self
                .call("list_issue_statuses", json!({"team": team_id}))
                .await?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::follow;

    #[test]
    fn paging_stops_on_empty_pages_and_repeated_cursors() {
        assert_eq!(
            follow(false, Some("c2".into()), None).as_deref(),
            Some("c2")
        );
        assert_eq!(follow(false, None, Some("c1")), None);
        assert_eq!(follow(true, Some("c3".into()), Some("c2")), None);
        assert_eq!(follow(false, Some("c2".into()), Some("c2")), None);
    }
}
