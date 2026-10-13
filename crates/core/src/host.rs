//! The host: what a desktop or headless program runs around the engine. It binds the IPC
//! endpoint (one host per user), resolves the executors' environment, opens the run records,
//! connects to Linear with the stored credentials, runs the engine's background loop and the IPC
//! server, and lets the Linear connection change while it runs. Without credentials, with a
//! failed connection or with changed Linear tools it still starts: the engine gets a
//! `Disconnected` stand-in whose checks fail with the reason (design section 13), and a failed
//! connection is retried every minute.
//!
//! Stopping ends things in an order that matters. The single-instance lock belongs to the IPC
//! server, so the server stops last, after the engine's loop has ended and every run has been
//! recorded: a second host let in earlier would mark the first one's runs failed and end their
//! executors. The engine and the server therefore have a shutdown token each.
//!
//! Locking: the connection's mutex is never held across an `.await`. The two task slots are
//! async mutexes held while their task is joined, and nothing a task does takes them, so a stop
//! that waits for the runs holds neither and another stop can take them.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::doctor::{ReadinessProbe, SystemReadiness};
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

/// A limit no job table reaches, so that `Engine::list_jobs` returns every record. It is
/// `i64::MAX`, the largest limit SQLite takes.
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
        // OAuth now: a saved key would only stand in the way of the next start.
        blocking(auth::clear_api_key).await?;
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
            // Stored OAuth credentials win at the next start; the user chose the key.
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
}

/// The Linear connection as the host reports it. It changes together with the engine's
/// connection, under one lock (`Engine::set_linear` only takes short locks of its own), so
/// `Host::linear_setup` never describes a connection the engine is not using.
struct Link {
    setup: LinearSetup,
    /// How many times the user changed the connection (login, API key, logout). A reconnect that
    /// began before such a change must not undo it.
    user_changes: u64,
}

struct Inner {
    engine: Engine,
    env: SharedEnv,
    paths: AppPaths,
    connector: Arc<dyn Connector>,
    link: Mutex<Link>,
    /// Ends the engine's loop, which tells the runs in progress to stop, and the retries of a
    /// failed connection.
    engine_shutdown: CancellationToken,
    /// Ends the IPC server, whose lock keeps a second host out. Cancelled only once the engine's
    /// loop and its runs have ended.
    ipc_shutdown: CancellationToken,
    worker: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    ipc: tokio::sync::Mutex<Option<JoinHandle<Result<()>>>>,
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
/// already did. The cell's lock is held until the task has ended, so a call that comes meanwhile
/// waits for it too, and the handle leaves the cell only then, so a call that is dropped midway
/// leaves the waiting to the next one.
async fn join<T>(cell: &tokio::sync::Mutex<Option<JoinHandle<T>>>) -> Option<Result<T, JoinError>> {
    let mut cell = cell.lock().await;
    let handle = cell.as_mut()?;
    let ended = handle.await;
    *cell = None;
    Some(ended)
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

impl Host {
    /// Binds the IPC endpoint (fails with `AlreadyRunning` when another host serves this user),
    /// connects to Linear and starts the engine's loop and the IPC server.
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
        let (linear, setup) = connect_stored(connector.as_ref()).await;
        let engine = Engine::new(EngineDeps {
            config_path: paths.config_file(),
            store: Store::open(&paths.state_db())?,
            linear,
            readiness: parts
                .readiness
                .unwrap_or_else(|| Arc::new(SystemReadiness::new(READINESS_TTL, env.clone()))),
            job_runner: parts
                .job_runner
                .unwrap_or_else(|| Arc::new(ProcessJobRunner::new(env.clone()))),
            logs_dir: paths.logs_dir(),
        })?;
        let engine_shutdown = CancellationToken::new();
        let ipc_shutdown = CancellationToken::new();
        let ipc = tokio::spawn(server.run(engine.clone(), ipc_shutdown.clone()));
        // `run_forever`, not a bare loop: it ends the executors an earlier host left running
        // before anything new starts.
        let worker = {
            let engine = engine.clone();
            let shutdown = engine_shutdown.clone();
            tokio::spawn(async move { engine.run_forever(shutdown).await })
        };
        let host = Host {
            inner: Arc::new(Inner {
                engine,
                env,
                paths,
                connector,
                link: Mutex::new(Link {
                    setup,
                    user_changes: 0,
                }),
                engine_shutdown,
                ipc_shutdown,
                worker: tokio::sync::Mutex::new(Some(worker)),
                ipc: tokio::sync::Mutex::new(Some(ipc)),
            }),
        };
        tokio::spawn(
            host.clone()
                .retry_failed_connections(parts.retry_every.unwrap_or(RETRY_EVERY)),
        );
        Ok(host)
    }

    /// The engine the host runs; a desktop program calls it directly.
    pub fn engine(&self) -> &Engine {
        &self.inner.engine
    }

    /// The executors' environment as last resolved.
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
    /// and `setup` is what `linear_setup` reports. A reconnect that began earlier no longer
    /// applies its result.
    fn switch_by_user(&self, linear: Arc<dyn LinearApi>, setup: LinearSetup) {
        let mut link = self.link();
        link.user_changes += 1;
        self.inner.engine.set_linear(linear);
        link.setup = setup;
    }

    /// Connects again with the stored credentials (after a failure, or after an update fixed an
    /// interface change) and returns what came of it. If the user logged in, switched to a key or
    /// logged out while this was connecting, their change stands and this returns it instead.
    pub async fn reconnect(&self) -> LinearSetup {
        let user_changes = self.link().user_changes;
        let (linear, setup) = connect_stored(self.inner.connector.as_ref()).await;
        let mut link = self.link();
        if link.user_changes != user_changes {
            return link.setup.clone();
        }
        self.inner.engine.set_linear(linear);
        link.setup = setup.clone();
        setup
    }

    /// Logs in with the browser; `open_url` opens the authorization page. On success the engine
    /// uses the new connection at once. The viewer's name is cleaned (`clean_text`).
    pub async fn login(&self, open_url: impl Fn(&str) + Send + Sync + 'static) -> Result<Viewer> {
        let linear = self.inner.connector.login(Box::new(open_url)).await?;
        let viewer = display_viewer(linear.viewer().await?);
        self.switch_by_user(
            linear,
            LinearSetup::Connected {
                method: AuthMethod::OAuth,
            },
        );
        Ok(viewer)
    }

    /// Switches to a personal API key, kept only if Linear accepts it. A rejected key changes
    /// nothing. The viewer's name is cleaned (`clean_text`).
    pub async fn use_api_key(&self, key: &str) -> Result<Viewer> {
        let linear = self.inner.connector.api_key(key).await?;
        let viewer = display_viewer(linear.viewer().await?);
        self.switch_by_user(
            linear,
            LinearSetup::Connected {
                method: AuthMethod::ApiKey,
            },
        );
        Ok(viewer)
    }

    /// Forgets the stored credentials; checks fail until the next login.
    pub async fn logout(&self) -> Result<()> {
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
    pub async fn stop(&self, mode: StopMode) {
        let engine = &self.inner.engine;
        if mode == StopMode::WaitForRuns {
            // Nothing is locked while this waits: a `StopRuns` that comes meanwhile takes the
            // task slots below.
            engine.set_draining(true);
            engine.wait_idle().await;
        }
        // `run_forever` tells the runs still in progress to stop on its way out.
        self.inner.engine_shutdown.cancel();
        if let Some(Err(error)) = join(&self.inner.worker).await {
            tracing::error!(%error, "the engine task failed; stopping the runs from here");
            stop_runs(engine);
        }
        engine.wait_idle().await;
        // Last: the lock goes with the server, and nothing is left for a second host to disturb.
        self.inner.ipc_shutdown.cancel();
        match join(&self.inner.ipc).await {
            Some(Ok(Err(error))) => {
                tracing::warn!(error = %format!("{error:#}"), "the IPC server stopped with an error")
            }
            Some(Err(error)) => tracing::warn!(%error, "the IPC server task failed"),
            Some(Ok(Ok(()))) | None => {}
        }
    }

    /// Tries a failed connection again every `every` until it works, the user changes the
    /// connection, or the host stops.
    async fn retry_failed_connections(self, every: Duration) {
        let shutdown = &self.inner.engine_shutdown;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(every) => {}
            }
            if matches!(self.linear_setup(), LinearSetup::ConnectFailed { .. }) {
                let setup = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    setup = self.reconnect() => setup,
                };
                if let LinearSetup::ConnectFailed { message } = &setup {
                    tracing::debug!(%message, "Linear is still unreachable");
                }
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
        assert!(tokio::time::timeout(patience, join(&cell)).await.is_err());

        let mut first = Box::pin(join(&cell));
        let mut second = Box::pin(join(&cell));
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
        assert!(
            join(&cell).await.is_none(),
            "and nothing is left to wait for"
        );
    }

    #[test]
    fn every_setup_has_the_json_shape_the_panel_reads() {
        let shapes = [
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
