//! Test double for `claude` / `codex`, driven by a JSON script. Only used by tests.
//!
//! Usage: `fake-agent <script.json>`. The script is an array of steps run in order; the exit code
//! is 0 unless an `exit` step says otherwise. Each step holds one of:
//!
//! - `{"out": text}` / `{"err": text}`: print a line to stdout / stderr
//! - `{"sleep_ms": n}`: sleep
//! - `{"stdin_to": path}`: read all of stdin into a file
//! - `{"heartbeat": path, "every_ms": n}`: start a background child that appends a byte to the
//!   file every `n` ms (it stops by itself after 120 s)
//! - `{"repeat_out": text, "every_ms": n}` / `{"repeat_err": text, "every_ms": n}`: print a line
//!   to stdout / stderr every `n` ms, forever
//! - `{"exit": code}`: exit at once
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde_json::Value;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--heartbeat") {
        let file = args
            .get(2)
            .expect("usage: fake-agent --heartbeat <file> <ms>");
        let every = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
        heartbeat(Path::new(file), every);
    }
    let script = args.get(1).expect("usage: fake-agent <script.json>");
    let text = std::fs::read_to_string(script).expect("read script");
    let steps: Vec<Value> = serde_json::from_str(&text).expect("parse script");
    let mut code = 0;
    for step in steps {
        if let Some(line) = step.get("out").and_then(Value::as_str) {
            println!("{line}");
            std::io::stdout().flush().ok();
        }
        if let Some(line) = step.get("err").and_then(Value::as_str) {
            eprintln!("{line}");
        }
        if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            std::thread::sleep(Duration::from_millis(ms));
        }
        if let Some(path) = step.get("stdin_to").and_then(Value::as_str) {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .expect("read stdin");
            std::fs::write(path, input).expect("write stdin copy");
        }
        if let Some(path) = step.get("heartbeat").and_then(Value::as_str) {
            let every = step.get("every_ms").and_then(Value::as_u64).unwrap_or(100);
            // On Windows the grandchild inherits this process's inheritable handles, including
            // the runner's pipes, so readers must not wait for EOF while a heartbeat is alive.
            // Deliberately left running: the runner tests check that ending the run also ends this grandchild.
            #[allow(clippy::zombie_processes)]
            Command::new(std::env::current_exe().expect("current exe"))
                .args(["--heartbeat", path, &every.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn heartbeat");
        }
        if let Some(line) = step.get("repeat_out").and_then(Value::as_str) {
            let every = step.get("every_ms").and_then(Value::as_u64).unwrap_or(100);
            loop {
                println!("{line}");
                std::io::stdout().flush().ok();
                std::thread::sleep(Duration::from_millis(every));
            }
        }
        if let Some(line) = step.get("repeat_err").and_then(Value::as_str) {
            let every = step.get("every_ms").and_then(Value::as_u64).unwrap_or(100);
            loop {
                eprintln!("{line}");
                std::thread::sleep(Duration::from_millis(every));
            }
        }
        if let Some(exit) = step.get("exit").and_then(Value::as_i64) {
            code = exit as i32;
            break;
        }
    }
    std::process::exit(code);
}

fn heartbeat(file: &Path, every_ms: u64) -> ! {
    // A stray heartbeat would otherwise keep running forever, and on Windows a running
    // fake-agent.exe blocks the next build from relinking. So it stops after 120 seconds,
    // and as soon as its file cannot be opened (for example once the test directory is gone).
    let cap = Duration::from_secs(120);
    let started = Instant::now();
    loop {
        if started.elapsed() >= cap {
            std::process::exit(0);
        }
        let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        else {
            std::process::exit(0);
        };
        let _ = f.write_all(b".");
        let remaining = cap.saturating_sub(started.elapsed());
        std::thread::sleep(Duration::from_millis(every_ms).min(remaining));
    }
}
