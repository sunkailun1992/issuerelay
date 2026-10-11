mod common;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use common::{
    engine_kit::{FakeReadiness, Harness, Script, ScriptedRunner},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    config::{Binding, Config, ExecutorKind, PermissionProfile, TriggerNames},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{Engine, EngineDeps, JobRunner, ProcessJobRunner, StatusSnapshot},
    executor::CommandSpec,
    linear::{LinearApi, Project, Team, Viewer},
    model::{Issue, Job, JobState},
    runner::{ExitKind, RunLimits},
    store::{NewJob, Store},
};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn bound_issue_runs_to_awaiting_acceptance() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::AwaitingAcceptance);
    assert_eq!(job.summary.as_deref(), Some("pushed to test"));
    assert_eq!(job.verified_updated_at, Some(h.linear.get("1").updated_at));
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    let spec = h.runner.specs.lock().unwrap()[0].clone();
    assert_eq!(spec.program, std::path::PathBuf::from("/fake/claude"));
    assert!(spec.args.contains(&"bypassPermissions".to_string()));
    assert!(
        spec.stdin.contains("DEMO-1") && spec.stdin.contains("Never set the status to \"Done\"")
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(
        h.runner.started().len(),
        1,
        "a finished issue must not start again"
    );
}

#[tokio::test]
async fn unbound_issue_waits_then_runs_after_binding() {
    let h = Harness::new(2, false);
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let waiting = h.latest("1");
    assert_eq!(waiting.state, JobState::WaitingBinding);
    assert!(waiting.reason.unwrap().contains("not bound"));
    assert!(h.runner.started().is_empty());
    h.bind("proj-1", "repo-a");
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(
        h.engine.list_jobs(100).unwrap().len(),
        1,
        "the waiting job is reused"
    );
}

#[tokio::test]
async fn same_directory_runs_one_at_a_time() {
    let h = Harness::new(5, true);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(
        (h.engine.status().running, h.engine.status().queued),
        (1, 1)
    );
    h.runner.gate.add_permits(1);
    h.settle().await;
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn one_folder_named_two_ways_runs_one_at_a_time() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "repo-a");
    h.bind("proj-2", "repo-a/../repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    assert_eq!(
        (h.engine.status().running, h.engine.status().queued),
        (1, 1)
    );
    h.runner.gate.add_permits(2);
    h.settle().await;
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 2);
}

#[tokio::test]
async fn global_cap_limits_parallel_runs() {
    let h = Harness::new(2, true);
    for (project, dir) in [("proj-1", "a"), ("proj-2", "b"), ("proj-3", "c")] {
        h.bind(project, dir);
    }
    for (uuid, project) in [("1", "proj-1"), ("2", "proj-2"), ("3", "proj-3")] {
        h.linear.upsert(issue(uuid, project));
    }
    h.engine.tick().await.unwrap();
    assert_eq!(
        (h.engine.status().running, h.engine.status().queued),
        (2, 1)
    );
    h.runner.gate.add_permits(3);
    h.settle().await;
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 3);
}

#[tokio::test]
async fn queued_issue_losing_label_is_canceled() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("2").state, JobState::Queued);
    h.linear.update("2", |i| i.labels.clear());
    h.engine.poll_once().await.unwrap();
    let job = h.latest("2");
    assert_eq!(job.state, JobState::Canceled);
    assert!(job.reason.unwrap().contains("no longer matches"));
    h.runner.gate.add_permits(1);
    h.settle().await;
}

#[tokio::test]
async fn a_changed_binding_directory_moves_the_queued_job() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap(); // DEMO-1 takes the only slot
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("2").state, JobState::Queued);
    let moved = h.bind("proj-2", "c");
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("2").directory.as_deref(), Some(moved.as_path()));
    h.runner.gate.add_permits(2);
    h.settle().await;
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.specs.lock().unwrap().last().unwrap().cwd, moved);
}

#[tokio::test]
async fn pre_dispatch_recheck_cancels_changed_issue() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
    h.linear
        .update("1", |i| i.assignee_id = Some("someone-else".into()));
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Canceled);
    assert!(job.reason.unwrap().contains("changed before"));
    assert!(h.runner.started().is_empty());
}

#[tokio::test]
async fn failed_issue_requeues_only_after_linear_update() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(3));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let failed = h.latest("1");
    assert_eq!(failed.state, JobState::Failed);
    assert!(failed.reason.unwrap().contains("code 3"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(
        h.runner.started().len(),
        1,
        "unchanged issue must not run again"
    );
    h.linear
        .update("1", |i| i.title = "Demo task 1 (clarified)".into());
    h.runner.script("DEMO-1", Script::success());
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 2);
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.engine.list_jobs(100).unwrap().len(), 2);
}

#[tokio::test]
async fn needs_info_is_recorded_and_not_rerun() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::needs_info());
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::NeedsInfo);
    assert_eq!(job.summary.as_deref(), Some("which page?"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 1);
}

#[tokio::test]
async fn mark_done_updates_linear_and_job() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    h.engine.mark_done(job.id).await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
    assert_eq!(h.linear.get("1").state, "Done");
    assert_eq!(
        h.linear.set_state_calls(),
        [("1".to_string(), "Done".to_string())]
    );
    assert!(
        h.engine.mark_done(job.id).await.is_err(),
        "only awaiting-acceptance jobs can be marked done"
    );
}

#[tokio::test]
async fn cancel_running_job() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Running);
    h.engine.cancel_job(job.id).unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::Canceled);
}

#[tokio::test]
async fn a_run_that_panics_is_recorded_as_failed_and_frees_its_slot() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script(
        "DEMO-1",
        Script {
            panic: true,
            ..Script::exit_code(0)
        },
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert!(job.reason.unwrap().contains("ended unexpectedly"));
    assert_eq!(h.engine.status().running, 0);
}

#[tokio::test]
async fn retry_requeues_failed_job() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    h.runner.script("DEMO-1", Script::success());
    h.engine.retry_job(job.id).unwrap();
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.engine.list_jobs(100).unwrap().len(), 1);
    assert!(h.engine.retry_job(job.id).is_err());
}

#[tokio::test]
async fn a_second_retry_is_rejected() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    h.engine.retry_job(job.id).unwrap();
    assert!(
        h.engine.retry_job(job.id).is_err(),
        "the run is queued again; a second click must not requeue it"
    );
    assert_eq!(h.latest("1").state, JobState::Queued);
}

#[tokio::test]
async fn retry_rejects_an_older_run() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let first = h.latest("1");
    // Someone edits the issue in Linear, so it comes back as a new run, which fails too.
    h.linear.update("1", |i| i.title.push_str(" (edited)"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let second = h.latest("1");
    assert_ne!(second.id, first.id);
    assert!(h.engine.retry_job(first.id).is_err());
    h.engine.retry_job(second.id).unwrap();
}

#[tokio::test]
async fn paused_engine_does_nothing_and_pause_is_saved() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.pause().unwrap();
    h.engine.tick().await.unwrap();
    assert!(h.engine.list_jobs(10).unwrap().is_empty());
    assert!(h.engine.status().paused);
    assert!(
        Config::load(&h.root.path().join("config.toml"))
            .unwrap()
            .settings
            .paused
    );
    h.engine.resume().unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 1);
}

#[tokio::test]
async fn poll_error_is_reported_without_state_changes() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.fail_next("Linear is down");
    assert!(h.engine.tick().await.is_err());
    let status = h.engine.status();
    assert!(!status.linear_ok);
    assert!(status.last_error.unwrap().contains("Linear is down"));
    assert!(h.engine.list_jobs(10).unwrap().is_empty());
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert!(h.engine.status().linear_ok);
}

#[tokio::test]
async fn interrupted_runs_fail_on_restart() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("state.db");
    {
        let store = Store::open(&db).unwrap();
        let i = issue("1", "proj-1");
        let job = store
            .insert(NewJob {
                issue: &i,
                binding: None,
                executor: None,
                state: JobState::Queued,
                reason: None,
            })
            .unwrap();
        store
            .mark_started(job.id, &root.path().join("1.jsonl"))
            .unwrap();
    }
    let linear = FakeLinear::new("user-1");
    let engine = Engine::new(EngineDeps {
        config_path: root.path().join("config.toml"),
        store: Store::open(&db).unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.path().join("logs"),
    })
    .unwrap();
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::Failed);
    assert!(job.reason.unwrap().contains("interrupted"));
}

#[tokio::test]
async fn an_interrupted_run_is_not_rerun_until_linear_changes() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("state.db");
    let repo = root.path().join("repo-a");
    std::fs::create_dir_all(&repo).unwrap();
    let config_path = root.path().join("config.toml");
    let mut cfg = Config::default();
    cfg.bindings.push(Binding {
        id: "proj-1".into(),
        project_id: "proj-1".into(),
        project_name: "Demo Project".into(),
        directory: repo,
        executor: None,
        permission: None,
        model: None,
        prompt_override: None,
    });
    cfg.save(&config_path).unwrap();
    {
        let store = Store::open(&db).unwrap();
        let i = issue("1", "proj-1");
        let job = store
            .insert(NewJob {
                issue: &i,
                binding: None,
                executor: None,
                state: JobState::Queued,
                reason: None,
            })
            .unwrap();
        store
            .mark_started(job.id, &root.path().join("1.jsonl"))
            .unwrap();
    }
    let linear = FakeLinear::new("user-1");
    linear.upsert(issue("1", "proj-1"));
    let runner = Arc::new(ScriptedRunner::new(linear.clone(), false));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: runner.clone(),
        logs_dir: root.path().join("logs"),
    })
    .unwrap();
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    assert!(
        runner.started().is_empty(),
        "an unchanged issue must not run again after a crash"
    );
    linear.update("1", |i| i.title.push_str(" (edited)"));
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(runner.started(), ["DEMO-1"]);
}

#[tokio::test]
async fn log_tail_returns_last_lines() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(
        h.engine.log_tail(h.latest("1").id, 10).unwrap(),
        ["{\"type\":\"result\"}"]
    );
}

#[tokio::test]
async fn log_tail_reads_only_the_end_of_a_large_log() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    let text: String = (0..100_000)
        .map(|n| format!("{{\"line\":{n}}}\n"))
        .collect();
    std::fs::write(job.log_path.as_ref().unwrap(), &text).unwrap();
    assert_eq!(
        h.engine.log_tail(job.id, 2).unwrap(),
        ["{\"line\":99998}", "{\"line\":99999}"]
    );
    let window = h.engine.log_tail(job.id, 1_000_000).unwrap();
    assert!(window.len() < 100_000, "only the last 1 MiB is read");
    assert!(
        window[0].starts_with("{\"line\":") && window[0].ends_with('}'),
        "no partial first line: {}",
        window[0]
    );
}

// Everything below pins behavior that the brief and the review notes require but the tests
// above do not exercise.

/// Lets the other tasks of the single-threaded test runtime run until `done` holds.
async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("gave up waiting for {what}");
}

/// Lets every ready task run until it waits again, without moving the paused clock.
async fn quiesce() {
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
}

/// Cancels the shutdown token and waits for the loop to return. The wait only yields: if the
/// loop needed the paused clock to move before it noticed the shutdown, this would give up.
async fn stop_loop(shutdown: &CancellationToken, worker: tokio::task::JoinHandle<()>) {
    shutdown.cancel();
    eventually("the loop to stop", || worker.is_finished()).await;
    worker.await.unwrap();
}

fn spawn_loop(engine: &Engine, shutdown: &CancellationToken) -> tokio::task::JoinHandle<()> {
    let (engine, shutdown) = (engine.clone(), shutdown.clone());
    tokio::spawn(async move { engine.run_forever(shutdown).await })
}

fn job_by_id(h: &Harness, id: i64) -> Job {
    h.engine
        .list_jobs(100)
        .unwrap()
        .into_iter()
        .find(|job| job.id == id)
        .expect("no such job")
}

fn demo_binding(project: &str, directory: &Path) -> Binding {
    Binding {
        id: project.into(),
        project_id: project.into(),
        project_name: "Demo Project".into(),
        directory: directory.to_path_buf(),
        executor: None,
        permission: None,
        model: None,
        prompt_override: None,
    }
}

/// A configuration with `demo_binding("proj-1", <a new folder under root>)`.
fn config_with_one_binding(root: &Path) -> Config {
    let dir = root.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &dir));
    cfg
}

/// An engine over the given fakes whose configuration file is saved in `root` first.
fn build_engine(
    root: &Path,
    cfg: &Config,
    linear: Arc<dyn LinearApi>,
    readiness: Arc<dyn ReadinessProbe>,
    job_runner: Arc<dyn JobRunner>,
) -> Engine {
    let config_path = root.join("config.toml");
    cfg.save(&config_path).unwrap();
    Engine::new(EngineDeps {
        config_path,
        store: Store::open_in_memory().unwrap(),
        linear,
        readiness,
        job_runner,
        logs_dir: root.join("logs"),
    })
    .unwrap()
}

/// A `FakeLinear` with the knobs the plain fake lacks, for the engine's less usual paths.
struct SpyLinear {
    inner: FakeLinear,
    /// When set, every candidate query answers with these issues, matching or not.
    candidates: Mutex<Option<Vec<Issue>>>,
    /// The first call after `viewer()` fails; in `doctor` that is the workspace check's.
    fail_after_viewer: bool,
    /// Runs once, just before the first `issue()` call is answered.
    before_issue: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    viewer_calls: AtomicUsize,
}

impl SpyLinear {
    fn new(inner: FakeLinear) -> Self {
        Self {
            inner,
            candidates: Mutex::new(None),
            fail_after_viewer: false,
            before_issue: Mutex::new(None),
            viewer_calls: AtomicUsize::new(0),
        }
    }

    fn viewer_calls(&self) -> usize {
        self.viewer_calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LinearApi for SpyLinear {
    async fn viewer(&self) -> Result<Viewer> {
        self.viewer_calls.fetch_add(1, Ordering::SeqCst);
        let viewer = self.inner.viewer().await;
        if self.fail_after_viewer {
            self.inner.fail_next("Linear is down");
        }
        viewer
    }

    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        let fixed = self.candidates.lock().unwrap().clone();
        match fixed {
            Some(all) => Ok(all),
            None => self.inner.candidate_issues(label, state).await,
        }
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        let hook = self.before_issue.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        self.inner.issue(id).await
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        self.inner.set_state(id, state).await
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        self.inner.projects().await
    }

    async fn labels(&self) -> Result<Vec<String>> {
        self.inner.labels().await
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        self.inner.teams().await
    }

    async fn statuses(&self, team_id: &str) -> Result<Vec<String>> {
        self.inner.statuses(team_id).await
    }
}

/// Claude is always ready; Codex is ready while the flag is set (not logged in otherwise).
struct CodexReadiness(Arc<AtomicBool>);

#[async_trait::async_trait]
impl ReadinessProbe for CodexReadiness {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        let mut reports = FakeReadiness.executor_reports(fresh).await;
        for report in &mut reports {
            if report.kind == ExecutorKind::Codex && !self.0.load(Ordering::SeqCst) {
                report.logged_in = false;
                report.problems.push("not logged in".into());
            }
        }
        reports
    }
}

/// Records whether each executor check asked for fresh results.
struct RecordsFresh(Arc<Mutex<Vec<bool>>>);

#[async_trait::async_trait]
impl ReadinessProbe for RecordsFresh {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        self.0.lock().unwrap().push(fresh);
        FakeReadiness.executor_reports(fresh).await
    }
}

// The background loop.

#[tokio::test(start_paused = true)]
async fn the_loop_checks_on_a_timer_and_when_woken() {
    let h = Harness::new(5, false);
    for (project, dir) in [("proj-1", "a"), ("proj-2", "b"), ("proj-3", "c")] {
        h.bind(project, dir);
    }
    h.linear.upsert(issue("1", "proj-1"));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually("the first check to start DEMO-1", || {
        h.runner.started() == ["DEMO-1"]
    })
    .await;
    h.settle().await;
    quiesce().await;
    // The paused clock only moves when the test moves it: a new issue waits for the timer
    // (30 s by default).
    h.linear.upsert(issue("2", "proj-2"));
    tokio::time::advance(Duration::from_secs(29)).await;
    quiesce().await;
    assert_eq!(
        h.runner.started(),
        ["DEMO-1"],
        "no check before the interval is over"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    eventually("the timer to start DEMO-2", || {
        h.runner.started().len() == 2
    })
    .await;
    h.settle().await;
    quiesce().await;
    // `poll_now` wakes the loop without waiting for the timer.
    h.linear.upsert(issue("3", "proj-3"));
    h.engine.poll_now();
    eventually("the wake-up to start DEMO-3", || {
        h.runner.started().len() == 3
    })
    .await;
    stop_loop(&shutdown, worker).await;
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn resuming_wakes_the_loop_without_waiting_for_the_timer() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.pause().unwrap();
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    quiesce().await;
    assert!(h.runner.started().is_empty(), "nothing starts while paused");
    h.engine.resume().unwrap();
    eventually("the resumed loop to start DEMO-1", || {
        h.runner.started() == ["DEMO-1"]
    })
    .await;
    stop_loop(&shutdown, worker).await;
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn a_finished_run_wakes_the_loop_so_the_next_job_starts_at_once() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually("DEMO-1 to start", || h.runner.started() == ["DEMO-1"]).await;
    quiesce().await;
    assert_eq!(
        h.engine.status().queued,
        1,
        "DEMO-2 waits for the only slot"
    );
    // No time passes, so only the finishing run can wake the loop for DEMO-2.
    h.runner.gate.add_permits(1);
    eventually("DEMO-2 to start", || {
        h.runner.started() == ["DEMO-1", "DEMO-2"]
    })
    .await;
    h.runner.gate.add_permits(1);
    h.settle().await;
    stop_loop(&shutdown, worker).await;
}

#[tokio::test(start_paused = true)]
async fn retrying_a_run_wakes_the_loop() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually("the first run to start", || {
        h.runner.started() == ["DEMO-1"]
    })
    .await;
    h.settle().await;
    quiesce().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    h.runner.script("DEMO-1", Script::success());
    h.engine.retry_job(job.id).unwrap();
    eventually("the retry to start", || h.runner.started().len() == 2).await;
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    stop_loop(&shutdown, worker).await;
}

#[tokio::test(start_paused = true)]
async fn shutting_the_loop_down_stops_the_runs_in_progress() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually("DEMO-1 to be running", || h.engine.status().running == 1).await;
    // The loop returns once the runs were told to stop; recording their end takes `wait_idle`.
    stop_loop(&shutdown, worker).await;
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::Canceled);
    assert_eq!(h.engine.status().running, 0);
}

#[tokio::test(start_paused = true)]
async fn a_loop_that_is_shut_down_before_it_starts_runs_no_cycle() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    h.engine.poll_now();
    spawn_loop(&h.engine, &shutdown).await.unwrap();
    assert!(h.engine.list_jobs(10).unwrap().is_empty());
    assert!(h.runner.started().is_empty());
}

// Concurrency.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_poll_waits_for_a_dispatch_in_progress() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner,
    );
    engine.poll_once().await.unwrap();
    // While the dispatch re-reads the issue before starting it, another task asks for a poll.
    // The poll must wait: it would reassign rows the dispatch is about to start.
    let polled = Arc::new(AtomicBool::new(false));
    let poller = Arc::new(Mutex::new(None));
    let (e, flag, seen, slot) = (
        engine.clone(),
        polled.clone(),
        polled.clone(),
        poller.clone(),
    );
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        let task = tokio::spawn(async move {
            e.poll_once().await.unwrap();
            flag.store(true, Ordering::SeqCst);
        });
        *slot.lock().unwrap() = Some(task);
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !seen.load(Ordering::SeqCst),
            "a poll ran while a dispatch was starting jobs"
        );
    }));
    assert_eq!(engine.dispatch().await.unwrap().len(), 1);
    let task: tokio::task::JoinHandle<()> = poller.lock().unwrap().take().unwrap();
    task.await.unwrap();
    assert!(
        polled.load(Ordering::SeqCst),
        "the poll ran once the dispatch was done"
    );
    engine.wait_idle().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_engine_works_on_a_multi_threaded_runtime() {
    let h = Harness::new(3, false);
    for (uuid, project, dir) in [
        ("1", "proj-1", "a"),
        ("2", "proj-2", "b"),
        ("3", "proj-3", "c"),
    ] {
        h.bind(project, dir);
        h.linear.upsert(issue(uuid, project));
    }
    // Hosts spawn the engine's futures, so they must be `Send`.
    let engine = h.engine.clone();
    tokio::spawn(async move { engine.tick().await })
        .await
        .unwrap()
        .unwrap();
    h.settle().await;
    for uuid in ["1", "2", "3"] {
        assert_eq!(h.latest(uuid).state, JobState::AwaitingAcceptance);
    }
    assert_eq!(h.runner.started().len(), 3);
    let (engine, id) = (h.engine.clone(), h.latest("1").id);
    tokio::spawn(async move { engine.mark_done(id).await })
        .await
        .unwrap()
        .unwrap();
    let engine = h.engine.clone();
    let report = tokio::spawn(async move { engine.doctor().await })
        .await
        .unwrap();
    assert!(report.viewer.is_some());
    let engine = h.engine.clone();
    tokio::spawn(async move { engine.wait_idle().await })
        .await
        .unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
}

// Which jobs start, and how.

#[tokio::test]
async fn a_paused_engine_starts_no_queued_job() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    h.engine.pause().unwrap();
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").state, JobState::Queued);
    h.engine.resume().unwrap();
    assert_eq!(h.engine.dispatch().await.unwrap().len(), 1);
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1"]);
}

#[tokio::test]
async fn urgent_and_older_issues_start_first() {
    let h = Harness::new(1, true);
    for (project, dir) in [("proj-1", "a"), ("proj-2", "b"), ("proj-3", "c")] {
        h.bind(project, dir);
    }
    // Inserted in the order DEMO-1, DEMO-2, DEMO-3; they should run as 3, 2, 1.
    h.linear.upsert(issue("1", "proj-1"));
    let mut older = issue("2", "proj-2");
    older.created_at -= chrono::Duration::hours(1);
    h.linear.upsert(older);
    let mut urgent = issue("3", "proj-3");
    urgent.priority = 1;
    urgent.created_at += chrono::Duration::hours(1);
    h.linear.upsert(urgent);
    for first in ["3", "2", "1"] {
        h.engine.tick().await.unwrap();
        assert_eq!(
            h.latest(first).state,
            JobState::Running,
            "DEMO-{first} is next"
        );
        h.runner.gate.add_permits(1);
        h.settle().await;
    }
    assert_eq!(h.runner.started(), ["DEMO-3", "DEMO-2", "DEMO-1"]);
}

#[tokio::test]
async fn a_directory_with_a_run_in_progress_is_not_used_again() {
    let h = Harness::new(5, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    // A second issue of the same project arrives while the first run is still going.
    h.linear.upsert(issue("2", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(
        (h.engine.status().running, h.engine.status().queued),
        (1, 1)
    );
    h.runner.gate.add_permits(1);
    h.settle().await;
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("2").state, JobState::Running);
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn a_second_check_does_not_exceed_the_cap_while_runs_are_in_progress() {
    let h = Harness::new(2, true);
    for (uuid, project, dir) in [
        ("1", "proj-1", "a"),
        ("2", "proj-2", "b"),
        ("3", "proj-3", "c"),
    ] {
        h.bind(project, dir);
        h.linear.upsert(issue(uuid, project));
    }
    h.engine.tick().await.unwrap();
    h.engine.tick().await.unwrap();
    assert_eq!(
        (h.engine.status().running, h.engine.status().queued),
        (2, 1)
    );
    h.runner.gate.add_permits(3);
    h.settle().await;
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started().len(), 3);
}

#[tokio::test]
async fn bindings_choose_executor_permission_model_and_prompt() {
    let dirs = tempfile::tempdir().unwrap();
    let (alpha, beta) = (dirs.path().join("alpha"), dirs.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.settings.prompt_template = "Global rules apply.".into();
    cfg.settings.comment_language = "ja".into();
    cfg.bindings.push(Binding {
        executor: Some(ExecutorKind::Codex),
        permission: Some(PermissionProfile::Auto),
        model: Some("gpt-5.5".into()),
        prompt_override: Some("   ".into()), // blank: the global template still applies
        ..demo_binding("proj-1", &alpha)
    });
    cfg.bindings.push(Binding {
        prompt_override: Some("Use the beta branch.".into()),
        ..demo_binding("proj-2", &beta)
    });
    let h = Harness::with_config(cfg, false);
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let runs: Vec<(CommandSpec, ExecutorKind)> = {
        let specs = h.runner.specs.lock().unwrap().clone();
        let kinds = h.runner.kinds.lock().unwrap().clone();
        specs.into_iter().zip(kinds).collect()
    };
    let run_of = |identifier: &str| {
        runs.iter()
            .find(|(spec, _)| spec.stdin.contains(identifier))
    };
    let (codex, kind) = run_of("DEMO-1").unwrap();
    assert_eq!(*kind, ExecutorKind::Codex);
    assert_eq!(codex.program, PathBuf::from("/fake/codex"));
    assert_eq!(codex.cwd, alpha);
    assert!(codex.args.contains(&"--approve-for-me".to_string()));
    assert!(codex.args.windows(2).any(|pair| pair == ["-m", "gpt-5.5"]));
    assert!(codex.stdin.contains("Global rules apply."));
    assert!(codex.stdin.contains("this language: ja"));
    let (claude, kind) = run_of("DEMO-2").unwrap();
    assert_eq!(*kind, ExecutorKind::Claude);
    assert_eq!(claude.program, PathBuf::from("/fake/claude"));
    assert_eq!(claude.cwd, beta);
    assert!(claude.args.contains(&"bypassPermissions".to_string()));
    assert!(claude.stdin.contains("Use the beta branch."));
    assert!(
        !claude.stdin.contains("Global rules apply."),
        "a real override replaces the global template"
    );
    // The job rows say which binding, directory and executor each run used.
    let (first, second) = (h.latest("1"), h.latest("2"));
    assert_eq!(first.binding_id.as_deref(), Some("proj-1"));
    assert_eq!(first.directory.as_deref(), Some(alpha.as_path()));
    assert_eq!(first.executor, Some(ExecutorKind::Codex));
    assert_eq!(second.binding_id.as_deref(), Some("proj-2"));
    assert_eq!(second.directory.as_deref(), Some(beta.as_path()));
    assert_eq!(second.executor, Some(ExecutorKind::Claude));
}

#[tokio::test]
async fn custom_trigger_names_drive_polling_the_prompt_and_the_result() {
    let mut cfg = Config::default();
    cfg.settings.trigger = TriggerNames {
        label: "robot".into(),
        todo: "Backlog".into(),
        in_progress: "Doing".into(),
        in_testing: "QA".into(),
        done: "Shipped".into(),
    };
    let h = Harness::with_config(cfg, false);
    h.bind("proj-1", "a");
    let mut custom = issue("1", "proj-1");
    custom.labels = vec!["Robot".into()];
    custom.state = "backlog".into();
    h.linear.upsert(custom);
    h.runner.script(
        "DEMO-1",
        Script {
            set_state: Some("QA"),
            ..Script::success()
        },
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    // The fixed rules of the prompt carry the configured names, Done included.
    let prompt = h.runner.specs.lock().unwrap()[0].stdin.clone();
    for expected in [
        "set the issue status to \"Doing\"",
        "set it to \"QA\"",
        "Never set the status to \"Shipped\"",
        "remove the label \"robot\"",
        "back to \"Backlog\"",
    ] {
        assert!(
            prompt.contains(expected),
            "{expected} is missing from the prompt"
        );
    }
    h.engine.mark_done(h.latest("1").id).await.unwrap();
    assert_eq!(h.linear.get("1").state, "Shipped");
}

#[tokio::test]
async fn run_limits_come_from_the_settings_in_seconds_and_cannot_overflow() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(
        h.runner.limits.lock().unwrap().clone(),
        [RunLimits {
            total: Duration::from_secs(60 * 60),
            stall: Duration::from_secs(15 * 60),
        }]
    );
    // The largest value a configuration file can hold (TOML integers are signed 64-bit).
    let mut cfg = Config::default();
    cfg.settings.run_timeout_mins = i64::MAX as u64;
    cfg.settings.stall_timeout_mins = i64::MAX as u64;
    let h = Harness::with_config(cfg, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(
        h.runner.limits.lock().unwrap().clone(),
        [RunLimits {
            total: Duration::from_secs(u64::MAX),
            stall: Duration::from_secs(u64::MAX),
        }]
    );
}

#[tokio::test]
async fn a_job_stopped_while_its_issue_is_being_read_does_not_start() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.poll_once().await.unwrap();
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::Queued);
    // The user stops the job at the moment the engine re-reads the issue before starting it.
    let stopper = engine.clone();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        stopper.cancel_job(job.id).unwrap();
    }));
    assert!(engine.dispatch().await.unwrap().is_empty());
    engine.wait_idle().await;
    assert_eq!(
        engine.status().running,
        0,
        "nothing stays registered as running"
    );
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::Canceled);
    assert_eq!(job.reason.as_deref(), Some("stopped by the user"));
    assert!(runner.started().is_empty());
}

#[tokio::test]
async fn the_viewer_is_asked_for_once_and_remembered() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner,
    );
    fake.upsert(issue("1", "proj-1"));
    for _ in 0..3 {
        engine.tick().await.unwrap();
        engine.wait_idle().await;
    }
    assert_eq!(spy.viewer_calls(), 1);
    assert!(engine.status().viewer.is_some());
}

#[tokio::test]
async fn issues_that_do_not_match_the_trigger_are_ignored_even_when_linear_returns_them() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let mut other_assignee = issue("1", "proj-1");
    other_assignee.assignee_id = Some("someone-else".into());
    let mut no_label = issue("2", "proj-1");
    no_label.labels.clear();
    let mut in_progress = issue("3", "proj-1");
    in_progress.state = "In Progress".into();
    let fake = FakeLinear::new("user-1");
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    // Linear answers every query with these issues, whatever their assignee, label or status.
    for issue in [&other_assignee, &no_label, &in_progress] {
        fake.upsert(issue.clone());
    }
    *spy.candidates.lock().unwrap() = Some(vec![other_assignee, no_label, in_progress]);
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    assert!(engine.list_jobs(10).unwrap().is_empty());
    assert!(runner.started().is_empty());
    // A queued job whose issue stops matching is canceled although Linear still lists the issue.
    *spy.candidates.lock().unwrap() = None;
    fake.upsert(issue("4", "proj-1"));
    engine.poll_once().await.unwrap();
    let queued = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(queued.state, JobState::Queued);
    fake.update("4", |i| i.assignee_id = Some("someone-else".into()));
    *spy.candidates.lock().unwrap() = Some(vec![fake.get("4")]);
    engine.poll_once().await.unwrap();
    let canceled = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(canceled.state, JobState::Canceled);
    assert!(canceled.reason.unwrap().contains("no longer matches"));
}

// Jobs that wait.

#[tokio::test]
async fn a_queued_job_waits_again_when_its_directory_disappears() {
    let h = Harness::new(2, false);
    let dir = h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
    std::fs::remove_dir_all(&dir).unwrap();
    h.engine.poll_once().await.unwrap();
    let job = h.latest("1");
    assert_eq!(job.state, JobState::WaitingBinding);
    assert!(job.reason.unwrap().contains("does not exist"));
    assert_eq!((job.binding_id, job.directory), (None, None));
    assert_eq!(h.engine.status().waiting, 1);
    // Once the folder is back, the same job is queued again and runs.
    std::fs::create_dir_all(&dir).unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.engine.list_jobs(10).unwrap().len(), 1);
}

#[tokio::test]
async fn a_directory_that_disappears_before_the_start_keeps_the_job_waiting() {
    let h = Harness::new(2, false);
    let dir = h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    let job = h.latest("1");
    assert_eq!(job.state, JobState::WaitingBinding);
    assert!(job.reason.unwrap().contains("does not exist"));
    assert!(h.runner.started().is_empty());
    std::fs::create_dir_all(&dir).unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn a_binding_removed_while_queued_keeps_the_job_from_starting() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert!(h.engine.remove_binding("proj-1").unwrap());
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert!(h.runner.started().is_empty());
    // The next check moves it to waiting, with the reason.
    h.engine.poll_once().await.unwrap();
    let job = h.latest("1");
    assert_eq!(job.state, JobState::WaitingBinding);
    assert!(job.reason.unwrap().contains("not bound"));
}

#[tokio::test]
async fn a_queued_job_follows_its_project_to_a_replaced_binding() {
    let h = Harness::new(2, false);
    let dir = h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").binding_id.as_deref(), Some("proj-1"));
    // The project gets a new binding for the same folder; the old binding id is gone.
    h.engine
        .add_binding(Binding {
            id: "bind-2".into(),
            ..demo_binding("proj-1", &dir)
        })
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::AwaitingAcceptance);
    assert_eq!(job.binding_id.as_deref(), Some("bind-2"));
}

/// An engine whose `proj-1` binding runs Codex and whose `proj-2` binding runs Claude (the
/// default), with Codex ready while the returned flag is set.
fn codex_and_claude_engine() -> (
    tempfile::TempDir,
    Engine,
    FakeLinear,
    Arc<ScriptedRunner>,
    Arc<AtomicBool>,
) {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(Binding {
        executor: Some(ExecutorKind::Codex),
        ..demo_binding("proj-1", &alpha)
    });
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let linear = FakeLinear::new("user-1");
    let runner = Arc::new(ScriptedRunner::new(linear.clone(), false));
    let codex_ready = Arc::new(AtomicBool::new(true));
    let engine = build_engine(
        root.path(),
        &cfg,
        Arc::new(linear.clone()),
        Arc::new(CodexReadiness(codex_ready.clone())),
        runner.clone(),
    );
    linear.upsert(issue("1", "proj-1"));
    linear.upsert(issue("2", "proj-2"));
    (root, engine, linear, runner, codex_ready)
}

fn job_of(engine: &Engine, uuid: &str) -> Job {
    engine
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|job| job.issue_uuid == uuid)
        .expect("no job for issue")
}

#[tokio::test]
async fn an_executor_that_is_not_ready_keeps_its_job_waiting() {
    let (_root, engine, _linear, runner, codex_ready) = codex_and_claude_engine();
    codex_ready.store(false, Ordering::SeqCst);
    engine.poll_once().await.unwrap();
    // The Codex job waits from the start; the Claude job is not held up by it.
    let waiting = job_of(&engine, "1");
    assert_eq!(waiting.state, JobState::WaitingBinding);
    assert_eq!(
        waiting.reason.as_deref(),
        Some("executor codex is not ready")
    );
    assert_eq!(job_of(&engine, "2").state, JobState::Queued);
    engine.dispatch().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(runner.started(), ["DEMO-2"]);
    // Once Codex works, the same job is queued again and runs.
    codex_ready.store(true, Ordering::SeqCst);
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(runner.started(), ["DEMO-2", "DEMO-1"]);
    assert_eq!(job_of(&engine, "1").state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn an_executor_that_stops_being_ready_before_the_start_keeps_its_job_waiting() {
    let (_root, engine, _linear, runner, codex_ready) = codex_and_claude_engine();
    engine.poll_once().await.unwrap();
    assert_eq!(job_of(&engine, "1").state, JobState::Queued);
    codex_ready.store(false, Ordering::SeqCst);
    engine.dispatch().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(runner.started(), ["DEMO-2"]);
    let waiting = job_of(&engine, "1");
    assert_eq!(waiting.state, JobState::WaitingBinding);
    assert_eq!(
        waiting.reason.as_deref(),
        Some("executor codex is not ready")
    );
}

#[tokio::test]
async fn a_waiting_job_is_canceled_when_its_issue_no_longer_matches() {
    let h = Harness::new(2, false);
    h.linear.upsert(issue("1", "proj-1")); // no binding
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::WaitingBinding);
    h.linear.update("1", |i| i.state = "In Progress".into());
    h.engine.tick().await.unwrap();
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Canceled);
    assert!(job.reason.unwrap().contains("no longer matches"));
    assert_eq!(h.engine.status().waiting, 0);
}

// How a run is recorded.

#[tokio::test]
async fn the_stored_summary_is_one_clean_line() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    let long = "x".repeat(600);
    let report = format!(r#"{{"result":"in_testing","summary":"pushed\nto\ttest {long}"}}"#);
    h.runner.script(
        "DEMO-1",
        Script {
            final_text: Some(report),
            ..Script::success()
        },
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    let summary = h.latest("1").summary.unwrap();
    assert!(summary.starts_with("pushed to test xxx"), "{summary}");
    assert_eq!(summary.chars().count(), 501);
    assert!(summary.ends_with('…'));
    assert!(!summary.contains(['\n', '\t']));
}

#[tokio::test]
async fn a_result_that_cannot_be_verified_fails_the_job() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    // The run ends once the gate opens; the first thing the engine then does is read the issue.
    h.linear.fail_next("Linear is down");
    h.runner.gate.add_permits(1);
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(
        job.reason.as_deref(),
        Some("could not verify the result in Linear: Linear is down")
    );
    assert_eq!(job.verified_updated_at, None);
    assert_eq!(job.summary.as_deref(), Some("pushed to test"));
}

#[tokio::test]
async fn a_stopped_run_stays_stopped_when_linear_cannot_be_read() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.engine.cancel_job(h.latest("1").id).unwrap();
    h.linear.fail_next("Linear is down");
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Canceled);
    assert!(
        job.reason
            .unwrap()
            .starts_with("could not verify the result in Linear"),
    );
}

#[tokio::test]
async fn the_process_runner_runs_the_executor_and_reports_its_final_text() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    let result =
        r#"{"type":"result","result":"done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}"}"#;
    let steps = serde_json::json!([{ "out": result }, { "exit": 0 }]);
    std::fs::write(&script, steps.to_string()).unwrap();
    let spec = CommandSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_fake-agent")),
        args: vec![script.to_string_lossy().into_owned()],
        cwd: dir.path().to_path_buf(),
        stdin: "prompt".into(),
    };
    let limits = RunLimits {
        total: Duration::from_secs(30),
        stall: Duration::from_secs(30),
    };
    let log = dir.path().join("logs").join("1.jsonl");
    let output = ProcessJobRunner
        .run(
            spec,
            ExecutorKind::Claude,
            log.clone(),
            limits,
            CancellationToken::new(),
        )
        .await;
    assert_eq!(output.exit, ExitKind::Exited(Some(0)));
    assert_eq!(
        output.final_text.as_deref(),
        Some("done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}")
    );
    assert!(log.exists());
}

// Stop, retry and mark done.

#[tokio::test]
async fn a_queued_or_waiting_job_can_be_stopped_before_it_starts() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.linear.upsert(issue("3", "proj-3")); // no binding
    h.engine.tick().await.unwrap();
    let (queued, waiting) = (h.latest("2"), h.latest("3"));
    assert_eq!(queued.state, JobState::Queued);
    assert_eq!(waiting.state, JobState::WaitingBinding);
    h.engine.cancel_job(queued.id).unwrap();
    h.engine.cancel_job(waiting.id).unwrap();
    for id in [queued.id, waiting.id] {
        let job = job_by_id(&h, id);
        assert_eq!(job.state, JobState::Canceled);
        assert_eq!(job.reason.as_deref(), Some("stopped by the user"));
    }
    let err = h.engine.cancel_job(queued.id).unwrap_err();
    assert!(err.to_string().contains("cannot be stopped"), "{err}");
    let err = h.engine.cancel_job(9999).unwrap_err();
    assert!(err.to_string().contains("not found"), "{err}");
    // The stopped jobs stay stopped; the run in progress is not affected.
    h.runner.gate.add_permits(1);
    h.settle().await;
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn a_stopped_run_can_be_retried() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let job = h.latest("1");
    assert!(h.engine.retry_job(job.id).is_err(), "it is still running");
    h.engine.cancel_job(job.id).unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::Canceled);
    h.engine.retry_job(job.id).unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
}

#[tokio::test]
async fn a_run_that_needs_info_can_be_retried_but_an_unknown_job_cannot() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::needs_info());
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::NeedsInfo);
    assert!(job.summary.is_some() && job.finished_at.is_some());
    h.engine.retry_job(job.id).unwrap();
    // The row starts over: nothing of the previous attempt's outcome is left on it.
    let queued = h.latest("1");
    assert_eq!(queued.state, JobState::Queued);
    assert!(
        queued.summary.is_none() && queued.finished_at.is_none() && queued.started_at.is_none()
    );
    assert!(h.engine.retry_job(9999).is_err());
}

#[tokio::test]
async fn mark_done_waits_for_linear_and_uses_the_configured_status_name() {
    let mut cfg = Config::default();
    cfg.settings.trigger.done = "Released".into();
    let h = Harness::with_config(cfg, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    h.linear.fail_next("Linear is down");
    let err = h.engine.mark_done(job.id).await.unwrap_err();
    assert!(format!("{err:#}").contains("Linear is down"));
    assert_eq!(
        h.latest("1").state,
        JobState::AwaitingAcceptance,
        "Linear refused, so the job is unchanged"
    );
    assert!(h.linear.set_state_calls().is_empty());
    h.engine.mark_done(job.id).await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
    assert_eq!(
        h.linear.set_state_calls(),
        [("1".to_string(), "Released".to_string())]
    );
    assert!(h.engine.mark_done(9999).await.is_err());
}

// Configuration, status and doctor.

#[tokio::test]
async fn bindings_are_replaced_removed_validated_and_saved() {
    let h = Harness::new(2, false);
    let config_path = h.root.path().join("config.toml");
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    assert_eq!(h.engine.bindings().len(), 2);
    // The same project again replaces its binding.
    let moved = h.bind("proj-1", "c");
    let bindings = h.engine.bindings();
    assert_eq!(bindings.len(), 2);
    let replaced = bindings.iter().find(|b| b.project_id == "proj-1").unwrap();
    assert_eq!(replaced.directory, moved);
    assert_eq!(
        Config::load(&config_path).unwrap().bindings,
        bindings,
        "saved"
    );
    // So does the same id with another project.
    let renamed = Binding {
        project_id: "proj-7".into(),
        ..replaced.clone()
    };
    h.engine.add_binding(renamed.clone()).unwrap();
    let bindings = h.engine.bindings();
    assert_eq!(bindings.len(), 2);
    assert!(bindings.contains(&renamed));
    assert!(bindings.iter().all(|b| b.project_id != "proj-1"));
    // And so does the same project under another id: a project has one folder.
    let other_id = Binding {
        id: "bind-9".into(),
        ..renamed.clone()
    };
    h.engine.add_binding(other_id.clone()).unwrap();
    let bindings = h.engine.bindings();
    assert_eq!(bindings.len(), 2);
    assert!(bindings.contains(&other_id) && !bindings.contains(&renamed));
    // Invalid bindings are refused and change nothing, in memory or on disk.
    let blank_project = Binding {
        project_id: "  ".into(),
        ..renamed.clone()
    };
    let bad_model = Binding {
        id: "proj-9".into(),
        project_id: "proj-9".into(),
        model: Some("--oops".into()),
        ..renamed
    };
    assert!(h.engine.add_binding(blank_project).is_err());
    assert!(h.engine.add_binding(bad_model).is_err());
    assert_eq!(h.engine.bindings(), bindings);
    assert_eq!(Config::load(&config_path).unwrap().bindings, bindings);
    // Removal says whether there was anything to remove.
    assert!(h.engine.remove_binding("proj-2").unwrap());
    assert!(!h.engine.remove_binding("proj-2").unwrap());
    assert_eq!(h.engine.bindings().len(), 1);
    assert_eq!(
        Config::load(&config_path).unwrap().bindings,
        h.engine.bindings()
    );
    // It is the binding's id that counts, not its project.
    assert!(!h.engine.remove_binding("proj-7").unwrap());
    assert!(h.engine.remove_binding("bind-9").unwrap());
    assert!(h.engine.bindings().is_empty());
}

#[tokio::test]
async fn an_unreadable_configuration_stops_the_engine_from_starting() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("config.toml"), "settings = 5\n").unwrap();
    let linear = FakeLinear::new("user-1");
    let result = Engine::new(EngineDeps {
        config_path: root.path().join("config.toml"),
        store: Store::open_in_memory().unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.path().join("logs"),
    });
    let err = result.err().expect("a broken configuration must not start");
    assert!(format!("{err:#}").contains("config.toml"), "{err:#}");
}

#[tokio::test]
async fn status_reports_viewer_poll_time_and_waiting_jobs() {
    let h = Harness::new(1, true);
    assert_eq!(h.engine.status(), StatusSnapshot::default());
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2")); // no binding
    h.engine.tick().await.unwrap();
    let status = h.engine.status();
    assert_eq!(
        status.viewer,
        Some(Viewer {
            id: "user-1".into(),
            name: "Jane Doe".into()
        })
    );
    assert!(status.linear_ok && status.last_error.is_none() && !status.paused);
    assert!(status.last_poll_at.is_some());
    assert_eq!((status.running, status.queued, status.waiting), (1, 0, 1));
    // A paused engine makes no check, so the time of the last one stays.
    h.engine.pause().unwrap();
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.engine.status().last_poll_at, status.last_poll_at);
    h.runner.gate.add_permits(1);
    h.settle().await;
}

#[tokio::test]
async fn doctor_reports_viewer_workspace_executors_and_bindings() {
    let h = Harness::new(2, false);
    let dir = h.bind("proj-1", "a");
    let report = h.engine.doctor().await;
    assert_eq!(
        report.viewer.map(|viewer| viewer.id),
        Some("user-1".to_string())
    );
    assert_eq!(report.linear_error, None);
    assert!(report.workspace.unwrap().ok());
    assert_eq!(report.executors.len(), 2);
    assert_eq!(report.bindings.len(), 1);
    assert!(report.bindings[0].directory_exists && report.bindings[0].executor_ready);
    // Each call looks again: a deleted folder and a missing status show up.
    std::fs::remove_dir_all(dir).unwrap();
    h.linear.set_statuses(&["Todo", "In Progress", "Done"]);
    let report = h.engine.doctor().await;
    assert!(!report.bindings[0].directory_exists);
    assert_eq!(
        report.workspace.unwrap().missing_states,
        ["Demo: In Testing"]
    );
}

#[tokio::test]
async fn doctor_reports_a_linear_failure_and_still_checks_the_rest() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.fail_next("Linear is down");
    let report = h.engine.doctor().await;
    assert!(report.viewer.is_none());
    assert!(report.linear_error.unwrap().contains("Linear is down"));
    assert!(
        report.workspace.is_none(),
        "no workspace check without a connection"
    );
    assert_eq!(report.executors.len(), 2);
    assert_eq!(report.bindings.len(), 1);
}

#[tokio::test]
async fn doctor_keeps_the_error_of_a_failed_workspace_check() {
    let root = tempfile::tempdir().unwrap();
    let linear = SpyLinear {
        fail_after_viewer: true,
        ..SpyLinear::new(FakeLinear::new("user-1"))
    };
    let engine = build_engine(
        root.path(),
        &Config::default(),
        Arc::new(linear),
        Arc::new(FakeReadiness),
        Arc::new(ScriptedRunner::new(FakeLinear::new("user-1"), false)),
    );
    let report = engine.doctor().await;
    assert!(report.viewer.is_some(), "Linear itself answered");
    assert!(report.workspace.is_none());
    assert_eq!(
        report.linear_error.as_deref(),
        Some("workspace check failed: Linear is down")
    );
}

#[tokio::test]
async fn doctor_asks_for_fresh_executor_checks_and_the_loop_for_cached_ones() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), false));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let engine = build_engine(
        root.path(),
        &cfg,
        Arc::new(fake.clone()),
        Arc::new(RecordsFresh(calls.clone())),
        runner,
    );
    engine.doctor().await;
    assert_eq!(*calls.lock().unwrap(), [true]);
    calls.lock().unwrap().clear();
    fake.upsert(issue("1", "proj-1"));
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    let calls = calls.lock().unwrap().clone();
    assert!(
        !calls.is_empty() && calls.iter().all(|fresh| !fresh),
        "{calls:?}"
    );
}

// Logs.

#[tokio::test]
async fn logs_live_under_the_logs_directory_and_missing_ones_are_reported() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    let queued = h.latest("1");
    assert_eq!(queued.log_path, None);
    assert!(h.engine.log_tail(queued.id, 10).unwrap().is_empty());
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    let log = h.root.path().join("logs").join(format!("{}.jsonl", job.id));
    assert_eq!(job.log_path.as_deref(), Some(log.as_path()));
    assert!(h.engine.log_tail(9999, 10).is_err(), "no such job");
    std::fs::remove_file(&log).unwrap();
    let err = h.engine.log_tail(job.id, 10).unwrap_err();
    assert!(format!("{err:#}").contains("cannot read"), "{err:#}");
}

#[tokio::test]
async fn log_tail_covers_exactly_the_last_mebibyte() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    let log = job.log_path.clone().unwrap();
    // 11,000 lines of exactly 100 bytes (99 characters and the newline) are 1,100,000 bytes.
    // The last 1 MiB starts at byte 51,424, inside line 514, so line 515 is the first whole one.
    let text: String = (0..11_000)
        .map(|n| format!("{n:06}{}\n", "x".repeat(93)))
        .collect();
    std::fs::write(&log, text).unwrap();
    let tail = h.engine.log_tail(job.id, 100_000).unwrap();
    assert_eq!(tail.len(), 10_485);
    assert!(tail[0].starts_with("000515"), "{}", tail[0]);
    assert!(tail.last().unwrap().starts_with("010999"));
    // A log that fits in the window is returned from its very first line.
    std::fs::write(&log, "first\nsecond\n").unwrap();
    assert_eq!(h.engine.log_tail(job.id, 10).unwrap(), ["first", "second"]);
    assert_eq!(h.engine.log_tail(job.id, 1).unwrap(), ["second"]);
    assert!(h.engine.log_tail(job.id, 0).unwrap().is_empty());
}
