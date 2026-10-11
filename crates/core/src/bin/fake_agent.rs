//! Test double for `claude` / `codex`, driven by a JSON script. Only used by tests.
use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
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
        if let Some(exit) = step.get("exit").and_then(Value::as_i64) {
            code = exit as i32;
            break;
        }
    }
    std::process::exit(code);
}

fn heartbeat(file: &Path, every_ms: u64) -> ! {
    loop {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        {
            let _ = f.write_all(b".");
        }
        std::thread::sleep(Duration::from_millis(every_ms));
    }
}
