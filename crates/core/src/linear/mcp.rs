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
    ISSUE_DETAIL_FIELDS, ISSUE_FIELDS, LinearApi, PROJECT_FIELDS, Project, REQUIRED_ARGUMENTS,
    REQUIRED_FIELDS, REQUIRED_TOOLS, Team, Viewer, parse,
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
    /// Linear's tools no longer accept arguments IssueRelay sends; holds `tool.argument` pairs,
    /// and `tool.fields: value` for each `fields` value a tool no longer offers.
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

/// What the offered tools' input schemas lack of what IssueRelay sends: first the `tool.argument`
/// pairs of `REQUIRED_ARGUMENTS` that are not listed under `properties`, in that order, then a
/// `tool.fields: value` for each value of `REQUIRED_FIELDS` that is not in the tool's `fields`
/// enum (`properties.fields.items.enum`). A tool that is not offered at all has every one of its
/// arguments reported, and no `fields` values: `MissingTools` already names the tool. A tool whose
/// `fields` argument or enum is gone has every value reported, as nothing says it is offered.
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
    for (tool, fields) in REQUIRED_FIELDS {
        let Some(offered) = tools.iter().find(|t| t.name == *tool) else {
            continue;
        };
        let allowed = offered
            .input_schema
            .get("properties")
            .and_then(|properties| properties.get("fields"))
            .and_then(|argument| argument.get("items"))
            .and_then(|items| items.get("enum"))
            .and_then(Value::as_array);
        for field in *fields {
            if !allowed.is_some_and(|values| values.iter().any(|v| v.as_str() == Some(*field))) {
                missing.push(format!("{tool}.fields: {field}"));
            }
        }
    }
    missing
}

impl McpLinear {
    /// Checks with `tools/list` that Linear still offers the tools IssueRelay calls, accepts
    /// the arguments it sends them and offers the `fields` values it asks for (design section
    /// 7). Fails with `LinearError::MissingTools`, then `LinearError::MissingArguments`. The
    /// listing is bounded by `CALL_TIMEOUT`.
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

    /// Ends the MCP session and waits for the connection task to stop, but for at most
    /// `CALL_TIMEOUT`: on a half-open connection the transport may never finish closing, and a
    /// shutdown or a reconnect must not hang on it. Fails with "did not close the connection"
    /// when the time is up; the connection task is then left behind.
    pub async fn close(self) -> Result<()> {
        tokio::time::timeout(CALL_TIMEOUT, self.client.cancel())
            .await
            .map_err(|_| anyhow!("Linear did not close the connection within {CALL_TIMEOUT:?}"))?
            .context("closing the connection to Linear")?;
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
            items.extend(page.items);
            match follow(page.next_cursor, last.as_deref())
                .with_context(|| format!("reading `{tool}`"))?
            {
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

/// The cursor to request next, or `None` after the last page (Linear's last page has
/// `hasNextPage: false` and no cursor; M1 acceptance 5b.5). A page may be empty and still have
/// a next one. A cursor that repeats the one just used would loop forever, so it is an error.
fn follow(next: Option<String>, last: Option<&str>) -> Result<Option<String>> {
    match next {
        Some(cursor) if Some(cursor.as_str()) == last => {
            Err(anyhow!("Linear repeated the page cursor {cursor:?}"))
        }
        next => Ok(next),
    }
}

#[async_trait::async_trait]
impl LinearApi for McpLinear {
    async fn viewer(&self) -> Result<Viewer> {
        parse::parse_viewer(&self.call("get_user", json!({"query": "me"})).await?)
    }

    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        // Pages in creation order. Linear's default order, `updatedAt`, moves an issue edited
        // during the walk to another page, where it can be missed; the engine would then cancel
        // its queued job as no longer matching the trigger.
        let args = json!({"assignee": "me", "label": label, "state": state, "limit": 50, "orderBy": "createdAt", "fields": ISSUE_FIELDS});
        self.all_pages("list_issues", args, parse::parse_issue_page)
            .await
    }

    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>> {
        // One page, the most recently updated first: the panel lists recent work, not every
        // issue that ever carried the label.
        let args = json!({"assignee": "me", "label": label, "limit": 50, "orderBy": "updatedAt", "fields": ISSUE_FIELDS});
        Ok(parse::parse_issue_page(&self.call("list_issues", args).await?)?.items)
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        match self
            .call(
                "get_issue",
                json!({"id": id, "fields": ISSUE_DETAIL_FIELDS}),
            )
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
        // `fields` names `uuid` explicitly (issues refer to a project by it) and `teams` (the
        // workspace check looks only at the teams of bound projects).
        self.all_pages(
            "list_projects",
            json!({"limit": 50, "fields": PROJECT_FIELDS}),
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
    fn paging_follows_every_new_cursor_and_rejects_a_repeated_one() {
        assert_eq!(
            follow(Some("c2".into()), None).unwrap().as_deref(),
            Some("c2")
        );
        assert_eq!(follow(None, Some("c1")).unwrap(), None);
        assert_eq!(
            follow(Some("c3".into()), Some("c2")).unwrap().as_deref(),
            Some("c3"),
            "an empty page with a new cursor is followed: later pages may still hold issues"
        );
        let error = follow(Some("c2".into()), Some("c2")).unwrap_err();
        assert!(error.to_string().contains("repeated"), "{error}");
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
        assert!(properties.remove("orderBy").is_some());
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            ["list_issues.orderBy"]
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
        // No argument is accepted, so no `fields` value is either.
        let mut expected = vec!["get_issue.id".to_string(), "get_issue.fields".to_string()];
        expected.extend(every_field_of("get_issue"));
        assert_eq!(missing_arguments(&read_as_tools(tools)), expected);
    }

    #[test]
    fn arguments_of_a_tool_that_is_gone_count_as_missing() {
        let mut tools = m0_tools_json();
        tools.retain(|t| t["name"] != "get_user");
        assert_eq!(missing_arguments(&read_as_tools(tools)), ["get_user.query"]);
    }

    /// What `missing_arguments` says when a tool offers none of the `fields` values it must.
    fn every_field_of(tool: &str) -> Vec<String> {
        let (_, fields) = REQUIRED_FIELDS
            .iter()
            .find(|(name, _)| *name == tool)
            .expect("a tool with required fields");
        fields
            .iter()
            .map(|field| format!("{tool}.fields: {field}"))
            .collect()
    }

    #[test]
    fn a_missing_fields_value_fails_the_connect_check() {
        let mut tools = m0_tools_json();
        let projects = tools
            .iter_mut()
            .find(|t| t["name"] == "list_projects")
            .unwrap();
        let fields = projects["inputSchema"]["properties"]["fields"]["items"]["enum"]
            .as_array_mut()
            .unwrap();
        fields.retain(|f| f != "teams");
        let missing = missing_arguments(&read_as_tools(tools));
        assert_eq!(missing, ["list_projects.fields: teams"]);
        assert!(
            missing_arguments(&m0_tools()).is_empty(),
            "the real M0 list has every field"
        );
    }

    #[test]
    fn missing_fields_values_follow_the_argument_names_in_list_order() {
        let mut tools = m0_tools_json();
        for (tool, value) in [
            ("list_projects", "teams"),
            ("list_issues", "uuid"),
            ("get_issue", "projectId"),
            ("get_issue", "url"),
        ] {
            let t = tools.iter_mut().find(|t| t["name"] == tool).unwrap();
            let offered = t["inputSchema"]["properties"]["fields"]["items"]["enum"]
                .as_array_mut()
                .unwrap();
            offered.retain(|f| f != value);
        }
        let t = tools
            .iter_mut()
            .find(|t| t["name"] == "save_issue")
            .unwrap();
        let properties = t["inputSchema"]["properties"].as_object_mut().unwrap();
        assert!(properties.remove("state").is_some());
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            [
                "save_issue.state",
                "list_issues.fields: uuid",
                "get_issue.fields: url",
                "get_issue.fields: projectId",
                "list_projects.fields: teams",
            ]
        );
    }

    #[test]
    fn a_tool_that_offers_no_fields_list_has_every_field_reported() {
        // The `fields` argument is gone altogether: the argument is reported, and with it every
        // value, because nothing is left to ask for them with.
        let mut tools = m0_tools_json();
        let t = tools
            .iter_mut()
            .find(|t| t["name"] == "list_projects")
            .unwrap();
        let properties = t["inputSchema"]["properties"].as_object_mut().unwrap();
        assert!(properties.remove("fields").is_some());
        let mut expected = vec!["list_projects.fields".to_string()];
        expected.extend(every_field_of("list_projects"));
        assert_eq!(missing_arguments(&read_as_tools(tools)), expected);
        // The argument is there but names no values (free text, or an enum Linear dropped):
        // nothing says the fields are offered, so none is taken for granted.
        let mut tools = m0_tools_json();
        let t = tools.iter_mut().find(|t| t["name"] == "get_issue").unwrap();
        let items = t["inputSchema"]["properties"]["fields"]["items"]
            .as_object_mut()
            .unwrap();
        assert!(items.remove("enum").is_some());
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            every_field_of("get_issue")
        );
    }

    #[test]
    fn a_tool_that_is_gone_is_reported_by_its_arguments_only() {
        // `MissingTools` names the tool; its fields are not listed value by value as well.
        let mut tools = m0_tools_json();
        tools.retain(|t| t["name"] != "list_projects");
        assert_eq!(
            missing_arguments(&read_as_tools(tools)),
            [
                "list_projects.limit",
                "list_projects.fields",
                "list_projects.cursor"
            ]
        );
    }

    #[test]
    fn every_tool_sent_fields_is_checked_for_them() {
        for (tool, fields) in REQUIRED_FIELDS {
            assert!(
                REQUIRED_TOOLS.contains(tool),
                "{tool} is not a required tool"
            );
            assert!(!fields.is_empty(), "{tool} lists no fields");
            let (_, arguments) = REQUIRED_ARGUMENTS
                .iter()
                .find(|(name, _)| name == tool)
                .unwrap_or_else(|| panic!("{tool} has no entry in REQUIRED_ARGUMENTS"));
            assert!(
                arguments.contains(&"fields"),
                "{tool} has required fields but `fields` is not one of its arguments"
            );
        }
        for (tool, arguments) in REQUIRED_ARGUMENTS {
            if arguments.contains(&"fields") {
                assert!(
                    REQUIRED_FIELDS.iter().any(|(name, _)| name == tool),
                    "{tool} is sent `fields` but their values are not checked at connect time"
                );
            }
        }
    }

    #[test]
    fn only_get_issue_asks_for_relations() {
        let fields_of = |tool: &str| {
            REQUIRED_FIELDS
                .iter()
                .find(|(name, _)| *name == tool)
                .map(|(_, fields)| *fields)
                .unwrap()
        };
        assert!(fields_of("get_issue").contains(&"relations"));
        assert!(!fields_of("list_issues").contains(&"relations"));
        // Everything else a list reply carries, a single issue carries too.
        for field in ISSUE_FIELDS {
            assert!(fields_of("get_issue").contains(&field), "{field}");
        }
        assert!(ISSUE_FIELDS.contains(&"statusType"));
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
