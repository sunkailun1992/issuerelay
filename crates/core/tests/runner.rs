#[cfg(target_os = "macos")]
mod common;

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(target_os = "macos")]
use common::orphan::Orphan;
use issuerelay_core::{
    config::ExecutorKind,
    env::ExecEnv,
    executor::CommandSpec,
    model::ProcessTag,
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
        env: Vec::new(),
        lean: false,
        notes: Vec::new(),
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

/// The text of one of IssueRelay's own lines in the log (`{"stream":"runner",…}`); `None` for
/// any other line.
fn runner_note(line: &str) -> Option<String> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    (entry["stream"] == "runner").then(|| entry["text"].as_str().unwrap_or_default().to_string())
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
        &ExecEnv::inherit(),
        None,
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

// The run's output carries what the executor printed about its session and usage (run-improvements
// design, section 7), in the shapes of the 2026-10-27 probes.
#[tokio::test]
async fn a_run_reports_the_session_and_usage_its_executor_printed() {
    let dir = tempfile::tempdir().unwrap();
    let init = r#"{"type":"system","subtype":"init","session_id":"b88d7053-d27f-4825-b81c-80f6fbf109d0","mcp_servers":[]}"#;
    let result = r#"{"type":"result","subtype":"success","result":"ok","total_cost_usd":0.0551374,"num_turns":1,"usage":{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":18687,"cache_creation_input_tokens":6414}}"#;
    let s = script(
        dir.path(),
        serde_json::json!([{"out": init}, {"out": result}, {"exit": 0}]),
    );
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("1.jsonl"),
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(
        out.session_id.as_deref(),
        Some("b88d7053-d27f-4825-b81c-80f6fbf109d0")
    );
    let usage = out.usage.expect("usage");
    assert_eq!(usage.session_cost_micro_usd, Some(55_137));
    assert_eq!(usage.cache_read_tokens, Some(18_687));
    assert_eq!(usage.turns, Some(1));
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
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(3)));
    assert_eq!(out.final_text, None);
}

// The proxy variable is set on the child only, through `env`; this process never sets it. The
// nested-session marker comes from the environment of whoever runs the tests (set when the tests
// run inside a Claude Code session) and must not reach the child either way.
#[tokio::test]
async fn the_run_gets_the_resolved_environment() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"print_env": "ISSUERELAY_TEST_PROXY"}, {"print_env": "CLAUDECODE"}]),
    );
    let log = dir.path().join("1.jsonl");
    let env = ExecEnv {
        proxy: vec![("ISSUERELAY_TEST_PROXY".into(), "http://127.0.0.1:9".into())],
        ..ExecEnv::inherit()
    };
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &env,
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    let lines = log_lines(&log);
    assert!(
        lines.contains(&"ISSUERELAY_TEST_PROXY=http://127.0.0.1:9".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"CLAUDECODE=<unset>".to_string()),
        "{lines:?}"
    );
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
    let env = ExecEnv::inherit();
    let mut run = Box::pin(runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(5_000, 60_000),
        CancellationToken::new(),
        &env,
        None,
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
        &ExecEnv::inherit(),
        None,
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
            &ExecEnv::inherit(),
            None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
            &ExecEnv::inherit(),
            None,
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
    let env = ExecEnv::inherit();
    let mut run = Box::pin(runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(60_000, 60_000),
        CancellationToken::new(),
        &env,
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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
        &ExecEnv::inherit(),
        None,
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

// A retry reuses the log file of the previous attempt, and the file mode given at creation does
// not apply to a file that exists: the run has to make it private itself.
#[tokio::test]
async fn a_reused_log_file_is_made_private_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(dir.path(), serde_json::json!([{"out": "hello"}]));
    let log = dir.path().join("3.jsonl");
    std::fs::write(&log, "old attempt\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&log).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "log mode is {mode:o}");
    }
    // Windows has no mode to tighten: the file keeps the access of the per-user directory.
    assert_eq!(log_lines(&log), ["hello"], "the old attempt is gone");
}

// One output line without an end must not use up the host's memory: the first 4 MiB of it are
// kept, the rest is read and dropped, a note in the log says so, and the lines after it still
// arrive, the final result included.
#[tokio::test]
async fn a_runaway_output_line_is_cut_and_the_run_goes_on() {
    const LIMIT: usize = 4 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": "x".repeat(LIMIT + 1000)}, {"out": RESULT_LINE}, {"exit": 0}]),
    );
    let log = dir.path().join("l.jsonl");
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &log,
        limits(60_000, 60_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(out.final_text.as_deref(), Some(RESULT_TEXT));
    let lines = log_lines(&log);
    assert_eq!(lines.len(), 3, "the long line, the note, the result");
    assert_eq!(lines[0].len(), LIMIT);
    let note: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(note["stream"], "stderr");
    // The whole line was read: its text and the newline.
    assert_eq!(
        note["text"],
        format!(
            "[IssueRelay] a {}-byte output line was cut to {LIMIT} bytes",
            LIMIT + 1001
        )
    );
    assert_eq!(lines[2], RESULT_LINE);
}

/// Seconds since the epoch on the wall clock, the clock process start times are kept in.
#[cfg(target_os = "macos")]
fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// On macOS the run reports its executor's PID and start time right after the start: the record a
// host started after a crash uses to end what the run left behind.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_run_reports_its_executor_process_when_it_starts() {
    let dir = tempfile::tempdir().unwrap();
    // The executor stays a moment, so that it surely still runs when the hook looks at it.
    let s = script(
        dir.path(),
        serde_json::json!([{"out": "hi"}, {"sleep_ms": 300}]),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let hook: runner::SpawnHook = Box::new(move |tag| {
        let live = runner::process_tag(tag.pid);
        tx.send((tag, live)).unwrap();
    });
    let before = unix_seconds();
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("1.jsonl"),
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        Some(hook),
    )
    .await;
    let after = unix_seconds();
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    let (tag, live) = rx.try_recv().expect("the hook ran");
    assert!(tag.pid > 1);
    assert_eq!(live.as_ref(), Some(&tag), "the PID had another start time");
    let (secs, micros) = tag.started.split_once('.').expect("seconds.microseconds");
    let secs: u64 = secs.parse().unwrap();
    assert!(
        (before.saturating_sub(1)..=after + 1).contains(&secs),
        "{} is not between {before} and {after}",
        tag.started
    );
    assert_eq!(micros.len(), 6, "{}", tag.started);
    assert!(
        micros.bytes().all(|b| b.is_ascii_digit()),
        "{}",
        tag.started
    );
}

// Elsewhere nothing is recorded and the sweep has nothing to end: on Windows the Job Object ends
// the whole tree with the host that created it, and other Unix systems are not target platforms.
#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn no_executor_process_is_recorded_where_the_tree_ends_with_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": "hi"}, {"sleep_ms": 300}]),
    );
    let (tx, rx) = std::sync::mpsc::channel::<ProcessTag>();
    let hook: runner::SpawnHook = Box::new(move |tag| tx.send(tag).unwrap());
    let out = runner::run(
        spec(agent(), dir.path(), &s, ""),
        ExecutorKind::Claude,
        &dir.path().join("1.jsonl"),
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        Some(hook),
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert!(rx.try_recv().is_err(), "a process was recorded");
    assert_eq!(runner::process_tag(std::process::id()), None);
    let tag = ProcessTag {
        pid: u32::MAX - 7,
        started: "1.000001".into(),
    };
    assert!(!runner::end_leftover(&tag).await);
}

/// Starts `steps` as an executor that a crashed host left behind (see `Orphan`) and returns once
/// every file in `beats` has content, that is once its heartbeat children run.
#[cfg(target_os = "macos")]
async fn leftover(dir: &Path, steps: serde_json::Value, beats: &[&Path]) -> Orphan {
    let orphan = Orphan::spawn(&script(dir, steps));
    for beat in beats {
        assert!(
            wait_for_content(beat).await,
            "{} never got a heartbeat",
            beat.display()
        );
    }
    orphan
}

// A host started after a crash ends the executor a run left behind the way `TreeGuard` ends a
// stopped run: its process group, and its descendants that left the group.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_leftover_executor_tree_is_ended_on_the_next_start() {
    use std::os::unix::process::ExitStatusExt;

    let dir = tempfile::tempdir().unwrap();
    let (beat, escaped) = (dir.path().join("beat"), dir.path().join("escaped"));
    let steps = serde_json::json!([
        {"heartbeat": beat, "every_ms": 50},
        {"escaped_heartbeat": escaped, "every_ms": 50},
        {"sleep_ms": 30_000}
    ]);
    let mut orphan = leftover(dir.path(), steps, &[&beat, &escaped]).await;
    let tag = runner::process_tag(orphan.id()).expect("a live process has a start time");
    assert!(runner::end_leftover(&tag).await);
    // Checked before the leader is reaped, so that the guard can still end the group if a
    // heartbeat survived.
    tokio::join!(
        assert_stops_growing(
            &beat,
            "the heartbeat child in the executor's group survived"
        ),
        assert_stops_growing(&escaped, "the heartbeat child that left the group survived"),
    );
    let status = orphan
        .exit_within(Duration::from_secs(10))
        .await
        .expect("the leftover executor still runs");
    // fake-agent does not ignore SIGTERM, so it ended before the grace period was over.
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}

// A PID whose start time differs from the record belongs to an unrelated process now: it and its
// children are left alone, also when only the microseconds differ. A PID that cannot exist is not
// an error either.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_pid_with_another_start_time_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let steps = serde_json::json!([{"heartbeat": beat, "every_ms": 50}, {"sleep_ms": 30_000}]);
    let mut orphan = leftover(dir.path(), steps, &[&beat]).await;
    let real = runner::process_tag(orphan.id()).unwrap();
    let (secs, micros) = real.started.split_once('.').unwrap();
    let next_micro = (micros.parse::<u32>().unwrap() + 1) % 1_000_000;
    for started in ["1.000001".to_string(), format!("{secs}.{next_micro:06}")] {
        let tag = ProcessTag {
            pid: real.pid,
            started,
        };
        assert!(!runner::end_leftover(&tag).await, "{tag:?}");
    }
    let before = file_len(&beat);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        orphan.is_running(),
        "an unrelated process must keep running"
    );
    assert!(
        file_len(&beat) > before,
        "the child of an unrelated process must keep running"
    );
    for pid in [u32::MAX - 7, 99_999_999] {
        let gone = ProcessTag {
            pid,
            started: "1.000001".into(),
        };
        assert!(!runner::end_leftover(&gone).await, "{pid}");
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
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), prompt);
}

/// The first line of a Claude run whose lean configuration did not hold: a second MCP server.
const TWO_SERVER_INIT: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"linear-server","status":"connected"},{"name":"demo-tools","status":"connected"}]}"#;

#[tokio::test]
async fn a_lean_violation_ends_the_run_like_a_stop() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": TWO_SERVER_INIT}, {"sleep_ms": 30_000}, {"exit": 0}]),
    );
    let log = dir.path().join("logs").join("1.jsonl");
    let started = Instant::now();
    let out = runner::run(
        CommandSpec {
            lean: true,
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Claude,
        &log,
        limits(60_000, 60_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    match &out.exit {
        ExitKind::LeanNotApplied(detail) => assert!(detail.contains("demo-tools"), "{detail}"),
        other => panic!("expected a lean violation, got {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the run was stopped, not waited for: {:?}",
        started.elapsed()
    );
    // The line that showed it, then IssueRelay's own line saying what happens.
    let lines = log_lines(&log);
    let at = lines.iter().position(|l| l.contains("demo-tools"));
    let said = at
        .and_then(|at| runner_note(lines.get(at + 1)?))
        .unwrap_or_default();
    assert!(
        said.starts_with("lean run did not take effect: ")
            && said.contains("demo-tools")
            && said.ends_with("; stopping the run"),
        "{lines:?}"
    );
}

// A lean stop ends the tree as a cancel does, including a grandchild that left the run's process
// group (Linux, not a target platform, skips it as above). The violation comes only once that
// grandchild is running.
#[cfg(any(target_os = "macos", windows))]
#[tokio::test]
async fn an_escaped_grandchild_is_ended_when_lean_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let hb = dir.path().join("hb.txt");
    let s = script(
        dir.path(),
        serde_json::json!([
            {"escaped_heartbeat": hb, "every_ms": 50},
            {"wait_for": hb},
            {"out": TWO_SERVER_INIT},
            {"sleep_ms": 30_000}
        ]),
    );
    let out = runner::run(
        CommandSpec {
            lean: true,
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Claude,
        &dir.path().join("l.jsonl"),
        limits(60_000, 60_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert!(
        matches!(out.exit, ExitKind::LeanNotApplied(_)),
        "{:?}",
        out.exit
    );
    assert!(file_len(&hb) > 0, "heartbeat child never ran");
    assert_stops_growing(&hb, "the escaped grandchild survived the lean stop").await;
}

/// A Codex event that names no MCP server.
const THREAD_STARTED: &str =
    r#"{"type":"thread.started","thread_id":"00000000-0000-0000-0000-000000000000"}"#;

/// A Codex event of a call to Linear's MCP server, as a lean run makes.
const LINEAR_CALL: &str = r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","server":"linear","tool":"get_issue"}}"#;

/// Runs the fake agent with `steps` as a Codex run, lean or not, and returns how it ended and
/// the lines of its log.
async fn codex_run(dir: &Path, lean: bool, steps: serde_json::Value) -> (ExitKind, Vec<String>) {
    let s = script(dir, steps);
    let log = dir.join("l.jsonl");
    let out = runner::run(
        CommandSpec {
            lean,
            ..spec(agent(), dir, &s, "p")
        },
        ExecutorKind::Codex,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    (out.exit, log_lines(&log))
}

// A lean Codex run that ends cleanly without one call to Linear the check recognized may have
// used an event format the check cannot read, so its lean configuration is not confirmed.
#[tokio::test]
async fn a_clean_lean_codex_run_without_a_linear_call_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (exit, lines) = codex_run(
        dir.path(),
        true,
        serde_json::json!([{"out": THREAD_STARTED}, {"exit": 0}]),
    )
    .await;
    match &exit {
        ExitKind::LeanNotApplied(detail) => assert!(detail.contains("`linear`"), "{detail}"),
        other => panic!("expected a lean violation, got {other:?}"),
    }
    assert_eq!(lines[0], THREAD_STARTED);
    let said = lines
        .get(1)
        .and_then(|line| runner_note(line))
        .unwrap_or_default();
    assert!(
        said.starts_with("lean run did not take effect: ")
            && said.contains("`linear`")
            && said.ends_with("; the executor had already exited"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_clean_lean_codex_run_that_called_linear_passes() {
    let dir = tempfile::tempdir().unwrap();
    let (exit, lines) = codex_run(
        dir.path(),
        true,
        serde_json::json!([{"out": LINEAR_CALL}, {"exit": 0}]),
    )
    .await;
    assert_eq!(exit, ExitKind::Exited(Some(0)));
    assert_eq!(lines, [LINEAR_CALL]);
}

// A run that failed keeps its exit code: that it made no call to Linear then says nothing about
// lean (a Codex that stops at its usage limit makes none).
#[tokio::test]
async fn a_failed_lean_codex_run_keeps_its_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let (exit, lines) = codex_run(
        dir.path(),
        true,
        serde_json::json!([{"out": THREAD_STARTED}, {"exit": 1}]),
    )
    .await;
    assert_eq!(exit, ExitKind::Exited(Some(1)));
    assert_eq!(lines, [THREAD_STARTED]);
}

#[tokio::test]
async fn a_codex_run_that_is_not_lean_needs_no_linear_call() {
    let dir = tempfile::tempdir().unwrap();
    let (exit, _) = codex_run(
        dir.path(),
        false,
        serde_json::json!([{"out": THREAD_STARTED}, {"exit": 0}]),
    )
    .await;
    assert_eq!(exit, ExitKind::Exited(Some(0)));
}

/// A Codex event of a call to an MCP server other than Linear's. Only the Unix test below uses it.
#[cfg(unix)]
const OTHER_SERVER_CALL: &str = r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","server":"demo-tools","tool":"search"}}"#;

// The executor exits first and its last line comes after that, from a process that left its
// process group: the lines still read once the executor has exited are checked too. The line comes
// about a second after the exit, a second before the drain ends. (Windows: right after the exit
// the runner ends the Job Object and that process with it, so this case cannot be made there; a
// line written before the exit and read after it is checked by the same code.)
#[cfg(unix)]
#[tokio::test]
async fn a_violation_read_after_the_executor_exited_still_fails_a_lean_run() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"late_out": OTHER_SERVER_CALL, "after_ms": 1000}, {"exit": 0}]),
    );
    let log = dir.path().join("l.jsonl");
    let out = runner::run(
        CommandSpec {
            lean: true,
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Codex,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    match &out.exit {
        ExitKind::LeanNotApplied(detail) => assert!(detail.contains("demo-tools"), "{detail}"),
        other => panic!("expected a lean violation, got {other:?}"),
    }
    let lines = log_lines(&log);
    assert!(lines[0].contains("demo-tools"), "{lines:?}");
    let said = lines
        .get(1)
        .and_then(|line| runner_note(line))
        .unwrap_or_default();
    assert!(
        said.starts_with("lean run did not take effect: ")
            && said.contains("demo-tools")
            && said.ends_with("; the executor had already exited"),
        "{lines:?}"
    );
}

// How a clean lean run often ends: the result comes once the executor has exited, so the drain
// reads it. The drain goes on with the check that saw the init event, so the result is no
// violation (a check of its own would take it for an event before the init event). The pause
// lets the loop read the init event before the exit.
#[cfg(unix)]
#[tokio::test]
async fn a_clean_lean_run_whose_result_comes_after_the_exit_passes() {
    let dir = tempfile::tempdir().unwrap();
    let init = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"linear-server","status":"connected"}]}"#;
    let s = script(
        dir.path(),
        serde_json::json!([
            {"out": init},
            {"sleep_ms": 200},
            {"late_out": RESULT_LINE, "after_ms": 1000},
            {"exit": 0}
        ]),
    );
    let log = dir.path().join("l.jsonl");
    let out = runner::run(
        CommandSpec {
            lean: true,
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    assert_eq!(out.final_text.as_deref(), Some(RESULT_TEXT));
    let lines = log_lines(&log);
    assert_eq!(lines, [init, RESULT_LINE]);
}

// Once the check has stopped a run, the lines drained after that are logged, not checked: the run
// keeps the reason of the violation that stopped it, and one note. Both calls come in one write,
// so the second is in the pipe before the run is stopped.
#[tokio::test]
async fn the_violation_that_stopped_a_lean_run_is_the_one_it_keeps() {
    let dir = tempfile::tempdir().unwrap();
    let call = |server: &str| {
        format!(
            r#"{{"type":"item.started","item":{{"id":"item_1","type":"mcp_tool_call","server":"{server}","tool":"search"}}}}"#
        )
    };
    let s = script(
        dir.path(),
        serde_json::json!([
            {"out": format!("{}\n{}", call("demo-a"), call("demo-b"))},
            {"sleep_ms": 30_000}
        ]),
    );
    let log = dir.path().join("l.jsonl");
    let out = runner::run(
        CommandSpec {
            lean: true,
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Codex,
        &log,
        limits(60_000, 60_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    match &out.exit {
        ExitKind::LeanNotApplied(detail) => {
            assert!(
                detail.contains("demo-a") && !detail.contains("demo-b"),
                "{detail}"
            )
        }
        other => panic!("expected a lean violation, got {other:?}"),
    }
    let lines = log_lines(&log);
    assert!(lines.iter().any(|l| l.contains("demo-b")), "{lines:?}");
    let notes: Vec<String> = lines.iter().filter_map(|l| runner_note(l)).collect();
    assert_eq!(notes.len(), 1, "{lines:?}");
    assert!(notes[0].ends_with("; stopping the run"), "{notes:?}");
}

#[tokio::test]
async fn a_full_run_is_not_checked() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": TWO_SERVER_INIT}, {"sleep_ms": 50}, {"exit": 0}]),
    );
    let log = dir.path().join("logs").join("1.jsonl");
    let out = runner::run(
        spec(agent(), dir.path(), &s, "p"),
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
}

#[tokio::test]
async fn notes_come_first_in_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"out": RESULT_LINE}, {"exit": 0}]),
    );
    let log = dir.path().join("logs").join("1.jsonl");
    let out = runner::run(
        CommandSpec {
            notes: vec!["demo note".into()],
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &ExecEnv::inherit(),
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    let lines = log_lines(&log);
    // IssueRelay's own line, which the panel labels as such, not the executor's stderr.
    assert_eq!(
        runner_note(&lines[0]).as_deref(),
        Some("demo note"),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
}

#[tokio::test]
async fn the_runs_own_variables_reach_the_executor() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([{"print_env": "ISSUERELAY_DEMO_VAR"}, {"exit": 0}]),
    );
    for (env, expected) in [
        (
            vec![("ISSUERELAY_DEMO_VAR".to_string(), "1".to_string())],
            "ISSUERELAY_DEMO_VAR=1",
        ),
        (Vec::new(), "ISSUERELAY_DEMO_VAR=<unset>"),
    ] {
        let log = dir.path().join("logs").join(format!("{}.jsonl", env.len()));
        let out = runner::run(
            CommandSpec {
                env,
                ..spec(agent(), dir.path(), &s, "p")
            },
            ExecutorKind::Claude,
            &log,
            limits(30_000, 30_000),
            CancellationToken::new(),
            &ExecEnv::inherit(),
            None,
        )
        .await;
        assert_eq!(out.exit, ExitKind::Exited(Some(0)));
        let lines = log_lines(&log);
        assert!(lines.iter().any(|l| l == expected), "{lines:?}");
    }
}

// The run's own variables are set last: they win over a variable the executor environment sets
// (a proxy here) and over one it removes (the nested-session marker).
#[tokio::test]
async fn the_runs_own_variables_win_over_the_executor_environment() {
    let dir = tempfile::tempdir().unwrap();
    let s = script(
        dir.path(),
        serde_json::json!([
            {"print_env": "ISSUERELAY_TEST_PROXY"},
            {"print_env": "CLAUDECODE"},
            {"exit": 0}
        ]),
    );
    let log = dir.path().join("logs").join("1.jsonl");
    let env = ExecEnv {
        proxy: vec![("ISSUERELAY_TEST_PROXY".into(), "http://127.0.0.1:9".into())],
        ..ExecEnv::inherit()
    };
    let out = runner::run(
        CommandSpec {
            env: vec![
                ("ISSUERELAY_TEST_PROXY".into(), "http://127.0.0.1:8".into()),
                ("CLAUDECODE".into(), "demo".into()),
            ],
            ..spec(agent(), dir.path(), &s, "p")
        },
        ExecutorKind::Claude,
        &log,
        limits(30_000, 30_000),
        CancellationToken::new(),
        &env,
        None,
    )
    .await;
    assert_eq!(out.exit, ExitKind::Exited(Some(0)));
    let lines = log_lines(&log);
    for expected in [
        "ISSUERELAY_TEST_PROXY=http://127.0.0.1:8",
        "CLAUDECODE=demo",
    ] {
        assert!(lines.iter().any(|l| l == expected), "{expected}: {lines:?}");
    }
}
