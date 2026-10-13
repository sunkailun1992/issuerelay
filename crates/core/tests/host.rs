//! The host around the engine: Linear connection changes, the single-instance lock and the two
//! ways of stopping. Starting and stopping the host, and every wait for a run, have a limit, so a
//! deadlock fails a test instead of hanging the run.
mod common;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use common::{
    engine_kit::{FakeReadiness, ScriptedRunner},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    config::{Binding, ExecutorKind},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::JobRunner,
    env::ExecEnv,
    executor::CommandSpec,
    host::{AuthMethod, Connector, Host, HostParts, LinearSetup, OpenUrl, StopMode},
    ipc::{
        client::{self, NotRunning},
        server::AlreadyRunning,
    },
    linear::{LinearApi, mcp::LinearError},
    model::JobState,
    paths::AppPaths,
    runner::{ExitKind, RunLimits, RunOutput, SpawnHook},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// What the fake connector answers.
#[derive(Clone)]
enum Stored {
    Nothing,
    Works(FakeLinear, AuthMethod),
    Fails(&'static str),
    InterfaceChanged,
    /// Linear no longer accepts an argument, and something between added context to the error.
    ArgumentsChanged,
    /// The inner answer, but only once the test adds a permit to the gate.
    After(Arc<Semaphore>, Box<Stored>),
}

struct FakeConnector {
    stored: Mutex<Vec<Stored>>,
    stored_calls: Mutex<usize>,
    linear: FakeLinear,
    logouts: Mutex<usize>,
    /// `logout` fails like a locked keychain does.
    logout_fails: AtomicBool,
}

impl FakeConnector {
    /// `stored` answers the first calls to `stored()` in order; the last answer repeats.
    fn new(stored: Vec<Stored>) -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(stored),
            stored_calls: Mutex::new(0),
            linear: FakeLinear::new("user-1"),
            logouts: Mutex::new(0),
            logout_fails: AtomicBool::new(false),
        })
    }

    fn stored_calls(&self) -> usize {
        *self.stored_calls.lock().unwrap()
    }
}

#[async_trait::async_trait]
impl Connector for FakeConnector {
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>> {
        let answer = {
            let mut stored = self.stored.lock().unwrap();
            *self.stored_calls.lock().unwrap() += 1;
            if stored.len() > 1 {
                stored.remove(0)
            } else {
                stored[0].clone()
            }
        };
        let answer = match answer {
            Stored::After(gate, inner) => {
                gate.acquire().await.expect("the gate closed").forget();
                *inner
            }
            other => other,
        };
        match answer {
            Stored::Nothing => Ok(None),
            Stored::Works(linear, method) => Ok(Some((Arc::new(linear), method))),
            Stored::Fails(message) => Err(anyhow!(message)),
            Stored::InterfaceChanged => {
                Err(LinearError::MissingTools(vec!["list_issues".into()]).into())
            }
            Stored::ArgumentsChanged => {
                Err(anyhow::Error::from(LinearError::MissingArguments(vec![
                    "list_issues.assignee".into(),
                ]))
                .context("connecting to Linear"))
            }
            Stored::After(..) => unreachable!("answers are not nested"),
        }
    }

    async fn login(&self, open_url: OpenUrl) -> Result<Arc<dyn LinearApi>> {
        open_url("https://mcp.linear.app/authorize?state=demo");
        Ok(Arc::new(self.linear.clone()))
    }

    async fn api_key(&self, key: &str) -> Result<Arc<dyn LinearApi>> {
        if key == "lin_api_good" {
            Ok(Arc::new(self.linear.clone()))
        } else {
            Err(anyhow!("Linear rejected the API key"))
        }
    }

    async fn logout(&self) -> Result<()> {
        *self.logouts.lock().unwrap() += 1;
        if self.logout_fails.load(Ordering::SeqCst) {
            return Err(anyhow!("system keychain: keychain is locked"));
        }
        Ok(())
    }
}

/// Waits until `condition` holds; the test fails after five seconds instead of hanging.
async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits for `future` at most five seconds, so a deadlock fails the test.
async fn within_five_secs<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .unwrap_or_else(|_| panic!("{what}"))
}

/// The parts every test host uses: no login shell, always-ready executors, a quick retry.
fn parts(runner: Option<Arc<dyn JobRunner>>) -> HostParts {
    HostParts {
        env: Some(ExecEnv::inherit().shared()),
        readiness: Some(Arc::new(FakeReadiness)),
        job_runner: runner,
        retry_every: Some(Duration::from_millis(50)),
    }
}

/// Starts a host for the user whose files are under `root`, within the limit.
async fn start(root: &Path, connector: Arc<FakeConnector>, parts: HostParts) -> Result<Host> {
    within_five_secs(
        "the host never started",
        Host::start(AppPaths::under(root), connector, parts),
    )
    .await
}

/// A host under a temporary root with fake readiness and, if given, a scripted runner.
async fn host(
    connector: Arc<FakeConnector>,
    runner: Option<Arc<dyn JobRunner>>,
) -> (Host, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let host = start(root.path(), connector, parts(runner)).await.unwrap();
    (host, root)
}

/// Stops the runs and the host, within the limit.
async fn stop_now(host: &Host) {
    within_five_secs("the stop never returned", host.stop(StopMode::StopRuns)).await;
}

#[tokio::test]
async fn without_credentials_the_host_runs_and_waits_for_a_login() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    let (host, _root) = host(connector.clone(), None).await;
    assert_eq!(host.linear_setup(), LinearSetup::NotConfigured);
    let error = host.engine().poll_once().await.unwrap_err();
    assert!(error.to_string().contains("log in first"), "{error:#}");
    let opened = Arc::new(Mutex::new(None));
    let seen = opened.clone();
    let viewer = host
        .login(move |url| *seen.lock().unwrap() = Some(url.to_string()))
        .await
        .unwrap();
    assert_eq!(viewer.id, "user-1");
    assert!(
        opened
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .starts_with("https://mcp.linear.app/")
    );
    assert_eq!(
        host.linear_setup(),
        LinearSetup::Connected {
            method: AuthMethod::OAuth
        }
    );
    host.engine().poll_once().await.unwrap();
    stop_now(&host).await;
}

#[tokio::test]
async fn changed_linear_tools_are_told_apart_from_other_failures() {
    let (host, _root) = host(FakeConnector::new(vec![Stored::InterfaceChanged]), None).await;
    assert!(matches!(
        host.linear_setup(),
        LinearSetup::InterfaceChanged { .. }
    ));
    stop_now(&host).await;
}

#[tokio::test]
async fn changed_linear_arguments_count_as_a_changed_interface_too() {
    let (host, _root) = host(FakeConnector::new(vec![Stored::ArgumentsChanged]), None).await;
    assert!(matches!(
        host.linear_setup(),
        LinearSetup::InterfaceChanged { ref message } if message.contains("list_issues.assignee")
    ));
    stop_now(&host).await;
}

#[tokio::test]
async fn a_failed_connection_is_retried_until_it_works() {
    let linear = FakeLinear::new("user-1");
    let connector = FakeConnector::new(vec![
        Stored::Fails("network is down"),
        Stored::Works(linear, AuthMethod::ApiKey),
    ]);
    let (host, _root) = host(connector, None).await;
    assert!(matches!(
        host.linear_setup(),
        LinearSetup::ConnectFailed { ref message } if message.contains("network is down")
    ));
    eventually("never reconnected", || {
        host.linear_setup()
            == LinearSetup::Connected {
                method: AuthMethod::ApiKey,
            }
    })
    .await;
    host.engine().poll_once().await.unwrap();
    stop_now(&host).await;
}

#[tokio::test]
async fn reconnecting_after_an_update_picks_up_the_fixed_interface() {
    let linear = FakeLinear::new("user-1");
    let connector = FakeConnector::new(vec![
        Stored::InterfaceChanged,
        Stored::Works(linear, AuthMethod::OAuth),
    ]);
    let (host, _root) = host(connector, None).await;
    assert!(matches!(
        host.linear_setup(),
        LinearSetup::InterfaceChanged { .. }
    ));
    assert!(host.engine().poll_once().await.is_err());
    let connected = LinearSetup::Connected {
        method: AuthMethod::OAuth,
    };
    assert_eq!(host.reconnect().await, connected);
    assert_eq!(host.linear_setup(), connected);
    host.engine().poll_once().await.unwrap();
    stop_now(&host).await;
}

#[tokio::test]
async fn an_api_key_and_a_logout_switch_the_connection() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    let (host, _root) = host(connector.clone(), None).await;
    assert!(host.use_api_key("lin_api_bad").await.is_err());
    assert_eq!(
        host.linear_setup(),
        LinearSetup::NotConfigured,
        "a rejected key changes nothing"
    );
    host.use_api_key("lin_api_good").await.unwrap();
    assert_eq!(
        host.linear_setup(),
        LinearSetup::Connected {
            method: AuthMethod::ApiKey
        }
    );
    host.logout().await.unwrap();
    assert_eq!(*connector.logouts.lock().unwrap(), 1);
    assert_eq!(host.linear_setup(), LinearSetup::NotConfigured);
    let error = host.engine().poll_once().await.unwrap_err();
    assert!(error.to_string().contains("logged out"), "{error:#}");
    stop_now(&host).await;
}

// Credentials that could not be deleted are still there at the next start, so the host must not
// pretend to be logged out.
#[tokio::test]
async fn a_logout_that_fails_changes_nothing() {
    let linear = FakeLinear::new("user-1");
    let connector = FakeConnector::new(vec![Stored::Works(linear, AuthMethod::OAuth)]);
    connector.logout_fails.store(true, Ordering::SeqCst);
    let (host, _root) = host(connector, None).await;
    let error = host.logout().await.unwrap_err();
    assert!(
        error.to_string().contains("keychain is locked"),
        "{error:#}"
    );
    assert_eq!(
        host.linear_setup(),
        LinearSetup::Connected {
            method: AuthMethod::OAuth
        }
    );
    host.engine().poll_once().await.unwrap();
    stop_now(&host).await;
}

// The failure text can carry what a server sent; the panel gets it clean.
#[tokio::test]
async fn a_connection_error_reaches_the_panel_clean() {
    let connector = FakeConnector::new(vec![Stored::Fails("down\u{1b}[2J\u{202e}\nagain")]);
    let (host, _root) = host(connector, None).await;
    assert_eq!(
        host.linear_setup(),
        LinearSetup::ConnectFailed {
            message: "down [2J again".into()
        }
    );
    stop_now(&host).await;
}

// Linear's text reaches the panel through the returned viewer: ESC, a right-to-left override and
// a line break must not.
#[tokio::test]
async fn the_viewer_a_login_or_a_key_returns_has_a_clean_name() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    connector
        .linear
        .set_viewer_name("Jane\u{1b}[2J\u{202e} Doe\nSmith");
    let (host, _root) = host(connector, None).await;
    let by_login = host.login(|_| {}).await.unwrap();
    let by_key = host.use_api_key("lin_api_good").await.unwrap();
    for viewer in [by_login, by_key] {
        assert_eq!(viewer.name, "Jane [2J Doe Smith");
        assert_eq!(viewer.id, "user-1", "the id stays as Linear sent it");
    }
    stop_now(&host).await;
}

// The retry loop and the user can both change the connection. A reconnect that was slow must not
// bring back what the user has just logged out of.
#[tokio::test]
async fn a_slow_reconnect_does_not_undo_what_the_user_did_meanwhile() {
    let gate = Arc::new(Semaphore::new(0));
    let linear = FakeLinear::new("user-1");
    let connector = FakeConnector::new(vec![
        Stored::Nothing,
        Stored::After(
            gate.clone(),
            Box::new(Stored::Works(linear, AuthMethod::OAuth)),
        ),
    ]);
    let (host, _root) = host(connector.clone(), None).await;
    let reconnecting = tokio::spawn({
        let host = host.clone();
        async move { host.reconnect().await }
    });
    eventually(
        "the reconnect never asked for the stored credentials",
        || connector.stored_calls() == 2,
    )
    .await;

    host.use_api_key("lin_api_good").await.unwrap();
    host.logout().await.unwrap();
    gate.add_permits(1);

    let answered = within_five_secs("the reconnect never ended", reconnecting)
        .await
        .unwrap();
    assert_eq!(
        answered,
        LinearSetup::NotConfigured,
        "a reconnect that lost to the user reports what holds now"
    );
    assert_eq!(host.linear_setup(), LinearSetup::NotConfigured);
    assert!(
        host.engine().poll_once().await.is_err(),
        "the engine must not be connected after a logout"
    );
    stop_now(&host).await;
}

#[tokio::test]
async fn the_host_shows_the_environment_and_the_paths_it_runs_with() {
    let root = tempfile::tempdir().unwrap();
    let env = ExecEnv {
        path: Some("/demo/bin".into()),
        ..ExecEnv::inherit()
    };
    let host = start(
        root.path(),
        FakeConnector::new(vec![Stored::Nothing]),
        HostParts {
            env: Some(env.shared()),
            ..parts(None)
        },
    )
    .await
    .unwrap();
    assert_eq!(host.env().path.as_deref(), Some("/demo/bin"));
    assert_eq!(host.paths(), &AppPaths::under(root.path()));
    stop_now(&host).await;
}

#[tokio::test]
async fn the_host_answers_the_command_line_until_it_has_stopped() {
    let (host, _root) = host(FakeConnector::new(vec![Stored::Nothing]), None).await;
    let endpoint = host.paths().ipc_endpoint();
    let status = client::call(&endpoint, "status", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(status["paused"], false);
    stop_now(&host).await;
    let error = client::call(&endpoint, "status", serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<NotRunning>().is_some(), "{error:#}");
}

#[tokio::test]
async fn a_second_host_for_the_same_user_is_refused() {
    let connector = FakeConnector::new(vec![Stored::Nothing]);
    let root = tempfile::tempdir().unwrap();
    let first = start(root.path(), connector.clone(), parts(None))
        .await
        .unwrap();
    let error = start(root.path(), connector, parts(None))
        .await
        .err()
        .unwrap();
    assert!(
        error.downcast_ref::<AlreadyRunning>().is_some(),
        "{error:#}"
    );
    stop_now(&first).await;
}

/// Binds the project `proj-1` to a folder under `root`, adds its issue to `linear` and waits until
/// the host's engine runs it.
async fn run_a_job(host: &Host, root: &Path, linear: &FakeLinear) {
    let dir = root.join("repo-a");
    std::fs::create_dir_all(&dir).unwrap();
    host.engine()
        .add_binding(Binding {
            id: "proj-1".into(),
            project_id: "proj-1".into(),
            project_name: "Demo Project".into(),
            directory: dir,
            executor: None,
            permission: None,
            model: None,
            prompt_override: None,
        })
        .unwrap();
    linear.upsert(issue("1", "proj-1"));
    host.engine().poll_now();
    eventually("the run never started", || {
        host.engine().status().running > 0
    })
    .await;
}

/// A host with one bound project whose issue is running on `runner`.
async fn host_running_a_job(
    connector: Arc<FakeConnector>,
    runner: Arc<dyn JobRunner>,
    linear: &FakeLinear,
) -> (Host, tempfile::TempDir) {
    let (host, root) = host(connector, Some(runner)).await;
    run_a_job(&host, root.path(), linear).await;
    (host, root)
}

/// A host whose running job the scripted runner holds until `gate` gets a permit.
async fn host_with_a_running_job(gated: bool) -> (Host, Arc<ScriptedRunner>, tempfile::TempDir) {
    let linear = FakeLinear::new("user-1");
    let runner = Arc::new(ScriptedRunner::new(linear.clone(), gated));
    let connector = FakeConnector::new(vec![Stored::Works(linear.clone(), AuthMethod::OAuth)]);
    let (host, root) = host_running_a_job(connector, runner.clone(), &linear).await;
    (host, runner, root)
}

fn latest_job_state(host: &Host) -> JobState {
    host.engine().list_jobs(10).unwrap().remove(0).state
}

#[tokio::test]
async fn waiting_for_runs_lets_the_running_job_finish() {
    let (host, runner, _root) = host_with_a_running_job(true).await;
    let stopping = tokio::spawn({
        let host = host.clone();
        async move { host.stop(StopMode::WaitForRuns).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!stopping.is_finished(), "the stop waits for the run");
    assert!(host.engine().status().draining);
    // The command line still gets its answers while the host waits.
    let status = client::call(
        &host.paths().ipc_endpoint(),
        "status",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    assert_eq!(status["draining"], true);
    runner.gate.add_permits(1);
    within_five_secs("the stop never returned", stopping)
        .await
        .unwrap();
    assert_eq!(latest_job_state(&host), JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn stopping_runs_ends_the_running_job() {
    let (host, _runner, _root) = host_with_a_running_job(true).await;
    stop_now(&host).await;
    assert_eq!(latest_job_state(&host), JobState::Canceled);
    // Stopping again does nothing.
    stop_now(&host).await;
}

// The panel's quit flow: the user chooses to wait, then changes their mind. Neither call may
// wait on a lock the other holds.
#[tokio::test]
async fn stopping_now_ends_a_stop_that_is_still_waiting_for_the_runs() {
    let (host, _runner, _root) = host_with_a_running_job(true).await;
    let waiting = tokio::spawn({
        let host = host.clone();
        async move { host.stop(StopMode::WaitForRuns).await }
    });
    eventually("the waiting stop never began", || {
        host.engine().status().draining
    })
    .await;
    assert!(!waiting.is_finished(), "the run is still going");

    within_five_secs(
        "stopping now waited for the stop that waits",
        host.stop(StopMode::StopRuns),
    )
    .await;
    within_five_secs("the waiting stop never returned", waiting)
        .await
        .unwrap();
    assert_eq!(latest_job_state(&host), JobState::Canceled);
}

#[tokio::test]
async fn two_stops_at_once_both_return_after_the_run_has_ended() {
    let (host, _runner, _root) = host_with_a_running_job(true).await;
    let stops = [StopMode::StopRuns, StopMode::StopRuns].map(|mode| {
        let host = host.clone();
        tokio::spawn(async move { host.stop(mode).await })
    });
    for stop in stops {
        within_five_secs("a stop never returned", stop)
            .await
            .unwrap();
    }
    assert_eq!(latest_job_state(&host), JobState::Canceled);
}

/// A run that goes on after it is told to stop, until the test lets it end: a process tree that
/// takes a while to die.
struct LingeringRunner {
    /// Gets a permit when a run has been told to stop.
    told_to_stop: Semaphore,
    /// A run that was told to stop ends once it gets a permit.
    release: Semaphore,
}

#[async_trait::async_trait]
impl JobRunner for LingeringRunner {
    async fn run(
        &self,
        _spec: CommandSpec,
        _kind: ExecutorKind,
        _log_path: PathBuf,
        _limits: RunLimits,
        cancel: CancellationToken,
        // Not called: a made-up process must never reach the run records, where a later host
        // could try to end it.
        _on_spawn: SpawnHook,
    ) -> RunOutput {
        cancel.cancelled().await;
        self.told_to_stop.add_permits(1);
        self.release.acquire().await.expect("closed").forget();
        RunOutput {
            exit: ExitKind::Canceled,
            final_text: None,
        }
    }
}

// While the first host's runs are still ending, a second host must not get in: it would mark
// those runs failed and end their executors. The lock goes with the IPC server, which stops last.
#[tokio::test]
async fn a_second_host_is_refused_until_the_first_has_fully_stopped() {
    let linear = FakeLinear::new("user-1");
    let runner = Arc::new(LingeringRunner {
        told_to_stop: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let connector = FakeConnector::new(vec![Stored::Works(linear.clone(), AuthMethod::OAuth)]);
    let (first, root) = host_running_a_job(connector.clone(), runner.clone(), &linear).await;

    let stopping = tokio::spawn({
        let first = first.clone();
        async move { first.stop(StopMode::StopRuns).await }
    });
    within_five_secs(
        "the run was never told to stop",
        runner.told_to_stop.acquire(),
    )
    .await
    .expect("closed")
    .forget();

    let refused = start(root.path(), connector.clone(), parts(None))
        .await
        .err()
        .expect("a second host must be refused while the first is still stopping");
    assert!(
        refused.downcast_ref::<AlreadyRunning>().is_some(),
        "{refused:#}"
    );
    assert!(!stopping.is_finished(), "the stop waits for the run");
    assert_eq!(
        latest_job_state(&first),
        JobState::Running,
        "the refused host must not touch the run records"
    );

    runner.release.add_permits(1);
    within_five_secs("the stop never returned", stopping)
        .await
        .unwrap();
    assert_eq!(latest_job_state(&first), JobState::Canceled);

    let second = start(root.path(), connector, parts(None)).await.unwrap();
    stop_now(&second).await;
}

/// Readiness that panics once armed, as a bug in the engine's loop would.
struct PanickingReadiness {
    armed: AtomicBool,
    /// Set just before the panic.
    panicked: AtomicBool,
}

#[async_trait::async_trait]
impl ReadinessProbe for PanickingReadiness {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        if self.armed.load(Ordering::SeqCst) {
            self.panicked.store(true, Ordering::SeqCst);
            panic!("scripted panic");
        }
        FakeReadiness.executor_reports(fresh).await
    }
}

// Nobody told the runs to stop if the engine's task died, so the host does it itself instead of
// waiting for them to end on their own (up to an hour).
#[tokio::test]
async fn a_dead_engine_task_does_not_leave_the_stop_waiting_for_the_runs() {
    let linear = FakeLinear::new("user-1");
    let runner = Arc::new(ScriptedRunner::new(linear.clone(), true));
    let readiness = Arc::new(PanickingReadiness {
        armed: AtomicBool::new(false),
        panicked: AtomicBool::new(false),
    });
    let root = tempfile::tempdir().unwrap();
    let connector = FakeConnector::new(vec![Stored::Works(linear.clone(), AuthMethod::OAuth)]);
    let host = start(
        root.path(),
        connector,
        HostParts {
            readiness: Some(readiness.clone()),
            ..parts(Some(runner.clone()))
        },
    )
    .await
    .unwrap();
    run_a_job(&host, root.path(), &linear).await;

    // The next check of the engine's loop panics, and its task ends.
    readiness.armed.store(true, Ordering::SeqCst);
    host.engine().poll_now();
    eventually("the engine's loop never ran the check", || {
        readiness.panicked.load(Ordering::SeqCst)
    })
    .await;

    stop_now(&host).await;
    assert_eq!(latest_job_state(&host), JobState::Canceled);
}
