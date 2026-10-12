//! The MCP control server offers only the safe control tools and says so when the engine is down.
use issuerelay_cli::mcp::ControlServer;
use issuerelay_core::paths::AppPaths;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, ClientConfig},
};

#[tokio::test]
async fn exposes_only_safe_control_tools_and_reports_engine_down() {
    let root = tempfile::tempdir().unwrap();
    let server = ControlServer::new(AppPaths::under(root.path()).ipc_endpoint());
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    let client = ClientConfig::default().serve(client_io).await.unwrap();
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
    let text = result.content[0].as_text().unwrap().text.clone();
    assert!(
        text.starts_with("error:") && text.contains("not running"),
        "{text}"
    );
    client.cancel().await.unwrap();
}
