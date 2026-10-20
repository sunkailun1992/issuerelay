//! The host: what a desktop or headless program runs around the engine. It binds the IPC
//! endpoint (one host per user), resolves the executors' environment, opens the run records, runs
//! the engine's background loop and the IPC server, connects to Linear with the stored
//! credentials, and lets the Linear connection change while it runs.
//!
//! `start` does not wait for Linear: an offline start, or a keychain prompt nobody has answered
//! yet, must not keep the engine and the command line down. Until the first connection ends the
//! engine has a `Disconnected` stand-in ("connecting to Linear") and the setup reads
//! `Connecting`. Without credentials, with a failed connection or with changed Linear tools the
//! stand-in stays and its checks fail with the reason (design section 13); a failed connection is
//! retried every minute.
//!
//! The connection has two kinds of writers. The user's changes (login, API key, logout) are
//! serialized, one at a time, so that the keychain and the host end with the same login. The
//! attempts to connect with the stored credentials (the first one, the retries, `reconnect`) can
//! overlap: an attempt applies its result only if the user has not changed the connection since it
//! began and no attempt that began later has applied its own, so a slow or old attempt never
//! undoes a newer result.
//!
//! Stopping ends things in an order that matters. The single-instance lock belongs to the IPC
//! server, so the server stops last, after the engine's loop has ended and every run has been
//! recorded: a second host let in earlier would mark the first one's runs failed and end their
//! executors. The engine and the server therefore have a shutdown token each.
//!
//! Locking: the connection's mutex is never held across an `.await`. The two task slots are
//! async mutexes held while their task is joined, and nothing a task does takes them, so a stop
//! that waits for the runs holds neither and another stop can take them. The lock for the user's
//! changes is held across their awaits, and `stop` never takes it.
use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::doctor::{self, LeanCheckPaths, ReadinessProbe, SystemReadiness};
use crate::engine::{Engine, EngineDeps, JobRunner, ProcessJobRunner, display_viewer};
use crate::env::{ExecEnv, SharedEnv};
use crate::ipc::server::IpcServer;
use crate::linear::mcp::{LinearError, McpLinear};
use crate::linear::{Disconnected, LinearApi, Viewer, auth};
use crate::model::JobState;
use crate::paths::AppPaths;
use crate::store::Store;
use crate::text::clean_text;

/// How long the readiness checks reuse a complete result (design section 6.4).
const READINESS_TTL: Duration = Duration::from_secs(5 * 60);

/// How often a failed connection is tried again.
const RETRY_EVERY: Duration = Duration::from_secs(60);

/// How long `stop` waits for the runs to end after the engine's task died. Stopping a run takes a
/// few seconds (the executor gets five to end the processes it started) and recording it one
/// more, but nothing is left to free the place of a run that the dead task had begun to register,
/// and a stop must not wait for that for ever.
const DEAD_ENGINE_WAIT: Duration = Duration::from_secs(60);

/// The most rows `Engine::list_jobs` can be asked for. `Store::list_recent` casts the count to
/// `i64` for SQLite's `LIMIT`, so on a 64-bit target this is `i64::MAX` and every record comes
/// back, where a count with the top bit set would turn negative.
const ALL_JOBS: usize = usize::MAX >> 1;

/// How the stored credentials authorize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    // snake_case would make it "o_auth".
    #[serde(rename = "oauth")]
    OAuth,
    ApiKey,
}

/// What the host knows about the Linear connection. Whether the last check worked is in
/// `Engine::status` (`linear_ok`, `viewer`, `last_error`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinearSetup {
    /// The first connection with the stored credentials has not ended: `Host::start` does not
    /// wait for it. The engine's checks fail with "connecting to Linear" meanwhile.
    Connecting,
    /// Connected with stored credentials or a login made in this session.
    Connected { method: AuthMethod },
    /// No stored credentials: the user has to log in.
    NotConfigured,
    /// Linear's tools or their arguments changed (design section 7): checks stay off until
    /// IssueRelay is updated.
    InterfaceChanged { message: String },
    /// Credentials exist but connecting failed; tried again every minute.
    ConnectFailed { message: String },
}

/// How `Host::stop` treats the runs in progress (design section 6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopMode {
    /// Start nothing new and wait for the runs in progress to finish.
    WaitForRuns,
    /// Stop the runs in progress (their process trees end) and wait until that is recorded.
    StopRuns,
}

/// What a login calls with the authorization URL, so the program can open or show the page. This
/// is `Box<dyn Fn(&str) + Send + Sync>` with the lifetime written out: `async_trait` turns an
/// elided lifetime in an argument into one of the whole method, and the callback could then only
/// be called with a string that outlives the whole call.
pub type OpenUrl = Box<dyn for<'a> Fn(&'a str) + Send + Sync>;

/// How the host reaches Linear. `SystemConnector` is the real one; tests use a fake.
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    /// Connects with stored credentials, OAuth first, then a saved API key; `None` when neither
    /// is stored.
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>>;
    /// Browser OAuth; `open_url` shows or opens the authorization page. Keeps the credentials.
    async fn login(&self, open_url: OpenUrl) -> Result<Arc<dyn LinearApi>>;
    /// Connects with a personal API key and keeps it, but only once it worked.
    async fn api_key(&self, key: &str) -> Result<Arc<dyn LinearApi>>;
    /// Forgets every stored credential.
    async fn logout(&self) -> Result<()>;
}

/// The system keychain and Linear's MCP server.
pub struct SystemConnector;

/// Runs a blocking keychain call on the blocking pool (it may show a system prompt).
async fn blocking<T: Send + 'static>(
    call: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(call)
        .await
        .context("the keychain call stopped unexpectedly")?
}

#[async_trait::async_trait]
impl Connector for SystemConnector {
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>> {
        if let Some(client) = auth::connect_stored(auth::LINEAR_MCP_URL).await? {
            return Ok(Some((
                Arc::new(McpLinear::new(client).await?),
                AuthMethod::OAuth,
            )));
        }
        if let Some(key) = blocking(auth::load_api_key).await? {
            let client = auth::connect_api_key(auth::LINEAR_MCP_URL, &key).await?;
            return Ok(Some((
                Arc::new(McpLinear::new(client).await?),
                AuthMethod::ApiKey,
            )));
        }
        Ok(None)
    }

    async fn login(&self, open_url: OpenUrl) -> Result<Arc<dyn LinearApi>> {
        let client = auth::login(auth::LINEAR_MCP_URL, open_url).await?;
        let linear = McpLinear::new(client).await?;
        // The OAuth credentials are saved by now, and they are tried first at every start. A saved
        // key that cannot be deleted is left behind unused, which is no reason to fail a login that
        // worked: it only warns.
        if let Err(error) = blocking(auth::clear_api_key).await {
            tracing::warn!(
                error = %format!("{error:#}"),
                "could not delete the saved API key after the login; the OAuth login stands"
            );
        }
        Ok(Arc::new(linear))
    }

    async fn api_key(&self, key: &str) -> Result<Arc<dyn LinearApi>> {
        let linear =
            McpLinear::new(auth::connect_api_key(auth::LINEAR_MCP_URL, key).await?).await?;
        linear
            .viewer()
            .await
            .context("Linear did not accept the API key")?;
        let key = key.to_string();
        blocking(move || {
            // Stored OAuth credentials win at the next start, and the user chose the key, so they
            // go first. If saving the key then fails, neither is stored and the next start has no
            // login. That order is on purpose: saving first would leave both stored when the
            // deletion fails, and the old OAuth login would silently win.
            auth::clear_oauth_credentials()?;
            auth::save_api_key(&key)
        })
        .await?;
        Ok(Arc::new(linear))
    }

    async fn logout(&self) -> Result<()> {
        blocking(auth::clear_credentials).await
    }
}

/// Replaceable parts, for tests; `HostParts::default()` uses the real ones.
#[derive(Default)]
pub struct HostParts {
    /// The executors' environment; resolved from the login shell and the system settings
    /// (`env::resolve`) when `None`.
    pub env: Option<SharedEnv>,
    /// Where the executor reports come from; `SystemReadiness` when `None`.
    pub readiness: Option<Arc<dyn ReadinessProbe>>,
    /// Starts the executor processes; `ProcessJobRunner` when `None`.
    pub job_runner: Option<Arc<dyn JobRunner>>,
    /// How often a failed connection is retried; `RETRY_EVERY` by default.
    pub retry_every: Option<Duration>,
    /// How long `stop` waits for the runs to end after the engine's task died; `DEAD_ENGINE_WAIT`
    /// by default.
    pub dead_engine_wait: Option<Duration>,
}

/// The Linear connection as the host reports it. It changes together with the engine's
/// connection, under one lock (`Engine::set_linear` only takes short locks of its own), so
/// `Host::linear_setup` never describes a connection the engine is not using.
struct Link {
    setup: LinearSetup,
    /// The number the next attempt to connect with the stored credentials takes.
    next_attempt: u64,
    /// Attempts numbered below this are out of date: the user changed the connection after they
    /// began, or an attempt that began later has applied its result.
    current_from: u64,
}

struct Inner {
    engine: Engine,
    env: SharedEnv,
    paths: AppPaths,
    connector: Arc<dyn Connector>,
    link: Mutex<Link>,
    /// Held for the whole of a `login`, `use_api_key` or `logout`: one change of the login at a
    /// time, so that the keychain and the host end with the same one. `stop` never takes it.
    user_actions: tokio::sync::Mutex<()>,
    /// Ends the engine's loop, which tells the runs in progress to stop, and the retries of a
    /// failed connection.
    engine_shutdown: CancellationToken,
    /// Ends the IPC server, whose lock keeps a second host out. Cancelled only once the engine's
    /// loop and its runs have ended.
    ipc_shutdown: CancellationToken,
    worker: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    /// Cancelled when the engine's task has ended, however it ended (a panic included), so that a
    /// stop waiting for the runs can tell that nothing will free a place the task left taken.
    worker_ended: CancellationToken,
    ipc: tokio::sync::Mutex<Option<JoinHandle<Result<()>>>>,
    /// Why the engine's task died, once `stop` has joined it and found it so, or why stopping the
    /// engine panicked.
    engine_failure: Mutex<Option<String>>,
    dead_engine_wait: Duration,
}

/// Cheap to clone; every clone is the same host. Call `stop` before the program ends: dropping
/// every clone does not stop the host.
#[derive(Clone)]
pub struct Host {
    inner: Arc<Inner>,
}

/// Whether connecting failed because Linear's tools or arguments changed.
fn is_interface_change(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<LinearError>(),
            Some(LinearError::MissingTools(_) | LinearError::MissingArguments(_))
        )
    })
}

/// Connects with the stored credentials. Never fails: without a connection the engine gets a
/// stand-in that says why.
async fn connect_stored(connector: &dyn Connector) -> (Arc<dyn LinearApi>, LinearSetup) {
    match connector.stored().await {
        Ok(Some((linear, method))) => (linear, LinearSetup::Connected { method }),
        Ok(None) => (
            Arc::new(Disconnected::new("not connected to Linear: log in first")),
            LinearSetup::NotConfigured,
        ),
        Err(e) => {
            let message = clean_text(&format!("{e:#}"));
            let setup = if is_interface_change(&e) {
                LinearSetup::InterfaceChanged {
                    message: message.clone(),
                }
            } else {
                LinearSetup::ConnectFailed {
                    message: message.clone(),
                }
            };
            (Arc::new(Disconnected::new(message)), setup)
        }
    }
}

/// Waits for the task in `cell` to end and returns how it ended; `None` when an earlier call
/// already did. `ended` sees how it ended while the cell is still locked, so that a call that
/// finds the cell empty can rely on whatever `ended` recorded. The cell's lock is held until the
/// task has ended, so a call that comes meanwhile waits for it too, and the handle leaves the
/// cell only then, so a call that is dropped midway leaves the waiting to the next one.
async fn join<T>(
    cell: &tokio::sync::Mutex<Option<JoinHandle<T>>>,
    ended: impl FnOnce(&Result<T, JoinError>),
) -> Option<Result<T, JoinError>> {
    let mut cell = cell.lock().await;
    let handle = cell.as_mut()?;
    let result = handle.await;
    ended(&result);
    *cell = None;
    Some(result)
}

/// Runs `future`, turning a panic while it is polled into `Err` with the panic's payload, as
/// `std::panic::catch_unwind` does for a closure. A future that panicked is not polled again; it
/// is dropped when this returns.
async fn catch_unwind<F: Future>(future: F) -> std::thread::Result<F::Output> {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(move |cx| {
        match std::panic::catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(payload) => Poll::Ready(Err(payload)),
        }
    })
    .await
}

/// What a panic said, when it said it with a string (`panic!` always does).
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

/// Tells the runs in progress to stop, through the engine's public calls. `run_forever` does this
/// itself on its way out; this is for when its task failed before it got there.
fn stop_runs(engine: &Engine) {
    let jobs = match engine.list_jobs(ALL_JOBS) {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "could not list the runs to stop them");
            return;
        }
    };
    for job in jobs
        .into_iter()
        .filter(|job| job.state == JobState::Running)
    {
        if let Err(e) = engine.cancel_job(job.id) {
            tracing::warn!(job_id = job.id, error = %format!("{e:#}"), "could not stop the run");
        }
    }
}

/// The readiness probe of a host that is not given one: the real checks, the lean ones with a
/// folder in the data folder and Codex's own configuration file.
fn system_readiness(paths: &AppPaths, env: SharedEnv) -> SystemReadiness {
    SystemReadiness::new(READINESS_TTL, env).with_lean_checks(LeanCheckPaths {
        empty_dir: paths.lean_check_dir(),
        codex_config: doctor::codex_config_path(),
    })
}

/// `CLAUDE_CONFIG_DIR` as the executors inherit it from the host (`ExecEnv` changes only the
/// PATH and the proxy), when it is set and not empty.
fn claude_config_dir() -> Option<std::path::PathBuf> {
    claude_config_dir_from(std::env::var_os("CLAUDE_CONFIG_DIR"))
}

fn claude_config_dir_from(value: Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    value
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
}

impl Host {
    /// Binds the IPC endpoint (fails with `AlreadyRunning` when another host serves this user),
    /// starts the engine's loop and the IPC server, and connects to Linear in the background:
    /// it returns without waiting for Linear, with `linear_setup` saying `Connecting` until the
    /// first connection has ended. (Resolving the executors' environment still happens before it
    /// returns, and can take a few seconds when it runs the login shell.)
    ///
    /// The bind comes first, before the run records are opened: `Engine::new` marks the runs a
    /// previous host left in progress as failed, and a host that is refused must not do that to
    /// the runs of the one that is running.
    pub async fn start(
        paths: AppPaths,
        connector: Arc<dyn Connector>,
        parts: HostParts,
    ) -> Result<Host> {
        let server = IpcServer::bind(&paths.ipc_endpoint()).await?;
        let env = match parts.env {
            Some(env) => env,
            None => crate::env::resolve().await.shared(),
        };
        let engine = Engine::new(EngineDeps {
            config_path: paths.config_file(),
            store: Store::open(&paths.state_db())?,
            linear: Arc::new(Disconnected::new("connecting to Linear")),
            readiness: parts
                .readiness
                .unwrap_or_else(|| Arc::new(system_readiness(&paths, env.clone()))),
            job_runner: parts
                .job_runner
                .unwrap_or_else(|| Arc::new(ProcessJobRunner::new(env.clone()))),
            logs_dir: paths.logs_dir(),
            user_home: dirs::home_dir(),
            claude_config_dir: claude_config_dir(),
        })?;
        let engine_shutdown = CancellationToken::new();
        let ipc_shutdown = CancellationToken::new();
        let ipc = tokio::spawn(server.run(engine.clone(), ipc_shutdown.clone()));
        // `run_forever`, not a bare loop: it ends the executors an earlier host left running
        // before anything new starts.
        let worker_ended = CancellationToken::new();
        let worker = {
            let engine = engine.clone();
            let shutdown = engine_shutdown.clone();
            // Dropped with the task's future, also while a panic unwinds it, and then cancels.
            let ended = worker_ended.clone().drop_guard();
            tokio::spawn(async move {
                let _ended = ended;
                engine.run_forever(shutdown).await
            })
        };
        let host = Host {
            inner: Arc::new(Inner {
                engine,
                env,
                paths,
                connector,
                link: Mutex::new(Link {
                    setup: LinearSetup::Connecting,
                    next_attempt: 0,
                    current_from: 0,
                }),
                user_actions: tokio::sync::Mutex::new(()),
                engine_shutdown,
                ipc_shutdown,
                worker: tokio::sync::Mutex::new(Some(worker)),
                worker_ended,
                ipc: tokio::sync::Mutex::new(Some(ipc)),
                engine_failure: Mutex::new(None),
                dead_engine_wait: parts.dead_engine_wait.unwrap_or(DEAD_ENGINE_WAIT),
            }),
        };
        tokio::spawn(
            host.clone()
                .connect_in_the_background(parts.retry_every.unwrap_or(RETRY_EVERY)),
        );
        Ok(host)
    }

    /// The engine the host runs; a desktop program calls it directly.
    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    /// The executors' environment as last resolved, with the real values: a proxy URL in it keeps
    /// its user name and password. It is for starting processes (the one-click setup commands
    /// run with it through `SystemRunner::with_env`) and for facts that hold no secret, such as
    /// whether a folder is on its PATH (`cli_path`); never show it or send it to the panel.
    /// `Engine::doctor` gives the form to show, with the proxy values masked
    /// (`ExecEnv::proxy_for_display`, which `ExecEnv`'s `Debug` uses too).
    pub fn env(&self) -> ExecEnv {
        self.inner
            .env
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Where the host keeps its files and its IPC endpoint.
    pub fn paths(&self) -> &AppPaths {
        &self.inner.paths
    }

    /// What the host knows about the Linear connection right now.
    pub fn linear_setup(&self) -> LinearSetup {
        self.link().setup.clone()
    }

    fn link(&self) -> MutexGuard<'_, Link> {
        self.inner
            .link
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The user changed the connection (login, API key, logout): the engine uses `linear` at once
    /// and `setup` is what `linear_setup` reports. Attempts to connect that began earlier no
    /// longer apply their result.
    fn switch_by_user(&self, linear: Arc<dyn LinearApi>, setup: LinearSetup) {
        let mut link = self.link();
        link.current_from = link.next_attempt;
        self.inner.engine.set_linear(linear);
        link.setup = setup;
    }

    /// Connects again with the stored credentials (after a failure, or after an update fixed an
    /// interface change) and returns what came of it.
    ///
    /// Attempts can overlap (the retry loop and a "reconnect" in the panel), and the user can
    /// change the connection meanwhile. An attempt applies its result unless the user has changed
    /// the connection since it began or an attempt that began later has applied its own; then it
    /// returns what holds now instead. So an old attempt never undoes a newer result, the user's
    /// change wins over any attempt, and an attempt that never ends holds nothing up.
    pub async fn reconnect(&self) -> LinearSetup {
        let attempt = {
            let mut link = self.link();
            link.next_attempt += 1;
            link.next_attempt - 1
        };
        let (linear, setup) = connect_stored(self.inner.connector.as_ref()).await;
        let mut link = self.link();
        if attempt < link.current_from {
            return link.setup.clone();
        }
        link.current_from = attempt + 1;
        self.inner.engine.set_linear(linear);
        link.setup = setup.clone();
        setup
    }

    /// Logs in with the browser; `open_url` opens the authorization page. The engine uses the new
    /// connection at once, and it does so before the viewer is read: the connector has changed the
    /// keychain by then, and the host follows it even if reading the viewer fails (the call then
    /// returns that error). The viewer's name is cleaned (`clean_text`).
    ///
    /// One change of the login at a time: this waits for a `login`, `use_api_key` or `logout` that
    /// is still going, and a login waits for the browser for up to ten minutes, so the panel
    /// should offer one change at a time. `stop` does not wait for it. Dropping the returned
    /// future before the browser has answered abandons the login.
    ///
    /// The errors are the connector's `anyhow` chains as they came, typed so that callers can
    /// `downcast_ref` them, and **not cleaned**: they can carry text from Linear or the system. The
    /// panel's command layer cleans them before it shows them.
    pub async fn login(&self, open_url: impl Fn(&str) + Send + Sync + 'static) -> Result<Viewer> {
        let _one_change_at_a_time = self.inner.user_actions.lock().await;
        let linear = self.inner.connector.login(Box::new(open_url)).await?;
        self.switch_by_user(
            linear.clone(),
            LinearSetup::Connected {
                method: AuthMethod::OAuth,
            },
        );
        Ok(display_viewer(linear.viewer().await?))
    }

    /// Switches to a personal API key, kept only if Linear accepts it. A rejected key changes
    /// nothing. Once the key is accepted the engine uses the new connection at once, before the
    /// viewer is read, as `login` does. The viewer's name is cleaned (`clean_text`).
    ///
    /// One change of the login at a time, as for `login`. The errors are the connector's `anyhow`
    /// chains as they came (typed, so `downcast_ref` works), **not cleaned**: the panel's command
    /// layer cleans them before it shows them.
    pub async fn use_api_key(&self, key: &str) -> Result<Viewer> {
        let _one_change_at_a_time = self.inner.user_actions.lock().await;
        let linear = self.inner.connector.api_key(key).await?;
        self.switch_by_user(
            linear.clone(),
            LinearSetup::Connected {
                method: AuthMethod::ApiKey,
            },
        );
        Ok(display_viewer(linear.viewer().await?))
    }

    /// Forgets the stored credentials; checks fail until the next login. If they cannot be
    /// deleted the call fails and nothing changes: they would be there at the next start.
    ///
    /// One change of the login at a time, as for `login`. The errors are the connector's `anyhow`
    /// chains as they came (typed, so `downcast_ref` works), **not cleaned**: the panel's command
    /// layer cleans them before it shows them.
    pub async fn logout(&self) -> Result<()> {
        let _one_change_at_a_time = self.inner.user_actions.lock().await;
        self.inner.connector.logout().await?;
        self.switch_by_user(
            Arc::new(Disconnected::new("logged out of Linear: log in again")),
            LinearSetup::NotConfigured,
        );
        Ok(())
    }

    /// Stops the host and returns once everything has ended, in this order: with `WaitForRuns`
    /// nothing new starts and the runs in progress finish; the engine's loop ends and tells the
    /// runs still going to stop; the runs' results are recorded; and only then the IPC server
    /// ends and with it the lock that keeps a second host out.
    ///
    /// May be called again, also at the same time as another call, and a later call may ask for
    /// more: `StopRuns` while a `WaitForRuns` is still waiting ends the runs and makes both calls
    /// return. A call that is dropped midway leaves the host as far along as it got (a dropped
    /// `WaitForRuns` leaves the engine draining), and the next call goes on from there.
    ///
    /// If the engine's task died nobody has told the runs to stop, so the host does (through the
    /// engine's public calls) and waits for them for a limited time only, then lets go of the
    /// lock anyway; a `WaitForRuns` that is waiting when the task dies goes the same way.
    /// `engine_failure` says that this happened. Even a panic while the engine is stopped does not
    /// keep the lock: the IPC server is stopped all the same.
    pub async fn stop(&self, mode: StopMode) {
        // A task that died in a section holding one of the engine's std locks (the store, the
        // running set) poisoned it, and the engine's calls in `stop_engine` that take it panic.
        // The server below must stop whatever happens there: its lock keeps every later host out.
        // So that part runs under `catch_unwind`. What follows takes none of the engine's locks:
        // `after_a_panic_in_stop` tells the engine to stop and waits a bounded while for its task,
        // and the rest touches only the IPC token and task slot. Going on after the panic so cannot
        // meet the state the panic left half changed, and a later `stop` still finds every slot as
        // this one left it (a tokio mutex is released, not poisoned, when a panic unwinds through
        // its guard).
        if let Err(panic) = catch_unwind(self.stop_engine(mode)).await {
            self.after_a_panic_in_stop(panic_message(&*panic)).await;
        }
        // Last: the lock goes with the server, and nothing is left for a second host to disturb.
        self.inner.ipc_shutdown.cancel();
        match join(&self.inner.ipc, |_| {}).await {
            Some(Ok(Err(error))) => {
                tracing::warn!(error = %format!("{error:#}"), "the IPC server stopped with an error")
            }
            Some(Err(error)) => tracing::warn!(%error, "the IPC server task failed"),
            Some(Ok(Ok(()))) | None => {}
        }
    }

    /// The part of `stop` before the IPC server: everything but the lock.
    async fn stop_engine(&self, mode: StopMode) {
        let engine = &self.inner.engine;
        if mode == StopMode::WaitForRuns {
            // Nothing is locked while this waits: a `StopRuns` that comes meanwhile takes the
            // task slots below. The wait also ends when the engine's task has ended: if it died
            // and left a place in the running set taken, nothing would ever free it, and the
            // branch for a dead engine below waits for a limited time only.
            engine.set_draining(true);
            tokio::select! {
                () = engine.wait_idle() => {}
                () = self.inner.worker_ended.cancelled() => {}
            }
        }
        // `run_forever` tells the runs still in progress to stop on its way out.
        self.inner.engine_shutdown.cancel();
        self.join_worker().await;
        if self.engine_failure().is_some() {
            stop_runs(engine);
            let wait = self.inner.dead_engine_wait;
            if tokio::time::timeout(wait, engine.wait_idle())
                .await
                .is_err()
            {
                tracing::error!(
                    seconds = wait.as_secs_f32(),
                    "runs are still marked as running after the engine task died; going on without them"
                );
            }
        } else {
            engine.wait_idle().await;
        }
    }

    /// Waits for the engine's task to end and, if it failed, records why (`engine_failure`).
    async fn join_worker(&self) {
        let _ = join(&self.inner.worker, |result| {
            if let Err(error) = result {
                tracing::error!(%error, "the engine task failed");
                *self
                    .inner
                    .engine_failure
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(error.to_string());
            }
        })
        .await;
    }

    /// What `stop` does when its engine part panicked with `panic`. The panic can come before the
    /// engine was told to stop (a `WaitForRuns` whose wait met a poisoned running lock), so it is
    /// told now, which ends its loop and the retries of the connection too. Then its task is waited
    /// for, a bounded while (`dead_engine_wait`), so that `engine_failure` names the engine's own
    /// failure when it had one: a task that died in a locked section is why the stop met a
    /// poisoned lock. The stop's panic is recorded only when nothing else was. Nothing here takes
    /// an engine lock, so it cannot panic the way the engine part did.
    async fn after_a_panic_in_stop(&self, panic: &str) {
        let why = format!("stopping the engine panicked: {panic}");
        tracing::error!("{why}; the IPC server is stopped all the same");
        self.inner.engine_shutdown.cancel();
        let wait = self.inner.dead_engine_wait;
        if tokio::time::timeout(wait, self.join_worker())
            .await
            .is_err()
        {
            tracing::error!(
                seconds = wait.as_secs_f32(),
                "the engine task did not end after the stop panicked; going on without it"
            );
        }
        self.inner
            .engine_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(why);
    }

    /// Why the engine's task died, once `stop` has found it so: nothing polled Linear or started
    /// runs after that, so the program that runs the host should say so and end with a failure.
    /// Also set when stopping the engine panicked. `None` while the engine ran until it was
    /// stopped.
    pub fn engine_failure(&self) -> Option<String> {
        self.inner
            .engine_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Makes the first connection with the stored credentials, which `start` does not wait for,
    /// and then tries a failed connection again every `every` until it works, the user changes the
    /// connection, or the host stops. Each attempt is `reconnect`, so it gives way to the user's
    /// changes and to attempts that began later; only `ConnectFailed` is retried.
    ///
    /// The stop is looked at first, every time (`biased`): an attempt reads the stored
    /// credentials from the keychain, which on macOS can wait for a prompt nobody answers, so a
    /// stop that came right after the start, before the first attempt, must not begin one.
    async fn connect_in_the_background(self, every: Duration) {
        let shutdown = &self.inner.engine_shutdown;
        let mut first = true;
        loop {
            if first || matches!(self.linear_setup(), LinearSetup::ConnectFailed { .. }) {
                first = false;
                let setup = tokio::select! {
                    biased;
                    _ = shutdown.cancelled() => return,
                    setup = self.reconnect() => setup,
                };
                if let LinearSetup::ConnectFailed { message } = &setup {
                    tracing::debug!(%message, "could not connect to Linear");
                }
            }
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(every) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_reads_stable_names() {
        assert_eq!(
            serde_json::to_string(&AuthMethod::OAuth).unwrap(),
            "\"oauth\""
        );
        assert_eq!(
            serde_json::to_string(&AuthMethod::ApiKey).unwrap(),
            "\"api_key\""
        );
        let setup = serde_json::to_value(LinearSetup::ConnectFailed {
            message: "down".into(),
        })
        .unwrap();
        assert_eq!(
            setup,
            serde_json::json!({"kind": "connect_failed", "message": "down"})
        );
    }

    #[test]
    fn claude_config_dir_is_the_variable_when_it_is_set_and_not_empty() {
        assert_eq!(claude_config_dir_from(None), None);
        assert_eq!(claude_config_dir_from(Some("".into())), None);
        assert_eq!(
            claude_config_dir_from(Some("/srv/demo-claude".into())),
            Some(std::path::PathBuf::from("/srv/demo-claude"))
        );
    }

    // Unix only because the fake `claude` is a shell script; that a check command runs in the
    // folder it is given is tested on both platforms in tests/doctor.rs.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_lean_checks_of_a_host_run_in_a_folder_of_its_data_folder() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let paths = AppPaths::under(root.path());
        let bin = tempfile::tempdir().unwrap();
        let record = bin.path().join("lean-check-folder");
        let claude = bin.path().join("claude");
        let signed_in = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#;
        std::fs::write(
            &claude,
            format!(
                "#!/bin/sh\ncase \"$*\" in\n\
                 --version) echo 2.1.285 ;;\n\
                 '--setting-sources project,local auth status') pwd -P > '{}'; echo '{signed_in}' ;;\n\
                 'auth status') echo '{signed_in}' ;;\n\
                 'mcp list') echo 'linear-server: https://mcp.linear.app/mcp (HTTP) - Connected' ;;\n\
                 esac\n",
                record.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Only the folder with the fake `claude` is on the PATH, so no executor installed on the
        // machine running the test is found.
        let env = ExecEnv {
            path: Some(bin.path().to_string_lossy().into_owned()),
            ..ExecEnv::inherit()
        }
        .shared();
        // Executing a script right after writing it can fail with `ETXTBSY` when another test
        // thread forks in between: try again, with a new probe since a probe keeps its result.
        let mut reports = Vec::new();
        for _ in 0..5 {
            reports = system_readiness(&paths, env.clone())
                .executor_reports(false)
                .await;
            if reports[0].version.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(reports[0].ready_lean(), "{reports:?}");
        let folder = std::fs::read_to_string(&record).unwrap();
        assert_eq!(
            std::path::Path::new(folder.trim_end()),
            paths.lean_check_dir().canonicalize().unwrap()
        );
    }

    // `Host::stop` may be called again, also at the same time and also after a call was dropped.
    #[tokio::test]
    async fn every_caller_waits_for_the_task_and_a_caller_that_gives_up_does_not_lose_it() {
        let (finish, finished) = tokio::sync::oneshot::channel::<()>();
        let cell = tokio::sync::Mutex::new(Some(tokio::spawn(async move {
            finished.await.unwrap();
            7
        })));
        let patience = Duration::from_millis(20);

        // A caller that gives up midway leaves the task where it was.
        assert!(
            tokio::time::timeout(patience, join(&cell, |_| {}))
                .await
                .is_err()
        );

        let recorded = std::cell::Cell::new(None);
        let mut first = Box::pin(join(&cell, |result| {
            recorded.set(Some(*result.as_ref().unwrap()))
        }));
        let mut second = Box::pin(join(&cell, |_| {}));
        assert!(tokio::time::timeout(patience, &mut first).await.is_err());
        assert!(
            tokio::time::timeout(patience, &mut second).await.is_err(),
            "the second caller waits for the task too"
        );
        finish.send(()).unwrap();
        assert_eq!(
            first.await.unwrap().unwrap(),
            7,
            "the first caller gets the result"
        );
        assert!(second.await.is_none(), "the second one only had to wait");
        assert_eq!(
            recorded.get(),
            Some(7),
            "how the task ended was recorded before the second caller looked"
        );
        assert!(
            join(&cell, |_| {}).await.is_none(),
            "and nothing is left to wait for"
        );
    }

    /// A host around `worker`, a task the test controls in place of `run_forever`, with nothing
    /// else running: no IPC server and no attempts to connect. Its engine's loop is told to stop
    /// through `engine_shutdown`.
    fn bare_host(
        root: &std::path::Path,
        worker: JoinHandle<()>,
        engine_shutdown: CancellationToken,
    ) -> Host {
        let paths = AppPaths::under(root);
        let env = ExecEnv::inherit().shared();
        let engine = Engine::new(EngineDeps {
            config_path: paths.config_file(),
            store: Store::open_in_memory().unwrap(),
            linear: Arc::new(Disconnected::new("not connected to Linear")),
            readiness: Arc::new(SystemReadiness::new(READINESS_TTL, env.clone())),
            job_runner: Arc::new(ProcessJobRunner::new(env.clone())),
            logs_dir: paths.logs_dir(),
            user_home: None,
            claude_config_dir: None,
        })
        .unwrap();
        Host {
            inner: Arc::new(Inner {
                engine,
                env,
                paths,
                connector: Arc::new(SystemConnector),
                link: Mutex::new(Link {
                    setup: LinearSetup::NotConfigured,
                    next_attempt: 0,
                    current_from: 0,
                }),
                user_actions: tokio::sync::Mutex::new(()),
                engine_shutdown,
                ipc_shutdown: CancellationToken::new(),
                worker: tokio::sync::Mutex::new(Some(worker)),
                worker_ended: CancellationToken::new(),
                ipc: tokio::sync::Mutex::new(None),
                engine_failure: Mutex::new(None),
                dead_engine_wait: Duration::from_secs(5),
            }),
        }
    }

    // `stop` catches a panic of its engine part, and that panic can come before the engine was
    // told to stop: a `WaitForRuns` whose wait met a poisoned running lock. The engine is told then
    // (its loop and the retries of the connection end with it) and its task is waited for.
    #[tokio::test]
    async fn after_a_panic_in_stop_the_engine_is_told_to_stop_and_its_task_is_waited_for() {
        let root = tempfile::tempdir().unwrap();
        let shutdown = CancellationToken::new();
        // Stands for `run_forever`, which ends once it is told to stop.
        let worker = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.cancelled().await }
        });
        let host = bare_host(root.path(), worker, shutdown.clone());
        tokio::time::timeout(
            Duration::from_secs(5),
            host.after_a_panic_in_stop("running lock: PoisonError { .. }"),
        )
        .await
        .expect("the engine task was not waited for in time");
        assert!(shutdown.is_cancelled(), "the engine was not told to stop");
        assert!(
            host.inner.worker.lock().await.is_none(),
            "the engine task was not waited for"
        );
        // The engine ended as told, so the stop's panic is the one failure to report.
        assert_eq!(
            host.engine_failure().as_deref(),
            Some("stopping the engine panicked: running lock: PoisonError { .. }")
        );
    }

    // An engine task that died is why the stop met a poisoned lock: its own failure is what
    // `engine_failure` names.
    #[tokio::test]
    async fn after_a_panic_in_stop_the_engines_own_failure_is_the_one_reported() {
        let root = tempfile::tempdir().unwrap();
        let worker = tokio::spawn(async { panic!("the engine's own panic") });
        for _ in 0..1_000 {
            if worker.is_finished() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(worker.is_finished(), "the stand-in engine task never ended");
        let host = bare_host(root.path(), worker, CancellationToken::new());
        host.after_a_panic_in_stop("store lock: PoisonError { .. }")
            .await;
        let failure = host.engine_failure().expect("a failure is reported");
        assert!(failure.contains("the engine's own panic"), "{failure}");
    }

    #[tokio::test]
    async fn a_panic_in_a_future_comes_back_as_an_error() {
        assert_eq!(catch_unwind(async { 7 }).await.unwrap(), 7);
        // A panic after the future has waited once, as the engine's calls in `stop` would.
        let panicked = catch_unwind(async {
            tokio::task::yield_now().await;
            panic!("store lock: poisoned");
        })
        .await;
        let payload = panicked.unwrap_err();
        assert_eq!(panic_message(&*payload), "store lock: poisoned");
        let formatted = catch_unwind(async { panic!("job {} failed", 3) }).await;
        assert_eq!(panic_message(&*formatted.unwrap_err()), "job 3 failed");
        let other = catch_unwind(async { std::panic::panic_any(7_u8) }).await;
        assert_eq!(panic_message(&*other.unwrap_err()), "no message");
    }

    #[test]
    fn every_setup_has_the_json_shape_the_panel_reads() {
        let shapes = [
            (
                LinearSetup::Connecting,
                serde_json::json!({"kind": "connecting"}),
            ),
            (
                LinearSetup::Connected {
                    method: AuthMethod::OAuth,
                },
                serde_json::json!({"kind": "connected", "method": "oauth"}),
            ),
            (
                LinearSetup::Connected {
                    method: AuthMethod::ApiKey,
                },
                serde_json::json!({"kind": "connected", "method": "api_key"}),
            ),
            (
                LinearSetup::NotConfigured,
                serde_json::json!({"kind": "not_configured"}),
            ),
            (
                LinearSetup::InterfaceChanged {
                    message: "changed".into(),
                },
                serde_json::json!({"kind": "interface_changed", "message": "changed"}),
            ),
            (
                LinearSetup::ConnectFailed {
                    message: "down".into(),
                },
                serde_json::json!({"kind": "connect_failed", "message": "down"}),
            ),
        ];
        for (setup, json) in shapes {
            assert_eq!(serde_json::to_value(&setup).unwrap(), json);
            assert_eq!(serde_json::from_value::<LinearSetup>(json).unwrap(), setup);
        }
    }
}
