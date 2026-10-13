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

#[tokio::test]
async fn control_tools_say_they_change_state_harmlessly_and_repeatably() {
    let (server, _) = recording_server(|_| Ok(json!({})));
    let client = connect(server).await;
    let tools = client.list_all_tools().await.unwrap();
    for name in ["pause", "resume", "poll_now"] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        let annotations = tool.annotations.as_ref().expect("annotated");
        assert_eq!(annotations.read_only_hint, Some(false), "{name}");
        assert_eq!(annotations.destructive_hint, Some(false), "{name}");
        assert_eq!(annotations.idempotent_hint, Some(true), "{name}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn tool_output_carries_no_invisible_characters() {
    // serde_json escapes U+0000..=U+001F inside strings. A direction override, DEL and the C1
    // controls pass raw, and U+009B is a one-byte CSI that a terminal-based client could read as
    // an escape sequence.
    let hostile = "boom\u{202e}gnp.exe\u{9b}31m\u{7f}!";
    let (server, _) = recording_server(move |_| Ok(json!({"last_error": hostile, "running": 1})));
    let client = connect(server).await;
    let reply = text_of(&call(&client, "status", None).await);
    for hidden in ['\u{202e}', '\u{9b}', '\u{7f}'] {
        assert!(
            !reply.contains(hidden),
            "U+{:04X} in {reply:?}",
            hidden as u32
        );
    }
    assert!(!reply.chars().any(char::is_control), "{reply:?}");
    // Still JSON with the keys it came with: only text inside a string changed, and nothing was
    // cut.
    let parsed: Value = serde_json::from_str(&reply).expect("the reply stays JSON");
    assert_eq!(
        parsed,
        json!({"last_error": "boomgnp.exe 31m !", "running": 1})
    );
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn every_tool_drops_what_shows_nothing_and_keeps_the_joiners() {
    // A zero-width space, a direction override, a line separator, the byte order mark, a tag
    // character (outside the basic plane) and the soft hyphen.
    let hostile = "a\u{200b}b\u{202e}c\u{2028}d\u{feff}e\u{e0041}f\u{00ad}g";
    // The zero-width joiner is part of how an emoji sequence reads, and it cannot reorder text.
    let family = "\u{1f469}\u{200d}\u{1f4bb}";
    let (server, _) =
        recording_server(move |_| Ok(json!({"text": hostile, "list": [hostile], "emoji": family})));
    let client = connect(server).await;
    let expected = json!({"text": "abcdefg", "list": ["abcdefg"], "emoji": family}).to_string();
    for tool in ["status", "list_jobs", "pause", "resume", "poll_now"] {
        let result = call(&client, tool, None).await;
        assert_eq!(text_of(&result), expected, "{tool}");
        assert_ne!(result.is_error, Some(true), "{tool}");
    }
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_failure_text_carries_no_invisible_characters_either() {
    // The failure text is no JSON, so nothing escaped it: an OSC 52 sequence (a clipboard write
    // for a terminal-based client) and line breaks would reach the client as they are.
    let (server, _) = recording_server(|method| match method {
        "pause" => Err(anyhow!("denied\u{202e}gnp").context("outer\u{200b}")),
        "resume" => Err(anyhow!("copied\u{1b}]52;c;ZGVtbw==\u{7} done")),
        _ => Err(anyhow!("inner\nline\u{9b}2J").context("outer\ttab")),
    });
    let client = connect(server).await;
    let result = call(&client, "pause", None).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(text_of(&result), "error: outer: deniedgnp");
    let text = text_of(&call(&client, "resume", None).await);
    assert!(
        !text.contains('\u{1b}') && !text.contains('\u{7}'),
        "{text:?}"
    );
    assert_eq!(text, "error: copied ]52;c;ZGVtbw==  done");
    // Every control character becomes a space, and the cause chain stays whole.
    let text = text_of(&call(&client, "poll_now", None).await);
    assert!(!text.chars().any(char::is_control), "{text:?}");
    assert_eq!(text, "error: outer tab: inner line 2J");
    client.cancel().await.unwrap();
}
