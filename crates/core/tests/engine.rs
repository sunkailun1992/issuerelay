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
#[cfg(target_os = "macos")]
use common::orphan::Orphan;
use common::{
    engine_kit::{FakeReadiness, FullOnlyReadiness, Harness, Homes, Script, ScriptedRunner},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    config::{
        Binding, Config, Effort, ExecutorKind, LeanStatus, PermissionProfile, Settings,
        TriggerNames,
    },
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{
        Engine, EngineDeps, EngineEvent, IssueStatus, IssueView, JobRunner, ProcessJobRunner,
        StatusSnapshot,
    },
    env::{EnvSource, ExecEnv},
    executor::CommandSpec,
    linear::{Disconnected, LinearApi, Project, Team, Viewer},
    model::{Issue, Job, JobState},
    runner::{ExitKind, RunLimits},
    store::{NewJob, Store},
    trigger::WaitReasonView,
};
#[cfg(target_os = "macos")]
use issuerelay_core::{model::ProcessTag, runner, store::Leftover};
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
    assert_eq!(
        h.linear.set_state_calls().len(),
        1,
        "a job that is not awaiting acceptance is rejected before Linear is called"
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
async fn a_config_file_edited_while_running_is_never_overwritten() {
    let h = Harness::new(2, false);
    let config_path = h.root.path().join("config.toml");
    // In M1 a hand edit is the only way to change the limits, intervals and templates.
    let saved = std::fs::read_to_string(&config_path).unwrap();
    let hand_edit = saved.replace("run_timeout_mins = 60", "run_timeout_mins = 2");
    assert_ne!(hand_edit, saved, "the file names run_timeout_mins");
    std::fs::write(&config_path, &hand_edit).unwrap();

    let err = h.engine.pause().unwrap_err();
    assert_eq!(
        err.to_string(),
        "config.toml was changed while IssueRelay was running; restart the host to use it \
         (nothing was saved)"
    );
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), hand_edit);
    assert!(
        !h.engine.status().paused,
        "the refused change reached memory"
    );

    // A restarted host loads the edit and saves on top of it.
    let linear = FakeLinear::new("user-1");
    let restarted = Engine::new(EngineDeps {
        config_path: config_path.clone(),
        store: Store::open_in_memory().unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: h.root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    restarted.pause().unwrap();
    let on_disk = Config::load(&config_path).unwrap();
    assert!(on_disk.settings.paused);
    assert_eq!(on_disk.settings.run_timeout_mins, 2);
    // Its own save does not count as a change made behind its back.
    restarted.resume().unwrap();
    assert!(!Config::load(&config_path).unwrap().settings.paused);
}

#[tokio::test]
async fn a_config_file_created_or_removed_while_running_is_a_change_too() {
    // Started without a file: the defaults.
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    let linear = FakeLinear::new("user-1");
    let engine = Engine::new(EngineDeps {
        config_path: config_path.clone(),
        store: Store::open_in_memory().unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    std::fs::write(&config_path, "[settings]\npoll_interval_secs = 10\n").unwrap();
    let err = engine.pause().unwrap_err();
    assert!(
        err.to_string()
            .contains("was changed while IssueRelay was running"),
        "{err:#}"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "[settings]\npoll_interval_secs = 10\n"
    );
    // A file the engine saved and someone then removed.
    std::fs::remove_file(&config_path).unwrap();
    engine.pause().unwrap();
    std::fs::remove_file(&config_path).unwrap();
    let err = engine.resume().unwrap_err();
    assert!(
        err.to_string()
            .contains("was changed while IssueRelay was running"),
        "{err:#}"
    );
    assert!(!config_path.exists(), "the removed file was written again");
    assert!(engine.status().paused);
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
        user_home: None,
        claude_config_dir: None,
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
        lean: true,
        effort: None,
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
        user_home: None,
        claude_config_dir: None,
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

// The engine puts the executor's process, as the runner reports it, on the row of the run: the
// record a host started after a crash uses to end what the run left behind. (The scripted runner
// reports a made-up process; no test lets a host end it.)
#[tokio::test]
async fn a_started_run_records_its_executor_process() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), true));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: Arc::new(fake),
        readiness: Arc::new(FakeReadiness),
        job_runner: runner.clone(),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    engine.tick().await.unwrap();
    // The scripted runner reports the process before it counts the run as started.
    eventually("the run to start", || !runner.started().is_empty()).await;
    let recorded: (Option<i64>, Option<String>) = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row("SELECT process_pid, process_started FROM jobs", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(
        recorded,
        (Some(4242), Some("1700000000.000001".to_string()))
    );
    runner.gate.add_permits(1);
    engine.wait_idle().await;
    assert_eq!(
        engine.list_jobs(10).unwrap()[0].state,
        JobState::AwaitingAcceptance
    );
}

/// Waits, for at most 10 seconds, until `path` exists.
#[cfg(target_os = "macos")]
async fn wait_for_path(path: &Path) {
    let give_up = std::time::Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < give_up,
            "{} never appeared",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits, for at most 10 seconds, until the process in `tag` no longer runs (a zombie counts as
/// ended), without reaping it: the guard of the test can still end its group if anything of it
/// survived.
#[cfg(target_os = "macos")]
async fn wait_until_gone(tag: &ProcessTag) {
    let give_up = std::time::Instant::now() + Duration::from_secs(10);
    while runner::process_tag(tag.pid).as_ref() == Some(tag) {
        assert!(
            std::time::Instant::now() < give_up,
            "the leftover executor still runs"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// What a host that died in the middle of a run left in the database at `db`: the run as started,
/// with the record of its executor. Returns the job's id.
#[cfg(target_os = "macos")]
fn record_interrupted_run(db: &Path, root: &Path, tag: &ProcessTag) -> i64 {
    let store = Store::open(db).unwrap();
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
    assert!(store.mark_started(job.id, &root.join("1.jsonl")).unwrap());
    store.set_process(job.id, tag).unwrap();
    job.id
}

/// An engine over the database at `db`, with fakes for Linear, the executors and the runs.
#[cfg(target_os = "macos")]
fn engine_on(db: &Path, root: &Path) -> Engine {
    let linear = FakeLinear::new("user-1");
    Engine::new(EngineDeps {
        config_path: root.join("config.toml"),
        store: Store::open(db).unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap()
}

// The loop of a host that starts after a crash ends the executor that a run of the previous host
// left running, and then forgets its record. Only macOS records executors.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_loop_ends_an_executor_the_previous_host_left_running() {
    let root = tempfile::tempdir().unwrap();
    let beat = root.path().join("beat");
    let script = root.path().join("script.json");
    let steps = serde_json::json!([{"heartbeat": beat, "every_ms": 50}, {"sleep_ms": 30_000}]);
    std::fs::write(&script, steps.to_string()).unwrap();
    let mut orphan = Orphan::spawn(&script);
    let beat_len = || std::fs::metadata(&beat).map_or(0, |m| m.len());
    let give_up = std::time::Instant::now() + Duration::from_secs(10);
    while beat_len() == 0 {
        assert!(
            std::time::Instant::now() < give_up,
            "the heartbeat child never ran"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let tag = runner::process_tag(orphan.id()).expect("a live process has a start time");
    let db = root.path().join("state.db");
    record_interrupted_run(&db, root.path(), &tag);
    let engine = engine_on(&db, root.path());
    assert!(orphan.is_running(), "creating the engine ends nothing yet");
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&engine, &shutdown);
    wait_until_gone(&tag).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = beat_len();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(beat_len(), settled, "the heartbeat child survived");
    assert!(
        orphan.exit_within(Duration::from_secs(10)).await.is_some(),
        "the leftover executor was not reaped"
    );
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .expect("the loop did not stop")
        .unwrap();
    assert_eq!(engine.list_jobs(10).unwrap()[0].state, JobState::Failed);
    let left = Store::open(&db).unwrap().leftovers().unwrap();
    assert!(left.is_empty(), "the swept record stayed: {left:?}");
}

// A host that stops in the middle of its sweep (here while the executor takes its grace period)
// leaves the record on the row, so the next host, which recovers no run, still finds the executor
// and ends it. The leftover takes the first SIGTERM and dies of the second.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_host_that_stops_during_the_sweep_leaves_it_to_the_next_start() {
    use std::os::unix::process::ExitStatusExt;

    let root = tempfile::tempdir().unwrap();
    let (ready, termed) = (root.path().join("ready"), root.path().join("termed"));
    let mut orphan = Orphan::shell(
        r#"trap 'trap - TERM; : > "$2"' TERM; : > "$1"; while :; do sleep 1; done"#,
        &[&ready, &termed],
    );
    wait_for_path(&ready).await;
    let tag = runner::process_tag(orphan.id()).expect("a live process has a start time");
    let db = root.path().join("state.db");
    let job_id = record_interrupted_run(&db, root.path(), &tag);
    let first = engine_on(&db, root.path());
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&first, &shutdown);
    wait_for_path(&termed).await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    drop(first);
    assert_eq!(
        runner::process_tag(orphan.id()).as_ref(),
        Some(&tag),
        "the first host did not stop in the middle of its sweep"
    );
    assert_eq!(
        Store::open(&db).unwrap().leftovers().unwrap(),
        [Leftover {
            job_id,
            process: tag.clone()
        }]
    );
    let second = engine_on(&db, root.path());
    let worker = spawn_loop(&second, &shutdown);
    wait_until_gone(&tag).await;
    let status = orphan
        .exit_within(Duration::from_secs(10))
        .await
        .expect("the leftover executor was not reaped");
    assert_eq!(status.signal(), Some(libc::SIGTERM));
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), worker)
        .await
        .expect("the loop did not stop")
        .unwrap();
    let left = Store::open(&db).unwrap().leftovers().unwrap();
    assert!(left.is_empty(), "the swept record stayed: {left:?}");
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
        lean: true,
        effort: None,
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
        user_home: None,
        claude_config_dir: None,
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
    /// Runs once, just before the first `set_state()` call is answered.
    before_set_state: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// `issue()` fails with this message for this uuid.
    fail_issue: Mutex<Option<(String, String)>>,
    /// The n-th `issue()` call (counting from 1) panics.
    panic_on_issue_call: Mutex<Option<usize>>,
    /// When set, every `viewer()` call waits for a permit before it is answered.
    viewer_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    /// When set, every `issue()` call waits for a permit before it is answered.
    issue_gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
    /// When set, `projects()` answers with these.
    projects: Mutex<Option<Vec<Project>>>,
    issue_calls: AtomicUsize,
    viewer_calls: AtomicUsize,
}

impl SpyLinear {
    fn new(inner: FakeLinear) -> Self {
        Self {
            inner,
            candidates: Mutex::new(None),
            fail_after_viewer: false,
            before_issue: Mutex::new(None),
            before_set_state: Mutex::new(None),
            fail_issue: Mutex::new(None),
            panic_on_issue_call: Mutex::new(None),
            viewer_gate: Mutex::new(None),
            issue_gate: Mutex::new(None),
            projects: Mutex::new(None),
            issue_calls: AtomicUsize::new(0),
            viewer_calls: AtomicUsize::new(0),
        }
    }

    fn viewer_calls(&self) -> usize {
        self.viewer_calls.load(Ordering::SeqCst)
    }

    fn issue_calls(&self) -> usize {
        self.issue_calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LinearApi for SpyLinear {
    async fn viewer(&self) -> Result<Viewer> {
        self.viewer_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self.viewer_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.expect("gate closed").forget();
        }
        let viewer = self.inner.viewer().await;
        if self.fail_after_viewer {
            self.inner.fail_next("Linear is down\n(try again)");
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

    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>> {
        self.inner.labeled_issues(label).await
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        let call = self.issue_calls.fetch_add(1, Ordering::SeqCst) + 1;
        let gate = self.issue_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.expect("gate closed").forget();
        }
        let hook = self.before_issue.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        let panics = *self.panic_on_issue_call.lock().unwrap() == Some(call);
        if panics {
            panic!("scripted panic in issue()");
        }
        let failure = self.fail_issue.lock().unwrap().clone();
        if let Some((uuid, message)) = failure
            && uuid == id
        {
            return Err(anyhow::anyhow!(message));
        }
        self.inner.issue(id).await
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        let hook = self.before_set_state.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
        self.inner.set_state(id, state).await
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        let fixed = self.projects.lock().unwrap().clone();
        match fixed {
            Some(projects) => Ok(projects),
            None => self.inner.projects().await,
        }
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

/// A probe that also resolved `env`, as `SystemReadiness` does.
struct ResolvesEnv(ExecEnv);

#[async_trait::async_trait]
impl ReadinessProbe for ResolvesEnv {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        FakeReadiness.executor_reports(fresh).await
    }

    fn env(&self) -> Option<ExecEnv> {
        Some(self.0.clone())
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
async fn a_moved_issue_is_started_by_the_next_cycle_without_waiting_for_the_timer() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &alpha));
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    // The issue moves to the other project while the first cycle re-reads it for the start.
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        fake.update("1", |i| i.project_id = Some("proj-2".into()));
    }));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&engine, &shutdown);
    // The clock never moves here: only the re-assignment itself can wake the loop for the start.
    eventually("the moved issue to start", || {
        runner.started() == ["DEMO-1"]
    })
    .await;
    assert_eq!(runner.specs.lock().unwrap()[0].cwd, beta);
    stop_loop(&shutdown, worker).await;
    engine.wait_idle().await;
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

// Paused time: the pauses between the reads that verify a result pass at once.
#[tokio::test(start_paused = true)]
async fn a_lean_violation_keeps_its_reason_when_linear_cannot_be_read() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script(
        "DEMO-1",
        Script {
            set_state: None,
            exit: ExitKind::LeanNotApplied("auto memory is on".into()),
            final_text: None,
            panic: false,
        },
    );
    h.engine.tick().await.unwrap();
    h.linear.fail_calls("Linear is down", 3);
    h.runner.gate.add_permits(1);
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(
        job.reason.as_deref(),
        Some("lean run did not take effect: auto memory is on")
    );
    // The reason would read the same after a read that worked: this one did not.
    assert_eq!(job.verified_updated_at, None);
}

// The reason of a lean violation keeps what the run showed, so the error that kept its result
// from being read goes into the host's log instead of being lost.
#[test]
fn a_lean_violation_whose_result_cannot_be_read_logs_why() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let (job, warnings) = logged_warnings(|| {
        runtime.block_on(async {
            let h = Harness::new(2, true);
            h.bind("proj-1", "a");
            h.linear.upsert(issue("1", "proj-1"));
            h.runner.script(
                "DEMO-1",
                Script {
                    set_state: None,
                    exit: ExitKind::LeanNotApplied("auto memory is on".into()),
                    final_text: None,
                    panic: false,
                },
            );
            h.engine.tick().await.unwrap();
            h.linear.fail_calls("Linear is down", 3);
            h.runner.gate.add_permits(1);
            h.settle().await;
            h.latest("1")
        })
    });
    assert_eq!(job.state, JobState::Failed);
    // The warnings before the second and the third read say "trying again"; this one comes after
    // the last read.
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("lean") && w.contains("Linear is down")),
        "{warnings:?}"
    );
}

// Paused time: the pauses between the reads that verify a result pass at once.
#[tokio::test(start_paused = true)]
async fn a_result_that_cannot_be_verified_fails_the_job() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    // The run ends once the gate opens; the first thing the engine then does is read the issue,
    // up to three times.
    h.linear.fail_calls("Linear is down", 3);
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

#[tokio::test(start_paused = true)]
async fn a_stopped_run_stays_stopped_when_linear_cannot_be_read() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.engine.cancel_job(h.latest("1").id).unwrap();
    h.linear.fail_calls("Linear is down", 3);
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Canceled);
    assert!(
        job.reason
            .unwrap()
            .starts_with("could not verify the result in Linear"),
    );
}

#[tokio::test(start_paused = true)]
async fn a_verification_read_that_fails_once_is_retried() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.linear.fail_next("Linear is down");
    let before = tokio::time::Instant::now();
    h.runner.gate.add_permits(1);
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::AwaitingAcceptance);
    assert_eq!(job.reason, None);
    assert_eq!(job.verified_updated_at, Some(h.linear.get("1").updated_at));
    // The second read came after the first pause, not at once.
    let waited = before.elapsed();
    assert!(
        waited >= Duration::from_secs(2) && waited < Duration::from_secs(7),
        "{waited:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_result_is_read_three_times_with_pauses_before_the_run_fails() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake, true));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.tick().await.unwrap();
    assert_eq!(spy.issue_calls(), 1, "the re-read before the start");
    *spy.fail_issue.lock().unwrap() = Some(("1".into(), "Linear is down".into()));
    let before = tokio::time::Instant::now();
    runner.gate.add_permits(1);
    engine.wait_idle().await;
    assert_eq!(spy.issue_calls(), 4, "three reads to verify the result");
    // 2 s before the second read, 5 s before the third.
    let waited = before.elapsed();
    assert!(
        waited >= Duration::from_secs(7) && waited < Duration::from_secs(8),
        "{waited:?}"
    );
    let job = job_of(&engine, "1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(
        job.reason.as_deref(),
        Some("could not verify the result in Linear: Linear is down")
    );
}

/// `script`, but the run leaves the issue's status alone: Linear still shows the issue as it was
/// before the run.
fn leaving_the_issue_alone(script: Script) -> Script {
    Script {
        set_state: None,
        ..script
    }
}

/// Lets the gated run end and returns how long, on the paused clock, recording its result took.
async fn finish_and_time(h: &Harness) -> Duration {
    let opened = tokio::time::Instant::now();
    h.runner.gate.add_permits(1);
    h.settle().await;
    opened.elapsed()
}

/// `waited` is `secs` seconds, give or take the timer's rounding.
fn assert_waited(waited: Duration, secs: u64) {
    let secs = Duration::from_secs(secs);
    assert!(
        waited >= secs && waited < secs + Duration::from_secs(1),
        "{waited:?}"
    );
}

// Paused time in the tests below: the pauses before reading a result again pass at once, and the
// paused clock measures them.

#[tokio::test(start_paused = true)]
async fn a_result_read_from_before_the_run_is_read_again_until_linear_shows_the_change() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    let before_run = h.linear.get("1");
    h.engine.tick().await.unwrap();
    // The run moved the issue to In Testing, but the next two reads still answer with the issue as
    // it was before the run, as Linear did in the lean-run acceptance.
    h.linear.serve_stale_reads(before_run, 2);
    let waited = finish_and_time(&h).await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::AwaitingAcceptance);
    assert_eq!(job.reason, None);
    assert_eq!(job.verified_updated_at, Some(h.linear.get("1").updated_at));
    // Read again 15 s and then 30 s later.
    assert_waited(waited, 45);
}

#[tokio::test(start_paused = true)]
async fn a_clean_run_that_left_the_issue_unchanged_fails_after_three_more_reads() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner
        .script("DEMO-1", leaving_the_issue_alone(Script::success()));
    h.engine.tick().await.unwrap();
    let waited = finish_and_time(&h).await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(
        job.reason.as_deref(),
        Some(r#"run finished but the issue status is "Todo", not "In Testing""#)
    );
    assert_eq!(job.verified_updated_at, Some(h.linear.get("1").updated_at));
    // Read again 15, 30 and 45 s later: a run that really failed is recorded about 90 s late.
    assert_waited(waited, 90);
}

#[tokio::test(start_paused = true)]
async fn a_clean_run_that_moved_the_issue_to_another_status_fails_at_once() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script(
        "DEMO-1",
        Script {
            set_state: Some("In Progress"),
            ..Script::success()
        },
    );
    h.engine.tick().await.unwrap();
    let waited = finish_and_time(&h).await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(
        job.reason.as_deref(),
        Some(r#"run finished but the issue status is "In Progress", not "In Testing""#)
    );
    // Linear shows a change made after the run started, so the read is taken as it is.
    assert_waited(waited, 0);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_exited_with_an_error_is_not_read_again() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::exit_code(1));
    h.engine.tick().await.unwrap();
    let waited = finish_and_time(&h).await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.reason.as_deref(), Some("executor exited with code 1"));
    assert_waited(waited, 0);
}

#[tokio::test(start_paused = true)]
async fn a_question_for_the_reporter_is_recorded_without_reading_again() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    // The read shows the issue unchanged (an older snapshot, say). A question leaves the status
    // at Todo either way, so it is recorded as one at once.
    h.runner
        .script("DEMO-1", leaving_the_issue_alone(Script::needs_info()));
    h.engine.tick().await.unwrap();
    let waited = finish_and_time(&h).await;
    assert_eq!(h.latest("1").state, JobState::NeedsInfo);
    assert_waited(waited, 0);
}

#[tokio::test(start_paused = true)]
async fn a_shutdown_ends_the_wait_to_read_a_result_again() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner
        .script("DEMO-1", leaving_the_issue_alone(Script::success()));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually("DEMO-1 to start", || h.runner.started() == ["DEMO-1"]).await;
    // The run ends, its result is read once, and the engine waits to read it again.
    h.runner.gate.add_permits(1);
    quiesce().await;
    let stopping = tokio::time::Instant::now();
    stop_loop(&shutdown, worker).await;
    h.settle().await;
    assert_waited(stopping.elapsed(), 0);
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(
        job.reason.as_deref(),
        Some(r#"run finished but the issue status is "Todo", not "In Testing""#)
    );
}

#[tokio::test(start_paused = true)]
async fn stopping_a_job_ends_the_wait_to_read_its_result_again() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner
        .script("DEMO-1", leaving_the_issue_alone(Script::success()));
    h.engine.tick().await.unwrap();
    h.runner.gate.add_permits(1);
    quiesce().await;
    let stopping = tokio::time::Instant::now();
    h.engine.cancel_job(h.latest("1").id).unwrap();
    h.settle().await;
    assert_waited(stopping.elapsed(), 0);
    // The run had ended on its own: what the last read showed is its result.
    assert_eq!(h.latest("1").state, JobState::Failed);
}

#[tokio::test]
async fn the_process_runner_runs_the_executor_and_reports_its_final_text() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    let result =
        r#"{"type":"result","result":"done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}"}"#;
    // The pause keeps the executor alive while its start is reported (below).
    let steps = serde_json::json!([{ "out": result }, { "sleep_ms": 200 }, { "exit": 0 }]);
    std::fs::write(&script, steps.to_string()).unwrap();
    let spec = CommandSpec {
        program: PathBuf::from(env!("CARGO_BIN_EXE_fake-agent")),
        args: vec![script.to_string_lossy().into_owned()],
        cwd: dir.path().to_path_buf(),
        stdin: "prompt".into(),
        env: Vec::new(),
        lean: false,
        notes: Vec::new(),
    };
    let limits = RunLimits {
        total: Duration::from_secs(30),
        stall: Duration::from_secs(30),
    };
    let log = dir.path().join("logs").join("1.jsonl");
    let reported = Arc::new(Mutex::new(None));
    let sink = reported.clone();
    let output = ProcessJobRunner::new(ExecEnv::inherit().shared())
        .run(
            spec,
            ExecutorKind::Claude,
            log.clone(),
            limits,
            CancellationToken::new(),
            Box::new(move |tag| *sink.lock().unwrap() = Some(tag)),
        )
        .await;
    assert_eq!(output.exit, ExitKind::Exited(Some(0)));
    assert_eq!(
        output.final_text.as_deref(),
        Some("done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}")
    );
    assert!(log.exists());
    // The executor's process reaches the hook where it is recorded: on macOS only.
    let reported = reported.lock().unwrap().clone();
    assert_eq!(
        reported.is_some(),
        cfg!(target_os = "macos"),
        "{reported:?}"
    );
}

// The readiness check replaces the shared environment; the runs that start afterwards get the
// new one.
#[tokio::test]
async fn the_process_runner_gives_each_run_the_current_shared_environment() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("script.json");
    let steps = serde_json::json!([{ "print_env": "ISSUERELAY_TEST_PROXY" }]);
    std::fs::write(&script, steps.to_string()).unwrap();
    let env = ExecEnv::inherit().shared();
    let runner = ProcessJobRunner::new(env.clone());
    let limits = RunLimits {
        total: Duration::from_secs(30),
        stall: Duration::from_secs(30),
    };
    let mut printed = Vec::new();
    for (round, proxy) in [None, Some("http://127.0.0.1:9")].into_iter().enumerate() {
        if let Some(proxy) = proxy {
            env.write().unwrap().proxy = vec![("ISSUERELAY_TEST_PROXY".into(), proxy.into())];
        }
        let spec = CommandSpec {
            program: PathBuf::from(env!("CARGO_BIN_EXE_fake-agent")),
            args: vec![script.to_string_lossy().into_owned()],
            cwd: dir.path().to_path_buf(),
            stdin: String::new(),
            env: Vec::new(),
            lean: false,
            notes: Vec::new(),
        };
        let log = dir.path().join(format!("{round}.jsonl"));
        let output = runner
            .run(
                spec,
                ExecutorKind::Claude,
                log.clone(),
                limits,
                CancellationToken::new(),
                Box::new(|_| {}),
            )
            .await;
        assert_eq!(output.exit, ExitKind::Exited(Some(0)));
        printed.push(std::fs::read_to_string(&log).unwrap());
    }
    assert_eq!(
        printed,
        [
            "ISSUERELAY_TEST_PROXY=<unset>\n",
            "ISSUERELAY_TEST_PROXY=http://127.0.0.1:9\n"
        ]
    );
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
async fn a_row_that_says_running_without_a_run_cannot_be_stopped() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: Arc::new(fake.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(fake, false)),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    engine.poll_once().await.unwrap();
    let job = engine.list_jobs(10).unwrap().remove(0);
    // A row that claims a run which is not there (a crash would leave one behind).
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute("UPDATE jobs SET state = 'running'", [])
        .unwrap();
    let err = engine.cancel_job(job.id).unwrap_err();
    assert!(err.to_string().contains("no run is in progress"), "{err}");
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

#[tokio::test]
async fn mark_done_does_not_overwrite_a_row_that_changed_while_linear_was_updated() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: spy.clone(),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(fake.clone(), false)),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::AwaitingAcceptance);
    // While Linear is being updated, something else changes the row.
    let db_path = db.clone();
    *spy.before_set_state.lock().unwrap() = Some(Box::new(move || {
        rusqlite::Connection::open(&db_path)
            .unwrap()
            .execute("UPDATE jobs SET state = 'failed'", [])
            .unwrap();
    }));
    let err = engine.mark_done(job.id).await.unwrap_err();
    assert!(
        err.to_string().contains("Linear has already been"),
        "the message must say that Linear was updated: {err}"
    );
    assert_eq!(engine.list_jobs(10).unwrap()[0].state, JobState::Failed);
    assert_eq!(fake.get("1").state, "Done");
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

/// The ids of the bindings, in the order the panel lists them.
fn binding_ids(h: &Harness) -> Vec<String> {
    h.engine.bindings().into_iter().map(|b| b.id).collect()
}

#[tokio::test]
async fn a_replaced_binding_keeps_its_place_in_the_list() {
    let h = Harness::new(2, false);
    let config_path = h.root.path().join("config.toml");
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.bind("proj-3", "c");
    assert_eq!(binding_ids(&h), ["proj-1", "proj-2", "proj-3"]);
    // Editing the first of three (same id, same project, another folder) leaves it first, in the
    // configuration file too: the panel lists the bindings in that order.
    let moved = h.bind("proj-1", "d");
    assert_eq!(binding_ids(&h), ["proj-1", "proj-2", "proj-3"]);
    assert_eq!(h.engine.bindings()[0].directory, moved);
    assert_eq!(
        Config::load(&config_path).unwrap().bindings,
        h.engine.bindings()
    );
    // The middle one and the last one stay where they are as well.
    h.bind("proj-2", "e");
    h.bind("proj-3", "f");
    assert_eq!(binding_ids(&h), ["proj-1", "proj-2", "proj-3"]);
    // A binding takes the place of the one it replaces whichever way it replaces it: the same id
    // with another project, or the same project under another id.
    let renamed = Binding {
        project_id: "proj-7".into(),
        ..h.engine.bindings()[1].clone()
    };
    h.engine.add_binding(renamed).unwrap();
    assert_eq!(binding_ids(&h), ["proj-1", "proj-2", "proj-3"]);
    assert_eq!(h.engine.bindings()[1].project_id, "proj-7");
    let other_id = Binding {
        id: "bind-9".into(),
        ..h.engine.bindings()[0].clone()
    };
    h.engine.add_binding(other_id).unwrap();
    assert_eq!(binding_ids(&h), ["bind-9", "proj-2", "proj-3"]);
    // One that replaces two (its id is one's, its project another's) stands where the first of
    // them stood, and the other goes.
    let both = Binding {
        id: "proj-3".into(),
        project_id: "proj-1".into(),
        ..h.engine.bindings()[0].clone()
    };
    assert_eq!(h.engine.bindings()[0].project_id, "proj-1");
    h.engine.add_binding(both).unwrap();
    assert_eq!(binding_ids(&h), ["proj-3", "proj-2"]);
    // Only a binding that replaces nothing goes last.
    h.bind("proj-4", "g");
    h.bind("proj-5", "h");
    assert_eq!(binding_ids(&h), ["proj-3", "proj-2", "proj-4", "proj-5"]);
    assert_eq!(
        Config::load(&config_path).unwrap().bindings,
        h.engine.bindings()
    );
}

#[tokio::test]
async fn a_binding_that_is_refused_does_not_move_anything() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    let before = h.engine.bindings();
    // An edit of the first one that the configuration refuses.
    let refused = Binding {
        directory: PathBuf::from("relative"),
        ..before[0].clone()
    };
    assert!(h.engine.add_binding(refused).is_err());
    assert_eq!(h.engine.bindings(), before);
}

#[tokio::test]
async fn a_binding_needs_an_absolute_directory_and_plain_names() {
    let h = Harness::new(2, false);
    let config_path = h.root.path().join("config.toml");
    let folder = h.bind("proj-1", "a");
    let kept = h.engine.bindings();
    let valid = demo_binding("proj-2", &folder);
    let refused = [
        (
            "a relative directory",
            Binding {
                directory: PathBuf::from("repo"),
                ..valid.clone()
            },
            "directory must be an absolute path",
        ),
        (
            "a dot-relative directory",
            Binding {
                directory: PathBuf::from("./repo"),
                ..valid.clone()
            },
            "directory must be an absolute path",
        ),
        (
            "an empty directory",
            Binding {
                directory: PathBuf::new(),
                ..valid.clone()
            },
            "directory must be an absolute path",
        ),
        (
            "an empty id",
            Binding {
                id: String::new(),
                ..valid.clone()
            },
            ".id must not be empty",
        ),
        (
            "a blank id",
            Binding {
                id: " \t".into(),
                ..valid.clone()
            },
            ".id must not be empty",
        ),
        (
            "an empty project id",
            Binding {
                project_id: String::new(),
                ..valid.clone()
            },
            ".project_id must not be empty",
        ),
        (
            "a blank project id",
            Binding {
                project_id: "  ".into(),
                ..valid.clone()
            },
            ".project_id must not be empty",
        ),
        (
            "a control character in the id",
            Binding {
                id: "bind\u{1b}2".into(),
                ..valid.clone()
            },
            ".id must not contain control characters",
        ),
        (
            "a control character in the project id",
            Binding {
                project_id: "proj\u{0}2".into(),
                ..valid.clone()
            },
            ".project_id must not contain control characters",
        ),
        (
            "an escape character in the project name",
            Binding {
                project_name: "Demo\u{1b}Project".into(),
                ..valid.clone()
            },
            ".project_name must not contain control characters",
        ),
        (
            "a newline in the project name",
            Binding {
                project_name: "Demo\nProject".into(),
                ..valid.clone()
            },
            ".project_name must not contain control characters",
        ),
    ];
    // Each refusal says what is wrong and changes nothing, in memory or on disk.
    for (what, binding, message) in refused {
        let err = h.engine.add_binding(binding).unwrap_err();
        assert!(err.to_string().contains(message), "{what}: {err}");
        assert_eq!(h.engine.bindings(), kept, "{what}");
        assert_eq!(Config::load(&config_path).unwrap().bindings, kept, "{what}");
    }
    // The same binding with an absolute directory and plain names is accepted.
    h.engine.add_binding(valid.clone()).unwrap();
    assert!(h.engine.bindings().contains(&valid));
}

#[tokio::test]
async fn a_hand_edited_binding_with_a_relative_directory_stops_the_engine_from_starting() {
    let root = tempfile::tempdir().unwrap();
    let config_path = root.path().join("config.toml");
    // TOML literal strings (single quotes) keep the backslashes of a Windows path as they are.
    let text = "[[bindings]]\nid = 'p'\nproject_id = 'p'\nproject_name = 'Demo Project'\ndirectory = 'repo'\n";
    std::fs::write(&config_path, text).unwrap();
    let linear = FakeLinear::new("user-1");
    let result = Engine::new(EngineDeps {
        config_path,
        store: Store::open_in_memory().unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    });
    let err = result.err().expect("a relative folder must not start");
    let message = format!("{err:#}");
    assert!(
        message.contains("bindings[0].directory must be an absolute path"),
        "{message}"
    );
    assert!(message.contains("config.toml"), "{message}");
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
        user_home: None,
        claude_config_dir: None,
    });
    let err = result.err().expect("a broken configuration must not start");
    assert!(format!("{err:#}").contains("config.toml"), "{err:#}");
}

/// Every `warn` or `error` event the current thread logs while `f` runs, one string per event:
/// the message, then `name=value` for each field.
fn logged_warnings<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    use std::fmt::{Debug, Write as _};
    use tracing::{Event, Level, Metadata, Subscriber, field::Visit, span};

    struct Capture(Arc<Mutex<Vec<String>>>);

    struct Fields(String);

    impl Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            if field.name() == "message" {
                write!(self.0, "{value:?}").unwrap();
            } else {
                write!(self.0, "{}={value:?}", field.name()).unwrap();
            }
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, metadata: &Metadata<'_>) -> bool {
            *metadata.level() <= Level::WARN
        }

        fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(1)
        }

        fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

        fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut fields = Fields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }

        fn enter(&self, _: &span::Id) {}

        fn exit(&self, _: &span::Id) {}
    }

    let events = Arc::new(Mutex::new(Vec::new()));
    // tracing caches whether a log call is wanted when the call is first reached, and with a
    // single registered subscriber it asks the default of the thread that reaches it first: a
    // parallel test's thread, which has none, would switch the call off here too (a verification
    // read fails in many tests). With a second, idle subscriber registered it asks every
    // registered one instead, as in `tests/host.rs`.
    let _idle = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let result = tracing::subscriber::with_default(Capture(events.clone()), f);
    let events = events.lock().unwrap().clone();
    (result, events)
}

#[test]
fn starting_with_config_keys_this_version_does_not_know_warns_naming_only_the_keys() {
    let start = |text: Option<&str>| {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        if let Some(text) = text {
            std::fs::write(&config_path, text).unwrap();
        }
        let linear = FakeLinear::new("user-1");
        let (engine, warnings) = logged_warnings(|| {
            Engine::new(EngineDeps {
                config_path,
                store: Store::open_in_memory().unwrap(),
                linear: Arc::new(linear.clone()),
                readiness: Arc::new(FakeReadiness),
                job_runner: Arc::new(ScriptedRunner::new(linear, false)),
                logs_dir: root.path().join("logs"),
                user_home: None,
                claude_config_dir: None,
            })
        });
        engine.expect("the file loads, unknown keys and all");
        warnings
    };
    // A value next to a misspelt key may be anything, a pasted key included: only names are logged.
    let warnings = start(Some(
        "api_key = \"secret-value\"\n[settings]\npol_interval = 5\npoll_interval_secs = 10\n",
    ));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for named in [
        "api_key",
        "settings.pol_interval",
        "dropped on the next save",
    ] {
        assert!(warnings[0].contains(named), "{named}: {warnings:?}");
    }
    assert!(!warnings[0].contains("secret-value"), "{warnings:?}");
    assert!(
        !warnings[0].contains("poll_interval_secs"),
        "a known key is not named: {warnings:?}"
    );
    // Nothing to say about a file this version knows completely, a partial one, or no file.
    let known = toml::to_string_pretty(&Config::default()).unwrap();
    for text in [
        Some(known.as_str()),
        Some("[settings]\npoll_interval_secs = 10\n"),
        None,
    ] {
        assert_eq!(start(text), Vec::<String>::new(), "{text:?}");
    }
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
async fn status_hands_out_the_viewers_name_cleaned() {
    // The name is Linear's text. The status line, `issuerelay status`, the MCP `status` tool and
    // the panel all read it from the engine's cache, so it is cleaned where the cache is filled.
    let h = Harness::new(1, false);
    h.linear
        .set_viewer_name("Jane\u{1b}]52;c;ZXZpbA==\u{7}\u{202e}Doe\n");
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let viewer = h.engine.status().viewer;
    assert_eq!(
        viewer,
        Some(Viewer {
            id: "user-1".into(),
            name: "Jane ]52;c;ZXZpbA== Doe".into()
        })
    );
    // The doctor report shows the same name, and only the text was cleaned: the issue assigned
    // to the user's id was still picked up and ran.
    assert_eq!(h.engine.doctor().await.viewer, viewer);
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
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
async fn doctor_checks_only_the_teams_of_the_bound_projects() {
    let h = Harness::new(2, false);
    h.linear
        .set_teams(&[("team-1", "Demo"), ("team-2", "Unrelated")]);
    h.linear
        .set_projects(&[("proj-1", "Demo Project", &["team-1"])]);
    h.linear.set_team_statuses("team-2", &["Backlog"]);
    // No binding yet: every team is looked at, and the unrelated one lacks the statuses.
    let workspace = h.engine.doctor().await.workspace.unwrap();
    assert_eq!(workspace.teams_checked, ["Demo", "Unrelated"]);
    assert!(!workspace.ok(), "{workspace:?}");
    // The binding names the project, whose team is the only one that matters.
    h.bind("proj-1", "a");
    let workspace = h.engine.doctor().await.workspace.unwrap();
    assert_eq!(workspace.teams_checked, ["Demo"]);
    assert!(workspace.ok(), "{workspace:?}");
}

#[tokio::test]
async fn doctor_cleans_the_names_it_took_from_linear() {
    // Linear's user and team names reach the panel and `doctor --json` as the engine hands them
    // over: an escape sequence, a direction override or a line break may not.
    let h = Harness::new(2, false);
    let hostile = "Te\u{1b}[2Jam\u{202e}\nforged line";
    h.linear
        .set_viewer_name("Jane\u{1b}]52;c;ZXZpbA==\u{7}\u{202e}Doe\n");
    h.linear
        .set_teams(&[("team-1", hostile), ("team-2", "Beta")]);
    // In Testing is missing from both teams.
    h.linear.set_statuses(&["Todo", "In Progress", "Done"]);
    let report = h.engine.doctor().await;
    let workspace = report.workspace.unwrap();
    assert_eq!(workspace.teams_checked, ["Te [2Jam forged line", "Beta"]);
    assert_eq!(
        workspace.missing_states,
        ["Te [2Jam forged line: In Testing", "Beta: In Testing"]
    );
    assert_eq!(
        report.viewer.map(|viewer| viewer.name),
        Some("Jane ]52;c;ZXZpbA== Doe".to_string())
    );
}

#[tokio::test]
async fn doctor_reports_a_linear_failure_and_still_checks_the_rest() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.fail_next("Linear is down\n(try again)");
    let report = h.engine.doctor().await;
    assert!(report.viewer.is_none());
    assert_eq!(
        report.linear_error.as_deref(),
        Some("Linear is down (try again)")
    );
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
        Some("workspace check failed: Linear is down (try again)")
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

#[tokio::test]
async fn doctor_reports_the_environment_the_probe_resolved() {
    let root = tempfile::tempdir().unwrap();
    let env = ExecEnv {
        path: Some("/opt/demo/bin".into()),
        path_source: EnvSource::LoginShell,
        ..ExecEnv::inherit()
    };
    let engine = build_engine(
        root.path(),
        &Config::default(),
        Arc::new(FakeLinear::new("user-1")),
        Arc::new(ResolvesEnv(env.clone())),
        Arc::new(ScriptedRunner::new(FakeLinear::new("user-1"), false)),
    );
    assert_eq!(engine.doctor().await.env, Some(env));
    // A probe that resolves nothing leaves the report without an environment.
    let h = Harness::new(2, false);
    assert_eq!(h.engine.doctor().await.env, None);
}

// The report goes out over IPC and `doctor --json`, and is shown in the panel: a password in the
// proxy URL must not go with it. The probe, whose environment the job runner shares, keeps the
// real value.
#[tokio::test]
async fn doctor_masks_proxy_credentials_in_the_report() {
    let root = tempfile::tempdir().unwrap();
    let env = ExecEnv {
        proxy: vec![
            (
                "HTTPS_PROXY".into(),
                "http://jane:s3cret@proxy.example:8080".into(),
            ),
            ("NO_PROXY".into(), "localhost".into()),
        ],
        proxy_source: EnvSource::LoginShell,
        ..ExecEnv::inherit()
    };
    let probe = Arc::new(ResolvesEnv(env.clone()));
    let engine = build_engine(
        root.path(),
        &Config::default(),
        Arc::new(FakeLinear::new("user-1")),
        probe.clone(),
        Arc::new(ScriptedRunner::new(FakeLinear::new("user-1"), false)),
    );
    let report = engine.doctor().await;
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("***@proxy.example:8080"), "{json}");
    assert!(!json.contains("s3cret"), "{json}");
    assert!(!format!("{report:?}").contains("s3cret"));
    // Only the credentials are hidden: the other variables and the sources are as they were.
    let shown = report.env.expect("the probe resolved an environment");
    assert_eq!(
        shown.proxy,
        [
            (
                "HTTPS_PROXY".to_string(),
                "http://***@proxy.example:8080".to_string()
            ),
            ("NO_PROXY".to_string(), "localhost".to_string()),
        ]
    );
    assert_eq!(shown.proxy_source, EnvSource::LoginShell);
    assert_eq!(probe.env().unwrap(), env, "the real value is untouched");
}

// Logs.

#[tokio::test]
async fn logs_live_under_the_logs_directory_and_unreadable_ones_are_reported() {
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
    // A log the cleanup deleted reads as empty; one that is there but cannot be read is an error.
    std::fs::remove_file(&log).unwrap();
    assert!(h.engine.log_tail(job.id, 10).unwrap().is_empty());
    std::fs::create_dir(&log).unwrap();
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

// Log retention.

/// Dates `path` `hours` hours back, as if it had been written then.
fn age_file(path: &Path, hours: u64) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(std::time::SystemTime::now() - Duration::from_secs(hours * 3600))
        .unwrap();
}

/// Waits until `done` holds, letting the other tasks run in between. The wait is in real time:
/// the cleanup runs on the blocking pool, which a paused clock does not move along.
async fn eventually_on_the_blocking_pool(what: &str, mut done: impl FnMut() -> bool) {
    let give_up = std::time::Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(
            std::time::Instant::now() < give_up,
            "gave up waiting for {what}"
        );
        tokio::task::yield_now().await;
    }
}

/// Lets the blocking pool finish what it was given, for a test that checks that nothing was
/// deleted: a cleanup started now would be done within this time.
async fn let_the_blocking_pool_run() {
    let until = std::time::Instant::now() + Duration::from_millis(100);
    while std::time::Instant::now() < until {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn old_logs_are_cleaned_and_a_cleaned_log_reads_as_empty() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    let log_path = job.log_path.clone().expect("the run wrote a log");
    let file = std::fs::File::options()
        .write(true)
        .open(&log_path)
        .unwrap();
    file.set_modified(std::time::SystemTime::now() - Duration::from_secs(40 * 86_400))
        .unwrap();
    let report = h.engine.clean_logs().await.unwrap();
    assert_eq!(report.removed, 1);
    assert!(!log_path.exists());
    assert_eq!(h.engine.log_tail(job.id, 10).unwrap(), Vec::<String>::new());
}

#[tokio::test]
async fn the_log_of_a_run_in_progress_is_not_cleaned() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    // The scripted run has written its log by the time it is listed as started.
    eventually("the run to start", || h.runner.started() == ["DEMO-1"]).await;
    let log = h.latest("1").log_path.expect("a running job has a log");
    age_file(&log, 40 * 24);
    let report = h.engine.clean_logs().await.unwrap();
    assert_eq!(report.removed, 0);
    assert!(log.exists(), "the log of a run in progress was deleted");
    // Once the run is over its log is like any other.
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.engine.clean_logs().await.unwrap().removed, 1);
    assert!(!log.exists());
}

#[tokio::test]
async fn the_cleanup_follows_the_retention_settings() {
    let mut cfg = Config::default();
    cfg.settings.log_retention_days = 2;
    cfg.settings.log_max_total_mb = 1;
    let h = Harness::with_config(cfg, false);
    let logs = h.root.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let write = |name: &str, kib: usize, hours: u64| {
        let path = logs.join(name);
        std::fs::write(&path, vec![b'x'; kib * 1024]).unwrap();
        age_file(&path, hours);
        path
    };
    // Only the age matters at first: the three logs are 801 KiB, under the 1 MiB limit.
    let expired = write("11.jsonl", 1, 72);
    let day_old = write("12.jsonl", 100, 24);
    let recent = write("13.jsonl", 700, 2);
    let report = h.engine.clean_logs().await.unwrap();
    assert_eq!(report.removed, 1);
    assert!(!expired.exists() && day_old.exists() && recent.exists());
    // Another 700 KiB bring them over the limit: the oldest go until the rest fits.
    let newest = write("14.jsonl", 700, 1);
    let report = h.engine.clean_logs().await.unwrap();
    assert_eq!(report.removed, 2);
    assert_eq!(report.freed_bytes, (100 + 700) * 1024);
    assert!(!day_old.exists() && !recent.exists());
    assert!(newest.exists());
}

// A run that started while the cleanup works could reuse the `<id>.jsonl` of an earlier attempt
// (a retried job) that the cleanup is about to remove. The cleanup takes the cycle lock, which
// every start of a run holds, so no run starts meanwhile: here a poll holds that lock while it
// waits for Linear, and the cleanup waits for the poll.
#[tokio::test]
async fn the_log_cleanup_waits_for_a_cycle_in_progress() {
    let root = tempfile::tempdir().unwrap();
    let fake = FakeLinear::new("user-1");
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *spy.viewer_gate.lock().unwrap() = Some(gate.clone());
    let engine = build_engine(
        root.path(),
        &Config::default(),
        spy.clone(),
        Arc::new(FakeReadiness),
        Arc::new(ScriptedRunner::new(fake, false)),
    );
    let logs = root.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let old = logs.join("31.jsonl");
    std::fs::write(&old, "x").unwrap();
    age_file(&old, 40 * 24);

    let poll = tokio::spawn({
        let engine = engine.clone();
        async move { engine.poll_once().await }
    });
    eventually("the poll to ask Linear for the user", || {
        spy.viewer_calls() == 1
    })
    .await;
    let cleanup = tokio::spawn({
        let engine = engine.clone();
        async move { engine.clean_logs().await }
    });
    let_the_blocking_pool_run().await;
    assert!(
        old.exists(),
        "the cleanup ran while a cycle was in progress"
    );
    assert!(!cleanup.is_finished());

    gate.add_permits(1);
    // Bounded, so that a deadlock fails the test instead of stalling the run.
    tokio::time::timeout(Duration::from_secs(5), poll)
        .await
        .expect("the poll never finished")
        .unwrap()
        .unwrap();
    let report = tokio::time::timeout(Duration::from_secs(5), cleanup)
        .await
        .expect("the cleanup never finished")
        .unwrap()
        .unwrap();
    assert_eq!(report.removed, 1);
    assert!(!old.exists());
}

#[tokio::test(start_paused = true)]
async fn the_loop_cleans_old_logs_when_it_starts() {
    let h = Harness::new(2, false);
    let logs = h.root.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let old = logs.join("21.jsonl");
    std::fs::write(&old, "x").unwrap();
    age_file(&old, 40 * 24);
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually_on_the_blocking_pool("the cleanup at start", || !old.exists()).await;
    // The file goes before the cleanup's task has returned: wait for the first check, which
    // comes after it, so that the loop is idle and `stop_loop`'s yields are enough.
    eventually_on_the_blocking_pool("the first check", || {
        h.engine.status().last_poll_at.is_some()
    })
    .await;
    stop_loop(&shutdown, worker).await;
}

#[tokio::test(start_paused = true)]
async fn the_loop_cleans_old_logs_every_six_hours() {
    let h = Harness::new(2, false);
    let logs = h.root.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    // The first check comes after the cleanup at start, so a log that is old only now is left for
    // the timer.
    eventually_on_the_blocking_pool("the first check", || {
        h.engine.status().last_poll_at.is_some()
    })
    .await;
    // The loop is waiting for its timer when the clock moves.
    quiesce().await;
    let old = logs.join("22.jsonl");
    std::fs::write(&old, "x").unwrap();
    age_file(&old, 40 * 24);
    let checked = h.engine.status().last_poll_at;
    tokio::time::advance(Duration::from_secs(6 * 3600 - 60)).await;
    eventually_on_the_blocking_pool("a check", || h.engine.status().last_poll_at != checked).await;
    let_the_blocking_pool_run().await;
    assert!(old.exists(), "cleaned before six hours were over");
    tokio::time::advance(Duration::from_secs(60)).await;
    eventually_on_the_blocking_pool("the cleanup after six hours", || !old.exists()).await;
    let_the_blocking_pool_run().await;
    // The next one is six hours after this one, not at the next check.
    let again = logs.join("23.jsonl");
    std::fs::write(&again, "x").unwrap();
    age_file(&again, 40 * 24);
    let checked = h.engine.status().last_poll_at;
    tokio::time::advance(Duration::from_secs(60)).await;
    eventually_on_the_blocking_pool("a check", || h.engine.status().last_poll_at != checked).await;
    let_the_blocking_pool_run().await;
    assert!(again.exists(), "cleaned again at the next check");
    stop_loop(&shutdown, worker).await;
}

// One run per directory, and a user's stop or retry is never overwritten.

/// Runs `first` and `second` on two threads that start together; `second` waits `delay` first.
async fn race<A, B>(
    first: impl FnOnce() -> A + Send + 'static,
    second: impl FnOnce() -> B + Send + 'static,
    delay: Duration,
) -> (A, B)
where
    A: Send + 'static,
    B: Send + 'static,
{
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let (b1, b2) = (barrier.clone(), barrier);
    let one = tokio::task::spawn_blocking(move || {
        b1.wait();
        first()
    });
    let two = tokio::task::spawn_blocking(move || {
        b2.wait();
        let start = std::time::Instant::now();
        while start.elapsed() < delay {
            std::hint::spin_loop();
        }
        second()
    });
    (one.await.unwrap(), two.await.unwrap())
}

/// The head start the first racer gets in `round`: from nothing to a few hundred microseconds,
/// so the two calls meet at every point of each other's critical sections.
fn head_start(round: u64) -> Duration {
    Duration::from_micros((round % 60) * 5)
}

#[tokio::test]
async fn an_issue_moved_to_a_busy_project_waits_for_that_projects_folder() {
    let h = Harness::new(5, true);
    h.bind("proj-1", "a");
    let b = h.bind("proj-2", "b");
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("2").state, JobState::Running);
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").binding_id.as_deref(), Some("proj-1"));
    // Between that poll and the dispatch the issue moves to the project whose folder is in use:
    // the scheduler picks the job for folder `a`, but it would run in `b`.
    h.linear
        .update("1", |i| i.project_id = Some("proj-2".into()));
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.engine.status().running, 1, "one run per folder");
    let moved = h.latest("1");
    assert_eq!(moved.state, JobState::Queued);
    assert_eq!(moved.binding_id.as_deref(), Some("proj-2"));
    assert_eq!(moved.directory.as_deref(), Some(b.as_path()));
    // Once the folder is free, the next cycle runs it there.
    h.runner.gate.add_permits(1);
    h.settle().await;
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Running);
    h.runner.gate.add_permits(1);
    h.settle().await;
    let cwds: Vec<PathBuf> = h
        .runner
        .specs
        .lock()
        .unwrap()
        .iter()
        .map(|spec| spec.cwd.clone())
        .collect();
    assert_eq!(cwds, [b.clone(), b]);
}

#[tokio::test]
async fn the_scheduler_picks_only_jobs_that_can_start() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        std::fs::create_dir_all(root.path().join(name)).unwrap();
    }
    let mut cfg = Config::default();
    cfg.settings.max_concurrent = 2;
    cfg.bindings
        .push(demo_binding("proj-1", &root.path().join("a")));
    // The same folder as proj-1, named another way.
    cfg.bindings
        .push(demo_binding("proj-4", &root.path().join("a/../a")));
    cfg.bindings
        .push(demo_binding("proj-2", &root.path().join("b")));
    cfg.bindings
        .push(demo_binding("proj-3", &root.path().join("c")));
    let fake = FakeLinear::new("user-1");
    for (uuid, project) in [
        ("1", "proj-1"),
        ("4", "proj-4"),
        ("2", "proj-2"),
        ("3", "proj-3"),
    ] {
        fake.upsert(issue(uuid, project));
    }
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake, true));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.poll_once().await.unwrap();
    // Only DEMO-1 and DEMO-2 can start: DEMO-4 shares DEMO-1's folder and DEMO-3 finds no slot
    // left. The other two are not even re-read in Linear.
    assert_eq!(engine.dispatch().await.unwrap().len(), 2);
    assert_eq!(spy.issue_calls(), 2);
    assert_eq!(engine.status().running, 2);
    // While both runs go on, a new round picks nothing.
    assert!(engine.dispatch().await.unwrap().is_empty());
    assert_eq!(spy.issue_calls(), 2);
    runner.gate.add_permits(2);
    engine.wait_idle().await;
}

#[tokio::test]
async fn a_free_slot_goes_to_a_job_whose_folder_is_free() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("0", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    // DEMO-1 queues behind the run in folder `a`; DEMO-2 can use the one free slot.
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 2);
    assert_eq!(h.latest("2").state, JobState::Running);
    assert_eq!(h.latest("1").state, JobState::Queued);
    h.runner.gate.add_permits(3);
    h.settle().await;
}

#[tokio::test]
async fn a_binding_that_moved_its_folder_is_picked_up_before_the_start() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    // The project's folder changes after the poll and before the dispatch; the job row must
    // name the folder the run uses, so the start waits for the next cycle.
    let new_folder = h.bind("proj-1", "c");
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(job.directory.as_deref(), Some(new_folder.as_path()));
    assert_eq!(h.engine.dispatch().await.unwrap().len(), 1);
    h.settle().await;
    assert_eq!(h.runner.specs.lock().unwrap()[0].cwd, new_folder);
}

#[tokio::test]
async fn a_binding_that_changed_its_executor_re_assigns_the_queued_job() {
    let h = Harness::new(2, false);
    let folder = h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").executor, Some(ExecutorKind::Claude));
    // The binding switches to Codex while the job is queued: the row must say what will run.
    h.engine
        .add_binding(Binding {
            executor: Some(ExecutorKind::Codex),
            ..demo_binding("proj-1", &folder)
        })
        .unwrap();
    h.engine.poll_once().await.unwrap();
    let job = h.latest("1");
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(job.executor, Some(ExecutorKind::Codex));
}

#[tokio::test]
async fn a_binding_that_changed_its_executor_is_picked_up_before_the_start() {
    let h = Harness::new(2, false);
    let folder = h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    h.engine
        .add_binding(Binding {
            executor: Some(ExecutorKind::Codex),
            ..demo_binding("proj-1", &folder)
        })
        .unwrap();
    // No poll in between: the start itself notices that the row names another executor.
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").executor, Some(ExecutorKind::Codex));
    assert_eq!(h.engine.dispatch().await.unwrap().len(), 1);
    h.settle().await;
    assert_eq!(*h.runner.kinds.lock().unwrap(), [ExecutorKind::Codex]);
}

#[tokio::test]
async fn a_stop_during_the_start_reread_is_not_undone_by_a_missing_folder() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let folder = root.path().join("repo");
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
    // While the dispatch re-reads the issue, the user stops the job and the folder goes missing:
    // the start would put the job back to waiting.
    let (stopper, gone) = (engine.clone(), folder.clone());
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        stopper.cancel_job(job.id).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
    }));
    assert!(engine.dispatch().await.unwrap().is_empty());
    let after = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(after.state, JobState::Canceled, "the stop was undone");
    assert_eq!(after.reason.as_deref(), Some("stopped by the user"));
    // The folder is back; the stopped job stays stopped.
    std::fs::create_dir_all(&folder).unwrap();
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(
        engine.list_jobs(10).unwrap().remove(0).state,
        JobState::Canceled
    );
    assert!(runner.started().is_empty(), "a stopped job ran");
}

/// Takes the issue out of every later poll, so rounds that share one harness stay independent.
fn retire(h: &Harness, uuid: &str) {
    h.linear.update(uuid, |i| i.labels.clear());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_poll_racing_a_retry_never_leaves_two_active_rows() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    for round in 0..150 {
        let uuid = format!("e{round}");
        h.linear.upsert(issue(&uuid, "proj-1"));
        h.runner
            .script(&format!("DEMO-{uuid}"), Script::exit_code(1));
        h.engine.tick().await.unwrap();
        h.settle().await;
        let failed = h.latest(&uuid);
        assert_eq!(failed.state, JobState::Failed);
        // Someone edits the issue, so the poll wants to record a new run, just as the user
        // retries the failed one.
        h.linear.update(&uuid, |i| i.title.push_str(" (edited)"));
        let (polling, retrying) = (h.engine.clone(), h.engine.clone());
        let handle = tokio::runtime::Handle::current();
        let (polled, _retried) = race(
            move || handle.block_on(polling.poll_once()),
            move || retrying.retry_job(failed.id).is_ok(),
            head_start(round),
        )
        .await;
        polled.unwrap();
        let active: Vec<(i64, JobState)> = h
            .engine
            .list_jobs(1000)
            .unwrap()
            .into_iter()
            .filter(|job| job.issue_uuid == uuid && job.state.is_active())
            .map(|job| (job.id, job.state))
            .collect();
        assert!(active.len() <= 1, "round {round}: active rows {active:?}");
        retire(&h, &uuid);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_poll_racing_a_stop_never_undoes_the_stop() {
    let h = Harness::new(2, false);
    for round in 0..600 {
        let uuid = format!("f{round}");
        // No binding: the job waits, and every poll rewrites its row.
        h.linear.upsert(issue(&uuid, "proj-unbound"));
        h.engine.tick().await.unwrap();
        let waiting = h.latest(&uuid);
        assert_eq!(waiting.state, JobState::WaitingBinding);
        let (polling, stopping) = (h.engine.clone(), h.engine.clone());
        let handle = tokio::runtime::Handle::current();
        let (polled, stopped) = race(
            move || handle.block_on(polling.poll_once()),
            move || stopping.cancel_job(waiting.id),
            head_start(round),
        )
        .await;
        polled.unwrap();
        stopped.unwrap_or_else(|e| panic!("round {round}: the stop was refused: {e:#}"));
        assert_eq!(
            h.latest(&uuid).state,
            JobState::Canceled,
            "round {round}: a stop that succeeded was undone"
        );
        retire(&h, &uuid);
    }
}

// The window this guards against (`cancel_job` looks at the running set, the run registers and
// starts, `cancel_job` then reads the row) is a few hundred nanoseconds wide, so a race like this
// hardly ever lands in it: the test is a smoke test of the invariant, not a detector.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stop_that_races_the_start_is_never_refused() {
    let h = Harness::new(2, true); // gated: a run ends only when it is stopped
    h.bind("proj-1", "a");
    for round in 0..150 {
        let uuid = format!("s{round}");
        h.linear.upsert(issue(&uuid, "proj-1"));
        h.engine.poll_once().await.unwrap();
        let queued = h.latest(&uuid);
        assert_eq!(queued.state, JobState::Queued);
        let (starting, stopping) = (h.engine.clone(), h.engine.clone());
        let handle = tokio::runtime::Handle::current();
        let (dispatched, stopped) = race(
            move || handle.block_on(starting.dispatch()),
            move || stopping.cancel_job(queued.id),
            head_start(round),
        )
        .await;
        dispatched.unwrap();
        stopped.unwrap_or_else(|e| panic!("round {round}: the stop was refused: {e:#}"));
        tokio::time::timeout(Duration::from_secs(10), h.settle())
            .await
            .unwrap_or_else(|_| panic!("round {round}: the stopped run never ended"));
        assert_eq!(h.latest(&uuid).state, JobState::Canceled, "round {round}");
        retire(&h, &uuid);
    }
}

#[tokio::test]
async fn an_issue_moved_to_another_project_with_the_same_folder_is_reassigned_first() {
    let h = Harness::new(2, false);
    let folder = h.bind("proj-1", "a");
    h.engine
        .add_binding(Binding {
            id: "bind-2".into(),
            ..demo_binding("proj-2", &folder)
        })
        .unwrap();
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").binding_id.as_deref(), Some("proj-1"));
    // Same folder, other binding: the run would be fine, but the row must name its binding.
    h.linear
        .update("1", |i| i.project_id = Some("proj-2".into()));
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").binding_id.as_deref(), Some("bind-2"));
    assert_eq!(h.engine.dispatch().await.unwrap().len(), 1);
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn a_stop_during_the_start_reread_keeps_its_reason_when_the_issue_changed_too() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.poll_once().await.unwrap();
    let job = engine.list_jobs(10).unwrap().remove(0);
    // The user stops the job and someone takes the issue away, both while it is re-read.
    let (stopper, linear) = (engine.clone(), fake.clone());
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        stopper.cancel_job(job.id).unwrap();
        linear.update("1", |i| i.assignee_id = Some("someone-else".into()));
    }));
    assert!(engine.dispatch().await.unwrap().is_empty());
    let after = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(after.state, JobState::Canceled);
    assert_eq!(after.reason.as_deref(), Some("stopped by the user"));
}

#[tokio::test]
async fn a_stop_during_the_start_reread_is_not_undone_by_a_project_change() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &alpha));
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.poll_once().await.unwrap();
    let job = engine.list_jobs(10).unwrap().remove(0);
    // While the issue is re-read the user stops the job and the issue moves to the other project:
    // moving the row to the new binding must not bring the stopped job back.
    let (stopper, linear) = (engine.clone(), fake.clone());
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        stopper.cancel_job(job.id).unwrap();
        linear.update("1", |i| i.project_id = Some("proj-2".into()));
    }));
    assert!(engine.dispatch().await.unwrap().is_empty());
    let after = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(after.state, JobState::Canceled);
    assert_eq!(
        after.binding_id.as_deref(),
        Some("proj-1"),
        "the stopped row was rewritten"
    );
    // The issue changed in Linear, so the next check may record a new run of it; the stopped
    // row itself stays stopped and never runs.
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    let rows = engine.list_jobs(10).unwrap();
    let stopped = rows.iter().find(|row| row.id == job.id).unwrap();
    assert_eq!(stopped.state, JobState::Canceled);
    assert_eq!(stopped.started_at, None, "the stopped job ran");
}

// Pause, shutdown and failures while a job starts.

#[tokio::test]
async fn pausing_during_a_start_stops_that_start_and_the_rest_of_the_round() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &alpha));
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-2"));
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
    // The user pauses while the first job's issue is re-read: neither that job nor the second
    // one of the same round may start.
    let pauser = engine.clone();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || pauser.pause().unwrap()));
    assert!(engine.dispatch().await.unwrap().is_empty());
    engine.wait_idle().await;
    assert!(
        runner.started().is_empty(),
        "a run started after pause() returned"
    );
    for row in engine.list_jobs(10).unwrap() {
        assert_eq!(row.state, JobState::Queued);
    }
    // Resumed, both start.
    engine.resume().unwrap();
    assert_eq!(engine.dispatch().await.unwrap().len(), 2);
    engine.wait_idle().await;
    let mut started = runner.started();
    started.sort();
    assert_eq!(started, ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn draining_during_a_start_stops_that_start_and_the_rest_of_the_round() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &alpha));
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-2"));
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
    // The host starts waiting for its runs while the first job's issue is re-read. Unlike a
    // pause, that leaves the configuration as it was: only the drain itself can stop the start.
    let drainer = engine.clone();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || drainer.set_draining(true)));
    assert!(engine.dispatch().await.unwrap().is_empty());
    engine.wait_idle().await;
    assert!(
        runner.started().is_empty(),
        "a run started after set_draining(true) returned"
    );
    assert_eq!(
        spy.issue_calls(),
        1,
        "the round ends without re-reading the next job's issue"
    );
    for row in engine.list_jobs(10).unwrap() {
        assert_eq!(row.state, JobState::Queued);
    }
    engine.set_draining(false);
    assert_eq!(engine.dispatch().await.unwrap().len(), 2);
    engine.wait_idle().await;
    let mut started = runner.started();
    started.sort();
    assert_eq!(started, ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn a_binding_removed_during_the_start_reread_stops_the_start() {
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
    let remover = engine.clone();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        assert!(remover.remove_binding("proj-1").unwrap());
    }));
    assert!(engine.dispatch().await.unwrap().is_empty());
    engine.wait_idle().await;
    assert!(
        runner.started().is_empty(),
        "a run started without a binding"
    );
    assert_eq!(engine.list_jobs(10).unwrap()[0].state, JobState::Queued);
}

#[tokio::test]
async fn a_binding_changed_during_the_start_reread_is_applied_to_the_run() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let folder = root.path().join("repo");
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
    // The user switches the binding to the stricter permission profile while the start re-reads
    // the issue: the run must not start with the profile the user just replaced.
    let changer = engine.clone();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        changer
            .add_binding(Binding {
                permission: Some(PermissionProfile::Personal),
                ..demo_binding("proj-1", &folder)
            })
            .unwrap();
    }));
    assert!(engine.dispatch().await.unwrap().is_empty());
    assert_eq!(engine.dispatch().await.unwrap().len(), 1);
    engine.wait_idle().await;
    let args = runner.specs.lock().unwrap()[0].args.clone();
    assert!(args.contains(&"dontAsk".to_string()), "{args:?}");
    assert!(!args.contains(&"bypassPermissions".to_string()), "{args:?}");
}

#[tokio::test]
async fn a_configuration_change_during_one_start_only_delays_that_start() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &alpha));
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-2"));
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
    // An unrelated binding is added while the first job's issue is re-read. That start is
    // delayed (it was built from the older configuration); the second one is not affected.
    let changer = engine.clone();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        changer
            .add_binding(demo_binding("proj-3", &elsewhere))
            .unwrap();
    }));
    let started = engine.dispatch().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(started, [job_of(&engine, "2").id]);
    assert_eq!(job_of(&engine, "1").state, JobState::Queued);
    assert_eq!(engine.dispatch().await.unwrap().len(), 1);
    engine.wait_idle().await;
    assert_eq!(runner.started(), ["DEMO-2", "DEMO-1"]);
}

#[tokio::test(start_paused = true)]
async fn a_start_deferred_by_a_configuration_change_happens_right_away() {
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
    // A binding is added while the first cycle re-reads the issue for the start.
    let changer = engine.clone();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || {
        changer
            .add_binding(demo_binding("proj-3", &elsewhere))
            .unwrap();
    }));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&engine, &shutdown);
    // The clock never moves here: the deferred start itself must bring the next cycle.
    eventually("the deferred job to start", || {
        runner.started() == ["DEMO-1"]
    })
    .await;
    stop_loop(&shutdown, worker).await;
    engine.wait_idle().await;
}

#[tokio::test(start_paused = true)]
async fn a_shutdown_requested_during_the_start_reread_starts_nothing() {
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
    let shutdown = CancellationToken::new();
    let token = shutdown.clone();
    *spy.before_issue.lock().unwrap() = Some(Box::new(move || token.cancel()));
    // The first cycle polls, queues the job and starts re-reading its issue; the shutdown
    // arrives during that re-read.
    let worker = spawn_loop(&engine, &shutdown);
    eventually("the loop to stop", || worker.is_finished()).await;
    worker.await.unwrap();
    engine.wait_idle().await;
    assert!(
        runner.started().is_empty(),
        "a run started after the shutdown"
    );
    assert_eq!(engine.list_jobs(10).unwrap()[0].state, JobState::Queued);
    // Nothing starts later either, however the engine is driven.
    assert!(engine.dispatch().await.unwrap().is_empty());
    engine.wait_idle().await;
    assert!(runner.started().is_empty());
}

#[tokio::test]
async fn one_job_that_cannot_be_started_does_not_block_the_others() {
    let root = tempfile::tempdir().unwrap();
    let (alpha, beta) = (root.path().join("alpha"), root.path().join("beta"));
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut cfg = Config::default();
    cfg.bindings.push(demo_binding("proj-1", &alpha));
    cfg.bindings.push(demo_binding("proj-2", &beta));
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-2"));
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
    *spy.fail_issue.lock().unwrap() = Some(("1".into(), "Linear is down\nfor DEMO-1".into()));
    let started = engine.dispatch().await.unwrap();
    engine.wait_idle().await;
    // The first job could not be re-read; the second still started.
    let second = job_of(&engine, "2");
    assert_eq!(started, [second.id]);
    assert_eq!(runner.started(), ["DEMO-2"]);
    assert_eq!(job_of(&engine, "1").state, JobState::Queued);
    // The status says what went wrong, until the next successful check replaces it.
    let error = engine.status().last_error.expect("the failure is reported");
    assert_eq!(error, "could not start DEMO-1: Linear is down for DEMO-1");
    engine.poll_once().await.unwrap();
    assert_eq!(engine.status().last_error, None);
}

#[tokio::test]
async fn a_panic_while_a_run_is_recorded_still_frees_its_folder() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-1")); // same project, so the same folder
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    // `issue()` calls: 1 = DEMO-1's start re-read, 2 = the check of its result (panics).
    *spy.panic_on_issue_call.lock().unwrap() = Some(2);
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.tick().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs after a panic while recording");
    assert_eq!(engine.status().running, 0);
    // The row does not stay `running` for a run that is over, and it is finished like any other.
    let first = job_of(&engine, "1");
    assert_eq!(first.state, JobState::Failed);
    assert!(
        first.finished_at.is_some(),
        "an abandoned run has no end time"
    );
    assert!(
        first
            .reason
            .unwrap()
            .contains("before its result was recorded")
    );
    // The folder is free again: the issue that was waiting for it runs.
    engine.tick().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs");
    assert_eq!(runner.started(), ["DEMO-1", "DEMO-2"]);
    assert_eq!(job_of(&engine, "2").state, JobState::AwaitingAcceptance);
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_run_wakes_the_loop_for_the_next_job_in_its_folder() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-1")); // the same folder
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    // `issue()` calls: 1 = DEMO-1's start re-read, 2 = the check of its result (panics).
    *spy.panic_on_issue_call.lock().unwrap() = Some(2);
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&engine, &shutdown);
    // The clock never moves here: only the abandoned run itself can bring the next cycle, which
    // starts the job that was waiting for the folder.
    eventually("the waiting job to start", || {
        runner.started() == ["DEMO-1", "DEMO-2"]
    })
    .await;
    stop_loop(&shutdown, worker).await;
    engine.wait_idle().await;
    assert_eq!(job_of(&engine, "1").state, JobState::Failed);
    assert_eq!(job_of(&engine, "2").state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn an_abandoned_retry_keeps_the_verified_value_of_the_earlier_run() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    // `issue()` calls: 1 and 2 = the first run's start and result check, 3 = the retry's start,
    // 4 = the retry's result check (panics).
    *spy.panic_on_issue_call.lock().unwrap() = Some(4);
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    runner.script("DEMO-1", Script::exit_code(1));
    let engine = build_engine(
        root.path(),
        &cfg,
        spy.clone(),
        Arc::new(FakeReadiness),
        runner.clone(),
    );
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    let first = job_of(&engine, "1");
    assert_eq!(first.state, JobState::Failed);
    assert!(first.verified_updated_at.is_some());
    engine.retry_job(first.id).unwrap();
    engine.dispatch().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs after a panic while recording");
    let retried = job_of(&engine, "1");
    assert_eq!(retried.id, first.id, "a retry reuses the row");
    assert_eq!(retried.state, JobState::Failed);
    assert!(retried.finished_at.is_some());
    assert_eq!(retried.verified_updated_at, first.verified_updated_at);
}

#[tokio::test]
async fn a_store_error_while_abandoning_a_run_does_not_stop_it_from_freeing_the_folder() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    fake.upsert(issue("2", "proj-1")); // the same folder
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    // `issue()` calls: 1 = DEMO-1's start re-read, 2 = the check of its result (panics).
    *spy.panic_on_issue_call.lock().unwrap() = Some(2);
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: spy.clone(),
        readiness: Arc::new(FakeReadiness),
        job_runner: runner.clone(),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    // The database refuses to store a failed run, so the abandoned run cannot be recorded.
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_failed BEFORE UPDATE ON jobs \
             WHEN NEW.state = 'failed' \
             BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .unwrap();
    engine.tick().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs when the abandoned run cannot be recorded");
    assert_eq!(engine.status().running, 0);
    // The error is logged, not raised: the folder is free and the next job in it runs.
    engine.tick().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs");
    assert_eq!(runner.started(), ["DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn an_abandoned_run_leaves_a_row_that_is_no_longer_running_alone() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    // `issue()` calls: 1 = the start re-read, 2 = the check of the result (panics).
    *spy.panic_on_issue_call.lock().unwrap() = Some(2);
    let runner = Arc::new(ScriptedRunner::new(fake, true));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: spy.clone(),
        readiness: Arc::new(FakeReadiness),
        job_runner: runner.clone(),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    engine.tick().await.unwrap();
    assert_eq!(engine.list_jobs(10).unwrap()[0].state, JobState::Running);
    // While the run goes on, the row is ended by someone else.
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE jobs SET state = 'canceled', reason = 'stopped by the user'",
            [],
        )
        .unwrap();
    runner.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs after a panic while recording");
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(
        job.state,
        JobState::Canceled,
        "the abandoned run overwrote the row"
    );
    assert_eq!(job.reason.as_deref(), Some("stopped by the user"));
    assert!(job.finished_at.is_none());
    assert_eq!(engine.status().running, 0);
}

#[tokio::test]
async fn a_result_that_cannot_be_stored_fails_the_job_and_frees_the_folder() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let runner = Arc::new(ScriptedRunner::new(fake.clone(), true));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: Arc::new(fake),
        readiness: Arc::new(FakeReadiness),
        job_runner: runner.clone(),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    engine.tick().await.unwrap();
    // From now on the database refuses to store a run as awaiting acceptance.
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER refuse_result BEFORE UPDATE ON jobs \
             WHEN NEW.state = 'awaiting_acceptance' \
             BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
        )
        .unwrap();
    runner.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs when the result cannot be stored");
    let job = engine.list_jobs(10).unwrap().remove(0);
    assert_eq!(job.state, JobState::Failed);
    let reason = job.reason.unwrap();
    assert!(
        reason.contains("could not record the run result"),
        "{reason}"
    );
    assert!(reason.contains("disk full"), "{reason}");
    assert_eq!(engine.status().running, 0);
}

// Text that reaches the panel, the CLI and MCP.

#[tokio::test(start_paused = true)]
async fn text_from_linear_in_a_recorded_reason_is_cleaned() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    // The error text comes from outside: a line break and a right-to-left override. Every read
    // that verifies the result fails with it.
    h.linear.fail_calls("Linear said\nno \u{202e}gnp", 3);
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(
        h.latest("1").reason.as_deref(),
        Some("could not verify the result in Linear: Linear said no gnp")
    );
}

#[tokio::test]
async fn issue_titles_are_stored_clean() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    let mut hostile = issue("1", "proj-1");
    hostile.title = "Fix\u{1b}[2J\n\u{202e}login".into();
    h.linear.upsert(hostile);
    // A job that waits for a binding is recorded clean too.
    let mut padded = issue("2", "proj-unbound");
    padded.title = "  Pay\tnow\u{200b}  ".into();
    h.linear.upsert(padded);
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
    assert_eq!(h.latest("1").title, "Fix [2J login");
    assert_eq!(h.latest("2").state, JobState::WaitingBinding);
    assert_eq!(h.latest("2").title, "Pay now");
}

#[tokio::test]
async fn a_poll_error_is_shown_as_one_clean_line() {
    let h = Harness::new(2, false);
    h.linear.fail_next("Linear said\nno \u{202e}gnp");
    assert!(h.engine.tick().await.is_err());
    assert_eq!(
        h.engine.status().last_error.as_deref(),
        Some("Linear said no gnp")
    );
}

#[tokio::test]
async fn the_last_output_is_kept_when_there_is_no_report() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.runner.script(
        "DEMO-1",
        Script {
            final_text: Some("I could not find the file".into()),
            ..Script::exit_code(3)
        },
    );
    h.runner.script(
        "DEMO-2",
        Script {
            final_text: Some("two\nlines\u{200b}!".into()),
            ..Script::exit_code(3)
        },
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    let first = h.latest("1");
    assert_eq!(first.state, JobState::Failed);
    assert_eq!(first.summary.as_deref(), Some("I could not find the file"));
    // It is cleaned like any other summary.
    assert_eq!(h.latest("2").summary.as_deref(), Some("two lines!"));
}

#[tokio::test]
async fn an_empty_summary_is_no_summary() {
    let h = Harness::new(2, false);
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    // A report line without text, and output that is nothing but invisible characters.
    h.runner.script(
        "DEMO-1",
        Script {
            final_text: Some(r#"{"result":"in_testing","summary":""}"#.into()),
            ..Script::success()
        },
    );
    h.runner.script(
        "DEMO-2",
        Script {
            final_text: Some(" \u{200b}\n".into()),
            ..Script::exit_code(1)
        },
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").summary, None);
    assert_eq!(h.latest("2").summary, None);
}

// The panel's interface: events, the issue list, settings, the Linear connection and draining.

#[tokio::test]
async fn subscribers_hear_about_every_job_change_and_each_check() {
    let h = Harness::new(1, false);
    let mut events = h.engine.subscribe();
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    h.engine.mark_done(h.latest("1").id).await.unwrap();
    let (mut states, mut saw_status, mut saw_config) = (Vec::new(), false, false);
    while let Ok(event) = events.try_recv() {
        match event {
            EngineEvent::Job { job } => states.push(job.state),
            EngineEvent::Status => saw_status = true,
            EngineEvent::Config => saw_config = true,
        }
    }
    assert!(saw_config, "binding the project saved the configuration");
    assert!(saw_status, "the check reported its result");
    assert_eq!(
        states,
        [
            JobState::Queued,
            JobState::Running,
            JobState::AwaitingAcceptance,
            JobState::Done
        ]
    );
}

#[tokio::test]
async fn retry_and_stop_are_announced_too() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.runner.script("DEMO-1", Script::exit_code(3));
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let mut events = h.engine.subscribe();
    let id = h.latest("1").id;
    h.engine.retry_job(id).unwrap();
    h.engine.cancel_job(id).unwrap();
    let states: Vec<JobState> = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|e| match e {
            EngineEvent::Job { job } => Some(job.state),
            _ => None,
        })
        .collect();
    assert_eq!(states, [JobState::Queued, JobState::Canceled]);
}

#[tokio::test]
async fn the_issue_list_explains_what_each_labeled_issue_is_waiting_for() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(Issue {
        state: "In Testing".into(),
        ..issue("2", "proj-1")
    });
    h.linear.upsert(issue("3", "proj-2"));
    let views = h.engine.issues().await.unwrap();
    let status_of = |views: &[IssueView], uuid: &str| {
        views
            .iter()
            .find(|v| v.issue.uuid == uuid)
            .unwrap()
            .status
            .clone()
    };
    assert_eq!(
        status_of(&views, "1"),
        IssueStatus::Eligible { picked_up: true }
    );
    assert_eq!(
        status_of(&views, "2"),
        IssueStatus::NotTodo {
            state: "In Testing".into()
        }
    );
    assert_eq!(
        status_of(&views, "3"),
        IssueStatus::Waiting {
            reason: WaitReasonView::ProjectNotBound
        }
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    let views = h.engine.issues().await.unwrap();
    let first = views.iter().find(|v| v.issue.uuid == "1").unwrap();
    assert_eq!(
        first.job.as_ref().unwrap().state,
        JobState::AwaitingAcceptance
    );
    h.engine.pause().unwrap();
    let views = h.engine.issues().await.unwrap();
    assert_eq!(status_of(&views, "3"), IssueStatus::Paused);
}

/// Whether the issue list says the next check picks the issue `uuid` up; it must be eligible.
fn picked_up(views: &[IssueView], uuid: &str) -> bool {
    match &views.iter().find(|v| v.issue.uuid == uuid).unwrap().status {
        IssueStatus::Eligible { picked_up } => *picked_up,
        other => panic!("DEMO-{uuid} should be eligible, not {other:?}"),
    }
}

#[tokio::test]
async fn the_issue_list_says_whether_the_next_check_picks_an_eligible_issue_up() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    // A run that ended without the result the issue needed; the issue stays in Todo.
    h.runner.script("DEMO-2", Script::exit_code(3));
    h.linear.upsert(issue("2", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("2").state, JobState::Failed);
    // An issue that has had no run at all.
    h.linear.upsert(issue("1", "proj-1"));

    let views = h.engine.issues().await.unwrap();
    assert!(
        picked_up(&views, "1"),
        "an issue without a run is picked up"
    );
    assert!(
        !picked_up(&views, "2"),
        "a finished run is not started again while Linear has not changed the issue"
    );
    // The poll does what the list says: it starts the one and leaves the other.
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-2", "DEMO-1"]);

    // Linear moves the issue's updatedAt past what the run recorded.
    h.linear.update("2", |_| {});
    let views = h.engine.issues().await.unwrap();
    assert!(
        picked_up(&views, "2"),
        "a change in Linear brings a finished issue back"
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-2", "DEMO-1", "DEMO-2"]);
}

#[tokio::test]
async fn an_issue_with_a_run_in_hand_is_picked_up_and_a_stopped_one_is_not_until_linear_changes() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-1"));
    // One run per folder: DEMO-1 runs (held at the gate), DEMO-2 is queued behind it.
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Running);
    assert_eq!(h.latest("2").state, JobState::Queued);
    let views = h.engine.issues().await.unwrap();
    assert!(picked_up(&views, "1"), "a run under way is in hand");
    assert!(picked_up(&views, "2"), "a queued run is in hand");

    // The user stops the queued run: the engine does not start it again by itself.
    h.engine.cancel_job(h.latest("2").id).unwrap();
    let views = h.engine.issues().await.unwrap();
    assert!(!picked_up(&views, "2"));
    h.linear.update("2", |_| {});
    let views = h.engine.issues().await.unwrap();
    assert!(picked_up(&views, "2"));

    h.runner.gate.add_permits(2);
    h.settle().await;
}

#[tokio::test]
async fn saving_settings_validates_them_and_keeps_the_pause_switch() {
    let h = Harness::new(1, false);
    h.engine.pause().unwrap();
    let mut settings = h.engine.settings();
    settings.poll_interval_secs = 120;
    settings.paused = false;
    h.engine.update_settings(settings).unwrap();
    let saved = h.engine.settings();
    assert_eq!(saved.poll_interval_secs, 120);
    assert!(
        saved.paused,
        "the settings page never switches monitoring on or off"
    );
    let mut bad = h.engine.settings();
    bad.max_concurrent = 0;
    assert!(h.engine.update_settings(bad).is_err());
    assert_eq!(
        h.engine.settings().max_concurrent,
        1,
        "a refused change changes nothing"
    );
}

#[tokio::test]
async fn the_panels_own_settings_are_saved_like_the_others() {
    let h = Harness::new(1, false);
    let before = h.engine.settings();
    assert!(before.auto_update && before.notifications.failed);
    assert!(!before.onboarding_completed && !before.full_access_acknowledged);
    assert!(
        before.show_in_dock,
        "the Dock icon stays unless it is switched off"
    );
    let mut settings = before.clone();
    settings.notifications.run_started = false;
    settings.notifications.failed = false;
    settings.auto_update = false;
    settings.onboarding_completed = true;
    settings.full_access_acknowledged = true;
    settings.show_in_dock = false;
    h.engine.update_settings(settings.clone()).unwrap();
    assert_eq!(h.engine.settings(), settings);
    // They are in the file, so a restarted host sees them, and a pause leaves them alone.
    h.engine.pause().unwrap();
    let on_disk = Config::load(&h.root.path().join("config.toml"))
        .unwrap()
        .settings;
    assert_eq!(
        on_disk,
        Settings {
            paused: true,
            ..settings
        }
    );
}

#[tokio::test]
async fn reconnecting_linear_moves_every_later_call_to_the_new_connection() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "repo-a");
    // The run leaves the old connection's copy of the issue alone.
    h.runner.script(
        "DEMO-1",
        Script {
            set_state: None,
            ..Script::success()
        },
    );
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    let other = FakeLinear::new("user-1");
    other.upsert(Issue {
        state: "In Testing".into(),
        ..issue("1", "proj-1")
    });
    h.engine.set_linear(Arc::new(other.clone()));
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(
        h.latest("1").state,
        JobState::AwaitingAcceptance,
        "the result was read through the new connection"
    );
    assert_eq!(
        h.runner.started().len(),
        1,
        "switching connections starts nothing twice"
    );
}

#[tokio::test]
async fn without_a_linear_connection_checks_fail_visibly_until_one_is_set() {
    let h = Harness::new(1, false);
    h.engine
        .set_linear(Arc::new(Disconnected::new("not logged in to Linear")));
    assert!(h.engine.poll_once().await.is_err());
    let status = h.engine.status();
    assert!(!status.linear_ok);
    assert!(
        status
            .last_error
            .unwrap()
            .contains("not logged in to Linear")
    );
    h.engine.set_linear(Arc::new(h.linear.clone()));
    h.engine.poll_once().await.unwrap();
    let status = h.engine.status();
    assert!(status.linear_ok);
    assert_eq!(status.viewer.unwrap().id, "user-1");
}

#[tokio::test]
async fn draining_lets_runs_finish_but_starts_nothing_new() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "repo-a");
    h.bind("proj-2", "repo-b");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    assert_eq!(h.engine.status().running, 1);
    // A run reports itself started from a task of its own, which this single-threaded runtime
    // only runs while the test waits.
    eventually("DEMO-1 to start", || h.runner.started() == ["DEMO-1"]).await;
    h.engine.set_draining(true);
    assert!(h.engine.status().draining);
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    // A run started by mistake would report itself now.
    quiesce().await;
    assert_eq!(
        h.runner.started(),
        ["DEMO-1"],
        "nothing new starts while draining"
    );
    assert_eq!(
        h.latest("2").state,
        JobState::Queued,
        "new issues are still recorded"
    );
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    h.engine.set_draining(false);
    h.engine.tick().await.unwrap();
    eventually("DEMO-2 to start", || h.runner.started().len() == 2).await;
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-2"]);
    h.runner.gate.add_permits(1);
    h.settle().await;
}

#[tokio::test]
async fn a_job_that_cannot_start_does_not_hold_up_the_queue() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.bind("proj-2", "repo-b");
    let earlier = issue("1", "proj-1").created_at - chrono::Duration::hours(1);
    h.linear.upsert(Issue {
        created_at: earlier,
        ..issue("1", "proj-1")
    });
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.poll_once().await.unwrap();
    h.linear.fail_issue_reads("1");
    h.engine.dispatch().await.unwrap();
    assert!(h.runner.started().is_empty());
    assert!(h.engine.status().last_error.unwrap().contains("DEMO-1"));
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    assert_eq!(
        h.runner.started(),
        ["DEMO-2"],
        "the failing job waits while the next one runs"
    );
    assert_eq!(
        h.latest("1").state,
        JobState::Queued,
        "and stays queued for a later try"
    );
}

/// Every event received so far, in order.
fn received(events: &mut tokio::sync::broadcast::Receiver<EngineEvent>) -> Vec<EngineEvent> {
    std::iter::from_fn(|| events.try_recv().ok()).collect()
}

#[tokio::test]
async fn pausing_draining_reconnecting_and_saving_are_announced() {
    let h = Harness::new(1, false);
    let mut events = h.engine.subscribe();
    h.engine.pause().unwrap();
    assert_eq!(
        received(&mut events),
        [EngineEvent::Config, EngineEvent::Status]
    );
    h.engine.resume().unwrap();
    assert_eq!(
        received(&mut events),
        [EngineEvent::Config, EngineEvent::Status]
    );
    h.engine.set_draining(true);
    h.engine.set_draining(false);
    assert_eq!(
        received(&mut events),
        [EngineEvent::Status, EngineEvent::Status]
    );
    h.engine
        .set_linear(Arc::new(Disconnected::new("logged out of Linear")));
    assert_eq!(received(&mut events), [EngineEvent::Status]);
    h.engine.update_settings(h.engine.settings()).unwrap();
    h.bind("proj-1", "repo-a");
    assert!(h.engine.remove_binding("proj-1").unwrap());
    assert_eq!(
        received(&mut events),
        [
            EngineEvent::Config,
            EngineEvent::Config,
            EngineEvent::Config
        ]
    );
}

#[tokio::test]
async fn the_end_of_a_run_is_announced_once_its_slot_is_free() {
    let h = Harness::new(1, true);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let mut events = h.engine.subscribe();
    h.runner.gate.add_permits(1);
    // The job's last change comes first, then a status event once the run no longer counts as
    // running: a panel that reads the status on either event ends up with the right count.
    let mut finished = false;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("no status event after the run ended")
            .unwrap();
        match event {
            EngineEvent::Job { job } if job.state == JobState::AwaitingAcceptance => {
                finished = true
            }
            EngineEvent::Status if finished => break,
            _ => {}
        }
    }
    assert_eq!(h.engine.status().running, 0);
}

#[tokio::test]
async fn a_check_that_changes_no_job_announces_none() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2")); // waits for a binding
    h.engine.poll_once().await.unwrap();
    let mut events = h.engine.subscribe();
    h.engine.poll_once().await.unwrap();
    assert_eq!(received(&mut events), [EngineEvent::Status]);
    assert_eq!(h.latest("1").state, JobState::Queued);
    assert_eq!(h.latest("2").state, JobState::WaitingBinding);
}

#[tokio::test]
async fn job_counts_that_cannot_be_read_are_reported_in_the_status() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let db = root.path().join("state.db");
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let engine = Engine::new(EngineDeps {
        config_path,
        store: Store::open(&db).unwrap(),
        linear: Arc::new(fake.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: Arc::new(ScriptedRunner::new(fake, false)),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    engine.poll_once().await.unwrap();
    assert_eq!(engine.status().queued, 1);
    // The queued row can no longer be read.
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute("UPDATE jobs SET created_at = 'yesterday'", [])
        .unwrap();
    let status = engine.status();
    assert_eq!((status.queued, status.waiting), (0, 0));
    let error = status.last_error.expect("the failure is reported");
    assert!(
        error.starts_with("cannot read the job records: "),
        "{error}"
    );
    assert!(status.linear_ok, "the check itself worked");
}

#[tokio::test]
async fn the_issue_list_shows_linear_text_clean() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(Issue {
        title: "Fix\u{1b}[2J\n\u{202e}login".into(),
        state: "In\nTesting\u{200b}".into(),
        labels: vec!["ai-ready".into(), "Bug\u{1b}".into()],
        project_name: Some("Demo\u{202e} Project".into()),
        parent_identifier: Some("DEMO-0\u{2028}".into()),
        ..issue("1", "proj-1")
    });
    let views = h.engine.issues().await.unwrap();
    let issue = &views[0].issue;
    assert_eq!(issue.title, "Fix [2J login");
    assert_eq!(issue.state, "In Testing");
    assert_eq!(issue.labels, ["ai-ready", "Bug"]);
    assert_eq!(issue.project_name.as_deref(), Some("Demo Project"));
    assert_eq!(issue.parent_identifier.as_deref(), Some("DEMO-0"));
    assert_eq!(
        views[0].status,
        IssueStatus::NotTodo {
            state: "In Testing".into()
        }
    );
    // The ids stay as Linear sent them.
    assert_eq!(
        (issue.uuid.as_str(), issue.identifier.as_str()),
        ("1", "DEMO-1")
    );
}

#[tokio::test]
async fn the_issue_list_fails_with_the_reason_while_disconnected() {
    let h = Harness::new(1, false);
    h.engine
        .set_linear(Arc::new(Disconnected::new("not logged in to Linear")));
    let err = h.engine.issues().await.unwrap_err();
    assert!(
        format!("{err:#}").contains("not logged in to Linear"),
        "{err:#}"
    );
}

#[tokio::test]
async fn one_job_and_the_projects_can_be_read() {
    let h = Harness::new(1, false);
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    let job = h.latest("1");
    assert_eq!(h.engine.job(job.id).unwrap(), Some(job.clone()));
    assert_eq!(h.engine.job(job.id + 1000).unwrap(), None);
    let projects = h.engine.projects().await.unwrap();
    assert_eq!(projects[0].id, "proj-1");
}

#[tokio::test(start_paused = true)]
async fn a_start_that_keeps_failing_is_tried_again_after_a_growing_pause() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    h.linear.fail_issue_reads("1");
    // A dispatch that tried the start leaves its failure in the status; a check clears it.
    let tried = |h: &Harness| h.engine.status().last_error.is_some();
    let mut events = h.engine.subscribe();
    h.engine.dispatch().await.unwrap();
    assert!(tried(&h));
    assert_eq!(
        received(&mut events),
        [EngineEvent::Status],
        "the failure is announced"
    );
    // 30 s after the first failure, then 60 s after the second.
    for (wait, retried) in [(29, false), (1, true), (59, false), (1, true)] {
        h.engine.poll_once().await.unwrap();
        tokio::time::advance(Duration::from_secs(wait)).await;
        h.engine.dispatch().await.unwrap();
        assert_eq!(tried(&h), retried, "after {wait} more seconds");
    }
    h.linear.clear_issue_failures();
    tokio::time::advance(Duration::from_secs(120)).await;
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1"]);
}

#[tokio::test]
async fn a_job_held_back_after_a_failed_start_says_when_it_is_tried_again() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert!(h.engine.status().held.is_empty());
    h.linear.fail_issue_reads("1");
    let before = chrono::Utc::now();
    h.engine.dispatch().await.unwrap();
    let after = chrono::Utc::now();
    let id = h.latest("1").id;
    let held = h.engine.status().held;
    assert_eq!(held.len(), 1, "{held:?}");
    assert_eq!(held[0].job_id, id);
    // The first pause is 30 s, given as a wall-clock time the panel can show.
    let pause = chrono::Duration::seconds(30);
    assert!(
        held[0].retry_at >= before + pause && held[0].retry_at <= after + pause,
        "{held:?}"
    );
    let json = serde_json::to_value(h.engine.status()).unwrap();
    assert_eq!(json["held"][0]["job_id"], id);
    // A stopped job is no longer listed, and a retry starts afresh.
    h.engine.cancel_job(id).unwrap();
    assert!(
        h.engine.status().held.is_empty(),
        "a stopped job is not held"
    );
    h.engine.retry_job(id).unwrap();
    assert!(h.engine.status().held.is_empty(), "a retry starts afresh");
}

#[tokio::test]
async fn a_retry_is_not_held_back_by_an_earlier_failed_start() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    h.linear.fail_issue_reads("1");
    h.engine.dispatch().await.unwrap();
    let id = h.latest("1").id;
    h.engine.cancel_job(id).unwrap();
    h.linear.clear_issue_failures();
    h.engine.retry_job(id).unwrap();
    h.engine.dispatch().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1"]);
}

#[tokio::test]
async fn a_check_under_way_when_linear_is_reconnected_keeps_to_one_connection() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.engine.status().viewer.unwrap().id, "user-1");
    // The first connection answers `viewer()` only when the test lets it.
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let first = Arc::new(SpyLinear::new(h.linear.clone()));
    *first.viewer_gate.lock().unwrap() = Some(gate.clone());
    h.engine.set_linear(first.clone());
    assert_eq!(
        h.engine.status().viewer,
        None,
        "the user of a new connection is not known yet"
    );
    let poll = tokio::spawn({
        let engine = h.engine.clone();
        async move { engine.poll_once().await }
    });
    eventually("the check to ask for the user", || {
        first.viewer_calls() == 1
    })
    .await;
    // Meanwhile the user logs in to another account.
    h.engine.set_linear(Arc::new(FakeLinear::new("user-2")));
    gate.add_permits(1);
    poll.await.unwrap().unwrap();
    // The check under way read its user and its issues through the first connection only.
    assert_eq!(h.latest("1").state, JobState::Queued);
    // Its user is not remembered for the second connection.
    assert_eq!(h.engine.status().viewer, None);
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.engine.status().viewer.unwrap().id, "user-2");
    assert_eq!(
        h.latest("1").state,
        JobState::Canceled,
        "the second account has no such issue"
    );
}

#[tokio::test]
async fn jobs_the_start_turns_down_are_announced() {
    let h = Harness::new(3, false);
    let gone = h.bind("proj-1", "repo-a");
    h.bind("proj-2", "repo-b");
    h.bind("proj-3", "repo-c");
    for (uuid, project) in [("1", "proj-1"), ("2", "proj-2"), ("3", "proj-3")] {
        h.linear.upsert(issue(uuid, project));
    }
    h.engine.poll_once().await.unwrap();
    // Before the start: DEMO-1's folder disappears, DEMO-2 loses its label and DEMO-3's
    // project moves to another folder.
    std::fs::remove_dir(&gone).unwrap();
    h.linear.update("2", |i| i.labels.clear());
    let moved = h.root.path().join("repo-d");
    std::fs::create_dir_all(&moved).unwrap();
    h.engine
        .add_binding(demo_binding("proj-3", &moved))
        .unwrap();
    let mut events = h.engine.subscribe();
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    let jobs: Vec<(String, JobState)> = received(&mut events)
        .into_iter()
        .filter_map(|e| match e {
            EngineEvent::Job { job } => Some((job.issue_identifier, job.state)),
            _ => None,
        })
        .collect();
    assert_eq!(
        jobs,
        [
            ("DEMO-1".to_string(), JobState::WaitingBinding),
            ("DEMO-2".to_string(), JobState::Canceled),
            ("DEMO-3".to_string(), JobState::Queued),
        ]
    );
    assert_eq!(h.latest("3").directory.as_deref(), Some(moved.as_path()));
}

#[tokio::test]
async fn a_run_whose_result_could_not_be_recorded_is_announced_as_failed() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config_with_one_binding(root.path());
    let fake = FakeLinear::new("user-1");
    fake.upsert(issue("1", "proj-1"));
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    // `issue()` calls: 1 = the start re-read, 2 = the check of the result (panics).
    *spy.panic_on_issue_call.lock().unwrap() = Some(2);
    let runner = Arc::new(ScriptedRunner::new(fake, false));
    let engine = build_engine(root.path(), &cfg, spy, Arc::new(FakeReadiness), runner);
    let mut events = engine.subscribe();
    engine.tick().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), engine.wait_idle())
        .await
        .expect("wait_idle hangs after a panic while recording");
    let states: Vec<JobState> = received(&mut events)
        .into_iter()
        .filter_map(|e| match e {
            EngineEvent::Job { job } => Some(job.state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [JobState::Queued, JobState::Running, JobState::Failed]
    );
}

#[tokio::test]
async fn events_and_issue_statuses_have_the_json_shape_the_panel_reads() {
    let h = Harness::new(1, false);
    let mut events = h.engine.subscribe();
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.poll_once().await.unwrap();
    let events: Vec<serde_json::Value> = received(&mut events)
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(events[0]["kind"], "job");
    assert_eq!(events[0]["job"]["issue_identifier"], "DEMO-1");
    assert_eq!(events[0]["job"]["state"], "waiting_binding");
    assert_eq!(events[1], serde_json::json!({"kind": "status"}));
    assert_eq!(
        serde_json::to_value(EngineEvent::Config).unwrap(),
        serde_json::json!({"kind": "config"})
    );
    let shapes = [
        (
            IssueStatus::Eligible { picked_up: true },
            serde_json::json!({"kind": "eligible", "picked_up": true}),
        ),
        (
            IssueStatus::Eligible { picked_up: false },
            serde_json::json!({"kind": "eligible", "picked_up": false}),
        ),
        (
            IssueStatus::NotTodo {
                state: "In Testing".into(),
            },
            serde_json::json!({"kind": "not_todo", "state": "In Testing"}),
        ),
        (
            IssueStatus::NotCandidate,
            serde_json::json!({"kind": "not_candidate"}),
        ),
        (
            IssueStatus::Waiting {
                reason: WaitReasonView::ProjectNotBound,
            },
            serde_json::json!({"kind": "waiting", "reason": {"code": "project_not_bound"}}),
        ),
        (
            IssueStatus::Waiting {
                reason: WaitReasonView::DirectoryMissing {
                    directory: "demo-app".into(),
                },
            },
            serde_json::json!({"kind": "waiting", "reason": {"code": "directory_missing", "directory": "demo-app"}}),
        ),
        (
            IssueStatus::Waiting {
                reason: WaitReasonView::ExecutorNotReady {
                    executor: ExecutorKind::Claude,
                },
            },
            serde_json::json!({"kind": "waiting", "reason": {"code": "executor_not_ready", "executor": "claude"}}),
        ),
        (IssueStatus::Paused, serde_json::json!({"kind": "paused"})),
    ];
    for (status, json) in shapes {
        assert_eq!(serde_json::to_value(&status).unwrap(), json);
        assert_eq!(serde_json::from_value::<IssueStatus>(json).unwrap(), status);
    }
    // The status of a host from before `draining` existed still reads.
    let older: StatusSnapshot = serde_json::from_value(serde_json::json!({
        "paused": false, "linear_ok": false, "viewer": null, "last_poll_at": null,
        "last_error": null, "running": 0, "queued": 0, "waiting": 0
    }))
    .unwrap();
    assert!(!older.draining);
    assert!(
        older.held.is_empty(),
        "a host from before `held` existed sends none"
    );
}

#[tokio::test(start_paused = true)]
async fn a_stop_does_not_wait_for_a_check_that_waits_for_linear() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    let shutdown = CancellationToken::new();
    let worker = spawn_loop(&h.engine, &shutdown);
    eventually("DEMO-1 to start", || h.runner.started() == ["DEMO-1"]).await;
    // From now on Linear never answers who the user is, so the next check hangs in that call.
    let silent = Arc::new(SpyLinear::new(h.linear.clone()));
    *silent.viewer_gate.lock().unwrap() = Some(Arc::new(tokio::sync::Semaphore::new(0)));
    h.engine.set_linear(silent.clone());
    eventually("the next check to wait for Linear", || {
        silent.viewer_calls() == 1
    })
    .await;
    // Paused clock: the limit only passes if the loop is still waiting for the check.
    let asked = tokio::time::Instant::now();
    shutdown.cancel();
    tokio::time::timeout(Duration::from_millis(500), worker)
        .await
        .expect("the stop waited for the check in progress")
        .unwrap();
    assert!(asked.elapsed() < Duration::from_millis(500));
    // The run in progress was told to stop.
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::Canceled);
}

#[tokio::test]
async fn a_stop_and_retry_during_a_failing_start_is_not_held_back() {
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
    let id = job_of(&engine, "1").id;
    // The start's re-read of the issue waits for the test, and then fails.
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *spy.issue_gate.lock().unwrap() = Some(gate.clone());
    *spy.fail_issue.lock().unwrap() = Some(("1".into(), "Linear is unreachable".into()));
    let dispatch = tokio::spawn({
        let engine = engine.clone();
        async move { engine.dispatch().await }
    });
    eventually("the start to read the issue", || spy.issue_calls() == 1).await;
    // Meanwhile the user stops the job and runs it again.
    engine.cancel_job(id).unwrap();
    engine.retry_job(id).unwrap();
    gate.add_permits(1);
    assert!(dispatch.await.unwrap().unwrap().is_empty());
    let error = engine.status().last_error.expect("the failure is reported");
    assert!(error.contains("Linear is unreachable"), "{error}");
    // That failure belongs to the attempt before the retry: the retried job starts at once.
    *spy.issue_gate.lock().unwrap() = None;
    *spy.fail_issue.lock().unwrap() = None;
    assert_eq!(engine.dispatch().await.unwrap(), [id]);
    engine.wait_idle().await;
    assert_eq!(runner.started(), ["DEMO-1"]);
}

#[tokio::test]
async fn project_names_are_shown_clean() {
    let root = tempfile::tempdir().unwrap();
    let fake = FakeLinear::new("user-1");
    let mut hostile = fake.projects().await.unwrap().remove(0);
    hostile.name = "Demo\u{202e} Pro\u{200b}ject\n\u{1b}".into();
    let spy = Arc::new(SpyLinear::new(fake.clone()));
    *spy.projects.lock().unwrap() = Some(vec![hostile.clone()]);
    let engine = build_engine(
        root.path(),
        &Config::default(),
        spy,
        Arc::new(FakeReadiness),
        Arc::new(ScriptedRunner::new(fake, false)),
    );
    let projects = engine.projects().await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "Demo Project");
    assert_eq!(projects[0].id, hostile.id, "the id stays as Linear sent it");
}

#[tokio::test(start_paused = true)]
async fn saved_settings_apply_without_waiting_for_the_timer() {
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
    // No time passes, so only the save can wake the loop for DEMO-2.
    let mut settings = h.engine.settings();
    settings.max_concurrent = 2;
    h.engine.update_settings(settings).unwrap();
    eventually("DEMO-2 to start", || {
        h.runner.started() == ["DEMO-1", "DEMO-2"]
    })
    .await;
    h.runner.gate.add_permits(2);
    h.settle().await;
    stop_loop(&shutdown, worker).await;
}

/// A harness for a user whose folders are `homes`, with executors that are ready for lean runs.
fn lean_harness(homes: Homes) -> Harness {
    Harness::with_parts(Config::default(), false, homes, Arc::new(FakeReadiness))
}

/// The user's home folder `home`, and no Claude configuration folder.
fn home_at(home: &Path) -> Homes {
    Homes {
        user_home: Some(home.to_path_buf()),
        claude_config_dir: None,
    }
}

/// Binds proj-1 to `dir` with the demo binding as `change` leaves it, then lets issue 1 run;
/// what its run was started with.
async fn started_spec(h: &Harness, dir: &Path, change: impl FnOnce(&mut Binding)) -> CommandSpec {
    std::fs::create_dir_all(dir).unwrap();
    let mut binding = demo_binding("proj-1", dir);
    change(&mut binding);
    h.engine.add_binding(binding).unwrap();
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let specs = h.runner.specs.lock().unwrap();
    assert_eq!(specs.len(), 1, "the run started");
    specs[0].clone()
}

/// The path a lean Claude run's `--settings` leaves out, as the JSON names it.
fn excluded(spec: &CommandSpec) -> Option<String> {
    let at = spec.args.iter().position(|a| a == "--settings")?;
    let settings: serde_json::Value = serde_json::from_str(&spec.args[at + 1]).unwrap();
    Some(
        settings["claudeMdExcludes"][0]
            .as_str()
            .unwrap()
            .to_string(),
    )
}

fn has(spec: &CommandSpec, arg: &str) -> bool {
    spec.args.iter().any(|a| a == arg)
}

const AUTO_MEMORY_OFF: (&str, &str) = ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");

fn auto_memory_off() -> Vec<(String, String)> {
    vec![(AUTO_MEMORY_OFF.0.to_string(), AUTO_MEMORY_OFF.1.to_string())]
}

/// The first line of every lean run's log, so that a finished run shows it was lean.
const CLAUDE_LEAN_NOTE: &str = "lean run: only the Linear MCP server and the repository's own \
    settings; no user-level settings, plugins, hooks or auto memory";
const CODEX_LEAN_NOTE: &str = "lean run: only the Linear MCP server; the user's Codex \
    configuration, hooks, plugins, apps and memories are left out";

#[tokio::test]
async fn a_lean_claude_binding_starts_with_the_lean_arguments() {
    let home = tempfile::tempdir().unwrap();
    let h = lean_harness(home_at(home.path()));
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |_| {}).await;
    assert!(spec.lean);
    assert!(has(&spec, "--strict-mcp-config"), "{:?}", spec.args);
    let instructions = home.path().join(".claude").join("CLAUDE.md");
    assert_eq!(excluded(&spec).as_deref(), instructions.to_str());
    assert_eq!(spec.env, auto_memory_off());
    assert_eq!(spec.notes, [CLAUDE_LEAN_NOTE]);
}

// The next three know no home folder: a lean Claude run would then get a note, which these runs
// must not.
#[tokio::test]
async fn a_binding_with_lean_off_starts_with_the_full_configuration() {
    let h = lean_harness(Homes::default());
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |b| b.lean = false).await;
    assert!(!spec.lean);
    assert!(!has(&spec, "--strict-mcp-config") && !has(&spec, "--settings"));
    assert!(spec.env.is_empty() && spec.notes.is_empty());
}

#[tokio::test]
async fn a_personal_binding_never_runs_lean() {
    let h = lean_harness(Homes::default());
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |b| {
        b.permission = Some(PermissionProfile::Personal)
    })
    .await;
    assert!(!spec.lean);
    assert!(has(&spec, "dontAsk") && !has(&spec, "--strict-mcp-config"));
    assert!(spec.env.is_empty() && spec.notes.is_empty());
}

#[tokio::test]
async fn a_binding_in_the_home_folder_never_runs_lean() {
    let home = tempfile::tempdir().unwrap();
    let h = lean_harness(home_at(home.path()));
    let spec = started_spec(&h, home.path(), |_| {}).await;
    assert!(!spec.lean);
    assert!(!has(&spec, "--strict-mcp-config"));
    assert!(spec.env.is_empty());
}

#[tokio::test]
async fn a_binding_in_the_home_folder_does_not_wait_for_the_lean_checks() {
    let home = tempfile::tempdir().unwrap();
    let h = Harness::with_parts(
        Config::default(),
        false,
        home_at(home.path()),
        Arc::new(FullOnlyReadiness),
    );
    // It started at all: a lean binding would wait for the lean checks.
    let spec = started_spec(&h, home.path(), |_| {}).await;
    assert!(!spec.lean);
    let report = h.engine.doctor().await;
    assert_eq!(report.bindings[0].lean, LeanStatus::HomeDirectory);
    assert!(report.bindings[0].executor_ready, "{:?}", report.bindings);
}

#[tokio::test]
async fn the_issue_list_does_not_make_a_binding_in_the_home_folder_wait_for_the_lean_checks() {
    let home = tempfile::tempdir().unwrap();
    let h = Harness::with_parts(
        Config::default(),
        false,
        home_at(home.path()),
        Arc::new(FullOnlyReadiness),
    );
    h.engine
        .add_binding(demo_binding("proj-1", home.path()))
        .unwrap();
    h.linear.upsert(issue("1", "proj-1"));
    // Before any poll: the list says what a poll would do with it.
    let issues = h.engine.issues().await.unwrap();
    assert_eq!(issues[0].status, IssueStatus::Eligible { picked_up: true });
}

#[tokio::test]
async fn a_lean_binding_waits_while_its_executor_is_not_ready_for_lean_runs() {
    let home = tempfile::tempdir().unwrap();
    let h = Harness::with_parts(
        Config::default(),
        false,
        home_at(home.path()),
        Arc::new(FullOnlyReadiness),
    );
    h.bind("proj-1", "repo-a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest("1");
    assert_eq!(job.state, JobState::WaitingBinding);
    assert_eq!(job.reason.as_deref(), Some("executor claude is not ready"));
    assert!(h.runner.started().is_empty());
    let report = h.engine.doctor().await;
    assert_eq!(report.bindings[0].lean, LeanStatus::On);
    assert!(!report.bindings[0].executor_ready, "{:?}", report.bindings);
}

#[tokio::test]
async fn the_binding_effort_wins_over_the_default() {
    let home = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.settings.default_effort = Some(Effort::High);
    let h = Harness::with_parts(cfg, false, home_at(home.path()), Arc::new(FakeReadiness));
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |b| {
        b.effort = Some(Effort::Max)
    })
    .await;
    let efforts: Vec<_> = spec
        .args
        .windows(2)
        .filter(|w| w[0] == "--effort")
        .map(|w| w[1].as_str())
        .collect();
    assert_eq!(efforts, ["max"]);
}

#[tokio::test]
async fn the_default_effort_applies_to_a_binding_without_its_own() {
    let home = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.settings.default_effort = Some(Effort::High);
    let h = Harness::with_parts(cfg, false, home_at(home.path()), Arc::new(FakeReadiness));
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |b| {
        b.executor = Some(ExecutorKind::Codex)
    })
    .await;
    assert!(
        spec.args
            .windows(2)
            .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
        "{:?}",
        spec.args
    );
}

#[tokio::test]
async fn an_unpassable_home_leaves_a_note_and_no_settings_argument() {
    let parent = tempfile::tempdir().unwrap();
    let home = parent.path().join("demo 50% home");
    std::fs::create_dir_all(&home).unwrap();
    let h = lean_harness(home_at(&home));
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |_| {}).await;
    assert!(spec.lean);
    assert_eq!(excluded(&spec), None);
    assert!(has(&spec, "--strict-mcp-config"));
    assert_eq!(
        spec.notes,
        [
            CLAUDE_LEAN_NOTE,
            "the user-level CLAUDE.md path cannot be passed safely, so this lean run still reads it"
        ]
    );
}

#[tokio::test]
async fn an_unpassable_claude_config_dir_leaves_a_note_too() {
    let home = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let config_dir = parent.path().join("claude!config");
    let h = lean_harness(Homes {
        user_home: Some(home.path().to_path_buf()),
        claude_config_dir: Some(config_dir),
    });
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |_| {}).await;
    assert_eq!(excluded(&spec), None);
    assert_eq!(
        spec.notes,
        [
            CLAUDE_LEAN_NOTE,
            "the user-level CLAUDE.md path cannot be passed safely, so this lean run still reads it"
        ]
    );
}

#[tokio::test]
async fn without_a_home_a_lean_claude_run_leaves_a_note() {
    let h = lean_harness(Homes::default());
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |_| {}).await;
    assert!(spec.lean);
    assert_eq!(excluded(&spec), None);
    assert_eq!(
        spec.notes,
        [
            CLAUDE_LEAN_NOTE,
            "could not tell where the user-level CLAUDE.md is, so this lean run may still read it"
        ]
    );
}

#[tokio::test]
async fn a_codex_lean_binding_starts_with_the_codex_lean_arguments() {
    let h = lean_harness(Homes::default());
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |b| {
        b.executor = Some(ExecutorKind::Codex)
    })
    .await;
    assert!(spec.lean);
    assert!(has(&spec, "--ignore-user-config"), "{:?}", spec.args);
    assert!(has(&spec, "mcp_servers.linear.required=true"));
    assert_eq!(spec.args.last().map(String::as_str), Some("-"));
    assert!(spec.env.is_empty());
    assert_eq!(spec.notes, [CODEX_LEAN_NOTE]);
}

#[tokio::test]
async fn claude_config_dir_moves_the_excluded_path() {
    let home = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let h = lean_harness(Homes {
        user_home: Some(home.path().to_path_buf()),
        claude_config_dir: Some(config_dir.path().to_path_buf()),
    });
    let spec = started_spec(&h, &h.root.path().join("repo-a"), |_| {}).await;
    let instructions = config_dir.path().join("CLAUDE.md");
    assert_eq!(excluded(&spec).as_deref(), instructions.to_str());
}
