//! Runs one executor process: feeds the prompt on stdin, streams stdout/stderr into a JSONL
//! log, enforces the total and no-output limits, and always ends the whole process tree when the
//! run ends: on Unix the process group and every descendant, including ones that left the group;
//! on Windows the Job Object. On macOS it also ends, at the next start, the executors of a host
//! that died without ending them (`end_leftover`).
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
use crate::executor::{CommandSpec, lean::LeanCheck, outcome::FinalTextTracker};
use crate::model::ProcessTag;

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
    /// The output of a lean run showed that lean did not take effect (`lean::LeanCheck`), what
    /// it showed being the text; the run was stopped as a cancelled one is.
    LeanNotApplied(String),
}

/// What one run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub exit: ExitKind,
    /// The executor's final message, when it printed one.
    pub final_text: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Line {
    Out(String),
    Err(String),
}

/// How many output lines may wait between the pipe readers and the log writer. A full queue makes
/// the readers wait, and with them the executor's writes, instead of growing without bound.
const LINE_QUEUE: usize = 1024;

/// The most bytes of one output line that are kept; the rest of a longer line is read and
/// dropped, and a note in the log says so.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// How long an executor that is being stopped (timeout, stall, cancel, a lean run whose output
/// showed that lean did not take effect) may take to exit after SIGTERM before the rest of its
/// tree is killed. It lets Claude Code / Codex end their own
/// detached tool processes. Windows has no such step: see `TreeGuard`.
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(5);

/// Owns the spawned child and ends its whole process tree, however `run` ends.
///
/// - `end`, the explicit cleanup, runs after every run. On Unix, while the executor still runs
///   (the run was stopped by a limit, cancelled, or stopped by the lean check), it records the executor's descendants by
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

    /// Whether `pid` may be signalled at all: not PID 0 (`kill` and `killpg` take it as the
    /// caller's own group), launchd, this process or its process group.
    pub(super) fn foreign(pid: libc::pid_t) -> bool {
        // SAFETY: getpid and getpgrp have no preconditions and cannot fail.
        let (me, my_group) = unsafe { (libc::getpid(), libc::getpgrp()) };
        pid > 1 && pid != me && pid != my_group
    }

    /// Sends SIGKILL to every recorded process whose PID still has the recorded start time, or
    /// to its whole process group when it leads one (the group includes it, and members it
    /// started after it was recorded). A PID whose start time differs belongs to another process
    /// now and is left alone, as are this process, its process group and launchd (`foreign`).
    pub(super) fn kill(recorded: &[Proc]) {
        for recorded in recorded {
            if !foreign(recorded.pid) {
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

/// Called once with the executor's process, right after it started.
pub type SpawnHook = Box<dyn FnOnce(ProcessTag) + Send>;

/// The live process `pid` with its start time, for `Store::set_process`. macOS only: Windows
/// needs no record (the Job Object ends the whole tree when the host dies) and other Unix systems
/// are not target platforms.
#[cfg(target_os = "macos")]
pub fn process_tag(pid: u32) -> Option<ProcessTag> {
    let proc = descendants::process(libc::pid_t::try_from(pid).ok()?)?;
    Some(ProcessTag {
        pid,
        started: format_started(proc.started),
    })
}

/// Windows and other Unix systems record no process: see the macOS version.
#[cfg(not(target_os = "macos"))]
pub fn process_tag(_pid: u32) -> Option<ProcessTag> {
    None
}

#[cfg(target_os = "macos")]
fn format_started((seconds, micros): (u64, u64)) -> String {
    format!("{seconds}.{micros:06}")
}

/// Ends what a run left behind after the host that started it died without ending it. If the
/// executor in `tag` still runs (same PID, same start time), its descendants are recorded, its
/// process group gets SIGTERM, it gets up to `TERM_GRACE` to exit, the descendants are recorded
/// again, and then the group (while its leader still runs) and every recorded descendant get
/// SIGKILL, as `TreeGuard::end` does for a run that is stopped. Each signal to the group is sent
/// only if the leader still has the recorded start time right before it. Returns whether the
/// executor was still running. A PID with another start time belongs to an unrelated process and
/// is left alone, and so are this process, its process group and launchd, whatever the record
/// says. An executor that has exited can no longer be followed to its descendants: they are out
/// of reach, like the orphans `run` cannot find.
#[cfg(target_os = "macos")]
pub async fn end_leftover(tag: &ProcessTag) -> bool {
    end_leftovers(std::slice::from_ref(tag)).await[0]
}

/// `end_leftover` for several executors at once. They share the grace period: executors that
/// ignore SIGTERM cost one grace period together, not one each. Returns whether each executor
/// was still running, in the order of `tags`.
#[cfg(target_os = "macos")]
pub async fn end_leftovers(tags: &[ProcessTag]) -> Vec<bool> {
    end_leftovers_within(tags, TERM_GRACE).await
}

/// `end_leftovers` with the grace period as a parameter, so that tests can use a short one.
#[cfg(target_os = "macos")]
async fn end_leftovers_within(tags: &[ProcessTag], grace: Duration) -> Vec<bool> {
    let mut sweeps: Vec<Option<Sweep>> = tags.iter().map(Sweep::find).collect();
    for sweep in sweeps.iter_mut().flatten() {
        descendants::record(&[sweep.leader], &mut sweep.recorded);
        sweep.signal_group(libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while sweeps.iter().flatten().any(Sweep::leader_runs) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for sweep in sweeps.iter_mut().flatten() {
        let mut roots = sweep.recorded.clone();
        if sweep.leader_runs() {
            roots.push(sweep.leader);
        }
        descendants::record(&roots, &mut sweep.recorded);
        sweep.signal_group(libc::SIGKILL);
        descendants::kill(&sweep.recorded);
    }
    sweeps.iter().map(Option::is_some).collect()
}

/// One leftover executor being ended: its leader as it was found, and the descendants recorded
/// for it so far.
#[cfg(target_os = "macos")]
struct Sweep {
    leader: descendants::Proc,
    recorded: Vec<descendants::Proc>,
}

#[cfg(target_os = "macos")]
impl Sweep {
    /// The executor in `tag`, if it still runs with the recorded start time and may be
    /// signalled at all (`descendants::foreign`).
    fn find(tag: &ProcessTag) -> Option<Self> {
        let pid = libc::pid_t::try_from(tag.pid).ok()?;
        if !descendants::foreign(pid) {
            return None;
        }
        let leader = descendants::process(pid)?;
        (format_started(leader.started) == tag.started).then(|| Self {
            leader,
            recorded: Vec::new(),
        })
    }

    /// Whether the leader still runs with the start time it was found with.
    fn leader_runs(&self) -> bool {
        descendants::process(self.leader.pid) == Some(self.leader)
    }

    /// Sends `signal` to the leader's process group, checked right before the call: a walk of
    /// many processes may lie between finding the leader and this, and once the leader is gone,
    /// its PID, and with it the group id, may belong to another process.
    fn signal_group(&self, signal: libc::c_int) {
        if self.leader_runs() {
            // SAFETY: a plain signal to the group of a process whose PID and start time were
            // checked just above.
            unsafe { libc::killpg(self.leader.pid, signal) };
        }
    }
}

/// Windows: the Job Object of a run ends with the host that created it
/// (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), so nothing is left to end. Other Unix systems record
/// no process.
#[cfg(not(target_os = "macos"))]
pub async fn end_leftover(_tag: &ProcessTag) -> bool {
    false
}

/// Nothing is left to end where nothing is recorded: see `end_leftover`.
#[cfg(not(target_os = "macos"))]
pub async fn end_leftovers(tags: &[ProcessTag]) -> Vec<bool> {
    vec![false; tags.len()]
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

/// Runs `spec` until it exits, a limit is hit, `cancel` fires or, in a lean run (`spec.lean`),
/// an output line shows that lean did not take effect (`lean::LeanCheck`): that line is logged
/// and the run is stopped as a cancelled one is, with `ExitKind::LeanNotApplied`. The JSONL file
/// at `log_path` is created (or truncated) and starts with `spec.notes`, each as a
/// `{"stream":"stderr","text":"[IssueRelay] …"}` line; then it gets one line per output line:
/// stdout verbatim, stderr as `{"stream":"stderr","text":…}`. A run that cannot start gets one
/// `{"stream":"runner","text":"spawn failed: …"}` line. Once the executor has started, its whole
/// process tree is ended however the run ends (before this returns), and also if this future is
/// dropped before it finishes (`TreeGuard` has the details): on Unix the process group and every
/// descendant, including ones that left the group (`setsid`, `setpgid`); on Windows the Job
/// Object. On Unix an executor stopped by a limit, `cancel` or the lean check gets SIGTERM and up
/// to `TERM_GRACE` to exit before the rest is killed. Descendants are found through their parents,
/// so a process whose parent exited before the run ended is out of reach, and after a normal exit
/// only the process group is left to end: the executor is gone, so its descendants that left the
/// group can no longer be found through it.
///
/// The output is read through a queue of `LINE_QUEUE` lines: an executor that prints faster than
/// the log is written is slowed down instead of filling the host's memory. A line longer than
/// `MAX_LINE_BYTES` is cut, with a stderr entry saying so. A log that cannot be written is
/// reported once in the host's log, and the run goes on. On Unix the log is made private to the
/// user (mode 0600) also when a retry reuses the previous attempt's file.
///
/// `env` supplies PATH and proxy variables on top of the host's environment and removes the
/// nested-session markers (`env::REMOVED_VARS`); `spec.env`, the run's own variables, is set
/// after that, so it wins over both. On Windows the executor starts without a console
/// window and out of reach of a Ctrl+C in the host's console (`creation_flags`).
///
/// `on_spawn` gets the executor's process (`process_tag`) right after it started, so that a host
/// started after a crash can end what the run left behind (`end_leftover`). It is not called
/// when the executor could not start or has already exited, nor where no process is recorded
/// (only macOS records one).
pub async fn run(
    spec: CommandSpec,
    kind: ExecutorKind,
    log_path: &Path,
    limits: RunLimits,
    cancel: CancellationToken,
    env: &ExecEnv,
    on_spawn: Option<SpawnHook>,
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
        // After `apply`, so that a variable of the run's own is never removed again.
        c.envs(spec.env.iter().map(|(name, value)| (name, value)));
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
    if let Some(hook) = on_spawn
        && let Some(tag) = guard.child.id().and_then(process_tag)
    {
        hook(tag);
    }

    if let Some(mut stdin) = guard.child.stdin().take() {
        let prompt = spec.stdin;
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let (tx, mut rx) = mpsc::channel(LINE_QUEUE);
    if let Some(stdout) = guard.child.stdout().take() {
        tokio::spawn(forward_lines(BufReader::new(stdout), tx.clone(), Line::Out));
    }
    if let Some(stderr) = guard.child.stderr().take() {
        tokio::spawn(forward_lines(BufReader::new(stderr), tx.clone(), Line::Err));
    }
    drop(tx);

    let mut tracker = FinalTextTracker::new(kind);
    let mut lean_check = spec.lean.then(|| LeanCheck::new(kind));
    let mut log_failures = LogFailures::new(log_path);
    // Before the loop reads the first line, so the notes open the log.
    for text in &spec.notes {
        let entry = note("stderr", &format!("[IssueRelay] {text}"));
        log_failures.note(log.write_all(entry.as_bytes()).await);
    }
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
                        let violation = match (&mut lean_check, &line) {
                            (Some(check), Line::Out(text)) => check.observe(text),
                            _ => None,
                        };
                        // The line goes into the log either way, so the log shows what was wrong.
                        log_failures.note(record(&mut log, &mut tracker, line).await);
                        if let Some(detail) = violation {
                            break ExitKind::LeanNotApplied(detail);
                        }
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
        log_failures.note(record(&mut log, &mut tracker, line).await);
    }
    // A tokio file reports a failed write with a later call, so the last one shows here.
    log_failures.note(log.flush().await);
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
/// private to the user, also when the file exists already: a retry reuses the previous attempt's
/// file.
async fn create_log(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    make_private(&mut options);
    let file = tokio::fs::OpenOptions::from(options).open(path).await?;
    // `mode` applies only to a file being created; a retry reuses the previous attempt's file.
    // (Windows has no mode to set: see `make_private`.)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .await?;
    }
    Ok(file)
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

/// Reads one line into `buf` like `read_until(b'\n', buf)`, but keeps at most `cap` bytes of it:
/// the rest of an overlong line is read and dropped, so one runaway line cannot use up memory.
/// Returns how many bytes were read, newline included; 0 at the end of the input.
async fn read_line_capped<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<usize> {
    let mut read = 0;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(read);
        }
        let (chunk, done) = match available.iter().position(|&b| b == b'\n') {
            Some(end) => (&available[..=end], true),
            None => (available, false),
        };
        let room = cap.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
        let used = chunk.len();
        reader.consume(used);
        read += used;
        if done {
            return Ok(read);
        }
    }
}

/// Sends each line of `reader` to `tx` (wrapped by `wrap`) until the end of input, a read error,
/// or the receiver going away. Lines end at `\n` (a `\r` before it is dropped) and are decoded
/// lossily: one line of invalid UTF-8 must not end the stream, or every later event, including
/// the final result, would be lost and the executor would get a broken pipe. A line longer than
/// `MAX_LINE_BYTES` is cut, followed by a stderr note saying so.
async fn forward_lines<R: AsyncBufRead + Unpin>(
    reader: R,
    tx: mpsc::Sender<Line>,
    wrap: fn(String) -> Line,
) {
    forward_capped(reader, tx, wrap, MAX_LINE_BYTES).await;
}

/// `forward_lines` with the length that is kept of a line as a parameter, so that tests can use a
/// small one.
async fn forward_capped<R: AsyncBufRead + Unpin>(
    mut reader: R,
    tx: mpsc::Sender<Line>,
    wrap: fn(String) -> Line,
    cap: usize,
) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // Room for a line ending after the `cap` bytes of text, so that the text can be told from
        // the ending: a line of exactly `cap` bytes is not cut, one byte more is, whatever ends it.
        let read = match read_line_capped(&mut reader, &mut buf, cap.saturating_add(2)).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        if buf.ends_with(b"\n") {
            buf.pop();
            if buf.ends_with(b"\r") {
                buf.pop();
            }
        }
        // Whatever `read_line_capped` dropped came after these bytes, so a line is cut exactly
        // when its text is longer than `cap`.
        let cut = buf.len() > cap;
        buf.truncate(cap);
        if tx
            .send(wrap(String::from_utf8_lossy(&buf).into_owned()))
            .await
            .is_err()
        {
            return;
        }
        if cut {
            let note = format!("[IssueRelay] a {read}-byte output line was cut to {cap} bytes");
            if tx.send(Line::Err(note)).await.is_err() {
                return;
            }
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

/// Writes one log line per call, in a single write so a live reader never sees half a line. The
/// tracker sees the line first, so a log that cannot be written does not lose the final result.
async fn record(log: &mut File, tracker: &mut FinalTextTracker, line: Line) -> std::io::Result<()> {
    let entry = match line {
        Line::Out(mut text) => {
            tracker.observe(&text);
            text.push('\n');
            text
        }
        Line::Err(text) => note("stderr", &text),
    };
    log.write_all(entry.as_bytes()).await
}

/// Reports the first failure to write a run's log in the host's log, and only the first: the run
/// goes on whatever happens to its log, and a full disk would fail every later line the same way.
struct LogFailures<'a> {
    path: &'a Path,
    reported: bool,
}

impl<'a> LogFailures<'a> {
    fn new(path: &'a Path) -> Self {
        Self {
            path,
            reported: false,
        }
    }

    /// Reports `result` if it is a failure and the first one; says whether it did.
    fn note(&mut self, result: std::io::Result<()>) -> bool {
        let Err(e) = result else {
            return false;
        };
        if std::mem::replace(&mut self.reported, true) {
            return false;
        }
        tracing::warn!(
            path = %self.path.display(),
            error = %e,
            "cannot write the run log; later lines may be missing"
        );
        true
    }
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

    use tokio::io::{AsyncReadExt, BufReader};

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

    // The record of a run's executor must compare equal with what a later host reads for the same
    // process, so the format is fixed: seconds, a dot and six digits of microseconds.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_start_time_is_written_with_six_digits_of_microseconds() {
        assert_eq!(format_started((1_700_000_000, 123)), "1700000000.000123");
        assert_eq!(format_started((1_700_000_000, 0)), "1700000000.000000");
        assert_eq!(
            format_started((1_700_000_000, 999_999)),
            "1700000000.999999"
        );
    }

    // Whatever a record says, PID 0 (`kill` and `killpg` take it as the caller's own group),
    // launchd, this process and its process group are never signalled.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_host_and_its_own_group_are_never_signalled() {
        // SAFETY: getpid and getpgrp have no preconditions and cannot fail.
        let (me, my_group) = unsafe { (libc::getpid(), libc::getpgrp()) };
        for pid in [0, 1, me, my_group] {
            assert!(!descendants::foreign(pid), "{pid} may be signalled");
        }
        // No PID is that high, so it is neither this process nor its group.
        assert!(descendants::foreign(libc::pid_t::MAX));
    }

    /// A shell that ignores SIGTERM and outlives each of its children, started like an executor
    /// (leading a process group of its own), so only a signal to the shell itself ends it; with
    /// its record. Its guard is only the cleanup: it reaps the shell at the end, and ends the
    /// tree should the test fail. The leftover is ended through its record, like one a crashed
    /// host left behind.
    #[cfg(target_os = "macos")]
    async fn stubborn_leftover(dir: &Path, ready: &str) -> (TreeGuard, ProcessTag) {
        let ready = dir.join(ready);
        let guard = shell_guard(
            r#"trap '' TERM; : > "$1"; while :; do sleep 1; done"#,
            &[&ready],
            Duration::from_millis(300),
        );
        wait_for_file(&ready).await;
        let pid = guard.leader().expect("the shell runs");
        let tag = process_tag(u32::try_from(pid).unwrap()).expect("a live process has a tag");
        (guard, tag)
    }

    /// Waits, for at most 5 seconds, until the process in `tag` no longer runs: gone, or a zombie
    /// its guard reaps. SIGKILL is delivered asynchronously.
    #[cfg(target_os = "macos")]
    async fn wait_until_gone(tag: &ProcessTag) {
        let give_up = Instant::now() + Duration::from_secs(5);
        while process_tag(tag.pid).as_ref() == Some(tag) {
            assert!(Instant::now() < give_up, "the leftover still runs");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    // A leftover executor that ignores SIGTERM is killed, with its process group, once the grace
    // period is over. Only macOS records executors (`process_tag`).
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_leftover_that_ignores_sigterm_is_killed_when_the_grace_period_ends() {
        let dir = tempfile::tempdir().unwrap();
        let (mut guard, tag) = stubborn_leftover(dir.path(), "ready").await;
        let grace = Duration::from_millis(300);
        let started = Instant::now();
        let ended = end_leftovers_within(std::slice::from_ref(&tag), grace).await;
        let took = started.elapsed();
        assert_eq!(ended, [true]);
        assert!(
            took >= grace,
            "killed before the grace period ended: {took:?}"
        );
        assert!(took < Duration::from_secs(10), "took {took:?}");
        wait_until_gone(&tag).await;
        guard.end().await;
        assert!(guard.done, "the tree was not reaped");
    }

    // Leftovers that ignore SIGTERM wait out one grace period together, not one each, before the
    // host's first check; each is reported in the order of the records.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn leftovers_that_ignore_sigterm_share_one_grace_period() {
        let dir = tempfile::tempdir().unwrap();
        let (mut first, first_tag) = stubborn_leftover(dir.path(), "first").await;
        let (mut second, second_tag) = stubborn_leftover(dir.path(), "second").await;
        let gone = ProcessTag {
            pid: u32::MAX - 7,
            started: "1.000001".into(),
        };
        let grace = Duration::from_millis(500);
        let started = Instant::now();
        let tags = [first_tag.clone(), gone, second_tag.clone()];
        let ended = end_leftovers_within(&tags, grace).await;
        let took = started.elapsed();
        assert_eq!(ended, [true, false, true]);
        assert!(
            took >= grace,
            "killed before the grace period ended: {took:?}"
        );
        assert!(
            took < grace * 2,
            "each leftover waited a grace period of its own: {took:?}"
        );
        wait_until_gone(&first_tag).await;
        wait_until_gone(&second_tag).await;
        for guard in [&mut first, &mut second] {
            guard.end().await;
            assert!(guard.done, "the tree was not reaped");
        }
    }

    // A leftover's group is signalled only while its leader still has the start time it was
    // found with: checked right before each signal, as `descendants::kill` does, since a walk of
    // many processes may lie between finding the leader and signalling its group.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn a_leftover_group_is_signalled_only_while_its_leader_is_the_one_found() {
        let dir = tempfile::tempdir().unwrap();
        let (mut guard, tag) = stubborn_leftover(dir.path(), "ready").await;
        let found = Sweep::find(&tag).expect("the leftover runs");
        // The same PID with another start time stands for a leader that exited after it was
        // found, its PID taken by another process since.
        let replaced = Sweep {
            leader: descendants::Proc {
                started: (found.leader.started.0 - 1, found.leader.started.1),
                ..found.leader
            },
            recorded: Vec::new(),
        };
        replaced.signal_group(libc::SIGKILL);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            process_tag(tag.pid).as_ref(),
            Some(&tag),
            "a group whose leader changed was signalled"
        );
        // With the leader as found, the group is signalled: the check above is not vacuous.
        found.signal_group(libc::SIGKILL);
        wait_until_gone(&tag).await;
        guard.end().await;
        assert!(guard.done, "the tree was not reaped");
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

    /// What `forward_capped` sends for `bytes` when it keeps `cap` bytes of a line. The input is
    /// read through a buffer of `chunk` bytes, so that line ends fall on and across the
    /// boundaries of what the reader sees at once.
    async fn forwarded(bytes: &[u8], cap: usize, chunk: usize) -> Vec<Line> {
        let (tx, mut rx) = mpsc::channel(LINE_QUEUE);
        forward_capped(BufReader::with_capacity(chunk, bytes), tx, Line::Out, cap).await;
        let mut lines = Vec::new();
        while let Some(line) = rx.recv().await {
            lines.push(line);
        }
        lines
    }

    /// The lines `forward_lines` sends for `bytes`; none of them may be a note.
    async fn collect(bytes: &[u8]) -> Vec<String> {
        forwarded(bytes, MAX_LINE_BYTES, 8192)
            .await
            .into_iter()
            .map(|line| match line {
                Line::Out(text) => text,
                Line::Err(note) => panic!("unexpected note: {note}"),
            })
            .collect()
    }

    fn out(text: &str) -> Line {
        Line::Out(text.into())
    }

    /// The note for a line of `read` bytes that was cut to 10.
    fn cut_note(read: usize) -> Line {
        Line::Err(format!(
            "[IssueRelay] a {read}-byte output line was cut to 10 bytes"
        ))
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
    async fn an_overlong_line_is_cut_and_the_next_line_still_arrives() {
        let input = [vec![b'a'; 50], b"\nshort\r\nlast".to_vec()].concat();
        let mut reader = tokio::io::BufReader::with_capacity(8, &input[..]);
        let mut buf = Vec::new();
        let read = read_line_capped(&mut reader, &mut buf, 10).await.unwrap();
        assert_eq!(read, 51, "the whole line, newline included, is consumed");
        assert_eq!(buf, vec![b'a'; 10]);
        buf.clear();
        assert_eq!(
            read_line_capped(&mut reader, &mut buf, 10).await.unwrap(),
            7
        );
        assert_eq!(buf, b"short\r\n");
        buf.clear();
        assert_eq!(
            read_line_capped(&mut reader, &mut buf, 10).await.unwrap(),
            4
        );
        assert_eq!(buf, b"last");
        buf.clear();
        assert_eq!(
            read_line_capped(&mut reader, &mut buf, 10).await.unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn a_cut_line_is_followed_by_a_note_and_the_next_line_still_arrives() {
        let input = [vec![b'a'; 50], b"\nshort\r\nlast".to_vec()].concat();
        for chunk in [1, 3, 8, 8192] {
            assert_eq!(
                forwarded(&input, 10, chunk).await,
                [out("aaaaaaaaaa"), cut_note(51), out("short"), out("last")],
                "read through {chunk} bytes at a time"
            );
        }
    }

    // The line ending does not count toward the limit: a line whose text is exactly 10 bytes, or
    // that has 10 bytes in all, is not cut, whatever ends it and wherever the buffer ends.
    #[tokio::test]
    async fn a_line_that_fits_the_limit_is_kept_whole_and_gets_no_note() {
        let cases: [(&[u8], &[&str]); 6] = [
            (b"aaaaaaaaaa\nnext", &["aaaaaaaaaa", "next"]),
            (b"aaaaaaaaaa\r\nnext", &["aaaaaaaaaa", "next"]),
            (b"aaaaaaaaa\r\nnext", &["aaaaaaaaa", "next"]),
            (b"aaaaaaaa\r\nnext", &["aaaaaaaa", "next"]),
            (b"aaaaaaaaaa", &["aaaaaaaaaa"]),
            (b"aaaaaaaaaa\n", &["aaaaaaaaaa"]),
        ];
        for chunk in [1, 3, 8, 8192] {
            for (input, texts) in cases {
                let expected: Vec<Line> = texts.iter().map(|text| out(text)).collect();
                assert_eq!(
                    forwarded(input, 10, chunk).await,
                    expected,
                    "{:?} read through {chunk} bytes at a time",
                    String::from_utf8_lossy(input)
                );
            }
        }
    }

    // A single byte over the limit already counts as cut, with a line feed or a CRLF after it, and
    // at the end of the input.
    #[tokio::test]
    async fn a_line_one_byte_over_the_limit_is_cut_and_gets_a_note() {
        let cases: [(&[u8], &[Line]); 4] = [
            (
                b"aaaaaaaaaaa\nnext",
                &[out("aaaaaaaaaa"), cut_note(12), out("next")],
            ),
            (
                b"aaaaaaaaaaa\r\nnext",
                &[out("aaaaaaaaaa"), cut_note(13), out("next")],
            ),
            (
                b"aaaaaaaaaaaa\nnext",
                &[out("aaaaaaaaaa"), cut_note(13), out("next")],
            ),
            (b"aaaaaaaaaaa", &[out("aaaaaaaaaa"), cut_note(11)]),
        ];
        for chunk in [1, 3, 8, 8192] {
            for (input, expected) in cases {
                assert_eq!(
                    forwarded(input, 10, chunk).await,
                    expected,
                    "{:?} read through {chunk} bytes at a time",
                    String::from_utf8_lossy(input)
                );
            }
        }
    }

    #[tokio::test]
    async fn a_full_queue_makes_the_reader_wait() {
        const LINES: &[u8] = b"0\n1\n2\n3\n4\n5\n";
        let (tx, mut rx) = mpsc::channel(2);
        let reader = tokio::spawn(forward_lines(BufReader::new(LINES), tx, Line::Out));
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(
            !reader.is_finished(),
            "the reader went on although nobody took its lines"
        );
        assert_eq!(rx.len(), 2, "only what fits in the queue is waiting");
        let mut lines = Vec::new();
        while let Some(line) = rx.recv().await {
            lines.push(line);
        }
        assert_eq!(
            lines,
            [out("0"), out("1"), out("2"), out("3"), out("4"), out("5")]
        );
        reader.await.unwrap();
    }

    #[tokio::test]
    async fn reading_stops_when_the_receiver_is_gone() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut reader = BufReader::with_capacity(4, &b"a\nb\nc\nd\n"[..]);
        forward_lines(&mut reader, tx, Line::Out).await;
        let mut rest = Vec::new();
        reader.read_to_end(&mut rest).await.unwrap();
        assert!(
            !rest.is_empty(),
            "the reader went on after the receiver was gone"
        );
    }

    // A tokio file reports a failed write with a later call: here the first failure shows at the
    // second line and the last one at the flush. Only the first is reported.
    #[tokio::test]
    async fn a_log_that_cannot_be_written_is_reported_once_and_the_final_text_survives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("1.jsonl");
        std::fs::write(&path, "").unwrap();
        // Opened for reading only, so that every write fails.
        let mut log = File::open(&path).await.unwrap();
        let mut tracker = FinalTextTracker::new(ExecutorKind::Claude);
        let mut failures = LogFailures::new(&path);
        let result = r#"{"type":"result","result":"done"}"#;
        let mut reported = 0;
        for line in [Line::Out(result.into()), out("x"), out("y")] {
            let written = record(&mut log, &mut tracker, line).await;
            reported += usize::from(failures.note(written));
        }
        reported += usize::from(failures.note(log.flush().await));
        assert_eq!(reported, 1, "later failures are not reported again");
        assert_eq!(tracker.into_final_text().as_deref(), Some("done"));
    }
}
