use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use issuerelay_core::{
    config::ExecutorKind,
    executor::CommandSpec,
    runner::{self, ExitKind, RunLimits},
};
use tokio_util::sync::CancellationToken;

fn agent() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-agent"))
}

fn script(dir: &Path, steps: serde_json::Value) -> PathBuf {
    let path = dir.join("script.json");
    std::fs::write(&path, steps.to_string()).unwrap();
    path
}

fn spec(program: PathBuf, cwd: &Path, script: &Path, stdin: &str) -> CommandSpec {
    CommandSpec {
        program,
        args: vec![script.to_string_lossy().into_owned()],
        cwd: cwd.to_path_buf(),
        stdin: stdin.into(),
    }
}

fn limits(total_ms: u64, stall_ms: u64) -> RunLimits {
    RunLimits {
        total: Duration::from_millis(total_ms),
        stall: Duration::from_millis(stall_ms),
    }
}

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

const RESULT_LINE: &str =
    r#"{"type":"result","result":"done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}"}"#;

/// What `RESULT_LINE` carries in its `result` field: the run's final text.
const RESULT_TEXT: &str = "done\n{\"result\":\"in_testing\",\"summary\":\"ok\"}";

/// Waits until `path` has content. Returns false if it is still empty after 10 s.
async fn wait_for_content(path: &Path) -> bool {
    let give_up = Instant::now() + Duration::from_secs(10);
    while file_len(path) == 0 {
        if Instant::now() >= give_up {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

/// Fails with `message` unless the heartbeat file at `path` has stopped growing: it is measured
/// 500 ms from now and again 500 ms later.
#[cfg(any(target_os = "macos", windows))]
async fn assert_stops_growing(path: &Path, message: &str) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = file_len(path);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(file_len(path), settled, "{message}");
}

/// A script whose heartbeat grandchild leaves the run's process group, then a long sleep.
#[cfg(any(target_os = "macos", windows))]
fn escaped_heartbeat_script(dir: &Path, heartbeat: &Path) -> PathBuf {
    script(
        dir,
        serde_json::json!([{"escaped_heartbeat": heartbeat, "every_ms": 50}, {"sleep_ms": 30_000}]),
    )
}

fn log_lines(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}

#[tokio::test]
async fn exits_zero_captures_final_text_and_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("stdin.txt");
    let s = script(
        dir.path(),
        serde_json::json!([
            {"stdin_to": copy}, {"out": "{\"type\":\"system\"}"}, {"err": "note"}, {"out": RESULT_LINE}, {"exit": 0}
        ]),
    );
    let log = dir.path().join("logs").join("1.jsonl");
    let prompt = "多语言 \"quoted\" prompt\nline 2";
    let out = runner::run(
        spec(agent(), dir.path(), &s, prompt),
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert!(out.final_text.unwrap().contains("in_testing"));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), prompt);
    let lines: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(lines.len(), 3);
    assert!(
        lines
            .iter()
            .any(|l| l.contains(r#""stream":"stderr""#) && l.contains("note"))
    );
}

#[tokio::test]
async fn nonzero_exit_code_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"exit": 3}]));
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Codex,
        &dir.path().join("l.jsonl"),
        limits(30_000, 30_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(3)));
    assert_eq!(out.final_text, None);
}

#[tokio::test]
async fn stalled_run_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": "x"}, {"sleep_ms": 20_000}]),
    );
    let started = Instant::now();
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(30_000, 400),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Stalled);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[tokio::test]
async fn chatty_run_hits_total_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"repeat_out": "tick", "every_ms": 50}]),
    );
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(600, 5_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::TimedOut);
}

#[tokio::test]
async fn cancel_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"sleep_ms": 20_000}]));
    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        canceller.cancel();
    });
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(30_000, 30_000),
        token,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Canceled);
}

#[tokio::test]
async fn kill_reaches_grandchildren() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(
        dir.path(),
        serde_json::json!([{"heartbeat": hb, "every_ms": 50}, {"sleep_ms": 20_000}]),
    );
    let token = CancellationToken::new();
    let canceller = token.clone();
    let heartbeat = hb.clone();
    tokio::spawn(async move {
        // Cancel only once the grandchild is really running; a fixed delay is too short when the
        // machine is busy or starts processes slowly (Windows).
        wait_for_content(&heartbeat).await;
        canceller.cancel();
    });
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(30_000, 30_000),
        token,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Canceled);
    assert!(file_len(&hb) > 0, "heartbeat child never ran");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let settled = file_len(&hb);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(file_len(&hb), settled, "grandchild survived the kill");
}

// Tool processes may leave the executor's process group: on macOS Claude Code starts its Bash tool
// shells with `setsid`. Ending a run must end them too. On Windows the Job Object holds them
// anyway. (Linux is not a target platform and only ends the process group, so it skips these.)
#[cfg(any(target_os = "macos", windows))]
#[tokio::test]
async fn an_escaped_grandchild_is_ended_on_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = escaped_heartbeat_script(dir.path(), &hb);
    let log = dir.path().join("l.jsonl");
    let mut run = Box::pin(runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(5_000, 60_000),
        CancellationToken::new(),
    ));
    // The total limit must cut into a grandchild that is really running, however slowly the
    // machine starts processes.
    tokio::select! {
        out = &mut run => panic!("the run ended before the heartbeat appeared: {:?}", out.exit),
        started = wait_for_content(&hb) => assert!(started, "heartbeat child never ran"),
    }
    assert_eq!(run.await.exit, ExitKind::TimedOut);
    assert_stops_growing(&hb, "the escaped grandchild survived the timeout").await;
}

#[cfg(any(target_os = "macos", windows))]
#[tokio::test]
async fn an_escaped_grandchild_is_ended_on_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = escaped_heartbeat_script(dir.path(), &hb);
    let token = CancellationToken::new();
    let canceller = token.clone();
    let heartbeat = hb.clone();
    let started = tokio::spawn(async move {
        let started = wait_for_content(&heartbeat).await;
        canceller.cancel();
        started
    });
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(60_000, 60_000),
        token,
    )
    .await;
    assert!(started.await.unwrap(), "heartbeat child never ran");
    assert_eq!(out.exit, ExitKind::Canceled);
    assert_stops_growing(&hb, "the escaped grandchild survived the cancel").await;
}

#[cfg(any(target_os = "macos", windows))]
#[tokio::test]
async fn an_escaped_grandchild_is_ended_when_the_run_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = escaped_heartbeat_script(dir.path(), &hb);
    let command = spec(agent(), dir.path(), &s, "");
    let log = dir.path().join("l.jsonl");
    let task = tokio::spawn(async move {
        runner::run(
            command,
            ExecutorKind::Claude,
            &log,
            limits(60_000, 60_000),
            CancellationToken::new(),
        )
        .await
    });
    assert!(wait_for_content(&hb).await, "heartbeat child never ran");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_stops_growing(&hb, "the escaped grandchild survived dropping the run").await;
}

#[tokio::test]
async fn normal_exit_returns_promptly_even_with_background_child() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(
        dir.path(),
        serde_json::json!([{"heartbeat": hb, "every_ms": 50}, {"out": RESULT_LINE}, {"exit": 0}]),
    );
    let started = Instant::now();
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(60_000, 60_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "waited for the background child"
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    let settled = file_len(&hb);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(file_len(&hb), settled, "background child was left running");
}

#[tokio::test]
async fn missing_program_reports_spawn_failure() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([]));
    let out = runner::run(
        spec(dir.path().join("no-such-program"), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(5_000, 5_000),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(out.exit, ExitKind::SpawnFailed(_)));
}

// The caller drops the run future instead of cancelling it (here by aborting its task). The tree
// must still end: on Unix process-wrap's drop only signals the leader, not the process group.
#[tokio::test]
async fn dropping_the_run_future_ends_the_process_tree() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(
        dir.path(),
        serde_json::json!([{"heartbeat": hb, "every_ms": 50}, {"sleep_ms": 30_000}]),
    );
    let command = spec(agent(), dir.path(), &s, "");
    let log = dir.path().join("l.jsonl");
    let task = tokio::spawn(async move {
        runner::run(
            command,
            ExecutorKind::Claude,
            &log,
            limits(60_000, 60_000),
            CancellationToken::new(),
        )
        .await
    });
    assert!(wait_for_content(&hb).await, "heartbeat child never ran");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = file_len(&hb);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        file_len(&hb),
        settled,
        "grandchild survived dropping the run"
    );
}

// An outer timeout around the run (the engine will wrap whole runs in one) drops it the same way.
#[tokio::test]
async fn an_outer_timeout_ends_the_process_tree() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(
        dir.path(),
        serde_json::json!([{"heartbeat": hb, "every_ms": 50}, {"sleep_ms": 30_000}]),
    );
    let log = dir.path().join("l.jsonl");
    let mut run = Box::pin(runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(60_000, 60_000),
        CancellationToken::new(),
    ));
    // Drive the run until the grandchild is alive, so the timeout below cuts into a real tree
    // however slow the machine is.
    tokio::select! {
        _ = &mut run => panic!("the run ended before the heartbeat appeared"),
        started = wait_for_content(&hb) => assert!(started, "heartbeat child never ran"),
    }
    let timed = tokio::time::timeout(Duration::from_millis(1500), &mut run).await;
    assert!(
        timed.is_err(),
        "the run should still be going when the timeout fires"
    );
    drop(run);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = file_len(&hb);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        file_len(&hb),
        settled,
        "grandchild survived the outer timeout"
    );
}

// Chatty output must keep resetting the stall timer: the run ends at the total limit, not at the
// much shorter stall limit. Both limits are generous because the stall clock also covers the
// executor's start-up, which can take hundreds of milliseconds on a busy machine.
#[tokio::test]
async fn stdout_output_keeps_the_stall_timer_from_firing() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"repeat_out": "tick", "every_ms": 50}]),
    );
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(3_000, 1_200),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::TimedOut);
}

// Same as above for stderr: the stall timer must not look at stdout only.
#[tokio::test]
async fn stderr_output_also_counts_as_activity() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"repeat_err": "tick", "every_ms": 50}]),
    );
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(3_000, 1_200),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::TimedOut);
}

// The executor prints a burst and exits at once, long before the runner has written it all down.
// Only the drain after the exit can deliver the tail, including the final result.
#[tokio::test]
async fn a_burst_before_exit_is_fully_drained() {
    let dir = tempfile::tempdir().unwrap();
    let mut steps: Vec<serde_json::Value> = (0..3000)
        .map(|n| serde_json::json!({"out": format!("line {n}")}))
        .collect();
    steps.push(serde_json::json!({"out": RESULT_LINE}));
    steps.push(serde_json::json!({"exit": 0}));
    let s = script(dir.path(), serde_json::Value::Array(steps));
    let log = dir.path().join("l.jsonl");
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(60_000, 60_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(out.final_text.as_deref(), Some(RESULT_TEXT));
    let lines = log_lines(&log);
    assert!(
        lines.len() >= 3001,
        "only {} of 3001 lines reached the log",
        lines.len()
    );
    assert_eq!(lines.last().map(String::as_str), Some(RESULT_LINE));
}

// Limits too large to add to the clock must mean "no deadline", not a panic in `Instant + Duration`.
#[tokio::test]
async fn huge_limits_mean_no_deadline_instead_of_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": RESULT_LINE}, {"exit": 0}]),
    );
    let huge = RunLimits {
        total: Duration::MAX,
        stall: Duration::MAX,
    };
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        huge,
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(out.final_text.as_deref(), Some(RESULT_TEXT));
}

#[tokio::test]
async fn spawn_failure_is_written_to_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([]));
    let log = dir.path().join("logs").join("1.jsonl");
    let out = runner::run(
        spec(dir.path().join("no-such-program"), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(5_000, 5_000),
        CancellationToken::new(),
    )
    .await;
    let ExitKind::SpawnFailed(message) = out.exit else {
        panic!("expected SpawnFailed, got {:?}", out.exit);
    };
    let lines = log_lines(&log);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let entry: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(entry["stream"], "runner");
    assert_eq!(entry["text"], format!("spawn failed: {message}"));
}

#[tokio::test]
async fn the_log_is_private_to_the_user() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"out": "x"}, {"exit": 0}]));
    let log = dir.path().join("l.jsonl");
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "log mode is {mode:o}");
    }
    #[cfg(windows)]
    {
        // Windows keeps the default access of the per-user directory the log lives in.
        assert_eq!(log_lines(&log), ["x"]);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn cmd_wrapper_passes_prompt_and_paths_intact() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("目录 with space");
    std::fs::create_dir(&work).unwrap();
    let copy = work.join("stdin.txt");
    let s = script(&work, serde_json::json!([{"stdin_to": copy}, {"exit": 0}]));
    let wrapper = work.join("agent.cmd");
    std::fs::write(
        &wrapper,
        format!("@echo off\r\n\"{}\" %*\r\n", agent().display()),
    )
    .unwrap();
    let prompt = "line1\n\"quoted\" & | ^ % 中文 'x'";
    let out = runner::run(
        spec(wrapper, &work, &s, prompt),
        ExecutorKind::Codex,
        &work.join("l.jsonl"),
        limits(30_000, 30_000),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), prompt);
}
