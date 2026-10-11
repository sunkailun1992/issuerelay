//! LinearApi backed by Linear's MCP server through rmcp.
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rmcp::{
    RoleClient,
    model::{CallToolRequestParams, ClientConfig, Tool},
    service::RunningService,
};
use serde_json::{Value, json};

use super::{
    ISSUE_FIELDS, LinearApi, Project, REQUIRED_ARGUMENTS, REQUIRED_TOOLS, Team, Viewer, parse,
};
use crate::model::Issue;

/// A connected, initialized rmcp client; the transport and its authorization are the caller's.
pub type LinearClient = RunningService<RoleClient, ClientConfig>;

const MAX_PAGES: usize = 20;

/// How long one call to Linear (and the `tools/list` at connect time) may take. A half-open
/// connection after sleep or a network switch never answers; without a limit that would freeze
/// polling or hold a run's slot for good.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// What `McpLinear` reports that callers may want to tell apart. It travels inside an
/// `anyhow::Error`: inspect it with `downcast_ref::<LinearError>()`.
#[derive(Debug, thiserror::Error)]
pub enum LinearError {
    /// Linear no longer offers tools IssueRelay calls; holds their names.
    #[error("Linear MCP is missing required tools: {0:?}")]
    MissingTools(Vec<String>),
    /// Linear's tools no longer accept arguments IssueRelay sends; holds `tool.argument` pairs.
    #[error("Linear MCP no longer accepts arguments IssueRelay sends: {0:?}")]
    MissingArguments(Vec<String>),
    /// The tool ran and answered with an error (`isError`); `message` is Linear's text.
    #[error("Linear tool `{tool}` failed: {message}")]
    ToolFailed { tool: String, message: String },
}

/// `LinearApi` over one rmcp connection to Linear's MCP server. Every call is bounded by a
/// timeout.
pub struct McpLinear {
    client: LinearClient,
    call_timeout: Duration,
}

/// True only for Linear's answer to an unknown issue, "Could not find referenced Issue." (M0
/// sample get_issue_missing.json). Any other failure stays an error, even one that says "not
/// found" about something else: reading it as "the issue is gone" would cancel queued work.
fn is_not_found(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<LinearError>() {
        Some(LinearError::ToolFailed { message, .. }) => message
            .to_lowercase()
            .contains("could not find referenced issue"),
        _ => false,
    }
}

/// The `tool.argument` pairs of `REQUIRED_ARGUMENTS` that the offered tools' input schemas do
/// not list under `properties`, in `REQUIRED_ARGUMENTS` order. A tool that is not offered at
/// all has every one of its arguments reported.
fn missing_arguments(tools: &[Tool]) -> Vec<String> {
    let mut missing = Vec::new();
    for (tool, arguments) in REQUIRED_ARGUMENTS {
        let properties = tools
            .iter()
            .find(|t| t.name == *tool)
            .and_then(|t| t.input_schema.get("properties"))
            .and_then(Value::as_object);
        for argument in *arguments {
            if !properties.is_some_and(|p| p.contains_key(*argument)) {
                missing.push(format!("{tool}.{argument}"));
            }
        }
    }
    missing
}

impl McpLinear {
    /// Checks with `tools/list` that Linear still offers the tools IssueRelay calls and accepts
    /// the arguments it sends them (design section 7). Fails with `LinearError::MissingTools`,
    /// then `LinearError::MissingArguments`. The listing is bounded by `CALL_TIMEOUT`.
    pub async fn new(client: LinearClient) -> Result<Self> {
        let tools = tokio::time::timeout(CALL_TIMEOUT, client.list_all_tools())
            .await
            .map_err(|_| anyhow!("Linear did not answer `tools/list` within {CALL_TIMEOUT:?}"))?
            .context("listing Linear's tools")?;
        let missing: Vec<String> = REQUIRED_TOOLS
            .iter()
            .filter(|tool| !tools.iter().any(|t| t.name == **tool))
            .map(|tool| tool.to_string())
            .collect();
        if !missing.is_empty() {
            return Err(LinearError::MissingTools(missing).into());
        }
        let missing = missing_arguments(&tools);
        if !missing.is_empty() {
            return Err(LinearError::MissingArguments(missing).into());
        }
        Ok(Self {
            client,
            call_timeout: CALL_TIMEOUT,
        })
    }

    /// Replaces the limit on one call (30 seconds by default).
    pub fn with_call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
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
        let request = CallToolRequestParams::new(tool).with_arguments(arguments);
        let result = tokio::time::timeout(self.call_timeout, self.client.call_tool(request))
            .await
            .map_err(|_| {
                anyhow!(
                    "Linear did not answer `{tool}` within {:?}",
                    self.call_timeout
                )
            })?
            .with_context(|| format!("calling Linear tool `{tool}`"))?;
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
            Ok(text) => parse::parse_single_issue(&text).map(Some),
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
        // 50 is the most `list_projects` accepts (M0 tools/list; the other lists allow 250).
        // `fields` names `uuid` explicitly: issues refer to a project by it.
        self.all_pages(
            "list_projects",
            json!({"limit": 50, "fields": ["id", "uuid", "name"]}),
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
    use rmcp::model::Tool;
    use serde_json::Value;

    use super::*;

    /// Linear's real `tools/list` reply, recorded in M0.
    const M0_TOOLS: &str = include_str!("../../tests/fixtures/linear-tools.json");

    fn m0_tools() -> Vec<Tool> {
        serde_json::from_str(M0_TOOLS).expect("the M0 tool list reads as rmcp tools")
    }

    /// The M0 tool list as JSON, to be altered before it is read as tools.
    fn m0_tools_json() -> Vec<Value> {
        serde_json::from_str(M0_TOOLS).unwrap()
    }

    fn read_as_tools(tools: Vec<Value>) -> Vec<Tool> {
        serde_json::from_value(Value::Array(tools)).unwrap()
    }

    fn tool_failed(message: &str) -> anyhow::Error {
        LinearError::ToolFailed {
            tool: "get_issue".into(),
            message: message.into(),
        }
        .into()
    }

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

    #[test]
    fn only_linears_missing_issue_error_means_not_found() {
        let missing =
            include_str!("../../../../docs/gates/samples/m0/linear/get_issue_missing.json");
        assert!(is_not_found(&tool_failed(missing)));
        assert!(is_not_found(&tool_failed(
            "COULD NOT FIND REFERENCED ISSUE."
        )));
        // Other failures, even ones that mention "not found", are real errors.
        assert!(!is_not_found(&tool_failed("Team not found")));
        assert!(!is_not_found(&tool_failed("Rate limit exceeded")));
        assert!(!is_not_found(&tool_failed(
            "Could not find referenced Team."
        )));
        // Only a tool error can mean "not found"; a transport error with the same words cannot.
        assert!(!is_not_found(&anyhow!("Could not find referenced Issue.")));
    }

    #[test]
    fn the_m0_tool_list_offers_every_tool_and_argument_we_use() {
        let tools = m0_tools();
        for name in REQUIRED_TOOLS {
            assert!(tools.iter().any(|t| t.name == name), "{name}");
        }
        assert_eq!(missing_arguments(&tools), Vec::<String>::new());
    }

    #[test]
    fn a_dropped_argument_is_reported_as_tool_dot_argument() {
        let mut tools = m0_tools_json();
        let list_issues = tools
            .iter_mut()
            .find(|t| t["name"] == "list_issues")
            .unwrap();
        let properties = list_issues["inputSchema"]["properties"]
            .as_object_mut()
            .unwrap();
        assert!(properties.remove("fields").is_some());
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            ["list_issues.fields"]
        );
    }

    #[test]
    fn every_dropped_argument_is_reported_in_list_order() {
        let mut tools = m0_tools_json();
        for (tool, argument) in [("save_issue", "state"), ("get_user", "query")] {
            let t = tools.iter_mut().find(|t| t["name"] == tool).unwrap();
            let properties = t["inputSchema"]["properties"].as_object_mut().unwrap();
            assert!(properties.remove(argument).is_some());
        }
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            ["save_issue.state", "get_user.query"]
        );
    }

    #[test]
    fn a_tool_without_a_property_list_has_no_accepted_arguments() {
        let mut tools = m0_tools_json();
        let t = tools.iter_mut().find(|t| t["name"] == "get_issue").unwrap();
        t["inputSchema"]
            .as_object_mut()
            .unwrap()
            .remove("properties");
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            ["get_issue.id", "get_issue.fields"]
        );
    }

    #[test]
    fn arguments_of_a_tool_that_is_gone_count_as_missing() {
        let mut tools = m0_tools_json();
        tools.retain(|t| t["name"] != "get_user");
        assert_eq!(missing_arguments(&read_as_tools(tools)), ["get_user.query"]);
    }

    #[test]
    fn required_tools_and_required_arguments_list_the_same_tools() {
        for name in REQUIRED_TOOLS {
            assert!(
                REQUIRED_ARGUMENTS.iter().any(|(tool, _)| *tool == name),
                "{name} has no entry in REQUIRED_ARGUMENTS"
            );
        }
        for (tool, arguments) in REQUIRED_ARGUMENTS {
            assert!(
                REQUIRED_TOOLS.contains(tool),
                "{tool} is not a required tool"
            );
            assert!(!arguments.is_empty(), "{tool} lists no arguments");
        }
    }

    #[test]
    fn connect_errors_name_what_is_missing() {
        assert_eq!(
            LinearError::MissingTools(vec!["get_issue".into()]).to_string(),
            r#"Linear MCP is missing required tools: ["get_issue"]"#
        );
        assert_eq!(
            LinearError::MissingArguments(vec!["list_issues.fields".into()]).to_string(),
            r#"Linear MCP no longer accepts arguments IssueRelay sends: ["list_issues.fields"]"#
        );
    }
}
