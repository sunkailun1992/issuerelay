//! The engine: polls Linear, records and schedules jobs, starts executor runs, verifies the
//! result in Linear and keeps the status the panel, CLI and MCP read.
//!
//! Locking: no std lock is ever held across an `.await`. They are taken and released within one
//! statement, except in short synchronous sections that must read and then write under one
//! guard: `update_config` (the config), `retry_job`, the poll's step for each issue and
//! `assign_if_queued` (the store), the registration of a run and the final loop of
//! `run_forever` (the running set). The async cycle lock is the one lock held across `.await`;
//! it keeps a poll from interleaving with a dispatch. A user's stop or retry can come at any
//! time, also while a poll or a start waits for Linear, so every write that follows such a
//! wait is conditional on the row still being in the state it was read in.
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::config::{Binding, Config, ExecutorKind, TriggerNames};
use crate::doctor::{self, DoctorReport, ExecutorReport, ReadinessProbe};
use crate::executor::{
    self, CommandSpec,
    outcome::{self, Report},
};
use crate::linear::{LinearApi, Viewer};
use crate::model::{Issue, Job, JobState};
use crate::prompt::{self, PromptInput};
use crate::runner::{self, ExitKind, RunLimits, RunOutput};
use crate::scheduler::{self, Candidate};
use crate::store::{NewJob, Store};
use crate::text::clean_text;
use crate::trigger::{self, Eligibility, WaitReason, same_name};

/// How much of the end of a log `Engine::log_tail` reads.
const LOG_TAIL_WINDOW: u64 = 1024 * 1024;

/// Starts one executor run and waits for it. The engine calls it from a task of its own.
#[async_trait::async_trait]
pub trait JobRunner: Send + Sync {
    /// Runs `spec` and reports how it ended, with the guarantees of `runner::run`: the whole
    /// process tree has ended when this returns, and also if the future is dropped before that.
    /// `cancel` stops the run early.
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
    ) -> RunOutput;
}

/// Runs real executor processes.
pub struct ProcessJobRunner;

#[async_trait::async_trait]
impl JobRunner for ProcessJobRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
    ) -> RunOutput {
        runner::run(spec, kind, &log_path, limits, cancel).await
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
    /// Why the last check failed, or why a job could not be started in the last round; the
    /// next successful check clears it.
    pub last_error: Option<String>,
    /// Runs in progress.
    pub running: usize,
    /// Jobs waiting for a free slot or directory.
    pub queued: usize,
    /// Jobs waiting for a binding, a directory or a ready executor.
    pub waiting: usize,
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
    store: Mutex<Store>,
    linear: Arc<dyn LinearApi>,
    readiness: Arc<dyn ReadinessProbe>,
    job_runner: Arc<dyn JobRunner>,
    logs_dir: PathBuf,
    running: Mutex<HashMap<i64, RunningJob>>,
    viewer: Mutex<Option<Viewer>>,
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
}

/// Cheap to clone; every clone drives the same engine.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

/// One spelling per folder for the one-run-per-directory rule: resolves symlinks, `..` and,
/// on case-insensitive volumes, letter case. Falls back to the path as written.
fn canonical_dir(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
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
    /// Loads the configuration and marks the runs a previous shutdown interrupted as failed.
    /// Only one engine may use a database at a time.
    pub fn new(deps: EngineDeps) -> Result<Self> {
        let config = Config::load(&deps.config_path)?;
        let recovered = deps.store.recover_interrupted()?;
        if recovered > 0 {
            tracing::warn!(
                recovered,
                "runs interrupted by the previous shutdown were marked failed"
            );
        }
        Ok(Self {
            inner: Arc::new(Inner {
                config_path: deps.config_path,
                config: RwLock::new(config),
                store: Mutex::new(deps.store),
                linear: deps.linear,
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
            }),
        })
    }

    fn config(&self) -> Config {
        self.inner.config.read().expect("config lock").clone()
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        self.inner.store.lock().expect("store lock")
    }

    fn count(&self, state: JobState) -> usize {
        self.store()
            .list_by_state(&[state])
            .map(|jobs| jobs.len())
            .unwrap_or(0)
    }

    /// The state the panel, CLI and MCP show. The job counts come from the store.
    pub fn status(&self) -> StatusSnapshot {
        let paused = self.config().settings.paused;
        let poll = self.inner.poll.lock().expect("poll lock").clone();
        let viewer = self.inner.viewer.lock().expect("viewer lock").clone();
        let running = self.inner.running.lock().expect("running lock").len();
        StatusSnapshot {
            paused,
            linear_ok: poll.linear_ok,
            viewer,
            last_poll_at: poll.at,
            last_error: poll.error,
            running,
            queued: self.count(JobState::Queued),
            waiting: self.count(JobState::WaitingBinding),
        }
    }

    /// Stops polling and starting runs, and remembers that across restarts. Runs already in
    /// progress continue.
    pub fn pause(&self) -> Result<()> {
        self.update_config(|c| c.settings.paused = true)
    }

    /// Undoes `pause` and checks at once.
    pub fn resume(&self) -> Result<()> {
        self.update_config(|c| c.settings.paused = false)?;
        self.poll_now();
        Ok(())
    }

    /// Wakes the background loop for an immediate check.
    pub fn poll_now(&self) {
        self.inner.poll_now.notify_one();
    }

    /// Validates the changed configuration, saves it and only then makes it current.
    fn update_config(&self, change: impl FnOnce(&mut Config)) -> Result<()> {
        let mut current = self.inner.config.write().expect("config lock");
        let mut next = current.clone();
        change(&mut next);
        next.validate()?;
        next.save(&self.inner.config_path)?;
        *current = next;
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

    /// The last `lines` lines of the job's log; empty while it has none. Reads at most the last
    /// `LOG_TAIL_WINDOW` bytes of the file, so a long log is cheap to ask for again and again.
    pub fn log_tail(&self, job_id: i64, lines: usize) -> Result<Vec<String>> {
        let job = self.store().get(job_id)?;
        let job = job.ok_or_else(|| anyhow!("job {job_id} not found"))?;
        let Some(path) = job.log_path else {
            return Ok(Vec::new());
        };
        let unreadable = || format!("cannot read {}", path.display());
        let mut file = std::fs::File::open(&path).with_context(unreadable)?;
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

    async fn viewer(&self) -> Result<Viewer> {
        let cached = self.inner.viewer.lock().expect("viewer lock").clone();
        if let Some(viewer) = cached {
            return Ok(viewer);
        }
        let viewer = self.inner.linear.viewer().await?;
        *self.inner.viewer.lock().expect("viewer lock") = Some(viewer.clone());
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
        result
    }

    async fn poll_inner(&self, cfg: &Config) -> Result<()> {
        let names = &cfg.settings.trigger;
        let me = self.viewer().await?;
        let issues = self
            .inner
            .linear
            .candidate_issues(&names.label, &names.todo)
            .await?;
        let (ready, _) = self.readiness().await;
        let mut seen = HashSet::new();
        for issue in issues
            .iter()
            .filter(|i| trigger::is_candidate(i, names, &me.id))
        {
            seen.insert(issue.uuid.clone());
            let eligibility = trigger::evaluate(issue, cfg, &me.id, &ready);
            // Read the latest run, decide and write under one store guard. A stop or a retry that
            // came in between would otherwise be overwritten, or leave the issue with two
            // active rows.
            let store = self.store();
            let latest = store.latest_for_issue(&issue.uuid)?;
            match latest {
                Some(job) if matches!(job.state, JobState::Queued | JobState::WaitingBinding) => {
                    Self::reassess(&store, &job, cfg, eligibility)?;
                }
                Some(job) if job.state == JobState::Running => {}
                latest => {
                    if trigger::should_enqueue(latest.as_ref(), issue) {
                        Self::enqueue(&store, issue, cfg, eligibility)?;
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
            self.store().set_state_if(
                job.id,
                &waiting,
                JobState::Canceled,
                Some("no longer matches the trigger (label, assignee or status changed)"),
            )?;
        }
        Ok(())
    }

    fn enqueue(store: &Store, issue: &Issue, cfg: &Config, eligibility: Eligibility) -> Result<()> {
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
            Eligibility::NotCandidate => return Ok(()),
        };
        store.insert(new)?;
        Ok(())
    }

    /// Brings a queued or waiting job in line with what the issue and the configuration say now.
    /// The caller holds the store guard it read the job under, so a stop or a retry cannot slip in
    /// between that read and these writes.
    fn reassess(store: &Store, job: &Job, cfg: &Config, eligibility: Eligibility) -> Result<()> {
        match eligibility {
            Eligibility::Eligible(binding) => {
                let unchanged = job.state == JobState::Queued
                    && job.binding_id.as_deref() == Some(binding.id.as_str())
                    && job.directory.as_deref() == Some(binding.directory.as_path())
                    && job.executor == Some(cfg.executor_for(&binding));
                if !unchanged {
                    store.assign(
                        job.id,
                        JobState::Queued,
                        Some(&binding),
                        Some(cfg.executor_for(&binding)),
                        None,
                    )?;
                }
            }
            Eligibility::Waiting(reason) => {
                store.assign(
                    job.id,
                    JobState::WaitingBinding,
                    None,
                    None,
                    Some(&reason.describe()),
                )?;
            }
            Eligibility::NotCandidate => {}
        }
        Ok(())
    }

    /// Starts the queued jobs the scheduler picks (one run per directory, at most
    /// `max_concurrent` in total) and returns their ids. A job that cannot be started is logged and
    /// reported in the status, and does not stop the others. Does nothing while paused or after a
    /// shutdown was requested.
    pub async fn dispatch(&self) -> Result<Vec<i64>> {
        let _cycle = self.inner.cycle.lock().await;
        let cfg = self.config();
        if cfg.settings.paused || self.stopping() {
            return Ok(Vec::new());
        }
        let queued = self.store().list_by_state(&[JobState::Queued])?;
        let candidates: Vec<Candidate> = queued
            .iter()
            .filter_map(|job| {
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
        let me = self.viewer().await?;
        let (ready, reports) = self.readiness().await;
        let mut started = Vec::new();
        let mut failure = None;
        for id in picks {
            // Each start works from the configuration as it is now, and a pause or a shutdown
            // that came meanwhile ends the round.
            let cfg = self.config();
            if cfg.settings.paused || self.stopping() {
                break;
            }
            let job = self.store().get(id);
            let started_job = match job {
                Ok(Some(job)) => {
                    let label = job.issue_identifier.clone();
                    self.start_job(&cfg, &me, &ready, &reports, job)
                        .await
                        .map_err(|e| (label, e))
                }
                Ok(None) => continue,
                Err(e) => Err((format!("job {id}"), e)),
            };
            match started_job {
                Ok(true) => started.push(id),
                Ok(false) => {}
                Err((label, e)) => {
                    tracing::warn!(job_id = id, error = %format!("{e:#}"), "could not start the job");
                    failure = Some(format!("could not start {label}: {e:#}"));
                }
            }
        }
        if let Some(message) = failure {
            self.inner.poll.lock().expect("poll lock").error = Some(clean_text(&message));
        }
        Ok(started)
    }

    async fn start_job(
        &self,
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
        let current = self.inner.linear.issue(&job.issue_uuid).await?;
        let Some(issue) =
            current.filter(|i| trigger::is_candidate(i, &cfg.settings.trigger, &me.id))
        else {
            self.store().set_state_if(
                job.id,
                &[JobState::Queued],
                JobState::Canceled,
                Some("issue changed before the run started"),
            )?;
            return Ok(false);
        };
        let binding = match trigger::evaluate(&issue, cfg, &me.id, ready) {
            Eligibility::Eligible(binding) => binding,
            Eligibility::Waiting(reason) => {
                self.assign_if_queued(
                    job.id,
                    JobState::WaitingBinding,
                    None,
                    None,
                    Some(&reason.describe()),
                )?;
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
            self.assign_if_queued(job.id, JobState::WaitingBinding, None, None, Some(&reason))?;
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
            let mut running = self.inner.running.lock().expect("running lock");
            if self.stopping()
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
        tokio::spawn(async move {
            // The runner gets a task of its own: if it panics, the job must still be recorded as
            // finished and give its slot back. Unwinding drops the runner's process-tree guard,
            // which ends the executor's processes. The result is still checked against Linear.
            let run =
                tokio::spawn(async move { runner.run(spec, kind, log_path, limits, cancel).await });
            let output = match run.await {
                Ok(output) => output,
                Err(e) => RunOutput {
                    exit: ExitKind::SpawnFailed(format!("the run task ended unexpectedly: {e}")),
                    final_text: None,
                },
            };
            if let Err(e) = engine
                .finish_job(job_id, &issue_uuid, output, &mut slot)
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
        output: RunOutput,
        slot: &mut RunSlot,
    ) -> Result<()> {
        let names = self.config().settings.trigger;
        let report = output.final_text.as_deref().and_then(outcome::parse_report);
        let verified = self.inner.linear.issue(issue_uuid).await;
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
        // Free the slot only after the result is stored: whoever sees the engine idle must also
        // see the result.
        slot.release();
        self.poll_now();
        recorded
    }

    /// Drops the run from the running set and wakes `wait_idle`. Also runs from `RunSlot::drop`,
    /// possibly while a panic unwinds, so a poisoned lock is used as it is instead of panicking.
    fn end_run(&self, job_id: i64) {
        self.inner
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&job_id);
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
                if self.store().set_state_if(
                    job_id,
                    &waiting,
                    JobState::Canceled,
                    Some("stopped by the user"),
                )? {
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
        }
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
        self.inner.linear.set_state(&job.issue_uuid, &done).await?;
        // The row may have changed while Linear was being updated; it becomes Done only from
        // awaiting acceptance.
        let waiting = [JobState::AwaitingAcceptance];
        if !self
            .store()
            .set_state_if(job_id, &waiting, JobState::Done, None)?
        {
            bail!(
                "job {job_id} changed while it was being marked done; Linear has already been \
                 updated to {done:?}"
            );
        }
        Ok(())
    }

    /// Checks the whole setup: executors (freshly, not from the cache), the Linear connection
    /// and workspace, and every binding.
    pub async fn doctor(&self) -> DoctorReport {
        let cfg = self.config();
        let executors = self.inner.readiness.executor_reports(true).await;
        let ready = doctor::ready_set(&executors);
        let (viewer, mut linear_error) = match self.inner.linear.viewer().await {
            Ok(viewer) => (Some(viewer), None),
            Err(e) => (None, Some(clean_text(&format!("{e:#}")))),
        };
        let mut workspace = None;
        if viewer.is_some() {
            match doctor::check_workspace(self.inner.linear.as_ref(), &cfg.settings.trigger).await {
                Ok(report) => workspace = Some(report),
                Err(e) => {
                    linear_error = Some(clean_text(&format!("workspace check failed: {e:#}")))
                }
            }
        }
        DoctorReport {
            viewer,
            linear_error,
            executors,
            workspace,
            bindings: doctor::check_bindings(&cfg, &ready),
        }
    }

    /// The background loop: a cycle now, then again after the poll interval or as soon as
    /// `poll_now` is called. Returns after `shutdown` is cancelled and the running jobs were told
    /// to stop; their results are recorded when they end (`wait_idle`). Once the shutdown is
    /// requested nothing new starts, not even in a cycle that is already in progress.
    pub async fn run_forever(&self, shutdown: CancellationToken) {
        *self
            .inner
            .shutdown
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(shutdown.clone());
        // Checked before every cycle, not only when waiting: if the shutdown and a wake-up are
        // ready together, `select!` may pick either, and no new cycle may start runs then.
        while !shutdown.is_cancelled() {
            if let Err(e) = self.tick().await {
                tracing::warn!(error = %format!("{e:#}"), "check failed");
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
