//! Runs one executor process: feeds the prompt on stdin, streams stdout/stderr into a JSONL
//! log, enforces the total and no-output limits, and always ends the whole process tree when the
//! run ends: on Unix the process group and every descendant, including ones that left the group;
//! on Windows the Job Object.
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
use crate::env::ExecEnv;
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

/// How long an executor that is being stopped (timeout, stall, cancel) may take to exit after
/// SIGTERM before the rest of its tree is killed. It lets Claude Code / Codex end their own
/// detached tool processes. Windows has no such step: see `TreeGuard`.
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(5);

/// Owns the spawned child and ends its whole process tree, however `run` ends.
///
/// - `end`, the explicit cleanup, runs after every run. On Unix, while the executor still runs
///   (the run was stopped by a limit or cancelled), it records the executor's descendants by
///   walking parent PIDs, each with its start time; sends SIGTERM to the process group and
///   gives the executor up to `TERM_GRACE` to exit; records the descendants again (adding to the
///   first record); then sends SIGKILL to the process group, and to every recorded descendant
///   whose PID still has the recorded start time, or to its whole process group when it leads
///   one (it may have started members after it was recorded). That reaches descendants that left
///   the group with `setsid` or `setpgid`, as Claude Code's tool shells do on macOS. After a
///   normal exit only the process group is killed: the executor is gone, so its descendants can
///   no longer be found through it. On Windows `end` terminates the Job Object at once; the job
///   does not allow breakaway, so it holds every descendant, and Windows has no SIGTERM to give
///   the executor first. On both platforms `end` then waits, for at most ten seconds, until the
///   tree is reaped.
/// - `Drop` covers a `run` future dropped before `end` finished: the task was aborted, an outer
///   timeout fired, the runtime is shutting down, a panic unwound. Immediately and synchronously,
///   without a grace period, it does on Unix what `end` does after the grace: it records the
///   descendants (before anything is killed: once the executor has died its children belong to
///   launchd and can no longer be found through it), sends SIGKILL to the process group, then to
///   the recorded descendants, including the ones an interrupted `end` had recorded. Without it
///   Unix would leave the grandchildren running, because process-wrap's `KillOnDrop` only
///   signals the leader there and `ProcessGroupChild` has no `Drop`. On Windows closing the job
///   handle already ends the tree (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`); the guard terminates
///   the job itself so both platforms behave alike.
///
/// The walk exists on macOS only; other Unix systems, which are not target platforms, record
/// nothing and end only the process group.
///
/// `done` is set once `end` has killed and reaped the tree. After that the guard must not signal
/// again: the process group id of a reaped tree may already belong to someone else.
struct TreeGuard {
    child: Box<dyn ChildWrapper>,
    done: bool,
    /// How long `end` waits for the executor after SIGTERM: `TERM_GRACE`, shorter in tests.
    /// (Unix only, like `recorded`: Windows terminates the Job Object at once.)
    #[cfg(unix)]
    grace: Duration,
    /// The descendants recorded so far, each with its start time.
    #[cfg(unix)]
    recorded: Vec<descendants::Proc>,
}

impl TreeGuard {
    fn new(child: Box<dyn ChildWrapper>) -> Self {
        Self {
            child,
            done: false,
            #[cfg(unix)]
            grace: TERM_GRACE,
            #[cfg(unix)]
            recorded: Vec::new(),
        }
    }

    /// The executor's PID while it has not been reaped. Once it has been waited for, tokio no
    /// longer reports it: the PID is free then and may belong to another process.
    #[cfg(unix)]
    fn leader(&self) -> Option<libc::pid_t> {
        self.child
            .id()
            .and_then(|pid| libc::pid_t::try_from(pid).ok())
    }

    /// Adds to `recorded` the descendants of the executor (while it has not been reaped) and of
    /// every process recorded before. Windows needs no such record: the Job Object holds every
    /// descendant.
    #[cfg(unix)]
    fn record_descendants(&mut self) {
        let mut roots: Vec<descendants::Proc> = self
            .leader()
            .and_then(descendants::process)
            .into_iter()
            .collect();
        roots.extend_from_slice(&self.recorded);
        descendants::record(&roots, &mut self.recorded);
    }

    /// Ends the tree as described on the type and waits, for at most ten seconds, until it is
    /// reaped.
    async fn end(&mut self) {
        #[cfg(unix)]
        {
            if self.leader().is_some() {
                self.record_descendants();
                let _ = self.child.signal(libc::SIGTERM);
                // SAFETY: as in `run`, only the native child is waited for; killing and reaping
                // the rest of the tree still goes through the wrapper below.
                if let Some(leader) = unsafe { self.child.try_inner_child_mut() } {
                    let _ = tokio::time::timeout(self.grace, leader.wait()).await;
                }
                self.record_descendants();
            }
            let _ = self.child.start_kill();
            descendants::kill(&self.recorded);
        }
        // Terminating the Job Object ends every process in it at once.
        #[cfg(windows)]
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
        if self.done {
            return;
        }
        // Unix: record first, while the executor is alive; once it has died its children are
        // launchd's and can no longer be found through it. (On Windows terminating the Job
        // Object below reaches every descendant.)
        #[cfg(unix)]
        self.record_descendants();
        let _ = self.child.start_kill();
        #[cfg(unix)]
        descendants::kill(&self.recorded);
    }
}

/// macOS: finds an executor's descendants by walking parent PIDs and ends them by PID. A PID is
/// reused once its process is gone, so every process is recorded with its start time, and is
/// only listed from or signalled while its PID still has that start time. (Windows has no
/// counterpart and needs none: the Job Object holds every descendant.)
#[cfg(target_os = "macos")]
mod descendants {
    use std::collections::{HashSet, VecDeque};

    /// How many generations below each starting point the walk looks.
    const MAX_DEPTH: usize = 64;
    /// The most processes recorded for one run.
    const MAX_RECORDED: usize = 4096;
    /// The most children read for one process.
    const MAX_CHILDREN: usize = 4096;
    /// What `proc_pidinfo` writes for `PROC_PIDTBSDINFO` (136 bytes).
    const INFO_SIZE: libc::c_int = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;

    /// A process as it was recorded.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub(super) struct Proc {
        pub(super) pid: libc::pid_t,
        /// The start time, in seconds and microseconds since the epoch.
        pub(super) started: (u64, u64),
    }

    /// What the kernel reports about a live process.
    struct Info {
        started: (u64, u64),
        ppid: libc::pid_t,
        pgid: libc::pid_t,
    }

    /// `None` when there is no such process, when it has exited (the kernel answers for a
    /// zombie as for a process that is gone) or when it cannot be inspected.
    fn info(pid: libc::pid_t) -> Option<Info> {
        // SAFETY: proc_bsdinfo holds only integers and byte arrays; all-zero bytes are a value.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a writable proc_bsdinfo of INFO_SIZE bytes, the layout that
        // PROC_PIDTBSDINFO fills; it is read only when the call filled all of it.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                std::ptr::from_mut(&mut info).cast(),
                INFO_SIZE,
            )
        };
        if written != INFO_SIZE {
            return None;
        }
        Some(Info {
            started: (info.pbi_start_tvsec, info.pbi_start_tvusec),
            ppid: libc::pid_t::try_from(info.pbi_ppid).ok()?,
            pgid: libc::pid_t::try_from(info.pbi_pgid).ok()?,
        })
    }

    /// The live process `pid` with its start time.
    pub(super) fn process(pid: libc::pid_t) -> Option<Proc> {
        info(pid).map(|info| Proc {
            pid,
            started: info.started,
        })
    }

    /// The children of `parent`, read into `buffer`; none once `parent` is gone.
    fn children(parent: Proc, buffer: &mut [libc::pid_t]) -> Vec<Proc> {
        let bytes =
            libc::c_int::try_from(std::mem::size_of_val(buffer)).unwrap_or(libc::c_int::MAX);
        // SAFETY: `buffer` is writable for `bytes` bytes; the call writes at most that many and
        // returns how many PIDs it wrote, or a negative number when it fails.
        let count =
            unsafe { libc::proc_listchildpids(parent.pid, buffer.as_mut_ptr().cast(), bytes) };
        let Ok(count) = usize::try_from(count) else {
            return Vec::new();
        };
        // The list is `parent`'s only if its PID still names the recorded process. Checked after
        // the listing: a PID is not reused while its process exists, so a match now means the
        // listing saw the right process.
        if process(parent.pid) != Some(parent) {
            return Vec::new();
        }
        buffer[..count.min(buffer.len())]
            .iter()
            .filter_map(|&pid| {
                let child = info(pid)?;
                // A child that exited after the listing may have passed its PID on already.
                (child.ppid == parent.pid).then_some(Proc {
                    pid,
                    started: child.started,
                })
            })
            .collect()
    }

    /// Adds to `recorded` every descendant of `roots` it does not hold yet, walking parent PIDs
    /// at most `MAX_DEPTH` generations down from each root and recording at most `MAX_RECORDED`
    /// processes. Processes that exit during the walk are left out.
    pub(super) fn record(roots: &[Proc], recorded: &mut Vec<Proc>) {
        let mut buffer = vec![0; MAX_CHILDREN];
        let mut walked = HashSet::new();
        let mut queue: VecDeque<(Proc, usize)> = roots.iter().map(|&root| (root, 0)).collect();
        while let Some((parent, depth)) = queue.pop_front() {
            if depth >= MAX_DEPTH || !walked.insert(parent) {
                continue;
            }
            for child in children(parent, &mut buffer) {
                if !recorded.contains(&child) {
                    if recorded.len() >= MAX_RECORDED {
                        return;
                    }
                    recorded.push(child);
                }
                queue.push_back((child, depth + 1));
            }
        }
    }

    /// Sends SIGKILL to every recorded process whose PID still has the recorded start time, or
    /// to its whole process group when it leads one (the group includes it, and members it
    /// started after it was recorded). A PID whose start time differs belongs to another process
    /// now and is left alone, as are this process, its process group and launchd.
    pub(super) fn kill(recorded: &[Proc]) {
        // SAFETY: getpid and getpgrp have no preconditions and cannot fail.
        let (me, my_group) = unsafe { (libc::getpid(), libc::getpgrp()) };
        for recorded in recorded {
            if recorded.pid <= 1 || recorded.pid == me || recorded.pid == my_group {
                continue;
            }
            let Some(now) = info(recorded.pid) else {
                continue;
            };
            if now.started != recorded.started {
                continue;
            }
            // SAFETY: plain signal calls for a PID that was checked just above.
            unsafe {
                if now.pgid == recorded.pid {
                    libc::killpg(recorded.pid, libc::SIGKILL);
                } else {
                    libc::kill(recorded.pid, libc::SIGKILL);
                }
            }
        }
    }
}

/// Other Unix systems (not target platforms): nothing is recorded, so only the process group is
/// ended and descendants that left it are not reached.
#[cfg(all(unix, not(target_os = "macos")))]
mod descendants {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub(super) struct Proc;

    pub(super) fn process(_pid: libc::pid_t) -> Option<Proc> {
        None
    }

    pub(super) fn record(_roots: &[Proc], _recorded: &mut Vec<Proc>) {}

    pub(super) fn kill(_recorded: &[Proc]) {}
}

/// Windows: how executors and check commands are created. `CREATE_NO_WINDOW` keeps a console
/// window from opening for every run when the host is the tray app (closing that window would
/// end the run); `CREATE_NEW_PROCESS_GROUP` keeps a Ctrl+C in the devhost's console from
/// reaching the executor, so a stopped run is recorded as stopped by the host instead of as a
/// failure. process-wrap keeps these flags when the Job Object adds `CREATE_SUSPENDED`.
#[cfg(windows)]
pub(crate) fn creation_flags() -> process_wrap::tokio::CreationFlags {
    use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
    process_wrap::tokio::CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
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
/// `{"stream":"runner","text":"spawn failed: …"}` line. Once the executor has started, its whole
/// process tree is ended however the run ends (before this returns), and also if this future is
/// dropped before it finishes (`TreeGuard` has the details): on Unix the process group and every
/// descendant, including ones that left the group (`setsid`, `setpgid`); on Windows the Job
/// Object. On Unix an executor stopped by a limit or `cancel` gets SIGTERM and up to
/// `TERM_GRACE` to exit before the rest is killed. Descendants are found through their parents,
/// so a process whose parent exited before the run ended is out of reach, and after a normal exit
/// only the process group is left to end: the executor is gone, so its descendants that left the
/// group can no longer be found through it.
///
/// `env` supplies PATH and proxy variables on top of the host's environment and removes the
/// nested-session markers (`env::REMOVED_VARS`). On Windows the executor starts without a console
/// window and out of reach of a Ctrl+C in the host's console (`creation_flags`).
pub async fn run(
    spec: CommandSpec,
    kind: ExecutorKind,
    log_path: &Path,
    limits: RunLimits,
    cancel: CancellationToken,
    env: &ExecEnv,
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
        env.apply(c);
    });
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    {
        command.wrap(creation_flags());
        command.wrap(JobObject);
    }
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

    /// A child that only counts how often its process tree is killed or otherwise signalled. It
    /// has no PID, like an executor that has already been reaped.
    #[derive(Debug)]
    struct CountingChild {
        kills: Arc<AtomicUsize>,
        /// Only Unix has `signal`.
        #[cfg_attr(windows, allow(dead_code))]
        signals: Arc<AtomicUsize>,
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

        fn id(&self) -> Option<u32> {
            None
        }

        fn start_kill(&mut self) -> io::Result<()> {
            self.kills.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
            Box::pin(async { Err(io::Error::other("already gone")) })
        }

        #[cfg(unix)]
        fn signal(&self, _sig: i32) -> io::Result<()> {
            self.signals.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn counting_guard() -> (TreeGuard, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let kills = Arc::new(AtomicUsize::new(0));
        let signals = Arc::new(AtomicUsize::new(0));
        let child = CountingChild {
            kills: kills.clone(),
            signals: signals.clone(),
        };
        (TreeGuard::new(Box::new(child)), kills, signals)
    }

    #[test]
    fn dropping_the_guard_early_ends_the_tree() {
        let (guard, kills, _) = counting_guard();
        drop(guard);
        assert_eq!(kills.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_reaped_tree_is_not_signalled_again_on_drop() {
        let (mut guard, kills, signals) = counting_guard();
        guard.end().await;
        assert_eq!(kills.load(Ordering::SeqCst), 1);
        // The executor had already exited: no SIGTERM and no grace period for it.
        assert_eq!(signals.load(Ordering::SeqCst), 0);
        drop(guard);
        assert_eq!(
            kills.load(Ordering::SeqCst),
            1,
            "a reaped process group id was signalled again"
        );
    }

    /// A guard over `/bin/sh -c <script> sh <args…>`, started like an executor (leading its own
    /// process group), whose `end` waits `grace` after SIGTERM.
    #[cfg(unix)]
    fn shell_guard(script: &str, args: &[&Path], grace: Duration) -> TreeGuard {
        let mut command = CommandWrap::with_new("/bin/sh", |c| {
            c.args(["-c", script, "sh"]).args(args);
        });
        command.wrap(ProcessGroup::leader());
        command.wrap(KillOnDrop);
        let mut guard = TreeGuard::new(command.spawn().expect("start sh"));
        guard.grace = grace;
        guard
    }

    /// Waits until `path` exists, for at most 10 seconds.
    #[cfg(unix)]
    async fn wait_for_file(path: &Path) {
        let give_up = Instant::now() + Duration::from_secs(10);
        while !path.exists() {
            assert!(
                Instant::now() < give_up,
                "{} never appeared",
                path.display()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    // Windows has no counterpart: a Job Object is terminated at once, there is no SIGTERM step.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stopped_executor_gets_sigterm_before_the_tree_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let (ready, stopped) = (dir.path().join("ready"), dir.path().join("stopped"));
        // `wait` lets the trap run as soon as the signal arrives.
        let mut guard = shell_guard(
            r#"trap 'echo stopped > "$2"; exit 0' TERM; : > "$1"; sleep 30 & wait"#,
            &[&ready, &stopped],
            Duration::from_secs(30),
        );
        wait_for_file(&ready).await;
        let started = Instant::now();
        guard.end().await;
        assert!(guard.done, "the tree was not reaped");
        assert_eq!(std::fs::read_to_string(&stopped).unwrap(), "stopped\n");
        // It exited on SIGTERM, so the grace period ended early.
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    // Windows has no counterpart: a Job Object is terminated at once, there is no SIGTERM step.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_executor_that_ignores_sigterm_is_killed_when_the_grace_period_ends() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let grace = Duration::from_millis(300);
        let mut guard = shell_guard(
            r#"trap '' TERM; : > "$1"; sleep 30 & wait"#,
            &[&ready],
            grace,
        );
        wait_for_file(&ready).await;
        let started = Instant::now();
        guard.end().await;
        let took = started.elapsed();
        assert!(guard.done, "the tree was not reaped");
        assert!(
            took >= grace,
            "killed before the grace period ended: {took:?}"
        );
        assert!(took < Duration::from_secs(10), "took {took:?}");
    }

    // Only macOS records descendants: Windows needs no walk (the Job Object holds them all), and
    // other Unix systems are not target platforms.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_recorded_process_whose_start_time_differs_is_not_signalled() {
        use std::os::unix::process::ExitStatusExt;

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = libc::pid_t::try_from(child.id()).unwrap();
        let real = descendants::process(pid).expect("the test's own child is visible");
        // The same PID with another start time stands for a process that reused the PID.
        let reused = descendants::Proc {
            started: (real.started.0 - 1, real.started.1),
            ..real
        };
        descendants::kill(&[reused]);
        std::thread::sleep(Duration::from_millis(300));
        let untouched = child.try_wait().unwrap().is_none();
        // With the recorded start time it is ended: the check above is not vacuous.
        descendants::kill(&[real]);
        let give_up = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if Instant::now() >= give_up {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(untouched, "a PID whose start time differs was signalled");
        assert_eq!(status.and_then(|s| s.signal()), Some(libc::SIGKILL));
    }

    #[cfg(windows)]
    #[test]
    fn executors_start_without_a_console_window_and_out_of_reach_of_ctrl_c() {
        // CREATE_NO_WINDOW (0x0800_0000) | CREATE_NEW_PROCESS_GROUP (0x0000_0200).
        assert_eq!(creation_flags().0.0, 0x0800_0200);
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
