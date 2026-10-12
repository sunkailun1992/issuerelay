//! The engine: polls Linear, records and schedules jobs, starts executor runs, verifies the
//! result in Linear, keeps the status the panel, CLI and MCP read, and tells subscribers what
//! changed (`EngineEvent`).
//!
//! Locking: no std lock is ever held across an `.await`. They are taken and released within one
//! statement, except in short synchronous sections that must read and then write under one
//! guard: `update_config` (the config, and under it the file's last known bytes), `retry_job`,
//! the poll's step for each issue and `assign_if_queued` (the store), the registration of a run
//! and the final loop of `run_forever` (the running set), filling the viewer cache (the cache,
//! and under it a read of the connection) and the start backoff (the backoff table). The async
//! cycle lock is the one lock held across `.await`; it keeps a poll from interleaving with a
//! dispatch. A user's stop or retry can come at any time, also while a poll or a start waits for
//! Linear, so every write that follows such a wait is conditional on the row still being in the
//! state it was read in (a start's backoff: on the job not having been retried meanwhile). The
//! Linear connection can be replaced at any time too (`set_linear`): a poll, a dispatch and the
//! issue list each take the connection once and use only that one. Every await of a cycle comes
//! before the synchronous blocks that decide and write, so a cycle can be dropped at any of its
//! awaits without leaving anything half done: a stop does not wait for it (`run_forever`).
//! Events are sent with no lock held, except by `emit_job`, which reads a job's row and sends it
//! under one store guard so that the last event for a job carries its newest row (sending never
//! blocks and runs no subscriber code); its callers must not hold the store guard.
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use crate::config::{Binding, Config, ExecutorKind, Settings, TriggerNames};
use crate::doctor::{self, DoctorReport, ExecutorReport, ReadinessProbe};
use crate::env::{ExecEnv, SharedEnv};
use crate::executor::{
    self, CommandSpec,
    outcome::{self, Report},
};
use crate::linear::{LinearApi, Project, Viewer};
use crate::logs::{self, CleanupReport};
use crate::model::{Issue, Job, JobState, ProcessTag};
use crate::prompt::{self, PromptInput};
use crate::runner::{self, ExitKind, RunLimits, RunOutput, SpawnHook};
use crate::scheduler::{self, Candidate};
use crate::store::{Leftover, NewJob, Store};
use crate::text::clean_text;
use crate::trigger::{self, Eligibility, WaitReason, same_name};

/// How much of the end of a log `Engine::log_tail` reads.
const LOG_TAIL_WINDOW: u64 = 1024 * 1024;

/// How often the background loop deletes old run logs (also once when it starts).
const LOG_CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The pauses before the second and the third read that verifies a run's result in Linear. One
/// failed read (a network hiccup) must not fail a run that did its work.
const VERIFY_RETRY_PAUSES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];

/// How many events wait for a slow subscriber before it lags.
const EVENT_CAPACITY: usize = 256;

/// How long a job whose start failed is passed over: doubled after each further failure, up to
/// `START_RETRY_MAX`. Meanwhile the jobs behind it can start.
const START_RETRY_BASE: Duration = Duration::from_secs(30);
const START_RETRY_MAX: Duration = Duration::from_secs(5 * 60);

/// Starts one executor run and waits for it. The engine calls it from a task of its own.
#[async_trait::async_trait]
pub trait JobRunner: Send + Sync {
    /// Runs `spec` and reports how it ended, with the guarantees of `runner::run`: the whole
    /// process tree has ended when this returns, and also if the future is dropped before that.
    /// `cancel` stops the run early. `on_spawn` gets the executor's process right after it
    /// starts, for the record a host started after a crash uses (where one is kept, see
    /// `runner::process_tag`).
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        on_spawn: SpawnHook,
    ) -> RunOutput;
}

/// Runs real executor processes, each with the environment the readiness checks resolved.
pub struct ProcessJobRunner {
    env: SharedEnv,
}

impl ProcessJobRunner {
    pub fn new(env: SharedEnv) -> Self {
        Self { env }
    }
}

#[async_trait::async_trait]
impl JobRunner for ProcessJobRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        on_spawn: SpawnHook,
    ) -> RunOutput {
        let env = self
            .env
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        runner::run(spec, kind, &log_path, limits, cancel, &env, Some(on_spawn)).await
    }
}

/// Everything the engine talks to. Tests pass fakes; the hosts pass the real implementations.
pub struct EngineDeps {
    /// The settings and bindings file; a missing file means the defaults.
    pub config_path: PathBuf,
    /// The run records. The engine takes it over: no other engine may use the same database.
    pub store: Store,
    pub linear: Arc<dyn LinearApi>,
    /// Where the executor reports come from.
    pub readiness: Arc<dyn ReadinessProbe>,
    /// Starts the executor processes; `ProcessJobRunner` in production.
    pub job_runner: Arc<dyn JobRunner>,
    /// Holds one `<job id>.jsonl` log per job.
    pub logs_dir: PathBuf,
}

/// What the panel, CLI and MCP show about the engine right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// Polling and starting runs are switched off.
    pub paused: bool,
    /// The last check of Linear worked.
    pub linear_ok: bool,
    /// The Linear user, once a check has worked.
    pub viewer: Option<Viewer>,
    /// When Linear was last checked, whether or not that worked. A paused engine makes no check.
    pub last_poll_at: Option<DateTime<Utc>>,
    /// Why the job records cannot be read (the counts then show 0); otherwise why the last check
    /// failed, or why a job could not be started in the last round, which the next successful
    /// check clears.
    pub last_error: Option<String>,
    /// Runs in progress.
    pub running: usize,
    /// Jobs waiting for a free slot or directory.
    pub queued: usize,
    /// Jobs waiting for a binding, a directory or a ready executor.
    pub waiting: usize,
    /// Nothing new starts while the runs in progress finish (the "wait for completion" exit).
    #[serde(default)]
    pub draining: bool,
}

/// What changed in the engine, for the desktop panel. A subscriber that falls behind gets
/// `RecvError::Lagged` and should read the whole state again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
// Not boxed: the channel holds at most `EVENT_CAPACITY` events, so the job inline costs a few
// dozen KiB per channel, while a box would cost an allocation for every event and every clone
// a subscriber receives.
#[allow(clippy::large_enum_variant)]
pub enum EngineEvent {
    /// The status line may have changed: a check finished, monitoring was paused or resumed,
    /// draining was switched, Linear was reconnected, a job could not be started, or a run gave
    /// its slot back.
    Status,
    /// A job was recorded or changed; `job` is the row as stored now.
    Job { job: Job },
    /// The settings or the bindings were saved.
    Config,
}

/// One issue of the panel's issue list: an issue assigned to the user with the trigger label,
/// its newest local run, and whether it would be picked up now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueView {
    /// The issue, with the text a person reads (title, status, labels, project and parent)
    /// cleaned for display; the ids are as Linear sent them.
    pub issue: Issue,
    pub job: Option<Job>,
    pub status: IssueStatus,
}

/// Whether an issue of the issue list would be picked up now, and if not, why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IssueStatus {
    /// It meets every condition. `IssueView::job` says what happened with it so far.
    Eligible,
    /// Its status is not the trigger's Todo status; `state` is its status, cleaned for display.
    NotTodo { state: String },
    /// It does not match the trigger's label or assignee as IssueRelay compares them.
    NotCandidate,
    /// It waits for a binding, a folder or a ready executor.
    Waiting { reason: String },
    /// Monitoring is paused.
    Paused,
}

#[derive(Debug, Clone, Default)]
struct PollState {
    at: Option<DateTime<Utc>>,
    error: Option<String>,
    linear_ok: bool,
}

struct RunningJob {
    directory: PathBuf,
    cancel: CancellationToken,
}

/// A queued job whose start failed: how many times in a row, and until when it is passed over.
struct StartBackoff {
    failures: u32,
    until: tokio::time::Instant,
}

/// What `dispatch` remembers about failed starts.
#[derive(Default)]
struct Backoffs {
    /// Queued jobs whose start failed lately, passed over until their pause ends.
    held: HashMap<i64, StartBackoff>,
    /// How often the user retried each queued job. A start that fails while its job is stopped
    /// and retried is not held against the retry: the count changed during the start's wait.
    retries: HashMap<i64, u64>,
}

/// A run's place in the running set, held by the task that watches the run. `release` frees the
/// place once the result is recorded; dropping the slot frees it too, so a task that ends any
/// other way (a panic, an abort) cannot leave the folder occupied or `wait_idle` waiting. The row
/// of such a run is marked failed.
struct RunSlot {
    engine: Engine,
    job_id: i64,
    released: bool,
}

impl RunSlot {
    /// Frees the place after the result was recorded.
    fn release(&mut self) {
        if !self.released {
            self.released = true;
            self.engine.end_run(self.job_id);
        }
    }
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        if !self.released {
            self.engine.abandon_run(self.job_id);
        }
    }
}

struct Inner {
    config_path: PathBuf,
    config: RwLock<Config>,
    /// The configuration file's bytes as this engine last loaded or saved them; `None` while
    /// there is no file. Only `update_config` reads or replaces it, under the config write lock.
    config_file: Mutex<Option<Vec<u8>>>,
    store: Mutex<Store>,
    /// The connection to Linear; `set_linear` replaces it while the engine runs.
    linear: RwLock<Arc<dyn LinearApi>>,
    readiness: Arc<dyn ReadinessProbe>,
    job_runner: Arc<dyn JobRunner>,
    logs_dir: PathBuf,
    running: Mutex<HashMap<i64, RunningJob>>,
    /// The user a connection is authorized as, with the connection that said so. Only that
    /// connection's answer is ever kept: `set_linear` empties it, and an answer that arrives
    /// after its connection was replaced is not stored.
    viewer: Mutex<Option<(Arc<dyn LinearApi>, Viewer)>>,
    poll: Mutex<PollState>,
    poll_now: Notify,
    idle: Notify,
    /// Held for the whole of a poll and of a dispatch, so the two never interleave: a dispatch
    /// starts runs from rows a poll has just brought up to date, and a poll never rewrites a row
    /// a dispatch is starting. It does not stand between those two and the user, who can stop or
    /// retry a job at any moment; the store guard and conditional writes cover that.
    cycle: tokio::sync::Mutex<()>,
    /// The token of the running `run_forever`: once it is cancelled nothing new starts, also in
    /// a cycle that is already in progress.
    shutdown: Mutex<Option<CancellationToken>>,
    /// The executors that runs interrupted by an earlier host may have left running, as recorded
    /// when they started (`Store::leftovers`). `run_forever` takes them and ends them before
    /// anything new starts; each record stays on its row until then.
    leftovers: Mutex<Vec<Leftover>>,
    /// Tells subscribers what changed; see `EngineEvent`.
    events: broadcast::Sender<EngineEvent>,
    /// Set by `set_draining`: nothing new starts, the runs in progress go on.
    draining: AtomicBool,
    /// The queued jobs `dispatch` passes over after a failed start, and how often each queued job
    /// was retried (`Backoffs`).
    start_backoff: Mutex<Backoffs>,
}

/// Cheap to clone; every clone drives the same engine.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

/// The bytes of the configuration file; `None` when there is none.
fn read_config_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// One spelling per folder for the one-run-per-directory rule: resolves symlinks, `..` and,
/// on case-insensitive volumes, letter case. Falls back to the path as written.
fn canonical_dir(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

/// How long a job whose start failed `failures` times in a row is passed over: `START_RETRY_BASE`
/// after the first failure, doubled after each further one, at most `START_RETRY_MAX`.
fn start_pause(failures: u32) -> Duration {
    START_RETRY_BASE
        .saturating_mul(1u32 << failures.saturating_sub(1).min(10))
        .min(START_RETRY_MAX)
}

/// The issue as the issue list shows it: the text a person reads is Linear's and goes through
/// `clean_text`; the ids stay as they are, the identifier and the url were checked when parsed.
fn display_issue(issue: Issue) -> Issue {
    Issue {
        title: clean_text(&issue.title),
        state: clean_text(&issue.state),
        labels: issue.labels.iter().map(|label| clean_text(label)).collect(),
        project_name: issue.project_name.as_deref().map(clean_text),
        parent_identifier: issue.parent_identifier.as_deref().map(clean_text),
        ..issue
    }
}

/// Maps a finished run to a job state. Only a clean exit whose issue reached the testing
/// status counts as success; the AI's own report alone never does.
pub fn decide(
    exit: &ExitKind,
    state: Option<&str>,
    report: Option<&Report>,
    names: &TriggerNames,
) -> (JobState, Option<String>) {
    let is = |wanted: &str| state.is_some_and(|s| same_name(s, wanted));
    match exit {
        ExitKind::Canceled => (JobState::Canceled, Some("stopped by the user".into())),
        ExitKind::TimedOut => (JobState::Failed, Some("run exceeded the time limit".into())),
        ExitKind::Stalled => (
            JobState::Failed,
            Some("no output for too long; the run was stopped".into()),
        ),
        ExitKind::SpawnFailed(e) => (
            JobState::Failed,
            Some(format!("could not start the executor: {e}")),
        ),
        ExitKind::Exited(code) if *code != Some(0) => {
            let code = code.map_or_else(|| "unknown".to_string(), |c| c.to_string());
            (
                JobState::Failed,
                Some(format!("executor exited with code {code}")),
            )
        }
        ExitKind::Exited(_) if is(&names.in_testing) => (JobState::AwaitingAcceptance, None),
        ExitKind::Exited(_) if is(&names.done) => (JobState::Done, None),
        ExitKind::Exited(_) if matches!(report, Some(Report::NeedsInfo(_))) && is(&names.todo) => {
            (JobState::NeedsInfo, None)
        }
        ExitKind::Exited(_) => (
            JobState::Failed,
            Some(format!(
                "run finished but the issue status is {:?}, not {:?}",
                state.unwrap_or("unknown"),
                names.in_testing
            )),
        ),
    }
}

impl Engine {
    /// Loads the configuration and marks the runs a previous shutdown interrupted as failed; the
    /// executors those runs left running, and any an earlier host did not finish sweeping, are
    /// ended when `run_forever` starts. Only one engine may use a database at a time: a second one
    /// would mark the first one's runs failed and end their executors.
    pub fn new(deps: EngineDeps) -> Result<Self> {
        // The bytes are read before the file is loaded: if it changes in between, the first save
        // refuses instead of overwriting the newer text.
        let config_file = read_config_file(&deps.config_path)?;
        let config = Config::load(&deps.config_path)?;
        let interrupted = deps.store.recover_interrupted()?;
        if !interrupted.is_empty() {
            tracing::warn!(
                recovered = interrupted.len(),
                "runs interrupted by the previous shutdown were marked failed"
            );
        }
        // Read from the rows, not from the runs just recovered: a host that stopped before its
        // sweep was done left the records of the executors it had not dealt with.
        let leftovers = deps.store.leftovers()?;
        Ok(Self {
            inner: Arc::new(Inner {
                config_path: deps.config_path,
                config: RwLock::new(config),
                config_file: Mutex::new(config_file),
                store: Mutex::new(deps.store),
                linear: RwLock::new(deps.linear),
                readiness: deps.readiness,
                job_runner: deps.job_runner,
                logs_dir: deps.logs_dir,
                running: Mutex::new(HashMap::new()),
                viewer: Mutex::new(None),
                poll: Mutex::new(PollState::default()),
                poll_now: Notify::new(),
                idle: Notify::new(),
                cycle: tokio::sync::Mutex::new(()),
                shutdown: Mutex::new(None),
                leftovers: Mutex::new(leftovers),
                events: broadcast::channel(EVENT_CAPACITY).0,
                draining: AtomicBool::new(false),
                start_backoff: Mutex::new(Backoffs::default()),
            }),
        })
    }

    fn config(&self) -> Config {
        self.inner.config.read().expect("config lock").clone()
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.inner.store.lock().expect("store lock")
    }

    /// The connection to Linear in use now.
    fn linear(&self) -> Arc<dyn LinearApi> {
        self.inner
            .linear
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn emit(&self, event: EngineEvent) {
        // No subscriber is fine: the CLI-only host has none.
        let _ = self.inner.events.send(event);
    }

    /// Announces the job as it is stored now. The row is read and sent under one store guard, so
    /// two changes of one job announced from different tasks arrive in the order they were stored
    /// and the last event carries the newest row. This is the one place that sends while holding
    /// a lock: `broadcast::Sender::send` never blocks and runs no subscriber code. It takes the
    /// store lock, so the caller must not hold it; a poisoned lock is used as it is, because
    /// `abandon_run` calls this while unwinding.
    fn emit_job(&self, job_id: i64) {
        let store = self
            .inner
            .store
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let read = store.get(job_id);
        match read {
            Ok(Some(job)) => self.emit(EngineEvent::Job { job }),
            Ok(None) => {}
            Err(e) => {
                drop(store);
                tracing::warn!(job_id, error = %format!("{e:#}"), "could not read the job to announce its change");
            }
        }
    }

    fn draining(&self) -> bool {
        self.inner.draining.load(Ordering::SeqCst)
    }

    fn count(&self, state: JobState) -> Result<usize> {
        Ok(self.store().list_by_state(&[state])?.len())
    }

    /// The state the panel, CLI and MCP show. The job counts come from the store; if it cannot be
    /// read the counts show 0 and `last_error` says why.
    pub fn status(&self) -> StatusSnapshot {
        let paused = self.config().settings.paused;
        let poll = self.inner.poll.lock().expect("poll lock").clone();
        let viewer = self
            .inner
            .viewer
            .lock()
            .expect("viewer lock")
            .clone()
            .map(|(_, viewer)| viewer);
        let running = self.inner.running.lock().expect("running lock").len();
        let counts = self
            .count(JobState::Queued)
            .and_then(|queued| Ok((queued, self.count(JobState::WaitingBinding)?)));
        let (queued, waiting, store_error) = match counts {
            Ok((queued, waiting)) => (queued, waiting, None),
            Err(e) => (
                0,
                0,
                Some(clean_text(&format!("cannot read the job records: {e:#}"))),
            ),
        };
        StatusSnapshot {
            paused,
            linear_ok: poll.linear_ok,
            viewer,
            last_poll_at: poll.at,
            last_error: store_error.or(poll.error),
            running,
            queued,
            waiting,
            draining: self.draining(),
        }
    }

    /// Tells subscribers about every later change; see `EngineEvent`.
    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// Stops polling and starting runs, and remembers that across restarts. Runs already in
    /// progress continue.
    pub fn pause(&self) -> Result<()> {
        self.update_config(|c| c.settings.paused = true)?;
        self.emit(EngineEvent::Status);
        Ok(())
    }

    /// Undoes `pause` and checks at once.
    pub fn resume(&self) -> Result<()> {
        self.update_config(|c| c.settings.paused = false)?;
        self.emit(EngineEvent::Status);
        self.poll_now();
        Ok(())
    }

    /// While on, nothing new starts but the runs in progress go on; polls still record new
    /// issues. Not saved: it serves the "wait for completion" exit and ends with the host.
    pub fn set_draining(&self, on: bool) {
        self.inner.draining.store(on, Ordering::SeqCst);
        self.emit(EngineEvent::Status);
        if !on {
            self.poll_now();
        }
    }

    /// Uses `linear` for every later call: after a login, a logout or a new API key. The cached
    /// user is forgotten and a check runs at once. Runs in progress continue; their results are
    /// read through the new connection.
    pub fn set_linear(&self, linear: Arc<dyn LinearApi>) {
        // The connection first, then the cache: `viewer` checks the connection under the cache's
        // lock before it fills the cache.
        *self
            .inner
            .linear
            .write()
            .unwrap_or_else(PoisonError::into_inner) = linear;
        *self.inner.viewer.lock().expect("viewer lock") = None;
        self.emit(EngineEvent::Status);
        self.poll_now();
    }

    /// Wakes the background loop for an immediate check.
    pub fn poll_now(&self) {
        self.inner.poll_now.notify_one();
    }

    /// Validates the changed configuration, saves it and only then makes it current. The file
    /// must still hold what this engine last loaded or saved: a hand edit made while the host
    /// runs (in M1 the only way to change limits, intervals and templates) is not in memory, and
    /// saving would silently revert it. Such a change is refused and nothing is saved or changed.
    fn update_config(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut current = self.inner.config.write().expect("config lock");
        let mut next = current.clone();
        change(&mut next);
        next.validate()?;
        let mut known = self.inner.config_file.lock().expect("config file lock");
        if read_config_file(&self.inner.config_path)? != *known {
            bail!(
                "config.toml was changed while IssueRelay was running; restart the host to use it \
                 (nothing was saved)"
            );
        }
        // The text `Config::save` writes, to compare against before the next save.
        let text = toml::to_string_pretty(&next)?;
        next.save(&self.inner.config_path)?;
        *known = Some(text.into_bytes());
        *current = next;
        // Announced once both locks are free: a subscriber may read the configuration at once.
        drop(known);
        drop(current);
        self.emit(EngineEvent::Config);
        Ok(())
    }

    /// The current settings.
    pub fn settings(&self) -> Settings {
        self.config().settings
    }

    /// Validates and saves `settings`, then wakes the loop so that a shorter interval or a higher
    /// limit applies at once instead of after the current wait. `paused` keeps its current value:
    /// monitoring is switched with `pause` and `resume`, never by saving the settings page.
    pub fn update_settings(&self, settings: Settings) -> Result<()> {
        self.update_config(|c| {
            let paused = c.settings.paused;
            c.settings = settings;
            c.settings.paused = paused;
        })?;
        self.poll_now();
        Ok(())
    }

    /// The configured project bindings.
    pub fn bindings(&self) -> Vec<Binding> {
        self.config().bindings
    }

    /// Adds the binding, replacing any existing one for the same project or id.
    ///
    /// The result is checked like any configuration (`Config::validate`: an absolute folder,
    /// non-blank ids, no control characters in names), and a refused binding changes nothing.
    pub fn add_binding(&self, binding: Binding) -> Result<()> {
        self.update_config(|c| {
            c.bindings
                .retain(|b| b.project_id != binding.project_id && b.id != binding.id);
            c.bindings.push(binding);
        })
    }

    /// Removes the binding with this id; `false` when there was none.
    pub fn remove_binding(&self, id: &str) -> Result<bool> {
        let mut removed = false;
        self.update_config(|c| {
            let before = c.bindings.len();
            c.bindings.retain(|b| b.id != id);
            removed = c.bindings.len() != before;
        })?;
        Ok(removed)
    }

    /// The newest jobs first, at most `limit`.
    pub fn list_jobs(&self, limit: usize) -> Result<Vec<Job>> {
        self.store().list_recent(limit)
    }

    /// One job by id; `None` when there is none.
    pub fn job(&self, id: i64) -> Result<Option<Job>> {
        self.store().get(id)
    }

    /// The last `lines` lines of the job's log; empty while it has none, and also once the log
    /// is gone: a log deleted by the cleanup reads as empty. Reads at most the last
    /// `LOG_TAIL_WINDOW` bytes of the file, so a long log is cheap to ask for again and again.
    pub fn log_tail(&self, job_id: i64, lines: usize) -> Result<Vec<String>> {
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        let Some(path) = job.log_path else {
            return Ok(Vec::new());
        };
        let unreadable = || format!("cannot read {}", path.display());
        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(unreadable),
        };
        let start = file
            .metadata()
            .with_context(unreadable)?
            .len()
            .saturating_sub(LOG_TAIL_WINDOW);
        file.seek(SeekFrom::Start(start)).with_context(unreadable)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).with_context(unreadable)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut text: &str = &text;
        if start > 0 {
            // The read began inside the file, so its first line is partial: drop it.
            text = text.split_once('\n').map_or("", |(_, rest)| rest);
        }
        let all: Vec<&str> = text.lines().collect();
        Ok(all[all.len().saturating_sub(lines)..]
            .iter()
            .map(|line| line.to_string())
            .collect())
    }

    /// Deletes run logs past the retention settings (`log_retention_days`, `log_max_total_mb`),
    /// never the log of a run in progress. The background loop calls it when it starts and every
    /// `LOG_CLEANUP_INTERVAL`.
    pub async fn clean_logs(&self) -> Result<CleanupReport> {
        let settings = self.config().settings;
        let in_use: HashSet<PathBuf> = self
            .store()
            .list_by_state(&[JobState::Running])?
            .into_iter()
            .filter_map(|job| job.log_path)
            .collect();
        let dir = self.inner.logs_dir.clone();
        let keep = Duration::from_secs(settings.log_retention_days.saturating_mul(86_400));
        let max_total = settings.log_max_total_mb.saturating_mul(1024 * 1024);
        let report = tokio::task::spawn_blocking(move || {
            logs::cleanup(&dir, keep, max_total, &in_use, std::time::SystemTime::now())
        })
        .await
        .context("the log cleanup stopped unexpectedly")??;
        if report.removed > 0 {
            tracing::info!(
                removed = report.removed,
                freed_bytes = report.freed_bytes,
                "old run logs deleted"
            );
        }
        Ok(report)
    }

    /// The Linear projects, for choosing what to bind. Their names are Linear's text and are
    /// cleaned for display (`clean_text`); the ids stay as Linear sent them.
    pub async fn projects(&self) -> Result<Vec<Project>> {
        let projects = self.linear().projects().await?;
        Ok(projects
            .into_iter()
            .map(|project| Project {
                name: clean_text(&project.name),
                ..project
            })
            .collect())
    }

    /// The panel's issue list (spec 9.2): the issues assigned to the user with the trigger label,
    /// each with its newest local run and whether it would be picked up now.
    pub async fn issues(&self) -> Result<Vec<IssueView>> {
        let cfg = self.config();
        let names = &cfg.settings.trigger;
        let linear = self.linear();
        let me = self.viewer(&linear).await?;
        let issues = linear.labeled_issues(&names.label).await?;
        let (ready, _) = self.readiness().await;
        let mut views = Vec::with_capacity(issues.len());
        for issue in issues {
            let status = if !same_name(&issue.state, &names.todo) {
                IssueStatus::NotTodo {
                    state: clean_text(&issue.state),
                }
            } else if cfg.settings.paused {
                IssueStatus::Paused
            } else {
                // Looks at the bound folder on disk, so it runs before the store is locked.
                match trigger::evaluate(&issue, &cfg, &me.id, &ready) {
                    Eligibility::Eligible(_) => IssueStatus::Eligible,
                    Eligibility::Waiting(reason) => IssueStatus::Waiting {
                        reason: reason.describe(),
                    },
                    Eligibility::NotCandidate => IssueStatus::NotCandidate,
                }
            };
            let job = self.store().latest_for_issue(&issue.uuid)?;
            views.push(IssueView {
                issue: display_issue(issue),
                job,
                status,
            });
        }
        Ok(views)
    }

    /// The user `linear` is authorized as. The cache only answers for the connection that filled
    /// it, so a cycle that took its connection before `set_linear` replaced it keeps to that one
    /// connection, and the answer of a replaced connection is never kept.
    async fn viewer(&self, linear: &Arc<dyn LinearApi>) -> Result<Viewer> {
        let cached = self.inner.viewer.lock().expect("viewer lock").clone();
        if let Some((owner, viewer)) = cached
            && Arc::ptr_eq(&owner, linear)
        {
            return Ok(viewer);
        }
        let viewer = linear.viewer().await?;
        // Checked under the cache's lock: `set_linear` replaces the connection before it empties
        // the cache under this lock, so a connection replaced meanwhile cannot fill it.
        let mut cache = self.inner.viewer.lock().expect("viewer lock");
        if Arc::ptr_eq(linear, &self.linear()) {
            *cache = Some((linear.clone(), viewer.clone()));
        }
        Ok(viewer)
    }

    async fn readiness(&self) -> (HashSet<ExecutorKind>, Vec<ExecutorReport>) {
        let reports = self.inner.readiness.executor_reports(false).await;
        (doctor::ready_set(&reports), reports)
    }

    /// One cycle of the background loop: poll Linear, then start whatever may start.
    pub async fn tick(&self) -> Result<()> {
        self.poll_once().await?;
        self.dispatch().await?;
        Ok(())
    }

    /// Reads the candidate issues from Linear and brings the jobs in line with them: new issues
    /// are recorded, queued and waiting jobs are reassessed, and those that no longer match
    /// the trigger are canceled. Does nothing while paused.
    pub async fn poll_once(&self) -> Result<()> {
        let _cycle = self.inner.cycle.lock().await;
        let cfg = self.config();
        if cfg.settings.paused {
            return Ok(());
        }
        let result = self.poll_inner(&cfg).await;
        let state = PollState {
            at: Some(Utc::now()),
            error: result.as_ref().err().map(|e| clean_text(&format!("{e:#}"))),
            linear_ok: result.is_ok(),
        };
        *self.inner.poll.lock().expect("poll lock") = state;
        self.emit(EngineEvent::Status);
        result
    }

    async fn poll_inner(&self, cfg: &Config) -> Result<()> {
        let names = &cfg.settings.trigger;
        // One connection for the whole poll, also if `set_linear` replaces it meanwhile: the user
        // and the issues it is compared with must come from the same one.
        let linear = self.linear();
        let me = self.viewer(&linear).await?;
        let issues = linear.candidate_issues(&names.label, &names.todo).await?;
        let (ready, _) = self.readiness().await;
        // The jobs this poll records or changes are announced once no store guard is held, also
        // when a store error ends the poll part way.
        let mut changed = Vec::new();
        let result = self.reconcile(cfg, &me, &issues, &ready, &mut changed);
        for id in changed {
            self.emit_job(id);
        }
        result
    }

    /// Brings the jobs in line with the candidate `issues`: new ones are recorded, queued and
    /// waiting jobs are reassessed, and those whose issue is no longer a candidate are canceled.
    /// Pushes the id of every job it records or changes onto `changed`.
    fn reconcile(
        &self,
        cfg: &Config,
        me: &Viewer,
        issues: &[Issue],
        ready: &HashSet<ExecutorKind>,
        changed: &mut Vec<i64>,
    ) -> Result<()> {
        let names = &cfg.settings.trigger;
        let mut seen = HashSet::new();
        for issue in issues
            .iter()
            .filter(|i| trigger::is_candidate(i, names, &me.id))
        {
            seen.insert(issue.uuid.clone());
            let eligibility = trigger::evaluate(issue, cfg, &me.id, ready);
            // Read the latest run, decide and write under one store guard. A stop or a retry that
            // came in between would otherwise be overwritten, or leave the issue with two
            // active rows.
            let store = self.store();
            let latest = store.latest_for_issue(&issue.uuid)?;
            match latest {
                Some(job) if matches!(job.state, JobState::Queued | JobState::WaitingBinding) => {
                    if Self::reassess(&store, &job, cfg, eligibility)? {
                        changed.push(job.id);
                    }
                }
                Some(job) if job.state == JobState::Running => {}
                latest => {
                    if trigger::should_enqueue(latest.as_ref(), issue)
                        && let Some(id) = Self::enqueue(&store, issue, cfg, eligibility)?
                    {
                        changed.push(id);
                    }
                }
            }
        }
        let pending = self
            .store()
            .list_by_state(&[JobState::Queued, JobState::WaitingBinding])?;
        let waiting = [JobState::Queued, JobState::WaitingBinding];
        for job in pending.iter().filter(|j| !seen.contains(&j.issue_uuid)) {
            // Only while it still waits: the user may have stopped or retried it since the list
            // was read.
            let canceled = self.store().set_state_if(
                job.id,
                &waiting,
                JobState::Canceled,
                Some("no longer matches the trigger (label, assignee or status changed)"),
            )?;
            if canceled {
                changed.push(job.id);
            }
        }
        Ok(())
    }

    /// Records a job for the issue and returns its id; `None` when the issue is no candidate.
    fn enqueue(
        store: &Store,
        issue: &Issue,
        cfg: &Config,
        eligibility: Eligibility,
    ) -> Result<Option<i64>> {
        // The title is Linear's text and reaches the panel, the CLI and MCP; the prompt never
        // contains it. The store gets a cleaned copy of the issue.
        let issue = &Issue {
            title: clean_text(&issue.title),
            ..issue.clone()
        };
        let new = match &eligibility {
            Eligibility::Eligible(binding) => NewJob {
                issue,
                binding: Some(binding),
                executor: Some(cfg.executor_for(binding)),
                state: JobState::Queued,
                reason: None,
            },
            Eligibility::Waiting(reason) => NewJob {
                issue,
                binding: None,
                executor: None,
                state: JobState::WaitingBinding,
                reason: Some(reason.describe()),
            },
            Eligibility::NotCandidate => return Ok(None),
        };
        let job = store.insert(new)?;
        // The identifier and the state only: titles and summaries are Linear's and the AI's text.
        let message = if job.state == JobState::Queued {
            "job queued"
        } else {
            "job waiting"
        };
        tracing::info!(
            job_id = job.id,
            issue = %job.issue_identifier,
            state = job.state.as_str(),
            "{message}"
        );
        Ok(Some(job.id))
    }

    /// Brings a queued or waiting job in line with what the issue and the configuration say now,
    /// and says whether that changed the row; a row that already says so is not written. The
    /// caller holds the store guard it read the job under, so a stop or a retry cannot slip in
    /// between that read and these writes.
    fn reassess(store: &Store, job: &Job, cfg: &Config, eligibility: Eligibility) -> Result<bool> {
        match eligibility {
            Eligibility::Eligible(binding) => {
                let unchanged = job.state == JobState::Queued
                    && job.binding_id.as_deref() == Some(binding.id.as_str())
                    && job.directory.as_deref() == Some(binding.directory.as_path())
                    && job.executor == Some(cfg.executor_for(&binding));
                if unchanged {
                    return Ok(false);
                }
                store.assign(
                    job.id,
                    JobState::Queued,
                    Some(&binding),
                    Some(cfg.executor_for(&binding)),
                    None,
                )?;
                if job.state == JobState::WaitingBinding {
                    tracing::info!(
                        job_id = job.id,
                        issue = %job.issue_identifier,
                        state = JobState::Queued.as_str(),
                        "job queued"
                    );
                }
                Ok(true)
            }
            Eligibility::Waiting(reason) => {
                let reason = reason.describe();
                // Waiting jobs are looked at on every poll; one that waits for the same reason
                // as before is left as it is.
                let unchanged = job.state == JobState::WaitingBinding
                    && job.binding_id.is_none()
                    && job.directory.is_none()
                    && job.executor.is_none()
                    && job.reason.as_deref() == Some(reason.as_str());
                if unchanged {
                    return Ok(false);
                }
                store.assign(job.id, JobState::WaitingBinding, None, None, Some(&reason))?;
                Ok(true)
            }
            Eligibility::NotCandidate => Ok(false),
        }
    }

    /// Starts the queued jobs the scheduler picks (one run per directory, at most
    /// `max_concurrent` in total) and returns their ids. A job that cannot be started is logged and
    /// reported in the status, does not stop the others, and is passed over for a while
    /// (`start_pause`) so that the jobs behind it can start. Does nothing while paused, while
    /// draining or after a shutdown was requested.
    pub async fn dispatch(&self) -> Result<Vec<i64>> {
        let _cycle = self.inner.cycle.lock().await;
        let cfg = self.config();
        if cfg.settings.paused || self.stopping() || self.draining() {
            return Ok(Vec::new());
        }
        let queued = self.store().list_by_state(&[JobState::Queued])?;
        let now = tokio::time::Instant::now();
        let held_back: HashSet<i64> = {
            let mut backoff = self.inner.start_backoff.lock().expect("backoff lock");
            let queued_ids: HashSet<i64> = queued.iter().map(|job| job.id).collect();
            // Nothing here outlives the job's stay in the queue. No start is waiting now (the
            // cycle lock), so no retry count is dropped from under one.
            backoff.held.retain(|id, _| queued_ids.contains(id));
            backoff.retries.retain(|id, _| queued_ids.contains(id));
            backoff
                .held
                .iter()
                .filter(|(_, b)| b.until > now)
                .map(|(id, _)| *id)
                .collect()
        };
        let candidates: Vec<Candidate> = queued
            .iter()
            .filter_map(|job| {
                if held_back.contains(&job.id) {
                    return None;
                }
                // The binding's current directory in one canonical spelling, so two bindings that
                // name the same folder differently never run at the same time.
                let binding = cfg
                    .bindings
                    .iter()
                    .find(|b| job.binding_id.as_deref() == Some(b.id.as_str()))?;
                Some(Candidate {
                    job_id: job.id,
                    directory: canonical_dir(&binding.directory),
                    priority: job.priority,
                    issue_created_at: job.issue_created_at,
                })
            })
            .collect();
        let (running_dirs, running_count) = {
            let running = self.inner.running.lock().expect("running lock");
            (
                running
                    .values()
                    .map(|r| r.directory.clone())
                    .collect::<HashSet<_>>(),
                running.len(),
            )
        };
        let picks = scheduler::pick(
            &candidates,
            &running_dirs,
            running_count,
            cfg.settings.max_concurrent,
        );
        if picks.is_empty() {
            return Ok(Vec::new());
        }
        // One connection for the whole round, like a poll: the user and the issues re-read before
        // each start come from the same one.
        let linear = self.linear();
        let me = self.viewer(&linear).await?;
        let (ready, reports) = self.readiness().await;
        let mut started = Vec::new();
        let mut failure = None;
        for id in picks {
            // Each start works from the configuration as it is now, and a pause, a drain or a
            // shutdown that came meanwhile ends the round.
            let cfg = self.config();
            if cfg.settings.paused || self.stopping() || self.draining() {
                break;
            }
            let job = self.store().get(id);
            let retries = self.retries_of(id);
            let started_job = match job {
                Ok(Some(job)) => {
                    let label = job.issue_identifier.clone();
                    self.start_job(linear.as_ref(), &cfg, &me, &ready, &reports, job)
                        .await
                        .map_err(|e| (label, e))
                }
                Ok(None) => continue,
                Err(e) => Err((format!("job {id}"), e)),
            };
            match started_job {
                Ok(started_now) => {
                    self.inner
                        .start_backoff
                        .lock()
                        .expect("backoff lock")
                        .held
                        .remove(&id);
                    if started_now {
                        started.push(id);
                    }
                }
                Err((label, e)) => {
                    tracing::warn!(job_id = id, error = %format!("{e:#}"), "could not start the job");
                    failure = Some(format!("could not start {label}: {e:#}"));
                    self.hold_back(id, retries);
                }
            }
        }
        if let Some(message) = failure {
            self.inner.poll.lock().expect("poll lock").error = Some(clean_text(&message));
            self.emit(EngineEvent::Status);
        }
        Ok(started)
    }

    /// How often the user retried the job while it was queued; `hold_back` compares it.
    fn retries_of(&self, job_id: i64) -> u64 {
        self.inner
            .start_backoff
            .lock()
            .expect("backoff lock")
            .retries
            .get(&job_id)
            .copied()
            .unwrap_or(0)
    }

    /// Passes a job whose start failed over for a while (`start_pause`), so that the jobs behind
    /// it can start. The start waited for Linear, and the user may have stopped or retried the job
    /// meanwhile: the pause is recorded only while the row is still queued and the job was not
    /// retried since the start began (`retries`, read by `retries_of` before it), so a retry
    /// starts afresh. `retry_job` queues the row before it counts the retry and drops any pause,
    /// so a retry that lands after this check still undoes what it recorded.
    fn hold_back(&self, job_id: i64, retries: u64) {
        let queued = matches!(
            self.store().get(job_id),
            Ok(Some(job)) if job.state == JobState::Queued
        );
        let mut backoff = self.inner.start_backoff.lock().expect("backoff lock");
        let retried = backoff.retries.get(&job_id).copied().unwrap_or(0) != retries;
        if !queued || retried {
            return;
        }
        let now = tokio::time::Instant::now();
        let entry = backoff.held.entry(job_id).or_insert(StartBackoff {
            failures: 0,
            until: now,
        });
        entry.failures = entry.failures.saturating_add(1);
        entry.until = now + start_pause(entry.failures);
    }

    async fn start_job(
        &self,
        linear: &dyn LinearApi,
        cfg: &Config,
        me: &Viewer,
        ready: &HashSet<ExecutorKind>,
        reports: &[ExecutorReport],
        job: Job,
    ) -> Result<bool> {
        // The issue may have changed while queued; re-read it right before starting. The user can
        // stop the job, pause the engine or change the configuration while this waits, so every
        // write below is conditional on the row still being queued, and the run only starts if
        // the configuration is still the one it was built from.
        let current = linear.issue(&job.issue_uuid).await?;
        let Some(issue) =
            current.filter(|i| trigger::is_candidate(i, &cfg.settings.trigger, &me.id))
        else {
            let canceled = self.store().set_state_if(
                job.id,
                &[JobState::Queued],
                JobState::Canceled,
                Some("issue changed before the run started"),
            )?;
            if canceled {
                self.emit_job(job.id);
            }
            return Ok(false);
        };
        let binding = match trigger::evaluate(&issue, cfg, &me.id, ready) {
            Eligibility::Eligible(binding) => binding,
            Eligibility::Waiting(reason) => {
                let waiting = self.assign_if_queued(
                    job.id,
                    JobState::WaitingBinding,
                    None,
                    None,
                    Some(&reason.describe()),
                )?;
                if waiting {
                    self.emit_job(job.id);
                }
                return Ok(false);
            }
            Eligibility::NotCandidate => return Ok(false),
        };
        let kind = cfg.executor_for(&binding);
        // The scheduler picked this job for the folder of the binding recorded on its row. If the
        // issue has moved to another project since (or the binding changed folder or executor),
        // the run would go where the scheduler did not look, or the row would name an executor
        // that does not run: record the new binding on the row and let the next cycle schedule it
        // against the runs in progress.
        let unchanged = job.binding_id.as_deref() == Some(binding.id.as_str())
            && job.directory.as_deref().map(canonical_dir)
                == Some(canonical_dir(&binding.directory))
            && job.executor == Some(kind);
        if !unchanged {
            let reassigned =
                self.assign_if_queued(job.id, JobState::Queued, Some(&binding), Some(kind), None)?;
            if reassigned {
                self.emit_job(job.id);
                // The next dispatch should not wait for the timer.
                self.poll_now();
            }
            return Ok(false);
        }
        let program = reports
            .iter()
            .find(|r| r.kind == kind)
            .and_then(|r| r.program.clone());
        let Some(program) = program else {
            let reason = WaitReason::ExecutorNotReady(kind).describe();
            let waiting =
                self.assign_if_queued(job.id, JobState::WaitingBinding, None, None, Some(&reason))?;
            if waiting {
                self.emit_job(job.id);
            }
            return Ok(false);
        };
        let language = cfg.settings.resolved_comment_language();
        // A blank override falls back to the global template instead of silently dropping it.
        let template = binding
            .prompt_override
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| cfg.settings.prompt_template.clone());
        let prompt_text = prompt::render(&PromptInput {
            identifier: &issue.identifier,
            url: &issue.url,
            names: &cfg.settings.trigger,
            comment_language: &language,
            template: &template,
        });
        let spec = executor::build(
            kind,
            program,
            cfg.permission_for(&binding),
            binding.model.as_deref(),
            &binding.directory,
            prompt_text,
        );
        // A manual retry reuses the row, so its log replaces the previous attempt's log.
        let log_path = self.inner.logs_dir.join(format!("{}.jsonl", job.id));
        // Right before the run is registered: a pause or a changed binding that came while the
        // issue was being read means this start must not happen. The job stays queued and the
        // next cycle, which should not wait for the timer, starts it from the new configuration,
        // or not at all.
        if self.config() != *cfg {
            self.poll_now();
            return Ok(false);
        }
        // Register the run before marking it started, so a stop request in between finds the token.
        let cancel = CancellationToken::new();
        let directory = canonical_dir(&binding.directory);
        {
            // The check and the insert happen under one lock. The scheduler's view of the running
            // set is older than this moment, so the folder and the total are checked again. The
            // shutdown is checked under the lock too, which `run_forever` also takes to stop the
            // runs: a run is either registered before that and gets stopped, or sees the shutdown.
            // A drain that began meanwhile stops the start as well.
            let mut running = self.inner.running.lock().expect("running lock");
            if self.stopping()
                || self.draining()
                || running.len() >= cfg.settings.max_concurrent
                || running.values().any(|run| run.directory == directory)
            {
                return Ok(false);
            }
            running.insert(
                job.id,
                RunningJob {
                    directory,
                    cancel: cancel.clone(),
                },
            );
        }
        let started = self.store().mark_started(job.id, &log_path);
        if !matches!(started, Ok(true)) {
            // The user stopped it while the issue was being re-read, or the store failed.
            self.end_run(job.id);
            return started.map(|_| false);
        }
        tracing::info!(
            job_id = job.id,
            issue = %job.issue_identifier,
            state = JobState::Running.as_str(),
            executor = kind.as_str(),
            "job started"
        );
        self.emit_job(job.id);
        let limits = RunLimits {
            total: Duration::from_secs(cfg.settings.run_timeout_mins.saturating_mul(60)),
            stall: Duration::from_secs(cfg.settings.stall_timeout_mins.saturating_mul(60)),
        };
        let mut slot = RunSlot {
            engine: self.clone(),
            job_id: job.id,
            released: false,
        };
        let engine = self.clone();
        let runner = self.inner.job_runner.clone();
        let job_id = job.id;
        let issue_uuid = job.issue_uuid.clone();
        let identifier = job.issue_identifier.clone();
        // The executor's process goes on the row once it runs, so that a host started after a
        // crash can end what this run left behind.
        let recorder = self.clone();
        let on_spawn: SpawnHook = Box::new(move |tag| {
            if let Err(e) = recorder.store().set_process(job_id, &tag) {
                tracing::warn!(job_id, error = %format!("{e:#}"), "could not record the executor process");
            }
        });
        tokio::spawn(async move {
            // The runner gets a task of its own: if it panics, the job must still be recorded as
            // finished and give its slot back. Unwinding drops the runner's process-tree guard,
            // which ends the executor's processes. The result is still checked against Linear.
            let run = tokio::spawn(async move {
                runner
                    .run(spec, kind, log_path, limits, cancel, on_spawn)
                    .await
            });
            let output = match run.await {
                Ok(output) => output,
                Err(e) => RunOutput {
                    exit: ExitKind::SpawnFailed(format!("the run task ended unexpectedly: {e}")),
                    final_text: None,
                },
            };
            if let Err(e) = engine
                .finish_job(job_id, &issue_uuid, &identifier, output, &mut slot)
                .await
            {
                tracing::error!(job_id, error = %format!("{e:#}"), "could not record the run result");
            }
        });
        Ok(true)
    }

    /// Sets the state, binding and reason of a job, but only while it is still queued, and says
    /// whether it did. The check and the write happen under one store guard, so a stop that came
    /// in meanwhile is never overwritten.
    fn assign_if_queued(
        &self,
        job_id: i64,
        state: JobState,
        binding: Option<&Binding>,
        executor: Option<ExecutorKind>,
        reason: Option<&str>,
    ) -> Result<bool> {
        let store = self.store();
        match store.get(job_id)? {
            Some(job) if job.state == JobState::Queued => {
                store.assign(job_id, state, binding, executor, reason)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Checks the result in Linear, records it, frees the run's slot and wakes the loop so the
    /// next queued job can start.
    async fn finish_job(
        &self,
        job_id: i64,
        issue_uuid: &str,
        identifier: &str,
        output: RunOutput,
        slot: &mut RunSlot,
    ) -> Result<()> {
        let names = self.config().settings.trigger;
        let report = output.final_text.as_deref().and_then(outcome::parse_report);
        let verified = self.verify_read(job_id, issue_uuid).await;
        let (state, reason, verified_at) = match verified {
            Ok(issue) => {
                let (state, reason) = decide(
                    &output.exit,
                    issue.as_ref().map(|i| i.state.as_str()),
                    report.as_ref(),
                    &names,
                );
                (state, reason, issue.map(|i| i.updated_at))
            }
            Err(e) => {
                let state = if output.exit == ExitKind::Canceled {
                    JobState::Canceled
                } else {
                    JobState::Failed
                };
                (
                    state,
                    Some(format!("could not verify the result in Linear: {e:#}")),
                    None,
                )
            }
        };
        // These texts come from outside (the AI, Linear, the operating system) and reach the
        // panel, the CLI and MCP. The summary is the AI's own; without a report line it is the
        // last thing the AI said.
        let reason = reason.map(|reason| clean_text(&reason));
        let summary = match &report {
            Some(report) => Some(clean_text(report.summary())),
            None => output.final_text.as_deref().map(clean_text),
        }
        .filter(|text| !text.is_empty());
        let recorded = self.store().mark_finished(
            job_id,
            state,
            reason.as_deref(),
            summary.as_deref(),
            verified_at,
        );
        if recorded.is_ok() {
            tracing::info!(job_id, issue = %identifier, state = state.as_str(), "job finished");
        }
        if let Err(e) = &recorded {
            // Without this the row would stay `running` for a run that is over.
            let reason = clean_text(&format!("could not record the run result: {e:#}"));
            if let Err(e) = self
                .store()
                .set_state(job_id, JobState::Failed, Some(&reason))
            {
                tracing::error!(job_id, error = %format!("{e:#}"), "could not mark the run failed either");
            }
        }
        // Announce the result and free the slot only after it is stored: whoever sees the engine
        // idle must also see the result, and has been told about it.
        self.emit_job(job_id);
        slot.release();
        self.poll_now();
        recorded
    }

    /// Reads the issue of a finished run, up to three times: a read that fails is tried again
    /// after the pauses in `VERIFY_RETRY_PAUSES`. An issue Linear reports as gone is an answer,
    /// not a failure, and is not read again. Each read goes through the connection in use at that
    /// moment, so a run that ends after a reconnect is checked through the new connection.
    async fn verify_read(&self, job_id: i64, issue_uuid: &str) -> Result<Option<Issue>> {
        let mut pauses = VERIFY_RETRY_PAUSES.into_iter();
        loop {
            let read = self.linear().issue(issue_uuid).await;
            match read {
                Ok(issue) => return Ok(issue),
                Err(e) => {
                    let Some(pause) = pauses.next() else {
                        return Err(e);
                    };
                    tracing::warn!(
                        job_id,
                        error = %format!("{e:#}"),
                        "could not read the issue to verify the run; trying again"
                    );
                    tokio::time::sleep(pause).await;
                }
            }
        }
    }

    /// Drops the run from the running set, tells subscribers that the count of runs changed and
    /// wakes `wait_idle`. Also runs from `RunSlot::drop`, possibly while a panic unwinds, so a
    /// poisoned lock is used as it is instead of panicking.
    fn end_run(&self, job_id: i64) {
        self.inner
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&job_id);
        self.emit(EngineEvent::Status);
        self.inner.idle.notify_waiters();
    }

    /// Ends a run whose task stopped before it recorded the result: the row, if it still says
    /// running, is finished as failed like any other run (so it gets its end time and keeps its
    /// verified value), then the run's place is freed and the loop is woken for the next job in
    /// that folder. Runs from `RunSlot::drop`, so like `end_run` it must not panic: a store error
    /// is logged, and a poisoned lock is used as it is.
    fn abandon_run(&self, job_id: i64) {
        {
            let store = self
                .inner
                .store
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            // Read and write under one guard, so a row that is not running is left alone.
            let recorded = match store.get(job_id) {
                Ok(Some(job)) if job.state == JobState::Running => store.mark_finished(
                    job_id,
                    JobState::Failed,
                    Some("the run ended before its result was recorded"),
                    None,
                    None,
                ),
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            };
            if let Err(e) = recorded {
                tracing::error!(job_id, error = %format!("{e:#}"), "could not record the abandoned run");
            }
        }
        self.emit_job(job_id);
        self.end_run(job_id);
        self.poll_now();
    }

    /// A shutdown was requested: nothing new may start.
    fn stopping(&self) -> bool {
        self.inner
            .shutdown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    /// Resolves once no run is in progress (used by tests and by shutdown).
    pub async fn wait_idle(&self) {
        loop {
            let notified = self.inner.idle.notified();
            if self.inner.running.lock().expect("running lock").is_empty() {
                return;
            }
            notified.await;
        }
    }

    /// Stops a running job, or cancels one that is still queued or waiting. A stopped run is
    /// recorded as canceled once its process tree has ended.
    pub fn cancel_job(&self, job_id: i64) -> Result<()> {
        if self.stop_run(job_id) {
            return Ok(());
        }
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        match job.state {
            JobState::Queued | JobState::WaitingBinding => {
                let waiting = [JobState::Queued, JobState::WaitingBinding];
                let canceled = self.store().set_state_if(
                    job_id,
                    &waiting,
                    JobState::Canceled,
                    Some("stopped by the user"),
                )?;
                if canceled {
                    self.emit_job(job_id);
                    return Ok(());
                }
                // It started between the two reads: stop the run itself.
                if self.stop_run(job_id) {
                    return Ok(());
                }
                bail!("job {job_id} changed state while being stopped; try again")
            }
            JobState::Running => {
                // It started after the first look at the running set.
                if self.stop_run(job_id) {
                    return Ok(());
                }
                bail!("job {job_id} is marked running but no run is in progress for it")
            }
            other => bail!("job {job_id} is {} and cannot be stopped", other.as_str()),
        }
    }

    /// Tells the run of this job, if there is one, to stop; `false` when there is none.
    fn stop_run(&self, job_id: i64) -> bool {
        let token = self
            .inner
            .running
            .lock()
            .expect("running lock")
            .get(&job_id)
            .map(|run| run.cancel.clone());
        match token {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// Queues a failed, stopped or needs-info run again. Only the issue's latest run qualifies.
    pub fn retry_job(&self, job_id: i64) -> Result<()> {
        {
            // One guard for the whole check-and-requeue: two clicks must not requeue a run that
            // the first one already started.
            let store = self.store();
            let job = store.get(job_id)?;
            let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
            if !matches!(
                job.state,
                JobState::Failed | JobState::Canceled | JobState::NeedsInfo
            ) {
                bail!(
                    "job {job_id} is {}; only failed, stopped or needs-info runs can be retried",
                    job.state.as_str()
                );
            }
            // Re-running an older row while a newer run of the issue exists would make two active rows.
            let latest = store.latest_for_issue(&job.issue_uuid)?;
            if latest.map(|j| j.id) != Some(job_id) {
                bail!(
                    "job {job_id} is not the latest run of {}; retry that one instead",
                    job.issue_identifier
                );
            }
            store.requeue(job_id)?;
            tracing::info!(
                job_id,
                issue = %job.issue_identifier,
                state = JobState::Queued.as_str(),
                "job queued again"
            );
        }
        // A manual retry starts afresh: no pause from an earlier failed start, and a start of this
        // job that is failing right now is not held against it (`hold_back`). Counted only after
        // the row is queued again.
        {
            let mut backoff = self.inner.start_backoff.lock().expect("backoff lock");
            backoff.held.remove(&job_id);
            *backoff.retries.entry(job_id).or_default() += 1;
        }
        self.emit_job(job_id);
        self.poll_now();
        Ok(())
    }

    /// Marks an accepted run Done in Linear. Only the desktop panel calls this (spec 5.6);
    /// IPC, CLI and MCP never expose it.
    pub async fn mark_done(&self, job_id: i64) -> Result<()> {
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        if job.state != JobState::AwaitingAcceptance {
            bail!("job {job_id} is not awaiting acceptance");
        }
        let done = self.config().settings.trigger.done;
        self.linear().set_state(&job.issue_uuid, &done).await?;
        // The row may have changed while Linear was being updated; it becomes Done only from
        // awaiting acceptance.
        let waiting = [JobState::AwaitingAcceptance];
        let marked = self
            .store()
            .set_state_if(job_id, &waiting, JobState::Done, None)?;
        if !marked {
            bail!(
                "job {job_id} changed while it was being marked done; Linear has already been \
                 updated to {done:?}"
            );
        }
        self.emit_job(job_id);
        Ok(())
    }

    /// Checks the whole setup: executors (freshly, not from the cache), the Linear connection
    /// and workspace, and every binding. The report also carries the PATH and proxy the
    /// executors get, when the readiness probe resolves them; the proxy URLs are masked there.
    pub async fn doctor(&self) -> DoctorReport {
        let cfg = self.config();
        let executors = self.inner.readiness.executor_reports(true).await;
        let ready = doctor::ready_set(&executors);
        let linear = self.linear();
        let (viewer, mut linear_error) = match linear.viewer().await {
            Ok(viewer) => (Some(viewer), None),
            Err(e) => (None, Some(clean_text(&format!("{e:#}")))),
        };
        let mut workspace = None;
        if viewer.is_some() {
            let checked =
                doctor::check_workspace(linear.as_ref(), &cfg.settings.trigger, &cfg.bindings);
            match checked.await {
                Ok(report) => workspace = Some(report),
                Err(e) => {
                    linear_error = Some(clean_text(&format!("workspace check failed: {e:#}")))
                }
            }
        }
        // The report goes out over IPC and `doctor --json` and is shown in the panel, so a proxy
        // URL leaves it without its user name and password. The environment the runs get, which
        // the probe holds, keeps the real value.
        let env = self.inner.readiness.env().map(|env| ExecEnv {
            proxy: env.proxy_for_display(),
            ..env
        });
        DoctorReport {
            viewer,
            linear_error,
            executors,
            workspace,
            bindings: doctor::check_bindings(&cfg, &ready),
            env,
        }
    }

    /// The background loop: a cycle now, then again after the poll interval or as soon as
    /// `poll_now` is called. Before the first cycle it ends the executors that runs interrupted
    /// by an earlier host left running (`runner::end_leftovers`), and only then clears their
    /// records. Returns after `shutdown` is cancelled and the running jobs were told to stop; their
    /// results are recorded when they end (`wait_idle`). Once the shutdown is requested nothing new
    /// starts, not even in a cycle that is already in progress, and the stop does not wait for such
    /// a cycle: it is dropped where it waits (for Linear, say).
    pub async fn run_forever(&self, shutdown: CancellationToken) {
        *self
            .inner
            .shutdown
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(shutdown.clone());
        // A previous host that died while runs were in progress may have left their executors
        // running: end them, all at once, before anything new starts.
        let leftovers = std::mem::take(
            &mut *self
                .inner
                .leftovers
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        if !leftovers.is_empty() {
            let tags: Vec<ProcessTag> = leftovers.iter().map(|l| l.process.clone()).collect();
            let ended = runner::end_leftovers(&tags).await;
            for (leftover, ended) in leftovers.iter().zip(ended) {
                if ended {
                    tracing::warn!(
                        job_id = leftover.job_id,
                        pid = leftover.process.pid,
                        "ended an executor the previous host left running"
                    );
                }
                // Only now that it was dealt with: a host that stops during the sweep leaves the
                // record for the next one.
                let cleared = self
                    .store()
                    .clear_process(leftover.job_id, &leftover.process);
                if let Err(e) = cleared {
                    tracing::warn!(
                        job_id = leftover.job_id,
                        error = %format!("{e:#}"),
                        "could not clear the executor record; the next start checks it again"
                    );
                }
            }
        }
        let mut last_cleanup = tokio::time::Instant::now();
        if let Err(e) = self.clean_logs().await {
            tracing::warn!(error = %format!("{e:#}"), "log cleanup failed");
        }
        // Checked before every cycle, not only when waiting: if the shutdown and a wake-up are
        // ready together, `select!` may pick either, and no new cycle may start runs then.
        while !shutdown.is_cancelled() {
            // A check may wait for Linear for up to 30 s a call; a stop does not wait for it. The
            // cycle is dropped at its current await, which leaves nothing half done: every await
            // of a cycle comes before the synchronous blocks that decide and write, and every
            // write after a wait is conditional (see the module doc).
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => break,
                result = self.tick() => {
                    if let Err(e) = result {
                        tracing::warn!(error = %format!("{e:#}"), "check failed");
                    }
                }
            }
            if last_cleanup.elapsed() >= LOG_CLEANUP_INTERVAL {
                last_cleanup = tokio::time::Instant::now();
                if let Err(e) = self.clean_logs().await {
                    tracing::warn!(error = %format!("{e:#}"), "log cleanup failed");
                }
            }
            let interval = Duration::from_secs(self.config().settings.poll_interval_secs.max(5));
            tokio::select! {
                _ = shutdown.cancelled() => {}
                _ = tokio::time::sleep(interval) => {}
                _ = self.inner.poll_now.notified() => {}
            }
        }
        let running = self.inner.running.lock().expect("running lock");
        for job in running.values() {
            job.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> TriggerNames {
        TriggerNames::default()
    }

    #[test]
    fn a_failed_start_waits_twice_as_long_each_time_up_to_five_minutes() {
        let secs = |failures| start_pause(failures).as_secs();
        assert_eq!([1, 2, 3, 4, 5, 6].map(secs), [30, 60, 120, 240, 300, 300]);
        // No overflow, however many times a start failed.
        assert_eq!(secs(40), 300);
        assert_eq!(secs(u32::MAX), 300);
    }

    #[test]
    fn decide_maps_outcomes() {
        let ok = ExitKind::Exited(Some(0));
        assert_eq!(
            decide(&ok, Some("In Testing"), None, &names()).0,
            JobState::AwaitingAcceptance
        );
        assert_eq!(decide(&ok, Some("done"), None, &names()).0, JobState::Done);
        let info = Report::NeedsInfo("q".into());
        assert_eq!(
            decide(&ok, Some("Todo"), Some(&info), &names()).0,
            JobState::NeedsInfo
        );
        assert_eq!(
            decide(&ok, Some("In Progress"), Some(&info), &names()).0,
            JobState::Failed
        );
        let claimed = Report::InTesting("done".into());
        let (state, reason) = decide(&ok, Some("In Progress"), Some(&claimed), &names());
        assert_eq!(
            state,
            JobState::Failed,
            "the AI's report alone is not success"
        );
        assert!(reason.unwrap().contains("In Progress"));
        assert_eq!(
            decide(
                &ExitKind::Exited(Some(2)),
                Some("In Testing"),
                None,
                &names()
            )
            .0,
            JobState::Failed
        );
        assert_eq!(
            decide(&ExitKind::Exited(None), None, None, &names())
                .1
                .unwrap(),
            "executor exited with code unknown"
        );
        assert_eq!(
            decide(&ExitKind::Canceled, Some("In Testing"), None, &names()).0,
            JobState::Canceled
        );
        assert_eq!(
            decide(&ExitKind::Stalled, None, None, &names()).0,
            JobState::Failed
        );
        assert_eq!(
            decide(&ExitKind::TimedOut, None, None, &names()).0,
            JobState::Failed
        );
        assert_eq!(
            decide(&ExitKind::SpawnFailed("x".into()), None, None, &names()).0,
            JobState::Failed
        );
    }
}
