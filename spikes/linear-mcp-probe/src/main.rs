//! M0 probe: authorize against Linear's MCP server with OAuth (loopback redirect),
//! store the token in the system keychain, and dump raw tool output for the parser.
use std::path::Path;

use anyhow::{Result, bail};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, ClientConfig},
    service::RunningService,
    transport::{
        StreamableHttpClientTransport,
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore,
            OAuthState, StoredCredentials,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const MCP_URL: &str = "https://mcp.linear.app/mcp";
const SERVICE: &str = "com.sunkailun.issuerelay.probe";
const ACCOUNT: &str = "linear-oauth";
const FIELDS: [&str; 13] = [
    "id", "uuid", "title", "url", "status", "labels", "project", "projectId", "assigneeId",
    "parentId", "priority", "createdAt", "updatedAt",
];

type Client = RunningService<RoleClient, ClientConfig>;

struct KeyringStore;

fn entry() -> Result<keyring::Entry, AuthError> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| AuthError::CredentialStoreError(e.to_string()))
}

#[async_trait::async_trait]
impl CredentialStore for KeyringStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match entry()?.get_password() {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| AuthError::CredentialStoreError(e.to_string())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        entry()?
            .set_password(&text)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))
    }

    async fn clear(&self) -> Result<(), AuthError> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("login") => login().await,
        Some("dump") => dump().await,
        _ => bail!("usage: linear-mcp-probe login|dump"),
    }
}

async fn login() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr()?.port());
    let mut state = OAuthState::new(MCP_URL, None).await?;
    if let OAuthState::Unauthorized(manager) = &mut state {
        manager.set_credential_store(KeyringStore);
    }
    state
        .start_authorization(AuthorizationRequest::new(redirect.as_str()).with_client_name("IssueRelay probe"))
        .await?;
    let url = state.get_authorization_url().await?;
    println!("Open this URL to authorize:\n{url}");
    let _ = open::that_detached(&url);
    let callback = wait_for_callback(&listener).await?;
    state.handle_callback_url(&callback).await?;
    let scopes = KeyringStore.load().await?.map(|c| c.granted_scopes).unwrap_or_default();
    println!("authorized; granted scopes: {scopes:?}");
    Ok(())
}

/// Waits for the browser redirect and returns the full callback URL, so rmcp can
/// validate `code`, `state` and the RFC 9207 `iss` parameter itself.
async fn wait_for_callback(listener: &TcpListener) -> Result<String> {
    let port = listener.local_addr()?.port();
    loop {
        let (mut socket, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await?;
        let request = String::from_utf8_lossy(&buf[..n]);
        if let Some(url) = callback_url(request.lines().next().unwrap_or_default(), port) {
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<h1>Authorized. You can close this tab.</h1>")
                .await?;
            return Ok(url);
        }
        socket.write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n").await?;
    }
}

/// Any request to /callback is the redirect, including an OAuth error response; rmcp
/// reports a missing `code` or a server error itself instead of the probe waiting forever.
fn callback_url(request_line: &str, port: u16) -> Option<String> {
    let target = request_line.split_whitespace().nth(1)?;
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).ok()?;
    (url.path() == "/callback").then(|| url.to_string())
}

async fn connect() -> Result<Client> {
    let mut manager = AuthorizationManager::new(MCP_URL).await?;
    manager.set_credential_store(KeyringStore);
    if !manager.initialize_from_store().await? {
        bail!("not logged in; run `login` first");
    }
    let transport = StreamableHttpClientTransport::with_client(
        AuthClient::new(reqwest::Client::default(), manager),
        StreamableHttpClientTransportConfig::with_uri(MCP_URL),
    );
    Ok(ClientConfig::default().serve(transport).await?)
}

async fn call(client: &Client, tool: &'static str, args: Value) -> Result<String> {
    let Value::Object(map) = args else { bail!("arguments must be an object") };
    let result = client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(map))
        .await?;
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    Ok(if result.is_error.unwrap_or(false) { format!("ERROR: {text}") } else { text })
}

fn save(dir: &Path, name: &str, text: &str) -> Result<()> {
    std::fs::write(dir.join(format!("{name}.json")), text)?;
    println!("saved {name}");
    Ok(())
}

async fn dump() -> Result<()> {
    let dir = Path::new("samples-raw");
    std::fs::create_dir_all(dir)?;
    let client = connect().await?;
    let tools = client.list_all_tools().await?;
    std::fs::write(dir.join("tools.json"), serde_json::to_string_pretty(&tools)?)?;
    save(dir, "get_user_me", &call(&client, "get_user", json!({"query": "me"})).await?)?;
    let page1 = call(&client, "list_issues", json!({"assignee": "me", "label": "ai-ready", "limit": 1, "fields": FIELDS})).await?;
    save(dir, "list_issues_page1", &page1)?;
    let v: Value = serde_json::from_str(&page1).unwrap_or(Value::Null);
    if let Some(cursor) = ["cursor", "nextCursor", "endCursor"].iter().find_map(|k| v.get(*k).and_then(Value::as_str)) {
        let page2 = call(&client, "list_issues", json!({"assignee": "me", "label": "ai-ready", "limit": 1, "cursor": cursor, "fields": FIELDS})).await?;
        save(dir, "list_issues_page2", &page2)?;
    }
    if let Some(id) = v["issues"][0]["id"].as_str() {
        save(dir, "get_issue", &call(&client, "get_issue", json!({"id": id, "fields": FIELDS})).await?)?;
    }
    save(dir, "get_issue_missing", &call(&client, "get_issue", json!({"id": "ZZZ-999999"})).await?)?;
    save(dir, "list_projects", &call(&client, "list_projects", json!({"limit": 50})).await?)?;
    save(dir, "list_issue_labels", &call(&client, "list_issue_labels", json!({"limit": 100})).await?)?;
    let teams = call(&client, "list_teams", json!({})).await?;
    save(dir, "list_teams", &teams)?;
    let tv: Value = serde_json::from_str(&teams).unwrap_or(Value::Null);
    if let Some(team) = tv["teams"][0]["id"].as_str() {
        save(dir, "list_issue_statuses", &call(&client, "list_issue_statuses", json!({"team": team})).await?)?;
    }
    client.cancel().await?;
    Ok(())
}
