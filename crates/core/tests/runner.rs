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
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
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
