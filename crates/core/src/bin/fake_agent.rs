//! Test double for `claude` / `codex`, driven by a JSON script. Only used by tests.
//!
//! Usage: `fake-agent <script.json>`. The script is an array of steps run in order; the exit code
//! is 0 unless an `exit` step says otherwise. Each step holds one of:
//!
//! - `{"out": text}` / `{"err": text}`: print a line to stdout / stderr
//! - `{"print_env": name}`: print `name=<value>` (or `name=<unset>`) to stdout
//! - `{"sleep_ms": n}`: sleep
//! - `{"stdin_to": path}`: read all of stdin into a file
//! - `{"heartbeat": path, "every_ms": n}`: start a background child that appends a byte to the
//!   file every `n` ms (it stops by itself after 120 s)
//! - `{"escaped_heartbeat": path, "every_ms": n}`: the same, but the child leaves the run's
//!   process group the way Claude Code's detached tool shells do (Unix: a process group of its
//!   own; Windows: a detached process in a new process group, still inside the run's Job Object)
//! - `{"wait_for": path}`: wait until the file has content, for at most 10 s (a heartbeat's file:
//!   the next step then comes once its child is running)
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
        if let Some(name) = step.get("print_env").and_then(Value::as_str) {
            let value = std::env::var(name).unwrap_or_else(|_| "<unset>".into());
            println!("{name}={value}");
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
            heartbeat_command(path, every)
                .spawn()
                .expect("spawn heartbeat");
        }
        if let Some(path) = step.get("escaped_heartbeat").and_then(Value::as_str) {
            let every = step.get("every_ms").and_then(Value::as_u64).unwrap_or(100);
            let mut command = heartbeat_command(path, every);
            leave_the_process_group(&mut command);
            // Deliberately left running, like the heartbeat above.
            #[allow(clippy::zombie_processes)]
            command.spawn().expect("spawn escaped heartbeat");
        }
        if let Some(path) = step.get("wait_for").and_then(Value::as_str) {
            let give_up = Instant::now() + Duration::from_secs(10);
            while std::fs::metadata(path).map_or(0, |m| m.len()) == 0 && Instant::now() < give_up {
                std::thread::sleep(Duration::from_millis(10));
            }
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

/// This program again, as a heartbeat child writing to `file` every `every_ms` ms. On Windows the
/// child inherits this process's inheritable handles, including the runner's pipes, so readers
/// must not wait for EOF while a heartbeat is alive.
fn heartbeat_command(file: &str, every_ms: u64) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .args(["--heartbeat", file, &every_ms.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// Unix: the child leads a process group of its own, so signalling the run's process group no
/// longer reaches it (Claude Code starts its tool shells with `setsid`, which has the same effect).
#[cfg(unix)]
fn leave_the_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// Windows: a detached process in a new process group. It stays in the run's Job Object anyway:
/// the job does not allow breakaway.
#[cfg(windows)]
fn leave_the_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    // Process creation flags from the Win32 API (winbase.h).
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
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
