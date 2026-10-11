//! Runs one executor process: feeds the prompt on stdin, streams stdout/stderr into a JSONL
//! log, enforces the total and no-output limits, and always kills the whole process tree
//! (process group on Unix, Job Object on Windows) when the run ends.
use std::{path::Path, process::Stdio, time::Duration};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
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

/// Owns the spawned child and ends its whole process tree if `run` is dropped before its own
/// cleanup has finished: the task was aborted, an outer timeout fired, the runtime is shutting
/// down, a panic unwound, or an early return was taken. Without it Unix would leave the
/// grandchildren running, because process-wrap's `KillOnDrop` only signals the leader there and
/// `ProcessGroupChild` has no `Drop`. On Windows closing the job handle already ends the tree
/// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`); the guard makes both platforms behave alike.
///
/// `done` is set once `end` has killed and reaped the tree. After that the guard must not signal
/// again: the process group id of a reaped tree may already belong to someone else.
struct TreeGuard {
    child: Box<dyn ChildWrapper>,
    done: bool,
}

impl TreeGuard {
    fn new(child: Box<dyn ChildWrapper>) -> Self {
        Self { child, done: false }
    }

    /// Kills whatever is left of the tree (also after a normal exit) and waits, for at most ten
    /// seconds, until it is reaped.
    async fn end(&mut self) {
        let _ = self.child.start_kill();
        if tokio::time::timeout(Duration::from_secs(10), self.child.wait())
            .await
            .is_ok()
        {
            self.done = true;
        }
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.child.start_kill();
        }
    }
}

fn failed(message: String) -> RunOutput {
    RunOutput {
        exit: ExitKind::SpawnFailed(message),
        final_text: None,
    }
}

/// Ends a run that could not start. The log gets a line saying why, so it explains itself.
async fn spawn_failed(log: &mut File, message: String) -> RunOutput {
    let entry = note("runner", &format!("spawn failed: {message}"));
    let _ = log.write_all(entry.as_bytes()).await;
    let _ = log.flush().await;
    failed(message)
}

/// Runs `spec` until it exits, a limit is hit or `cancel` fires. The JSONL file at `log_path` is
/// created (or truncated) and gets one line per output line: stdout verbatim, stderr as
/// `{"stream":"stderr","text":…}`; a run that cannot start gets one
/// `{"stream":"runner","text":"spawn failed: …"}` line. Once the executor has started, the whole
/// process tree is killed however the run ends (before this returns), and also if this future is
/// dropped before it finishes. Descendants that leave the process group on Unix (`setsid`,
/// `setpgid`) are not reached.
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
    let mut log = match create_log(log_path).await {
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
    // From here on the guard owns the tree: every way out of this function ends it.
    let mut guard = match command.spawn() {
        Ok(child) => TreeGuard::new(child),
        Err(e) => return spawn_failed(&mut log, e.to_string()).await,
    };

    if let Some(mut stdin) = guard.child.stdin().take() {
        let prompt = spec.stdin;
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let (tx, mut rx) = mpsc::unbounded_channel();
    if let Some(stdout) = guard.child.stdout().take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            for_each_line(BufReader::new(stdout), |line| {
                tx.send(Line::Out(line)).is_ok()
            })
            .await;
        });
    }
    if let Some(stderr) = guard.child.stderr().take() {
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
    let started = Instant::now();
    let deadline = checked_deadline(started, limits.total);
    let mut last_output = started;
    let mut pipes_open = true;
    let exit = {
        // Wait on the executor process itself, not through the process-wrap wrapper. On Windows
        // the wrapper's wait() watches the job's completion port and may return on the first
        // message instead of reliably waiting for the whole job, so what it does while a
        // background process lives is unpredictable. The native child's exit is the signal we
        // want; killing and reaping the rest of the tree still goes through the wrapper below.
        // SAFETY: we only call wait() on the native child, so no invariant of a wrapper in the
        // chain is bypassed; the guard still kills and reaps through the wrapper.
        let Some(leader) = (unsafe { guard.child.try_inner_child_mut() }) else {
            return spawn_failed(&mut log, "process wrapper hid the native child".into()).await;
        };
        loop {
            tokio::select! {
                status = leader.wait() => break ExitKind::Exited(status.ok().and_then(|s| s.code())),
                line = rx.recv(), if pipes_open => match line {
                    Some(line) => {
                        last_output = Instant::now();
                        record(&mut log, &mut tracker, line).await;
                    }
                    None => pipes_open = false,
                },
                _ = sleep_until_opt(deadline) => break ExitKind::TimedOut,
                _ = sleep_until_opt(checked_deadline(last_output, limits.stall)) => break ExitKind::Stalled,
                _ = cancel.cancelled() => break ExitKind::Canceled,
            }
        }
    };

    // Kill whatever is left of the tree (also after a normal exit), then reap.
    guard.end().await;
    // Keep lines already buffered; the final result often arrives right before exit.
    let drain_until = Instant::now() + Duration::from_secs(2);
    while let Ok(Some(line)) = tokio::time::timeout_at(drain_until, rx.recv()).await {
        record(&mut log, &mut tracker, line).await;
    }
    let _ = log.flush().await;
    RunOutput {
        exit,
        final_text: tracker.into_final_text(),
    }
}

/// `from + span`, or `None` (no deadline) when the clock cannot hold that point in time. The
/// timer rounds a deadline up by almost a millisecond, so a point too close to the end of the
/// clock's range counts as out of range too: adding that rounding would overflow inside tokio.
fn checked_deadline(from: Instant, span: Duration) -> Option<Instant> {
    let at = from.checked_add(span)?;
    at.checked_add(Duration::from_secs(1))?;
    Some(at)
}

/// Completes at `at`; never completes when there is no deadline.
async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Creates (or truncates) the log file. It can hold anything the executor printed, so it is
/// private to the user.
async fn create_log(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    make_private(&mut options);
    tokio::fs::OpenOptions::from(options).open(path).await
}

/// Unix: only the owner may read or write the new file.
#[cfg(unix)]
fn make_private(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}

/// Windows: nothing to set. The log lives in the per-user local app data directory, whose default
/// access already limits it to the user.
#[cfg(windows)]
fn make_private(_options: &mut std::fs::OpenOptions) {}

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

/// One log line (with its newline) for text that is not the executor's stdout:
/// `{"stream":…,"text":…}`.
fn note(stream: &str, text: &str) -> String {
    let mut entry = serde_json::json!({"stream": stream, "text": text}).to_string();
    entry.push('\n');
    entry
}

/// Writes one log line per call, in a single write so a live reader never sees half a line.
async fn record(log: &mut File, tracker: &mut FinalTextTracker, line: Line) {
    let entry = match line {
        Line::Out(mut text) => {
            tracker.observe(&text);
            text.push('\n');
            text
        }
        Line::Err(text) => note("stderr", &text),
    };
    let _ = log.write_all(entry.as_bytes()).await;
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        io,
        pin::Pin,
        process::ExitStatus,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tokio::io::BufReader;

    use super::*;

    /// A child that only counts how often its process tree is signalled.
    #[derive(Debug)]
    struct CountingChild {
        kills: Arc<AtomicUsize>,
    }

    impl ChildWrapper for CountingChild {
        fn inner(&self) -> &dyn ChildWrapper {
            self
        }

        fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
            self
        }

        fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
            self
        }

        fn start_kill(&mut self) -> io::Result<()> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
            Box::pin(async { Err(io::Error::other("already gone")) })
        }
    }

    fn counting_guard() -> (TreeGuard, Arc<AtomicUsize>) {
        let kills = Arc::new(AtomicUsize::new(0));
        let child = CountingChild {
            kills: kills.clone(),
        };
        (TreeGuard::new(Box::new(child)), kills)
    }

    #[test]
    fn dropping_the_guard_early_ends_the_tree() {
        let (guard, kills) = counting_guard();
        drop(guard);
        assert_eq!(kills.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_reaped_tree_is_not_signalled_again_on_drop() {
        let (mut guard, kills) = counting_guard();
        guard.end().await;
        assert_eq!(kills.load(Ordering::SeqCst), 1);
        drop(guard);
        assert_eq!(
            kills.load(Ordering::SeqCst),
            1,
            "a reaped process group id was signalled again"
        );
    }

    #[test]
    fn a_deadline_the_clock_cannot_hold_is_no_deadline() {
        let now = Instant::now();
        assert_eq!(checked_deadline(now, Duration::MAX), None);
        assert_eq!(
            checked_deadline(now, Duration::from_secs(60)),
            Some(now + Duration::from_secs(60))
        );
    }

    #[test]
    fn a_deadline_at_the_very_edge_of_the_clock_is_no_deadline() {
        // The timer rounds a deadline up by almost a millisecond; that addition must not overflow.
        let now = Instant::now();
        let (mut low, mut high) = (0_u64, u64::MAX);
        while low < high {
            let mid = low + (high - low).div_ceil(2);
            if now.checked_add(Duration::from_secs(mid)).is_some() {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let farthest = Duration::from_secs(low);
        assert!(now.checked_add(farthest).is_some());
        assert_eq!(checked_deadline(now, farthest), None);
    }

    #[tokio::test]
    async fn waiting_without_a_deadline_never_ends() {
        let waited = tokio::time::timeout(Duration::from_millis(50), sleep_until_opt(None)).await;
        assert!(waited.is_err());
        sleep_until_opt(Some(Instant::now())).await;
    }

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
