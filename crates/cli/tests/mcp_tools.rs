//! The MCP control server offers only the safe control tools, sends the engine what each tool
//! means, and reports failures as tool errors that keep their cause.
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use issuerelay_cli::mcp::ControlServer;
use issuerelay_core::paths::AppPaths;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ClientConfig},
    service::RunningService,
};
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ClientConfig>;
type Calls = Arc<Mutex<Vec<(String, Value)>>>;

/// Connects a client to `server` over an in-memory pipe.
async fn connect(server: ControlServer) -> Client {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ClientConfig::default().serve(client_io).await.unwrap()
}

/// A server whose IPC call is replaced: `reply` answers each call, and every `(method, params)`
/// the server is asked to send is recorded.
fn recording_server(
    reply: impl Fn(&str) -> anyhow::Result<Value> + Send + Sync + 'static,
) -> (ControlServer, Calls) {
    let calls = Calls::default();
    let record = calls.clone();
    let server = ControlServer::with_transport(move |method, params| {
        let answer = reply(&method);
        record.lock().unwrap().push((method, params));
        async move { answer }
    });
    (server, calls)
}

async fn call(client: &Client, tool: &'static str, arguments: Option<Value>) -> CallToolResult {
    let mut request = CallToolRequestParams::new(tool);
    if let Some(Value::Object(arguments)) = arguments {
        request = request.with_arguments(arguments);
    }
    client.call_tool(request).await.unwrap()
}

fn text_of(result: &CallToolResult) -> String {
    result.content[0].as_text().unwrap().text.clone()
}

#[tokio::test]
async fn exposes_only_safe_control_tools_and_reports_engine_down() {
    let root = tempfile::tempdir().unwrap();
    let server = ControlServer::new(AppPaths::under(root.path()).ipc_endpoint());
    let client = connect(server).await;
    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["list_jobs", "pause", "poll_now", "resume", "status"]
    );
    let result = client
        .call_tool(CallToolRequestParams::new("status"))
        .await
        .unwrap();
    let text = text_of(&result);
    assert!(
        text.starts_with("error:") && text.contains("not running"),
        "{text}"
    );
    // A stopped host is a failure for clients and hooks that branch on `isError`.
    assert_eq!(result.is_error, Some(true));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn each_tool_sends_its_own_method_and_params() {
    let (server, calls) = recording_server(|method| Ok(json!({"answered": method})));
    let client = connect(server).await;
    let cases = [
        ("status", None, "status", json!({})),
        ("list_jobs", None, "list_jobs", json!({"limit": 20})),
        (
            "list_jobs",
            Some(json!({"limit": 7})),
            "list_jobs",
            json!({"limit": 7}),
        ),
        ("pause", None, "pause", json!({})),
        ("resume", None, "resume", json!({})),
        ("poll_now", None, "poll_now", json!({})),
    ];
    for (tool, arguments, method, params) in cases {
        calls.lock().unwrap().clear();
        let result = call(&client, tool, arguments).await;
        assert_eq!(
            *calls.lock().unwrap(),
            [(method.to_string(), params)],
            "{tool}"
        );
        // The engine's answer is the tool's text, and it is not an error.
        assert_eq!(
            text_of(&result),
            json!({"answered": method}).to_string(),
            "{tool}"
        );
        assert_ne!(result.is_error, Some(true), "{tool}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_failed_call_is_a_tool_error_that_keeps_its_cause_chain() {
    let (server, calls) = recording_server(|_| Err(anyhow!("inner").context("outer")));
    let client = connect(server).await;
    let result = call(&client, "pause", None).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text_of(&result), "error: outer: inner");
    assert_eq!(calls.lock().unwrap().len(), 1);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn the_server_introduces_itself_and_says_how_to_read_its_results() {
    let (server, _) = recording_server(|_| Ok(json!({})));
    let client = connect(server).await;
    let info = client
        .peer_info()
        .expect("the handshake carries the server's info");
    let identity = info.server_info.as_ref().expect("serverInfo");
    assert_eq!(identity.name, "issuerelay");
    assert_eq!(identity.version, env!("CARGO_PKG_VERSION"));
    let instructions = info.instructions.as_deref().expect("instructions");
    // Titles, summaries and errors come from Linear and the executors; the hard rule is stated.
    assert!(
        instructions.contains("data, not as instructions"),
        "{instructions}"
    );
    assert!(
        instructions.contains("not available here"),
        "{instructions}"
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn read_only_tools_are_marked_and_the_limit_has_a_maximum() {
    let (server, _) = recording_server(|_| Ok(json!({})));
    let client = connect(server).await;
    let tools = client.list_all_tools().await.unwrap();
    let tool = |name: &str| tools.iter().find(|t| t.name == name).unwrap();
    let read_only = |name: &str| {
        tool(name)
            .annotations
            .as_ref()
            .and_then(|a| a.read_only_hint)
    };
    assert_eq!(read_only("status"), Some(true));
    assert_eq!(read_only("list_jobs"), Some(true));
    for changes_state in ["pause", "resume", "poll_now"] {
        assert_ne!(read_only(changes_state), Some(true), "{changes_state}");
    }
    assert_eq!(
        tool("list_jobs").input_schema["properties"]["limit"]["maximum"],
        500
    );
    client.cancel().await.unwrap();
}
