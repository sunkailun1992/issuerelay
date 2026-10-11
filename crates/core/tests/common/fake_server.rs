//! In-process MCP server that mimics Linear's tools, connected over an in-memory duplex. It
//! checks every call against Linear's own input schemas (recorded in M0), returns only the
//! `fields` a call asks for, and can be told to fail, answer garbage or never answer.
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use issuerelay_core::linear::mcp::LinearClient;
use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{
        CallToolRequestParams, CallToolResponse, ClientConfig, ListToolsResult,
        PaginatedRequestParams, ServerConfig,
    },
    service::{RequestContext, RxJsonRpcMessage, TxJsonRpcMessage},
    transport::{Transport, async_rw::AsyncRwTransport},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Linear's `tools/list` reply as recorded in M0 (a copy of
/// docs/gates/samples/m0/linear/tools.json).
const LINEAR_TOOLS: &str = include_str!("../fixtures/linear-tools.json");

fn linear_tools() -> &'static [Value] {
    static TOOLS: OnceLock<Vec<Value>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        serde_json::from_str(LINEAR_TOOLS).expect("linear-tools.json is a JSON array of tools")
    })
}

/// Checks `args` the way Linear's input schema for `tool` does: every non-null argument must be
/// a listed property (the schemas say `additionalProperties: false`), a number may not exceed
/// its `maximum`, array items must be among `items.enum` where the schema lists them, and every
/// `required` argument must be present. The error reads like the tool error Linear sends:
/// `invalid arguments for {tool}: …`.
pub fn validate_arguments(tool: &str, args: &Value) -> Result<(), String> {
    let invalid = |why: String| format!("invalid arguments for {tool}: {why}");
    let schema = linear_tools()
        .iter()
        .find(|t| t["name"] == tool)
        .map(|t| &t["inputSchema"])
        .ok_or_else(|| invalid("no such tool".into()))?;
    let properties = schema["properties"]
        .as_object()
        .ok_or_else(|| invalid("the tool lists no arguments".into()))?;
    let args = args
        .as_object()
        .ok_or_else(|| invalid("arguments must be an object".into()))?;
    for name in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if args.get(name).is_none_or(Value::is_null) {
            return Err(invalid(format!("`{name}` is required")));
        }
    }
    for (name, value) in args.iter().filter(|(_, value)| !value.is_null()) {
        let property = properties
            .get(name)
            .ok_or_else(|| invalid(format!("`{name}` is not an argument of this tool")))?;
        if let Some(maximum) = property["maximum"].as_f64()
            && let Some(number) = value.as_f64()
            && number > maximum
        {
            return Err(invalid(format!(
                "`{name}` is {number}, at most {maximum} is allowed"
            )));
        }
        if let Some(allowed) = property["items"]["enum"].as_array()
            && let Some(items) = value.as_array()
            && let Some(bad) = items.iter().find(|item| !allowed.contains(item))
        {
            return Err(invalid(format!("`{name}` has {bad}, which is not allowed")));
        }
    }
    Ok(())
}

/// What a programmed tool answers instead of its normal reply.
#[derive(Debug, Clone)]
pub enum Reply {
    /// A successful call with exactly this text, whether it is JSON or not.
    Text(String),
    /// A tool error (`isError`) with this text.
    ToolError(String),
    /// Never answers.
    Hang,
}

#[derive(Clone, Default)]
pub struct FakeLinearServer {
    /// Every call that reached a tool, in order: the tool name and its arguments as the typed
    /// struct serializes them (arguments the caller left out are `null`).
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    replies: Arc<Mutex<HashMap<String, Reply>>>,
    endless_pages: Arc<AtomicBool>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListIssuesArgs {
    pub assignee: Option<String>,
    pub label: Option<String>,
    pub state: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetIssueArgs {
    pub id: String,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveIssueArgs {
    pub id: String,
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetUserArgs {
    pub query: String,
}

/// The paging arguments of the list tools. Linear accepts `fields` only on `list_projects`;
/// `validate_arguments` enforces that per tool.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageArgs {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TeamArgs {
    pub team: String,
}

/// Fake issue `n` (1 or 2). Issue 2 is a sub-issue of issue 1, so the two together give every
/// field of `ISSUE_FIELDS` a value that tests can tell from an empty one.
pub fn issue_json(n: u32) -> Value {
    let parent = (n == 2).then_some("DEMO-1");
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
        "parentId": parent,
        "priority": {"value": 2, "name": "High"},
        "createdAt": "2026-10-01T08:00:00.000Z",
        "updatedAt": "2026-10-01T09:00:00.000Z"
    })
}

/// What Linear returns for a `fields` list: `id` always, plus each requested field, `null` when
/// the object has no value for it. Without `fields` the whole object comes back.
fn pick(full: &Value, fields: Option<&[String]>) -> Value {
    let Some(fields) = fields else {
        return full.clone();
    };
    let mut picked = Map::new();
    picked.insert("id".into(), full["id"].clone());
    for field in fields {
        let value = full.get(field).cloned().unwrap_or(Value::Null);
        picked.insert(field.clone(), value);
    }
    Value::Object(picked)
}

impl FakeLinearServer {
    /// From now on `tool` answers every call with `reply` instead of its normal reply. The
    /// arguments are still validated and the call is still recorded.
    pub fn reply(&self, tool: &str, reply: Reply) {
        self.replies.lock().unwrap().insert(tool.into(), reply);
    }

    /// Makes `list_issues` hand out another page with a new cursor on every call.
    pub fn endless_issue_pages(&self) {
        self.endless_pages.store(true, Ordering::SeqCst);
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

    /// Records the call, rejects arguments Linear's schema would reject, then gives the
    /// programmed reply if there is one, else `normal`.
    async fn answer(
        &self,
        tool: &str,
        args: &impl Serialize,
        normal: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        let args = serde_json::to_value(args).unwrap();
        self.calls.lock().unwrap().push((tool.into(), args.clone()));
        validate_arguments(tool, &args)?;
        let programmed = self.replies.lock().unwrap().get(tool).cloned();
        match programmed {
            Some(Reply::Text(text)) => Ok(text),
            Some(Reply::ToolError(message)) => Err(message),
            Some(Reply::Hang) => std::future::pending().await,
            None => normal(),
        }
    }
}

#[rmcp::tool_router(server_handler)]
impl FakeLinearServer {
    #[rmcp::tool(description = "List issues")]
    async fn list_issues(
        &self,
        Parameters(args): Parameters<ListIssuesArgs>,
    ) -> Result<String, String> {
        self.answer("list_issues", &args, || {
            let fields = args.fields.as_deref();
            let page = if self.endless_pages.load(Ordering::SeqCst) {
                let n = self.calls_to("list_issues").len();
                json!({"issues": [pick(&issue_json(1), fields)], "hasNextPage": true, "cursor": format!("n{n}")})
            } else if args.cursor.as_deref() == Some("c2") {
                json!({"issues": [pick(&issue_json(2), fields)], "hasNextPage": false})
            } else {
                json!({"issues": [pick(&issue_json(1), fields)], "hasNextPage": true, "cursor": "c2"})
            };
            Ok(page.to_string())
        })
        .await
    }

    #[rmcp::tool(description = "Get one issue")]
    async fn get_issue(
        &self,
        Parameters(args): Parameters<GetIssueArgs>,
    ) -> Result<String, String> {
        self.answer("get_issue", &args, || {
            // An issue is found by its identifier or by its uuid (which ends in `000n`).
            let known = [1, 2]
                .into_iter()
                .find(|n| args.id == format!("DEMO-{n}") || args.id.ends_with(&format!("{n:04}")));
            match known {
                Some(n) => Ok(pick(&issue_json(n), args.fields.as_deref()).to_string()),
                // Linear's real reply for an unknown issue (M0 sample get_issue_missing.json).
                None => Err(r#"{"error":"invalid_request","message":"Could not find referenced Issue.","status":400}"#.to_string()),
            }
        })
        .await
    }

    #[rmcp::tool(description = "Create or update an issue")]
    async fn save_issue(
        &self,
        Parameters(args): Parameters<SaveIssueArgs>,
    ) -> Result<String, String> {
        self.answer("save_issue", &args, || {
            Ok(json!({"id": args.id, "status": args.state}).to_string())
        })
        .await
    }

    #[rmcp::tool(description = "Get a user")]
    async fn get_user(&self, Parameters(args): Parameters<GetUserArgs>) -> Result<String, String> {
        self.answer("get_user", &args, || {
            Ok(
                json!({"id": "user-1", "name": "Jane Doe", "email": "jane@example.com"})
                    .to_string(),
            )
        })
        .await
    }

    #[rmcp::tool(description = "List projects")]
    async fn list_projects(
        &self,
        Parameters(args): Parameters<PageArgs>,
    ) -> Result<String, String> {
        self.answer("list_projects", &args, || {
            let project = json!({"id": "P-DEMO-1", "uuid": "proj-1", "name": "Demo Project"});
            Ok(
                json!({"projects": [pick(&project, args.fields.as_deref())], "hasNextPage": false})
                    .to_string(),
            )
        })
        .await
    }

    #[rmcp::tool(description = "List labels")]
    async fn list_issue_labels(
        &self,
        Parameters(args): Parameters<PageArgs>,
    ) -> Result<String, String> {
        self.answer("list_issue_labels", &args, || {
            Ok(json!({"labels": [{"id": "l1", "name": "ai-ready"}, {"id": "l2", "name": "Bug"}], "hasNextPage": false}).to_string())
        })
        .await
    }

    #[rmcp::tool(description = "List teams")]
    async fn list_teams(&self, Parameters(args): Parameters<PageArgs>) -> Result<String, String> {
        self.answer("list_teams", &args, || {
            Ok(
                json!({"teams": [{"id": "team-1", "name": "Demo"}], "hasNextPage": false})
                    .to_string(),
            )
        })
        .await
    }

    #[rmcp::tool(description = "List statuses of a team")]
    async fn list_issue_statuses(
        &self,
        Parameters(args): Parameters<TeamArgs>,
    ) -> Result<String, String> {
        self.answer("list_issue_statuses", &args, || {
            Ok(json!([
                {"id": "s1", "type": "unstarted", "name": "Todo"},
                {"id": "s2", "type": "started", "name": "In Progress"},
                {"id": "s3", "type": "started", "name": "In Testing"},
                {"id": "s4", "type": "completed", "name": "Done"}
            ])
            .to_string())
        })
        .await
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

/// How a `DriftedServer` differs from what IssueRelay expects of Linear.
#[derive(Clone)]
enum Drift {
    /// `tool` no longer lists `argument` in its input schema.
    DroppedArgument { tool: String, argument: String },
    /// `tools/list` never answers.
    HangingToolList,
}

/// The full fake, except for what `drift` changes about `tools/list`.
#[derive(Clone)]
struct DriftedServer {
    inner: FakeLinearServer,
    drift: Drift,
}

impl ServerHandler for DriftedServer {
    fn get_info(&self) -> ServerConfig {
        self.inner.get_info()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let (tool, argument) = match &self.drift {
            Drift::HangingToolList => return std::future::pending().await,
            Drift::DroppedArgument { tool, argument } => (tool, argument),
        };
        let mut listed = self.inner.list_tools(request, context).await?;
        for offered in listed.tools.iter_mut().filter(|t| t.name == tool.as_str()) {
            let mut schema = (*offered.input_schema).clone();
            if let Some(Value::Object(properties)) = schema.get_mut("properties") {
                properties.remove(argument);
            }
            offered.input_schema = Arc::new(schema);
        }
        Ok(listed)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.inner.call_tool(request, context).await
    }
}

/// Starts `server` on one end of an in-memory pipe and returns the other end.
fn serve_in_memory<S: ServerHandler>(server: S) -> tokio::io::DuplexStream {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    client_io
}

async fn connect<S: ServerHandler>(server: S) -> LinearClient {
    ClientConfig::default()
        .serve(serve_in_memory(server))
        .await
        .expect("client handshake")
}

/// A client transport whose `close` never finishes, like a half-open connection that never
/// acknowledges the shutdown. Everything else is passed on to the transport it wraps.
struct HangingClose<T>(T);

impl<T: Transport<RoleClient>> Transport<RoleClient> for HangingClose<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.0.send(item)
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.0.receive()
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        std::future::pending().await
    }
}

pub async fn connect_full(server: FakeLinearServer) -> LinearClient {
    connect(server).await
}

pub async fn connect_partial() -> LinearClient {
    connect(PartialServer).await
}

/// Linear after it dropped `argument` from `tool`: the other tools and arguments are all there.
pub async fn connect_without_argument(tool: &str, argument: &str) -> LinearClient {
    connect(DriftedServer {
        inner: FakeLinearServer::default(),
        drift: Drift::DroppedArgument {
            tool: tool.into(),
            argument: argument.into(),
        },
    })
    .await
}

/// A Linear that accepts the connection but never answers `tools/list`.
pub async fn connect_with_hanging_tool_list() -> LinearClient {
    connect(DriftedServer {
        inner: FakeLinearServer::default(),
        drift: Drift::HangingToolList,
    })
    .await
}

/// A Linear whose connection never finishes closing: everything works until `close`.
pub async fn connect_with_hanging_close() -> LinearClient {
    let (read, write) = tokio::io::split(serve_in_memory(FakeLinearServer::default()));
    ClientConfig::default()
        .serve(HangingClose(AsyncRwTransport::new_client(read, write)))
        .await
        .expect("client handshake")
}
