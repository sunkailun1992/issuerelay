//! Runs one executor process: feeds the prompt on stdin, streams stdout/stderr into a JSONL
//! log, enforces the total and no-output limits, and always kills the whole process tree
//! (process group on Unix, Job Object on Windows) when the run ends.
use std::{path::Path, process::Stdio, time::Duration};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use tokio::{
    fs::File,
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::config::ExecutorKind;
use crate::executor::{CommandSpec, outcome::FinalTextTracker};

/// The time limits of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    /// The whole run, however chatty the executor is.
    pub total: Duration,
    /// The longest silence, on stdout and stderr together, before the run counts as stuck.
    pub stall: Duration,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitKind {
    /// The executor exited on its own; `None` when a signal ended it.
    Exited(Option<i32>),
    /// The `total` limit was reached.
    TimedOut,
    /// Nothing was printed for the `stall` limit.
    Stalled,
    /// The caller cancelled the run.
    Canceled,
    /// The log file could not be created or the executor could not be started.
    SpawnFailed(String),
}

/// What one run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub exit: ExitKind,
    /// The executor's final message, when it printed one.
    pub final_text: Option<String>,
}

enum Line {
    Out(String),
    Err(String),
}

fn failed(message: String) -> RunOutput {
    RunOutput {
        exit: ExitKind::SpawnFailed(message),
        final_text: None,
    }
}

/// Runs `spec` until it exits, a limit is hit or `cancel` fires. The JSONL file at `log_path` is
/// created (or truncated) and gets one line per output line: stdout verbatim, stderr as
/// `{"stream":"stderr","text":…}`. Once the executor has started, the whole process tree is killed
/// before this returns, however the run ends.
pub async fn run(
    spec: CommandSpec,
    kind: ExecutorKind,
    log_path: &Path,
    limits: RunLimits,
    cancel: CancellationToken,
) -> RunOutput {
    if let Some(dir) = log_path.parent()
        && let Err(e) = tokio::fs::create_dir_all(dir).await
    {
        return failed(format!("cannot create log directory: {e}"));
    }
    let mut log = match File::create(log_path).await {
        Ok(file) => file,
        Err(e) => return failed(format!("cannot create log file: {e}")),
    };

    let mut command = CommandWrap::with_new(&spec.program, |c| {
        c.args(&spec.args)
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    command.wrap(KillOnDrop);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return failed(e.to_string()),
    };

    if let Some(mut stdin) = child.stdin().take() {
        let prompt = spec.stdin.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(stdout) = child.stdout().take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            for_each_line(BufReader::new(stdout), |line| {
                tx.send(Line::Out(line)).is_ok()
            })
            .await;
        });
    }
    if let Some(stderr) = child.stderr().take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            for_each_line(BufReader::new(stderr), |line| {
                tx.send(Line::Err(line)).is_ok()
            })
            .await;
        });
    }
    drop(tx);

    let mut tracker = FinalTextTracker::new(kind);
    let deadline = Instant::now() + limits.total;
    let mut last_output = Instant::now();
    let mut pipes_open = true;
    let exit = {
        // Wait on the executor process itself. process-wrap's own wait() also waits for every
        // process left in a Windows job, which would turn a normal exit into a stall whenever
        // the AI leaves a background process running.
        // SAFETY: we only call wait() on the native child; the wrappers' kill and reap logic
        // still runs below through `child`.
        let Some(leader) = (unsafe { child.try_inner_child_mut() }) else {
            return failed("process wrapper hid the native child".into());
        };
        loop {
            tokio::select! {
                status = leader.wait() => break ExitKind::Exited(status.ok().and_then(|s| s.code())),
                line = rx.recv(), if pipes_open => match line {
                    Some(line) => {
                        last_output = Instant::now();
                        record(&mut log, &mut tracker, &line).await;
                    }
                    None => pipes_open = false,
                },
                _ = tokio::time::sleep_until(deadline) => break ExitKind::TimedOut,
                _ = tokio::time::sleep_until(last_output + limits.stall) => break ExitKind::Stalled,
                _ = cancel.cancelled() => break ExitKind::Canceled,
            }
        }
    };

    // Kill whatever is left of the tree (also after a normal exit), then reap.
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
    // Keep lines already buffered; the final result often arrives right before exit.
    let drain_until = Instant::now() + Duration::from_secs(2);
    while let Ok(Some(line)) = tokio::time::timeout_at(drain_until, rx.recv()).await {
        record(&mut log, &mut tracker, &line).await;
    }
    let _ = log.flush().await;
    RunOutput {
        exit,
        final_text: tracker.into_final_text(),
    }
}

/// Calls `f` for each line of `reader` until end of input, a read error, or `f` returning `false`
/// (the receiver is gone). Lines end at `\n` (a `\r` before it is dropped) and are decoded
/// lossily: one line of invalid UTF-8 must not end the stream, or every later event, including
/// the final result, would be lost and the executor would get a broken pipe.
async fn for_each_line<R: AsyncBufRead + Unpin>(mut reader: R, mut f: impl FnMut(String) -> bool) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if buf.ends_with(b"\n") {
            buf.pop();
            if buf.ends_with(b"\r") {
                buf.pop();
            }
        }
        if !f(String::from_utf8_lossy(&buf).into_owned()) {
            return;
        }
    }
}

async fn record(log: &mut File, tracker: &mut FinalTextTracker, line: &Line) {
    let text = match line {
        Line::Out(text) => {
            tracker.observe(text);
            text.clone()
        }
        Line::Err(text) => serde_json::json!({"stream": "stderr", "text": text}).to_string(),
    };
    let _ = log.write_all(text.as_bytes()).await;
    let _ = log.write_all(b"\n").await;
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    async fn collect(bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for_each_line(BufReader::new(bytes), |line| {
            lines.push(line);
            true
        })
        .await;
        lines
    }

    #[tokio::test]
    async fn invalid_utf8_does_not_end_the_stream() {
        let lines = collect(&b"ok\n\xff\xfe\r\nafter\n"[..]).await;
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "ok");
        assert!(lines[1].contains('\u{FFFD}'), "{:?}", lines[1]);
        assert!(!lines[1].contains('\r'), "{:?}", lines[1]);
        assert_eq!(lines[2], "after");
    }

    #[tokio::test]
    async fn line_endings_and_an_unterminated_last_line() {
        assert_eq!(collect(b"").await, Vec::<String>::new());
        assert_eq!(collect(b"a\nb").await, ["a", "b"]);
        assert_eq!(collect(b"\n\nx\r\n").await, ["", "", "x"]);
    }

    #[tokio::test]
    async fn reading_stops_when_the_callback_declines() {
        let mut seen = Vec::new();
        for_each_line(BufReader::new(&b"a\nb\nc\n"[..]), |line| {
            seen.push(line);
            seen.len() < 2
        })
        .await;
        assert_eq!(seen, ["a", "b"]);
    }
}
