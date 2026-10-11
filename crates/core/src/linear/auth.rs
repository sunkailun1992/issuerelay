//! Connecting to Linear's MCP server: OAuth with a loopback redirect (tokens kept in the
//! system keychain) or a personal API key. Mirrors the flow verified by the M0 probe.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use rmcp::{
    ServiceExt,
    model::ClientConfig,
    transport::{
        StreamableHttpClientTransport,
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore,
            OAuthState, StoredCredentials,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::mcp::LinearClient;
use crate::paths::APP_ID;

/// Linear's hosted MCP endpoint.
pub const LINEAR_MCP_URL: &str = "https://mcp.linear.app/mcp";
/// Keychain account that holds the OAuth credentials (token, refresh token, client id).
pub const OAUTH_ACCOUNT: &str = "linear-oauth";
/// Keychain account that holds the personal API key.
pub const API_KEY_ACCOUNT: &str = "linear-api-key";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
/// A browser may open idle connections to the loopback port; each gets at most this long, so
/// one cannot hold up the real redirect for long.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause after a failed `accept`, so a persistent failure does not turn the wait into a busy loop.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);
/// Upper bound for the OAuth error text, which comes from the redirect and is shown to the user.
const MAX_OAUTH_ERROR_CHARS: usize = 300;

/// Keeps rmcp's OAuth credentials (token, refresh token, client id) in the system keychain.
/// Keychain calls can block (or wait on a system prompt), so they run on the blocking pool.
pub struct KeyringStore {
    account: &'static str,
}

impl KeyringStore {
    /// The store `login` saves to and `connect_stored` reads from.
    pub fn oauth() -> Self {
        Self {
            account: OAUTH_ACCOUNT,
        }
    }
}

/// Keyring errors are only ever formatted with `Display`, never `Debug`: `BadEncoding` and
/// `BadDataFormat` carry the stored secret bytes, and their `Debug` output prints them.
fn entry(account: &'static str) -> Result<keyring::Entry, AuthError> {
    keyring::Entry::new(APP_ID, account).map_err(|e| AuthError::CredentialStoreError(e.to_string()))
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, AuthError> + Send + 'static,
) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?
}

/// Reads back what `save` wrote. A failure reports only where parsing stopped: serde's own message
/// quotes a value of the wrong type, and in a stored credential that value can be a token.
fn parse_stored(text: &str) -> Result<StoredCredentials, AuthError> {
    serde_json::from_str(text).map_err(|e| {
        AuthError::CredentialStoreError(format!(
            "stored credentials are unreadable (line {}, column {})",
            e.line(),
            e.column()
        ))
    })
}

#[async_trait::async_trait]
impl CredentialStore for KeyringStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let account = self.account;
        blocking(move || match entry(account)?.get_password() {
            Ok(text) => parse_stored(&text).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        })
        .await
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        let account = self.account;
        // Windows Credential Manager caps one secret at 2560 bytes (UTF-16); the token's size there
        // is checked during the Windows acceptance run.
        blocking(move || {
            entry(account)?
                .set_password(&text)
                .map_err(|e| AuthError::CredentialStoreError(e.to_string()))
        })
        .await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let account = self.account;
        blocking(move || match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::CredentialStoreError(e.to_string())),
        })
        .await
    }
}

async fn connect_with(manager: AuthorizationManager, url: &str) -> Result<LinearClient> {
    let transport = StreamableHttpClientTransport::with_client(
        AuthClient::new(reqwest::Client::default(), manager),
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    Ok(ClientConfig::default().serve(transport).await?)
}

/// Uses OAuth credentials saved by an earlier `login`; `None` when there are none.
pub async fn connect_stored(url: &str) -> Result<Option<LinearClient>> {
    let mut manager = AuthorizationManager::new(url).await?;
    manager.set_credential_store(KeyringStore::oauth());
    if !manager.initialize_from_store().await? {
        return Ok(None);
    }
    Ok(Some(connect_with(manager, url).await?))
}

/// Personal API key fallback; rmcp sends it as `Authorization: Bearer <key>`.
pub async fn connect_api_key(url: &str, key: &str) -> Result<LinearClient> {
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(key.to_string()),
    );
    Ok(ClientConfig::default().serve(transport).await?)
}

/// Browser OAuth with a loopback redirect on a random port. `open_url` shows or opens the URL.
pub async fn login(url: &str, open_url: impl Fn(&str)) -> Result<LinearClient> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let mut state = OAuthState::new(url, None).await?;
    if let OAuthState::Unauthorized(manager) = &mut state {
        manager.set_credential_store(KeyringStore::oauth());
    }
    state
        .start_authorization(
            AuthorizationRequest::new(redirect.as_str()).with_client_name("IssueRelay"),
        )
        .await?;
    let authorization_url = state.get_authorization_url().await?;
    ensure_expected_authorization_url(&authorization_url, url)?;
    open_url(&authorization_url);
    let callback = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(&listener))
        .await
        .context("timed out waiting for the browser authorization")??;
    if let Some(error) = oauth_error(&callback) {
        bail!("Linear authorization failed: {error}");
    }
    // rmcp parses code, state and the RFC 9207 `iss` itself; Linear requires `iss` (M0 finding F5).
    state.handle_callback_url(&callback).await?;
    let manager = state
        .into_authorization_manager()
        .context("authorization did not complete")?;
    connect_with(manager, url).await
}

/// Keeps only the `Display` text of a keyring error (see `entry`): `{:#?}` on an `anyhow::Error`
/// prints the `Debug` of the error it wraps, which can contain the stored secret.
fn keychain_error(error: keyring::Error) -> anyhow::Error {
    anyhow!("system keychain: {error}")
}

fn keychain_entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(APP_ID, account).map_err(keychain_error)
}

/// Stores the personal API key in the system keychain, replacing any earlier one.
pub fn save_api_key(key: &str) -> Result<()> {
    keychain_entry(API_KEY_ACCOUNT)?
        .set_password(key)
        .map_err(keychain_error)
}

/// The saved personal API key; `None` when none is saved.
pub fn load_api_key() -> Result<Option<String>> {
    match keychain_entry(API_KEY_ACCOUNT)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain_error(e)),
    }
}

/// Deletes the saved OAuth credentials and API key; entries that do not exist are fine.
pub fn clear_credentials() -> Result<()> {
    for account in [OAUTH_ACCOUNT, API_KEY_ACCOUNT] {
        match keychain_entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(keychain_error(e)),
        }
    }
    Ok(())
}

async fn wait_for_callback(listener: &TcpListener) -> Result<String> {
    let port = listener.local_addr()?.port();
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            // Usually transient (for example too many open files): keep listening. The wait as a
            // whole is bounded by `LOGIN_TIMEOUT`.
            tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
            continue;
        };
        let mut buf = vec![0u8; 8192];
        let Ok(Ok(n)) = tokio::time::timeout(READ_TIMEOUT, socket.read(&mut buf)).await else {
            continue;
        };
        let request = String::from_utf8_lossy(&buf[..n]);
        if let Some(url) = callback_url(request.lines().next().unwrap_or_default(), port) {
            // The redirect has arrived; whether the browser still gets the page does not matter.
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<h1>IssueRelay received the browser's reply. You can close this tab and return to IssueRelay.</h1>")
                .await;
            return Ok(url);
        }
        // A failed reply is fine: the browser may already have closed an idle connection.
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
            .await;
    }
}

/// The full redirect URL (code, state, `iss`, or an OAuth error). Any request to /callback counts
/// as the redirect, so an error response ends the wait instead of hanging until the timeout.
/// `None` for other paths and for any target that is not origin-form (`/path?query`).
pub fn callback_url(request_line: &str, port: u16) -> Option<String> {
    // Origin-form only: an absolute-form target, or one starting with `@`, must never change the
    // host of the URL built below.
    let target = request_line
        .split_whitespace()
        .nth(1)
        .filter(|target| target.starts_with('/'))?;
    let url = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")).ok()?;
    (url.path() == "/callback").then(|| url.to_string())
}

/// `error` (with `error_description`) from an OAuth error redirect; rmcp does not read them.
/// The text comes from the redirect and ends up in messages shown to the user, so control
/// characters are dropped and the length is capped.
pub fn oauth_error(callback_url: &str) -> Option<String> {
    let url = url::Url::parse(callback_url).ok()?;
    let error = url.query_pairs().find(|(k, _)| k == "error")?.1;
    let text = match url.query_pairs().find(|(k, _)| k == "error_description") {
        Some((_, description)) => format!("{error}: {description}"),
        None => error.into_owned(),
    };
    Some(
        text.chars()
            .filter(|c| !c.is_control())
            .take(MAX_OAUTH_ERROR_CHARS)
            .collect(),
    )
}

/// The authorization URL comes from remote metadata: only open https URLs on the MCP server's own
/// origin (scheme, host and port).
fn ensure_expected_authorization_url(authorization_url: &str, mcp_url: &str) -> Result<()> {
    let authorization = url::Url::parse(authorization_url).context("invalid authorization URL")?;
    let mcp = url::Url::parse(mcp_url).context("invalid MCP URL")?;
    anyhow::ensure!(
        authorization.scheme() == "https" && authorization.origin() == mcp.origin(),
        "unexpected authorization URL origin: {}",
        authorization.origin().ascii_serialization()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_url_keeps_the_whole_redirect_query() {
        assert_eq!(
            callback_url(
                "GET /callback?code=abc%20def&state=xyz&iss=https%3A%2F%2Fmcp.linear.app HTTP/1.1",
                4321
            )
            .as_deref(),
            Some(
                "http://127.0.0.1:4321/callback?code=abc%20def&state=xyz&iss=https%3A%2F%2Fmcp.linear.app"
            )
        );
        // An OAuth error response is still the redirect; `login` reports it via `oauth_error`.
        assert_eq!(
            callback_url("GET /callback?error=access_denied&state=xyz HTTP/1.1", 4321).as_deref(),
            Some("http://127.0.0.1:4321/callback?error=access_denied&state=xyz")
        );
    }

    #[test]
    fn ignores_other_paths_and_garbage() {
        assert_eq!(callback_url("GET /favicon.ico HTTP/1.1", 4321), None);
        assert_eq!(callback_url("garbage", 4321), None);
        assert_eq!(callback_url("", 4321), None);
        assert_eq!(
            callback_url("GET @attacker.example/callback?code=c HTTP/1.1", 4321),
            None
        );
        assert_eq!(
            callback_url("GET http://attacker.example/callback?code=c HTTP/1.1", 4321),
            None
        );
    }

    #[test]
    fn oauth_error_reads_error_and_description() {
        assert_eq!(
            oauth_error(
                "http://127.0.0.1:1/callback?error=access_denied&error_description=User%20denied"
            )
            .as_deref(),
            Some("access_denied: User denied")
        );
        assert_eq!(
            oauth_error("http://127.0.0.1:1/callback?error=access_denied").as_deref(),
            Some("access_denied")
        );
        assert_eq!(
            oauth_error("http://127.0.0.1:1/callback?code=c&state=s"),
            None
        );
        assert_eq!(
            oauth_error(
                "http://127.0.0.1:1/callback?error=access_denied&error_description=a%1B%5B2Jb%0Ac"
            )
            .as_deref(),
            Some("access_denied: a[2Jbc")
        );
        let long = format!("http://127.0.0.1:1/callback?error={}", "x".repeat(1000));
        assert!(oauth_error(&long).unwrap().chars().count() <= 300);
    }

    #[test]
    fn only_https_authorization_urls_on_the_mcp_host_are_opened() {
        assert!(
            ensure_expected_authorization_url(
                "https://mcp.linear.app/authorize?x=1",
                LINEAR_MCP_URL
            )
            .is_ok()
        );
        assert!(
            ensure_expected_authorization_url("http://mcp.linear.app/authorize", LINEAR_MCP_URL)
                .is_err()
        );
        assert!(
            ensure_expected_authorization_url("https://attacker.example/authorize", LINEAR_MCP_URL)
                .is_err()
        );
        assert!(
            ensure_expected_authorization_url(
                "https://mcp.linear.app:8443/authorize",
                LINEAR_MCP_URL
            )
            .is_err()
        );
        assert!(
            ensure_expected_authorization_url(
                "https://mcp.linear.app@attacker.example/authorize",
                LINEAR_MCP_URL
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn listener_skips_other_paths_and_returns_the_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_callback(&listener).await });
        for target in ["/favicon.ico", "/callback?error=access_denied&state=s"] {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
        }
        let url = waiter.await.unwrap().unwrap();
        assert_eq!(
            url,
            format!("http://127.0.0.1:{port}/callback?error=access_denied&state=s")
        );
        assert_eq!(oauth_error(&url).as_deref(), Some("access_denied"));
    }

    #[tokio::test]
    async fn an_idle_connection_does_not_block_the_redirect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_callback(&listener).await });
        // Connected first and never sends anything, like a browser's speculative connection.
        let _idle = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(b"GET /callback?code=c&state=s HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let url = tokio::time::timeout(READ_TIMEOUT * 4, waiter)
            .await
            .expect("the idle connection blocked the redirect")
            .unwrap()
            .unwrap();
        assert_eq!(
            url,
            format!("http://127.0.0.1:{port}/callback?code=c&state=s")
        );
    }

    #[test]
    fn unreadable_stored_credentials_do_not_echo_their_content() {
        // serde quotes a value of the wrong type in its message; in a stored credential that value
        // could be a token.
        let error =
            parse_stored(r#"{"client_id":"c","token_response":"not-a-real-token"}"#).unwrap_err();
        assert!(!error.to_string().contains("not-a-real-token"), "{error}");
        assert!(parse_stored(r#"{"client_id":"c"}"#).is_ok());
    }

    /// Touches the real system keychain, so it only runs on request:
    /// `cargo test -p issuerelay-core -- --ignored keyring_round_trip`
    #[test]
    #[ignore]
    fn keyring_round_trip() {
        save_api_key("test-key-not-real").unwrap();
        assert_eq!(
            load_api_key().unwrap().as_deref(),
            Some("test-key-not-real")
        );
        clear_credentials().unwrap();
        assert_eq!(load_api_key().unwrap(), None);
    }
}
