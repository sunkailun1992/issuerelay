//! Connecting to Linear's MCP server: OAuth with a loopback redirect (tokens kept in the
//! system keychain) or a personal API key. Mirrors the flow verified by the M0 probe.
//!
//! Keyring errors are only ever formatted with `Display`, never `Debug`: `BadEncoding` and
//! `BadDataFormat` carry the stored secret bytes, and their `Debug` output prints them.

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
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    task::JoinSet,
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
/// The MCP handshake has no time limit of its own, so a server that accepts the connection and
/// then stays silent would hang the caller. (rmcp 3.5.1 already gives its own OAuth HTTP requests
/// a 30 s limit.)
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A browser may open idle connections to the loopback port. Each one is served on its own task
/// and gets at most this long to send its request, so none can hold up the real redirect.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Pause after a failed `accept`, so a persistent failure does not turn the wait into a busy loop.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);
/// Upper bound for the OAuth error text, which comes from the redirect and is shown to the user.
const MAX_OAUTH_ERROR_CHARS: usize = 300;
const REDIRECT_RECEIVED_PAGE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<h1>IssueRelay received the browser's reply. You can close this tab and return to IssueRelay.</h1>";
const NOT_FOUND_PAGE: &[u8] = b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n";

/// Keeps rmcp's OAuth credentials (token, refresh token, client id) in the system keychain.
/// Keychain calls can block (or wait on a system prompt), so they run on the blocking pool.
pub struct KeyringStore {
    service: String,
    account: &'static str,
}

impl KeyringStore {
    /// The store `login` saves to and `connect_stored` reads from.
    pub fn oauth() -> Self {
        Self::with_service(APP_ID)
    }

    /// The same store under another keychain service name, so each test can keep to its own.
    fn with_service(service: &str) -> Self {
        Self {
            service: service.to_string(),
            account: OAUTH_ACCOUNT,
        }
    }
}

/// The one place that opens a keychain entry. `service` is `APP_ID` everywhere but in tests.
fn entry(service: &str, account: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(service, account)
}

/// A failure of the OAuth credential store, as text.
fn store_error(error: impl std::fmt::Display) -> AuthError {
    AuthError::CredentialStoreError(error.to_string())
}

/// A keyring error as text only: `{:#?}` on an `anyhow::Error` prints the `Debug` of the error it
/// wraps, which can hold the stored secret.
fn keychain_error(error: keyring::Error) -> anyhow::Error {
    anyhow!("system keychain: {error}")
}

async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, AuthError> + Send + 'static,
) -> Result<T, AuthError> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(store_error)?
}

/// Reads back what `save` wrote. A failure reports only where parsing stopped: serde's own message
/// quotes a value of the wrong type, and in a stored credential that value can be a token.
fn parse_stored(text: &str) -> Result<StoredCredentials, AuthError> {
    serde_json::from_str(text).map_err(|e| {
        store_error(format!(
            "stored credentials are unreadable (line {}, column {}); \
             log in to Linear again to replace them",
            e.line(),
            e.column()
        ))
    })
}

#[async_trait::async_trait]
impl CredentialStore for KeyringStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let (service, account) = (self.service.clone(), self.account);
        blocking(move || {
            match entry(&service, account)
                .map_err(store_error)?
                .get_password()
            {
                Ok(text) => parse_stored(&text).map(Some),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(store_error(e)),
            }
        })
        .await
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let text = serde_json::to_string(&credentials).map_err(store_error)?;
        let (service, account) = (self.service.clone(), self.account);
        // Windows Credential Manager caps a credential blob at 2560 bytes
        // (CRED_MAX_CREDENTIAL_BLOB_SIZE), which keyring's Windows store checks as 1280 UTF-16 code
        // units (windows-native-keyring-store 1.1.0, checked 2026-10-11). Whether the token fits is
        // measured in the Task 23 acceptance run.
        blocking(move || {
            entry(&service, account)
                .map_err(store_error)?
                .set_password(&text)
                .map_err(store_error)
        })
        .await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let (service, account) = (self.service.clone(), self.account);
        blocking(move || {
            match entry(&service, account)
                .map_err(store_error)?
                .delete_credential()
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(store_error(e)),
            }
        })
        .await
    }
}

/// Runs an MCP handshake, giving up after `CONNECT_TIMEOUT`.
async fn within_connect_timeout<T, E>(handshake: impl Future<Output = Result<T, E>>) -> Result<T>
where
    E: Into<anyhow::Error>,
{
    match tokio::time::timeout(CONNECT_TIMEOUT, handshake).await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => bail!(
            "Linear MCP did not finish connecting within {}s",
            CONNECT_TIMEOUT.as_secs()
        ),
    }
}

async fn connect_with(manager: AuthorizationManager, url: &str) -> Result<LinearClient> {
    let transport = StreamableHttpClientTransport::with_client(
        AuthClient::new(reqwest::Client::default(), manager),
        StreamableHttpClientTransportConfig::with_uri(url),
    );
    within_connect_timeout(ClientConfig::default().serve(transport)).await
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

/// Personal API key fallback; rmcp sends it as `Authorization: Bearer <key>`. Fails before
/// connecting unless `url` is https, so the key never travels in the clear.
pub async fn connect_api_key(url: &str, key: &str) -> Result<LinearClient> {
    let parsed = url::Url::parse(url).context("invalid MCP URL")?;
    anyhow::ensure!(
        parsed.scheme() == "https",
        "the API key is only sent over https"
    );
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::default(),
        StreamableHttpClientTransportConfig::with_uri(url).auth_header(key.to_string()),
    );
    within_connect_timeout(ClientConfig::default().serve(transport)).await
}

/// Browser OAuth with a loopback redirect on a random port. `open_url` shows or opens the URL.
/// Only a redirect that carries this request's `state` (or an OAuth error without any `state`)
/// ends the wait, and the loopback port is closed at once, before the token is exchanged.
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
    let expected_state = authorization_state(&authorization_url)?;
    open_url(&authorization_url);
    let callback =
        tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(&listener, &expected_state))
            .await
            .context("timed out waiting for the browser authorization")??;
    // The callback has arrived; nothing else should reach the loopback port while the token is
    // exchanged.
    drop(listener);
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

/// Stores the personal API key in the system keychain, replacing any earlier one. Blocks (the
/// system may show a keychain prompt), so an async caller in a GUI host runs it on the blocking
/// pool.
pub fn save_api_key(key: &str) -> Result<()> {
    save_api_key_in(APP_ID, key)
}

fn save_api_key_in(service: &str, key: &str) -> Result<()> {
    let key_entry = entry(service, API_KEY_ACCOUNT).map_err(keychain_error)?;
    key_entry.set_password(key).map_err(keychain_error)
}

/// The saved personal API key; `None` when none is saved. Blocks (the system may show a keychain
/// prompt), so an async caller in a GUI host runs it on the blocking pool.
pub fn load_api_key() -> Result<Option<String>> {
    load_api_key_in(APP_ID)
}

fn load_api_key_in(service: &str) -> Result<Option<String>> {
    let key_entry = entry(service, API_KEY_ACCOUNT).map_err(keychain_error)?;
    match key_entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(keychain_error(e)),
    }
}

/// Deletes the saved OAuth credentials and API key; an entry that does not exist is fine. Both are
/// tried even when the first fails, and the first failure is reported. Blocks (the system may show
/// a keychain prompt), so an async caller in a GUI host runs it on the blocking pool.
pub fn clear_credentials() -> Result<()> {
    clear_credentials_in(APP_ID)
}

fn clear_credentials_in(service: &str) -> Result<()> {
    let mut first_failure = None;
    for account in [OAUTH_ACCOUNT, API_KEY_ACCOUNT] {
        if let Err(error) = clear_account_in(service, account) {
            first_failure.get_or_insert(error);
        }
    }
    first_failure.map_or(Ok(()), Err)
}

/// Deletes the saved OAuth credentials only (switching to an API key). Blocks; see
/// `clear_credentials`.
pub fn clear_oauth_credentials() -> Result<()> {
    clear_oauth_credentials_in(APP_ID)
}

fn clear_oauth_credentials_in(service: &str) -> Result<()> {
    clear_account_in(service, OAUTH_ACCOUNT)
}

/// Deletes the saved API key only (switching to OAuth). Blocks; see `clear_credentials`.
pub fn clear_api_key() -> Result<()> {
    clear_api_key_in(APP_ID)
}

fn clear_api_key_in(service: &str) -> Result<()> {
    clear_account_in(service, API_KEY_ACCOUNT)
}

/// Deletes one keychain entry; one that does not exist is fine.
fn clear_account_in(service: &str, account: &str) -> Result<()> {
    entry(service, account)
        .and_then(|item| match item.delete_credential() {
            Err(keyring::Error::NoEntry) => Ok(()),
            other => other,
        })
        .map_err(keychain_error)
}

/// Waits for the browser's redirect and returns its full URL. Every accepted connection is served
/// on its own task, and the first one that reports our redirect ends the wait; the tasks still
/// running are aborted when the `JoinSet` is dropped on return.
async fn wait_for_callback(listener: &TcpListener, expected_state: &str) -> Result<String> {
    let port = listener.local_addr()?.port();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => {
                    let expected_state = expected_state.to_string();
                    connections.spawn(async move {
                        serve_one(socket, port, &expected_state).await
                    });
                }
                Err(_) => {
                    // Usually transient (for example too many open files): keep listening. The
                    // wait as a whole is bounded by `LOGIN_TIMEOUT`.
                    tokio::time::sleep(ACCEPT_RETRY_DELAY).await;
                }
            },
            Some(finished) = connections.join_next() => {
                if let Ok(Some(url)) = finished {
                    return Ok(url);
                }
            }
        }
    }
}

/// Serves one loopback connection: reads the request (for at most `READ_TIMEOUT`), answers it, and
/// returns the redirect URL when the request is our own redirect (see `is_expected_redirect`).
async fn serve_one<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    port: u16,
    expected_state: &str,
) -> Option<String> {
    let mut buf = vec![0u8; 8192];
    let Ok(Ok(n)) = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf)).await else {
        return None;
    };
    let request = String::from_utf8_lossy(&buf[..n]);
    let redirect = callback_url(request.lines().next().unwrap_or_default(), port)
        .filter(|url| is_expected_redirect(url, expected_state));
    let page = if redirect.is_some() {
        REDIRECT_RECEIVED_PAGE
    } else {
        NOT_FOUND_PAGE
    };
    // A failed write changes nothing: the redirect has arrived (or this request was not ours),
    // and the browser may already have closed the connection.
    let _ = stream.write_all(page).await;
    redirect
}

/// The full redirect URL (code, state, `iss`, or an OAuth error) when the request is for
/// `/callback`, with the host fixed to the loopback address. `None` for other paths and for any
/// target that is not origin-form (`/path?query`). Whether it is the redirect we are waiting for
/// is decided by `serve_one`, which checks `state`.
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

/// Whether `callback` answers our authorization request: it echoes our `state`, or it is an OAuth
/// error response with no `state` at all. RFC 6749 section 4.1.2.1 says error responses echo
/// `state`, but a missing one must not leave the login waiting for the timeout.
fn is_expected_redirect(callback: &str, expected_state: &str) -> bool {
    let Ok(url) = url::Url::parse(callback) else {
        return false;
    };
    let mut states = url
        .query_pairs()
        .filter(|(key, _)| key == "state")
        .map(|(_, value)| value);
    match states.next() {
        // rmcp keeps the last `state` of a query, so a repeated one has to agree everywhere.
        Some(first) => first == expected_state && states.all(|state| state == expected_state),
        None => url.query_pairs().any(|(key, _)| key == "error"),
    }
}

/// `error` (with `error_description`) from an OAuth error redirect; rmcp does not read them.
/// The text comes from the redirect and ends up in messages shown to the user, so control
/// characters and characters that can reorder or hide text are dropped, and the length is capped.
pub fn oauth_error(callback_url: &str) -> Option<String> {
    let url = url::Url::parse(callback_url).ok()?;
    let error = url.query_pairs().find(|(k, _)| k == "error")?.1;
    let text = match url.query_pairs().find(|(k, _)| k == "error_description") {
        Some((_, description)) => format!("{error}: {description}"),
        None => error.into_owned(),
    };
    Some(
        text.chars()
            .filter(|c| !is_unsafe_to_show(*c))
            .take(MAX_OAUTH_ERROR_CHARS)
            .collect(),
    )
}

/// Control characters, and the characters that can reorder or hide text (`text::is_invisible`:
/// zero-width and direction marks, separators, bidi controls, tag characters and the like). The
/// joiners U+200C and U+200D stay, as everywhere else.
fn is_unsafe_to_show(c: char) -> bool {
    c.is_control() || crate::text::is_invisible(c)
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

/// The `state` of the authorization request (rmcp's CSRF token), which the browser's redirect has
/// to echo back. An authorization URL without one is an error.
fn authorization_state(authorization_url: &str) -> Result<String> {
    let url = url::Url::parse(authorization_url).context("invalid authorization URL")?;
    url.query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .filter(|state| !state.is_empty())
        .context("the authorization URL carries no state")
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
        // Format and separator characters that can reorder or hide text go too (here: a
        // right-to-left override, a zero-width space and a line separator).
        assert_eq!(
            oauth_error(
                "http://127.0.0.1:1/callback?error=access_denied&error_description=a%E2%80%AEb%E2%80%8Bc%E2%80%A8d"
            )
            .as_deref(),
            Some("access_denied: abcd")
        );
        let long = format!("http://127.0.0.1:1/callback?error={}", "x".repeat(1000));
        assert!(oauth_error(&long).unwrap().chars().count() <= 300);
    }

    #[test]
    fn oauth_error_drops_the_characters_the_engine_drops_and_keeps_the_joiners() {
        // A tag character, the Arabic letter mark, the soft hyphen and the Hangul fillers hide
        // or reorder text like the zero-width marks do (`text::is_invisible`); the joiners stay,
        // because emoji and Indic scripts need them.
        let description =
            "a\u{e0041}b\u{061c}c\u{00ad}d\u{115f}e\u{3164}f\u{ffa0}g\u{200d}h\u{200c}i";
        let callback = url::Url::parse_with_params(
            "http://127.0.0.1:1/callback",
            [
                ("error", "access_denied"),
                ("error_description", description),
            ],
        )
        .unwrap();
        assert_eq!(
            oauth_error(callback.as_str()).as_deref(),
            Some("access_denied: abcdefg\u{200d}h\u{200c}i")
        );
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

    #[test]
    fn the_authorization_url_must_carry_a_state() {
        assert_eq!(
            authorization_state(
                "https://mcp.linear.app/authorize?client_id=c&state=ab%20c&code_challenge=x"
            )
            .unwrap(),
            "ab c"
        );
        assert!(authorization_state("https://mcp.linear.app/authorize?client_id=c").is_err());
        assert!(authorization_state("https://mcp.linear.app/authorize?state=").is_err());
        assert!(authorization_state("not a url").is_err());
    }

    #[tokio::test]
    async fn listener_skips_other_paths_and_returns_the_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_callback(&listener, "s").await });
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
        let waiter = tokio::spawn(async move { wait_for_callback(&listener, "s").await });
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

    #[tokio::test]
    async fn a_callback_with_another_state_is_ignored() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_callback(&listener, "s").await });
        for target in [
            "/callback?code=forged&state=wrong",
            "/callback?code=forged",
            "/callback?code=c&state=s",
        ] {
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            stream
                .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).await.unwrap();
            assert_eq!(
                reply.starts_with("HTTP/1.1 200"),
                target.ends_with("state=s"),
                "{target}: {reply}"
            );
        }
        let url = waiter.await.unwrap().unwrap();
        assert_eq!(
            url,
            format!("http://127.0.0.1:{port}/callback?code=c&state=s")
        );
    }

    #[tokio::test]
    async fn several_idle_connections_do_not_delay_the_redirect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let started = std::time::Instant::now();
        let waiter = tokio::spawn(async move { wait_for_callback(&listener, "s").await });
        let mut idle = Vec::new();
        for _ in 0..3 {
            idle.push(
                tokio::net::TcpStream::connect(("127.0.0.1", port))
                    .await
                    .unwrap(),
            );
        }
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(b"GET /callback?code=c&state=s HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let url = tokio::time::timeout(READ_TIMEOUT * 4, waiter)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(url.ends_with("state=s"));
        assert!(
            started.elapsed() < READ_TIMEOUT,
            "idle connections delayed the redirect: {:?}",
            started.elapsed()
        );
    }

    /// One request on an in-memory connection: what `serve_one` reported, and the reply the
    /// browser saw.
    async fn serve_request(target: &str, expected_state: &str) -> (Option<String>, String) {
        let (mut browser, server_side) = tokio::io::duplex(4096);
        browser
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let served = serve_one(server_side, 4321, expected_state).await;
        let mut reply = String::new();
        browser.read_to_string(&mut reply).await.unwrap();
        (served, reply)
    }

    #[tokio::test]
    async fn only_our_own_redirect_or_an_error_without_state_ends_the_wait() {
        let accepted = [
            "/callback?code=c&state=s",
            "/callback?error=access_denied&state=s",
            // RFC 6749 says an error response echoes `state`, but a missing one must not leave
            // the login waiting for the timeout.
            "/callback?error=access_denied",
        ];
        let ignored = [
            "/callback?code=c",
            "/callback?code=c&state=wrong",
            "/callback?error=access_denied&state=wrong",
            "/callback?code=c&state=",
            // rmcp keeps the last `state` of a query, so a repeated one has to agree everywhere.
            "/callback?code=c&state=s&state=wrong",
            "/callback?code=c&state=wrong&state=s",
            "/favicon.ico",
        ];
        for target in accepted {
            let (served, reply) = serve_request(target, "s").await;
            assert_eq!(
                served,
                Some(format!("http://127.0.0.1:4321{target}")),
                "{target}"
            );
            assert!(reply.starts_with("HTTP/1.1 200"), "{target}: {reply}");
        }
        for target in ignored {
            let (served, reply) = serve_request(target, "s").await;
            assert_eq!(served, None, "{target}");
            assert!(reply.starts_with("HTTP/1.1 404"), "{target}: {reply}");
        }
    }

    #[tokio::test]
    async fn a_failed_success_page_write_still_returns_the_url() {
        let (mut browser, server_side) = tokio::io::duplex(4096);
        browser
            .write_all(b"GET /callback?code=c&state=s HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        drop(browser); // the browser is gone before the page is written
        assert_eq!(
            serve_one(server_side, 4321, "s").await.as_deref(),
            Some("http://127.0.0.1:4321/callback?code=c&state=s")
        );
    }

    #[tokio::test]
    async fn the_api_key_is_never_sent_over_plain_http() {
        let error = connect_api_key("http://127.0.0.1:9/mcp", "test-key-not-real")
            .await
            .expect_err("a plain http URL must be refused");
        let message = format!("{error:#}");
        assert!(message.contains("https"), "{message}");
        assert!(!message.contains("test-key-not-real"), "{message}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_server_cannot_hold_up_connecting() {
        // A handshake that finishes, well or badly, comes back unchanged.
        let finished = within_connect_timeout(async { Ok::<_, std::io::Error>(7) }).await;
        assert_eq!(finished.unwrap(), 7);
        let failed = within_connect_timeout(async {
            Err::<(), _>(std::io::Error::other("handshake failed"))
        })
        .await;
        assert!(failed.unwrap_err().to_string().contains("handshake failed"));
        // One that never answers, like a server that accepts the connection and then says
        // nothing, is cut off. (The outer timeout makes a regression fail instead of hang.)
        let silent = tokio::time::timeout(
            CONNECT_TIMEOUT * 2,
            within_connect_timeout(std::future::pending::<Result<(), std::io::Error>>()),
        )
        .await
        .expect("a silent server held up connecting");
        assert_eq!(
            silent.unwrap_err().to_string(),
            "Linear MCP did not finish connecting within 30s"
        );
    }

    #[test]
    fn connecting_and_logging_in_can_run_on_a_spawned_task() {
        // The engine and the GUI host spawn these. Creating the futures without polling them runs
        // nothing: no network, no keychain.
        fn assert_send<T: Send>(_: &T) {}
        assert_send(&connect_stored(LINEAR_MCP_URL));
        assert_send(&connect_api_key(LINEAR_MCP_URL, "test-key-not-real"));
        assert_send(&login(LINEAR_MCP_URL, |_url: &str| {}));
    }

    #[test]
    fn unreadable_stored_credentials_do_not_echo_their_content() {
        // serde quotes a value of the wrong type in its message; in a stored credential that value
        // could be a token.
        let error = parse_stored(r#"{"client_id":"c","token_response":"not-a-real-token"}"#)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("not-a-real-token"), "{error}");
        assert!(error.contains("line 1, column"), "{error}");
        assert!(
            error.ends_with("; log in to Linear again to replace them"),
            "{error}"
        );
        assert!(parse_stored(r#"{"client_id":"c"}"#).is_ok());
    }

    /// Keychain behavior against an in-memory store, so nothing here can reach the real keychain.
    ///
    /// Only where IssueRelay runs, macOS and Windows: keyring's Linux store connects to a Secret
    /// Service as soon as it is created, and `keyring::Entry::new` then refuses to work in a
    /// container without one, whatever store is installed afterwards.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    mod keychain {
        use std::{
            any::Any,
            collections::{HashMap, HashSet},
            sync::{Arc, LazyLock, Mutex, Once},
        };

        use keyring_core::api::{CredentialApi, CredentialStoreApi};

        use super::*;

        const OAUTH_CREDENTIALS: &str = r#"{
            "client_id": "client-1",
            "token_response": {
                "access_token": "not-a-real-token",
                "token_type": "bearer",
                "expires_in": 3600,
                "refresh_token": "not-a-real-refresh-token",
                "scope": "read write"
            },
            "granted_scopes": ["read", "write"],
            "token_received_at": 1760000000,
            "issuer": "https://mcp.linear.app"
        }"#;

        const STORE_ID: &str = "issuerelay-auth-tests";

        /// What the in-memory keychain holds: secrets by (service, user), and entries whose
        /// deletion fails, like in a locked keychain. A store of our own rather than
        /// `keyring_core::mock`: its docs say a mock credential does not persist beyond its entry,
        /// while its 1.0.0 code shares one credential per service and user, which is not a
        /// contract to build tests on.
        #[derive(Default)]
        struct Vault {
            secrets: HashMap<(String, String), Vec<u8>>,
            undeletable: HashSet<(String, String)>,
        }

        static VAULT: LazyLock<Mutex<Vault>> = LazyLock::new(Mutex::default);

        struct MemoryStore;

        struct MemoryCredential {
            key: (String, String),
        }

        impl CredentialStoreApi for MemoryStore {
            fn vendor(&self) -> String {
                "in-memory store for IssueRelay tests".to_string()
            }

            fn id(&self) -> String {
                STORE_ID.to_string()
            }

            fn build(
                &self,
                service: &str,
                user: &str,
                _modifiers: Option<&HashMap<&str, &str>>,
            ) -> keyring_core::Result<keyring_core::Entry> {
                Ok(keyring_core::Entry::new_with_credential(Arc::new(
                    MemoryCredential {
                        key: (service.to_string(), user.to_string()),
                    },
                )))
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl CredentialApi for MemoryCredential {
            fn set_secret(&self, secret: &[u8]) -> keyring_core::Result<()> {
                let mut vault = VAULT.lock().unwrap();
                vault.secrets.insert(self.key.clone(), secret.to_vec());
                Ok(())
            }

            fn get_secret(&self) -> keyring_core::Result<Vec<u8>> {
                let vault = VAULT.lock().unwrap();
                vault
                    .secrets
                    .get(&self.key)
                    .cloned()
                    .ok_or(keyring_core::Error::NoEntry)
            }

            fn delete_credential(&self) -> keyring_core::Result<()> {
                let mut vault = VAULT.lock().unwrap();
                if vault.undeletable.contains(&self.key) {
                    return Err(keyring_core::Error::NoStorageAccess(
                        "keychain is locked".into(),
                    ));
                }
                vault
                    .secrets
                    .remove(&self.key)
                    .map(drop)
                    .ok_or(keyring_core::Error::NoEntry)
            }

            fn get_credential(
                &self,
            ) -> keyring_core::Result<Option<Arc<keyring_core::Credential>>> {
                self.get_secret().map(|_| None)
            }

            fn get_specifiers(&self) -> Option<(String, String)> {
                Some(self.key.clone())
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        /// A service name for one test. The first call also makes the in-memory store keyring's
        /// default store, so every keychain call in this test binary stays out of the real
        /// keychain. Tests run in parallel against the one store, each under its own service.
        fn service(test: &str) -> String {
            static INSTALL: Once = Once::new();
            INSTALL.call_once(|| {
                // keyring installs the platform store the first time anything asks for an entry
                // (creating that store does not touch the keychain). Let that happen now, so it
                // cannot replace ours later.
                if let Err(error) = keyring::Entry::store_status() {
                    panic!("keyring could not set up its platform store: {error}");
                }
                keyring_core::set_default_store(Arc::new(MemoryStore));
            });
            let default_store = keyring_core::get_default_store().expect("a default store");
            assert_eq!(
                default_store.id(),
                STORE_ID,
                "the in-memory store is not keyring's default store"
            );
            format!("{APP_ID}.test.{test}")
        }

        #[tokio::test]
        async fn nothing_stored_loads_as_none() {
            let service = service("nothing-stored");
            assert_eq!(load_api_key_in(&service).unwrap(), None);
            assert!(
                KeyringStore::with_service(&service)
                    .load()
                    .await
                    .unwrap()
                    .is_none()
            );
        }

        #[test]
        fn an_api_key_is_saved_replaced_and_cleared() {
            let service = service("api-key");
            save_api_key_in(&service, "test-key-not-real").unwrap();
            assert_eq!(
                load_api_key_in(&service).unwrap().as_deref(),
                Some("test-key-not-real")
            );
            save_api_key_in(&service, "other-test-key-not-real").unwrap();
            assert_eq!(
                load_api_key_in(&service).unwrap().as_deref(),
                Some("other-test-key-not-real")
            );
            clear_credentials_in(&service).unwrap();
            assert_eq!(load_api_key_in(&service).unwrap(), None);
            // Nothing left to delete is not a failure.
            clear_credentials_in(&service).unwrap();
        }

        #[tokio::test]
        async fn oauth_credentials_are_saved_loaded_and_cleared() {
            let service = service("oauth");
            let store = KeyringStore::with_service(&service);
            let credentials: StoredCredentials = serde_json::from_str(OAUTH_CREDENTIALS).unwrap();

            store.save(credentials.clone()).await.unwrap();
            let loaded = store.load().await.unwrap().expect("saved credentials");
            assert_eq!(
                serde_json::to_value(&loaded).unwrap(),
                serde_json::to_value(&credentials).unwrap()
            );
            store.clear().await.unwrap();
            assert!(store.load().await.unwrap().is_none());
            store.clear().await.unwrap();

            // `clear_credentials` removes the OAuth entry as well.
            store.save(credentials).await.unwrap();
            clear_credentials_in(&service).unwrap();
            assert!(store.load().await.unwrap().is_none());
        }

        #[tokio::test]
        async fn a_corrupt_oauth_entry_fails_to_load_without_echoing_its_content() {
            let service = service("corrupt-oauth");
            keyring::Entry::new(&service, OAUTH_ACCOUNT)
                .unwrap()
                .set_password(r#"{"client_id":"c","token_response":"not-a-real-token"}"#)
                .unwrap();
            let error = KeyringStore::with_service(&service)
                .load()
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("line 1, column"), "{error}");
            assert!(!error.contains("not-a-real-token"), "{error}");
        }

        #[test]
        fn clearing_removes_the_api_key_even_when_the_oauth_entry_cannot_be_deleted() {
            let service = service("clear-with-failure");
            save_api_key_in(&service, "test-key-not-real").unwrap();
            keyring::Entry::new(&service, OAUTH_ACCOUNT)
                .unwrap()
                .set_password("{}")
                .unwrap();
            // The OAuth entry is the one `clear_credentials` tries first.
            VAULT
                .lock()
                .unwrap()
                .undeletable
                .insert((service.clone(), OAUTH_ACCOUNT.to_string()));
            let error = clear_credentials_in(&service).unwrap_err();
            assert!(error.to_string().contains("keychain is locked"), "{error}");
            assert_eq!(load_api_key_in(&service).unwrap(), None);
        }

        #[test]
        fn each_kind_of_credential_can_be_cleared_on_its_own() {
            let service = service("clear-one");
            save_api_key_in(&service, "lin_api_demo").unwrap();
            entry(&service, OAUTH_ACCOUNT)
                .unwrap()
                .set_password("{\"token\":\"demo\"}")
                .unwrap();
            clear_account_in(&service, OAUTH_ACCOUNT).unwrap();
            assert_eq!(
                load_api_key_in(&service).unwrap().as_deref(),
                Some("lin_api_demo")
            );
            assert!(matches!(
                entry(&service, OAUTH_ACCOUNT).unwrap().get_password(),
                Err(keyring::Error::NoEntry)
            ));
            clear_account_in(&service, API_KEY_ACCOUNT).unwrap();
            assert_eq!(load_api_key_in(&service).unwrap(), None);
            // Nothing left to delete is not a failure.
            clear_account_in(&service, API_KEY_ACCOUNT).unwrap();
        }

        // Swapped, `clear_api_key` would wipe the OAuth credentials a login has just saved.
        #[test]
        fn switching_the_login_method_clears_only_the_one_it_replaces() {
            let service = service("clear-kind");
            let save_both = || {
                save_api_key_in(&service, "lin_api_demo").unwrap();
                entry(&service, OAUTH_ACCOUNT)
                    .unwrap()
                    .set_password("{\"token\":\"demo\"}")
                    .unwrap();
            };
            let oauth_is_saved = || {
                entry(&service, OAUTH_ACCOUNT)
                    .unwrap()
                    .get_password()
                    .is_ok()
            };

            save_both();
            clear_oauth_credentials_in(&service).unwrap();
            assert!(!oauth_is_saved());
            assert_eq!(
                load_api_key_in(&service).unwrap().as_deref(),
                Some("lin_api_demo")
            );

            save_both();
            clear_api_key_in(&service).unwrap();
            assert_eq!(load_api_key_in(&service).unwrap(), None);
            assert!(oauth_is_saved());
        }

        #[tokio::test]
        async fn a_stored_secret_that_is_not_text_is_reported_without_its_bytes() {
            let service = service("not-text");
            // Invalid UTF-8 makes keyring fail with `BadEncoding`, whose `Debug` output holds the
            // stored bytes.
            for account in [API_KEY_ACCOUNT, OAUTH_ACCOUNT] {
                keyring::Entry::new(&service, account)
                    .unwrap()
                    .set_secret(b"\xFFplanted-secret")
                    .unwrap();
            }
            let from_api_key = load_api_key_in(&service).unwrap_err();
            assert!(
                from_api_key
                    .chain()
                    .all(|cause| cause.downcast_ref::<keyring::Error>().is_none())
            );
            let from_store = KeyringStore::with_service(&service)
                .load()
                .await
                .unwrap_err();
            for text in [
                from_api_key.to_string(),
                format!("{from_api_key:#?}"),
                from_store.to_string(),
                format!("{from_store:?}"),
            ] {
                assert!(
                    !text.contains("planted-secret") && !text.contains("BadEncoding"),
                    "{text}"
                );
            }
        }
    }
}
