//! Environment checks: are Claude Code / Codex installed, logged in and connected to Linear;
//! does the Linear workspace have the label and statuses IssueRelay relies on; are bindings usable.

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{
        Arc, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::config::{Binding, Config, Effort, ExecutorKind, LeanStatus, TriggerNames};
use crate::env::{ExecEnv, SharedEnv, Sources};
use crate::executor;
use crate::executor::lean::{CLAUDE_LINEAR_SERVER, CODEX_LINEAR_SERVER};
use crate::linear::{LinearApi, Viewer, auth::LINEAR_MCP_URL};
use crate::text::{is_invisible, strip_ansi};
use crate::trigger::{Readiness, same_name};

/// What a finished check command printed and how it exited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    /// `None` when the process was ended by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs one check command. `SystemRunner` is the real implementation; tests use a fake.
#[async_trait::async_trait]
pub trait CommandRunner: Send + Sync {
    /// `Err` means the command could not run to completion (it did not start, timed out, or what
    /// it printed could not be read); a command that ran and exited non-zero is an `Ok`.
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput>;

    /// `run` with an explicit time limit instead of the runner's own choice (the setup commands:
    /// a login waits for the user in the browser). The default ignores `limit`; `SystemRunner`
    /// honours it.
    async fn run_limited(
        &self,
        program: &Path,
        args: &[&str],
        limit: Duration,
    ) -> Result<CmdOutput> {
        let _ = limit;
        self.run(program, args).await
    }

    /// `run` in the folder `cwd` instead of the runner's own choice (the lean sign-in check runs
    /// in an empty folder). The default ignores `cwd`; `SystemRunner` honours it.
    async fn run_in(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<CmdOutput> {
        let _ = cwd;
        self.run(program, args).await
    }
}

/// Default limit for `mcp list`. `claude mcp list` health-checks every configured MCP server,
/// so its time grows with their number: measured on 2026-10-11 with about 35 servers it took
/// 11-14 s, while `codex mcp list` took 0.3 s.
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// Default limit for every other check (`--version`, `auth status`, `login status`), which
/// only read local state.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a check's pipes are still read once the command has exited and whatever it left in
/// its process group (Windows: Job Object) has been ended. Only a process that escaped the group
/// (Unix: `setsid`) can hold a pipe open past that; what was read by then is the output.
const PIPE_DRAIN: Duration = Duration::from_secs(2);

/// How long `SystemReadiness` keeps an executor's check that timed out, crashed or whose output
/// could not be read, instead of the full cache TTL: soon enough to recover from a transient
/// failure, late enough that the engine's poll loop does not re-run a slow `claude mcp list`
/// on every poll.
const FAILED_CHECK_TTL: Duration = Duration::from_secs(60);

/// The most characters of command output that a problem carries.
const MAX_PROBLEM_TEXT: usize = 200;

/// The host of Linear's MCP server (the host of `LINEAR_MCP_URL`). An `mcp list` line only
/// describes Linear when it names exactly this host.
const LINEAR_MCP_HOST: &str = "mcp.linear.app";

/// The lean sign-in checks (lean-run design, section 7), which run with the other checks and
/// under the same limit. Claude: `auth status` without the user settings, which lean runs leave
/// out. Codex: `login status` from the credentials file (`auth.json`), the store a run that
/// ignores the user configuration falls back to (`cli_auth_credentials_store` is a user setting).
/// Both commands run that way and report the sign-in (measured 2026-10-19 with Claude Code
/// 2.1.285 and codex-cli 0.159.2).
const CLAUDE_LEAN_LOGIN: [&str; 4] = ["--setting-sources", "project,local", "auth", "status"];
const CODEX_LEAN_LOGIN: [&str; 4] = ["-c", "cli_auth_credentials_store=file", "login", "status"];

/// Runs the real check commands, each under its own time limit: by default 60 s for
/// `mcp list` and 20 s for the rest (`MCP_LIST_TIMEOUT`, `CHECK_TIMEOUT`). `run_limited` takes
/// the limit from its caller instead, which the one-click setup commands do (a login waits up to
/// five minutes for the browser). A command runs in the user's home directory (`run_in`: in the
/// folder it is given), in a process group of its own (Windows: a Job Object, and without a
/// console window, like the executors), with the shared environment (the PATH and proxy the
/// executors will get). A check is done when the command's own process exits: whatever it left
/// running in its group is ended then (`claude mcp list` starts stdio MCP servers of its own,
/// which inherit its pipes), and the output is what the pipes delivered up to `PIPE_DRAIN` after
/// that. A pipe that cannot be read fails the call: half of an `mcp list` must not pass for the
/// whole answer. A command still running at its limit is killed together with everything it
/// started and the call fails with `timed out after 20s` (the limit as `Duration` prints it).
/// Every command also gets `NO_COLOR=1` on top of the inherited environment so the CLIs do not
/// colour what they print into the pipe (a `FORCE_COLOR` may still win, which is why the parsers
/// strip colour codes too).
pub struct SystemRunner {
    check_timeout: Duration,
    mcp_list_timeout: Duration,
    env: SharedEnv,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::with_env(ExecEnv::inherit().shared())
    }
}

impl SystemRunner {
    /// Runs check commands with `env` (the PATH and proxy the executors will get) and the default
    /// limits.
    pub fn with_env(env: SharedEnv) -> Self {
        Self {
            check_timeout: CHECK_TIMEOUT,
            mcp_list_timeout: MCP_LIST_TIMEOUT,
            env,
        }
    }

    /// For tests: `check` limits every command except `mcp list`, `mcp_list` limits that one.
    /// The environment is the host's own.
    pub fn with_limits(check: Duration, mcp_list: Duration) -> Self {
        Self {
            check_timeout: check,
            mcp_list_timeout: mcp_list,
            env: ExecEnv::inherit().shared(),
        }
    }

    fn limit_for(&self, args: &[&str]) -> Duration {
        if matches!(args, ["mcp", "list"]) {
            self.mcp_list_timeout
        } else {
            self.check_timeout
        }
    }

    /// Runs one check command in `cwd`, as the type's documentation describes.
    async fn run_checked(
        &self,
        program: &Path,
        args: &[&str],
        limit: Duration,
        cwd: Option<&Path>,
    ) -> Result<CmdOutput> {
        #[cfg(windows)]
        use process_wrap::tokio::JobObject;
        #[cfg(unix)]
        use process_wrap::tokio::ProcessGroup;
        use process_wrap::tokio::{CommandWrap, KillOnDrop};

        let env = self
            .env
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut command = CommandWrap::with_new(program, |c| {
            c.args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(cwd) = cwd {
                c.current_dir(cwd);
            }
            env.apply(c);
            c.env("NO_COLOR", "1");
        });
        #[cfg(unix)]
        command.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        {
            command.wrap(crate::runner::creation_flags());
            command.wrap(JobObject);
        }
        command.wrap(KillOnDrop);
        let mut child = command.spawn()?;
        // Each pipe is read by a task of its own, and the check does not wait for the pipes to
        // end: a process the command leaves behind inherits them and can keep them open.
        let stop_reading = CancellationToken::new();
        // The readers stop however this call ends, also when its future is dropped.
        let _stop_reading_on_return = stop_reading.clone().drop_guard();
        let stdout = read_pipe(child.stdout().take(), stop_reading.clone());
        let stderr = read_pipe(child.stderr().take(), stop_reading.clone());
        // Wait on the command's own process, not through the process-wrap wrapper: on Windows
        // the wrapper's wait() also waits for every other process in the Job Object, so a
        // leftover would turn a finished check into a timeout.
        // SAFETY: only wait() is called on the native child, so no invariant of a wrapper in the
        // chain is bypassed; killing the rest of the tree still goes through the wrapper below.
        let exited = match unsafe { child.try_inner_child_mut() } {
            Some(process) => tokio::time::timeout(limit, process.wait()).await,
            None => {
                let _ = child.start_kill();
                bail!("process wrapper hid the native child");
            }
        };
        // Whatever is left in the process group (Windows: the Job Object) is ended now, after an
        // exit as after a timeout; that also closes the pipes those processes inherited. After an
        // exit the command is already reaped, as in `runner`: on Unix its PID, the group's id, is
        // not handed out again while a member of the group lives.
        let _ = child.start_kill();
        let Ok(status) = exited else {
            bail!("timed out after {limit:?}");
        };
        let status = status?;
        let drain_until = Instant::now() + PIPE_DRAIN;
        let (stdout, stderr) = tokio::join!(
            collected(stdout, &stop_reading, drain_until, "stdout"),
            collected(stderr, &stop_reading, drain_until, "stderr"),
        );
        Ok(CmdOutput {
            code: status.code(),
            stdout: stdout?,
            stderr: stderr?,
        })
    }
}

#[async_trait::async_trait]
impl CommandRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
        self.run_limited(program, args, self.limit_for(args)).await
    }

    async fn run_limited(
        &self,
        program: &Path,
        args: &[&str],
        limit: Duration,
    ) -> Result<CmdOutput> {
        // `claude mcp list` answers per directory, and a host started from Finder starts in
        // `/`. The home directory matches the user-level registration (`--scope user`).
        let home = dirs::home_dir();
        self.run_checked(program, args, limit, home.as_deref())
            .await
    }

    async fn run_in(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<CmdOutput> {
        self.run_checked(program, args, self.limit_for(args), Some(cwd))
            .await
    }
}

/// Reads one pipe of a check command, in a task of its own, until the pipe ends or `stop` fires.
/// The task returns what it read, or the error that ended the reading; an interrupted read is
/// tried again.
fn read_pipe<R>(pipe: Option<R>, stop: CancellationToken) -> JoinHandle<io::Result<Vec<u8>>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        let Some(mut pipe) = pipe else {
            return Ok(bytes);
        };
        let mut chunk = [0_u8; 8192];
        loop {
            tokio::select! {
                // Cancel-safe: when `stop` wins, this read has taken nothing from the pipe.
                read = pipe.read(&mut chunk) => match read {
                    Ok(0) => return Ok(bytes),
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                },
                // The drain deadline passed (`collected`), or the check is over: what was read
                // so far stands.
                _ = stop.cancelled() => return Ok(bytes),
            }
        }
    })
}

/// What one pipe of a check delivered, as text: everything up to the end of the pipe when that
/// comes by `deadline`, otherwise what was read by then (`stop` ends the reader). A read error,
/// or a reader task that failed, is an error naming `stream`: half of an `mcp list` that exited
/// 0 would otherwise read as "Linear is not configured" and be cached as a verdict.
async fn collected(
    mut reader: JoinHandle<io::Result<Vec<u8>>>,
    stop: &CancellationToken,
    deadline: Instant,
    stream: &str,
) -> Result<String> {
    let joined = match tokio::time::timeout_at(deadline, &mut reader).await {
        Ok(joined) => joined,
        Err(_) => {
            stop.cancel();
            reader.await
        }
    };
    match joined {
        Ok(Ok(bytes)) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Ok(Err(e)) => bail!("cannot read its {stream}: {e}"),
        Err(e) => bail!("the reader of its {stream} failed: {e}"),
    }
}

/// Whether one executor can take jobs, and what is wrong when it cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorReport {
    pub kind: ExecutorKind,
    /// Where the executable was found; `None` when it is not installed.
    pub program: Option<PathBuf>,
    /// The first line of `--version` that shows anything, cleaned like the problems, when that
    /// ran.
    pub version: Option<String>,
    pub logged_in: bool,
    /// Linear's MCP server is configured, authorized and connected in this executor.
    pub linear_mcp: bool,
    /// Human-readable reasons for the checks that failed or could not run.
    pub problems: Vec<String>,
    /// The Linear server lean runs load (`executor::lean`) is configured under its exact name
    /// and URL, authorized and connected.
    #[serde(default)]
    pub lean_linear_mcp: bool,
    /// A lean run would be signed in, and the same way as a usual run (Codex: also to the same
    /// model provider; Claude: also with no gateway or proxy in the user settings' `env`).
    /// Judged only when the usual login check says logged in.
    #[serde(default)]
    pub lean_logged_in: bool,
    /// Why lean runs cannot use this executor, besides `problems`: what the lean checks found.
    /// A lean check that a failed usual check leaves unjudged adds nothing here.
    #[serde(default)]
    pub lean_problems: Vec<String>,
}

impl ExecutorReport {
    /// Installed, logged in and connected to Linear. A check that could not run leaves this
    /// `false`: readiness was not confirmed.
    pub fn ready(&self) -> bool {
        self.program.is_some() && self.logged_in && self.linear_mcp
    }

    /// Installed, logged in, and both lean checks passed: the executor can take lean runs.
    pub fn ready_lean(&self) -> bool {
        self.program.is_some() && self.logged_in && self.lean_linear_mcp && self.lean_logged_in
    }
}

/// What the lean sign-in checks need from the host (lean-run design, section 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeanCheckPaths {
    /// An empty folder of IssueRelay's own, made when missing, to run Claude's lean sign-in check
    /// in: in the home folder, the project settings it reads would be `~/.claude/settings.json`,
    /// the very user settings lean runs leave out. The project settings of the folders above it
    /// do not count (Claude Code 2.1.285, the sandbox acceptance of 2026-10-20).
    pub empty_dir: PathBuf,
    /// Codex's configuration file (`codex_config_path`), read for a model provider lean runs
    /// would not use; `None` when there is none to read.
    pub codex_config: Option<PathBuf>,
    /// Claude Code's user settings file (`claude_settings_path`), read for variables in its `env`
    /// that send Claude Code through a gateway or a proxy, which lean runs would not apply;
    /// `None` when there is none to read.
    pub claude_settings: Option<PathBuf>,
}

/// Where Codex keeps its configuration: `config.toml` in `CODEX_HOME`, else in `.codex` in the
/// home folder. Executors inherit the host's `CODEX_HOME` (`ExecEnv` changes only the PATH and
/// the proxy). `None` without either.
pub fn codex_config_path() -> Option<PathBuf> {
    codex_config_path_from(std::env::var_os("CODEX_HOME"), dirs::home_dir())
}

fn codex_config_path_from(
    codex_home: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    codex_home
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".codex")))
        .map(|dir| dir.join("config.toml"))
}

/// Where Claude Code keeps its user settings: `settings.json` in `CLAUDE_CONFIG_DIR`, else in
/// `.claude` in the home folder. Executors inherit the host's `CLAUDE_CONFIG_DIR` (`ExecEnv`
/// changes only the PATH and the proxy). `None` without either.
pub fn claude_settings_path() -> Option<PathBuf> {
    claude_settings_path_from(std::env::var_os("CLAUDE_CONFIG_DIR"), dirs::home_dir())
}

fn claude_settings_path_from(
    config_dir: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    config_dir
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".claude")))
        .map(|dir| dir.join("settings.json"))
}

/// Checks that one executor is installed, logged in and connected to Linear's MCP server.
/// `program` is where the executable was found, `None` when it was not found. The three
/// commands (`--version`, the login check, `mcp list`) run together, since `claude mcp list`
/// alone can take over ten seconds. A check that cannot run adds a problem saying so and leaves
/// the executor not ready, without also reporting the setup as missing. The lean sign-in check
/// does not run here (`SystemReadiness` runs it), so lean runs come out not ready.
pub async fn check_executor(
    kind: ExecutorKind,
    program: Option<PathBuf>,
    runner: &dyn CommandRunner,
) -> ExecutorReport {
    check_executor_detailed(kind, program, runner, None).await.0
}

/// `check_executor`, plus whether every check gave a usable answer, and with the lean checks
/// (lean-run design, section 7) when `lean` is given: the Linear server lean runs load is read
/// from the same `mcp list`, and the lean sign-in check runs as a fourth command, together with
/// the other three. Once that sign-in matches, what the executor's own configuration would change
/// in a lean run is read too: Codex's model provider (`codex_default_provider`), and a gateway or
/// proxy in the `env` of Claude Code's user settings (`claude_user_routing`). A check gives no
/// usable answer when its command could not run (it timed out
/// or did not start, or what it printed could not be read), when the login check's output
/// cannot be understood, or when `mcp list` exited non-zero without showing any Linear line (it
/// crashed). A negative answer that can be read ("not logged in", no Linear server) is usable.
/// `SystemReadiness` retries an incomplete result sooner. The lean sign-in check does not count
/// here: what it could not do is a lean problem only, so the usual readiness and how long it is
/// cached stay as they were.
async fn check_executor_detailed(
    kind: ExecutorKind,
    program: Option<PathBuf>,
    runner: &dyn CommandRunner,
    lean: Option<&LeanCheckPaths>,
) -> (ExecutorReport, bool) {
    let name = kind.program_name();
    let mut report = ExecutorReport {
        kind,
        program: program.clone(),
        version: None,
        logged_in: false,
        linear_mcp: false,
        problems: Vec::new(),
        lean_linear_mcp: false,
        lean_logged_in: false,
        lean_problems: Vec::new(),
    };
    let Some(program) = program else {
        report
            .problems
            .push(format!("`{name}` is not installed or not on PATH"));
        return (report, true);
    };
    let login_args: &[&str] = match kind {
        ExecutorKind::Claude => &["auth", "status"],
        ExecutorKind::Codex => &["login", "status"],
    };
    let (version, login, mcp, lean_login) = tokio::join!(
        runner.run(&program, &["--version"]),
        runner.run(&program, login_args),
        runner.run(&program, &["mcp", "list"]),
        lean_login_check(kind, &program, runner, lean),
    );
    let mut complete = true;
    match version {
        // Cleaned like the problems: the version reaches IPC, `--json` and the panel too.
        Ok(o) if o.code == Some(0) => {
            report.version = Some(first_line(&o.stdout)).filter(|line| !line.is_empty())
        }
        Ok(o) => report.problems.push(with_detail(
            format!("`{name} --version` failed"),
            first_line(&o.stderr),
        )),
        Err(e) => {
            report
                .problems
                .push(format!("`{name} --version` failed: {e}"));
            complete = false;
        }
    }
    match &login {
        Ok(o) => match login_answer(kind, o) {
            Some(logged_in) => {
                report.logged_in = logged_in;
                if !logged_in {
                    report.problems.push(format!("{name} is not logged in"));
                }
            }
            None => {
                report.problems.push(with_detail(
                    format!(
                        "`{name} {}` gave no usable answer (exit code {})",
                        login_args.join(" "),
                        exit_code_text(o.code)
                    ),
                    first_line(&o.stderr),
                ));
                complete = false;
            }
        },
        Err(e) => {
            report.problems.push(format!("login check failed: {e}"));
            complete = false;
        }
    }
    match mcp {
        Ok(o) => {
            report.linear_mcp = mcp_list_has_linear(kind, &o.stdout);
            if !report.linear_mcp {
                let (problem, usable) = linear_problem(kind, &o);
                report.problems.push(problem);
                complete &= usable;
            }
            // An `mcp list` that crashed says nothing about the server lean runs load either;
            // the problem above already says so.
            report.lean_linear_mcp = mcp_list_has_lean_linear(kind, &o.stdout);
            if !report.lean_linear_mcp && (o.code == Some(0) || has_linear_line(&o.stdout)) {
                report.lean_problems.push(lean_linear_problem(kind, &o));
            }
        }
        Err(e) => {
            report
                .problems
                .push(format!("`{name} mcp list` failed: {e}"));
            complete = false;
        }
    }
    // The lean sign-in is judged against the usual one, so only once that says logged in.
    if let (true, Ok(usual)) = (report.logged_in, &login) {
        let signed_in = lean_login.and_then(|lean_out| lean_sign_in(kind, usual, &lean_out));
        // What the executor's own configuration changes that a lean run would leave out.
        let configured = match (kind, lean) {
            (ExecutorKind::Codex, Some(lean)) => {
                codex_default_provider(lean.codex_config.as_deref()).await
            }
            (ExecutorKind::Claude, Some(lean)) => {
                claude_user_routing(lean.claude_settings.as_deref()).await
            }
            _ => Ok(()),
        };
        report.lean_logged_in = signed_in.is_ok() && configured.is_ok();
        report
            .lean_problems
            .extend([signed_in.err(), configured.err()].into_iter().flatten());
    }
    (report, complete)
}

/// Runs the lean sign-in check of `kind`: `CLAUDE_LEAN_LOGIN` in `lean.empty_dir` (made first
/// when missing), `CODEX_LEAN_LOGIN`. `Err` holds the lean problem when it did not run.
async fn lean_login_check(
    kind: ExecutorKind,
    program: &Path,
    runner: &dyn CommandRunner,
    lean: Option<&LeanCheckPaths>,
) -> Result<CmdOutput, String> {
    let Some(lean) = lean else {
        return Err("the lean sign-in check did not run".into());
    };
    let ran = match kind {
        ExecutorKind::Claude => {
            if let Err(e) = tokio::fs::create_dir_all(&lean.empty_dir).await {
                return Err(format!(
                    "could not prepare the folder for the lean sign-in check: {e}"
                ));
            }
            runner
                .run_in(program, &CLAUDE_LEAN_LOGIN, &lean.empty_dir)
                .await
        }
        ExecutorKind::Codex => runner.run(program, &CODEX_LEAN_LOGIN).await,
    };
    ran.map_err(|e| format!("lean sign-in check failed: {e}"))
}

/// Whether a lean run would be signed in the same way as a usual one, from the usual login
/// check's output (which says logged in) and the lean one's; `Err` holds the lean problem.
/// Claude: the lean `auth status` says logged in, with the same `authMethod` and `apiProvider`.
/// Codex: the lean `login status` says logged in, the same way (the text after "Logged in
/// using", compared whole and shown without a key: `codex_shown`). Both answers have to say how:
/// two answers that do not cannot be compared, and a check that cannot be made rules lean runs
/// out (an executor that changed what it prints must not let a lean run sign in some other way
/// unnoticed).
fn lean_sign_in(kind: ExecutorKind, usual: &CmdOutput, lean: &CmdOutput) -> Result<(), String> {
    let name = kind.program_name();
    let not_signed_in = || {
        format!(
            "in lean runs {name} is not signed in (its sign-in comes from the user settings); \
             turn lean runs off for this binding"
        )
    };
    let differently = |lean: String, usual: String| {
        format!(
            "in lean runs {name} would sign in differently ({lean} instead of {usual}); turn lean \
             runs off for this binding"
        )
    };
    let unchecked = |command: &str| {
        format!(
            "`{name} {command}` does not say how {name} signs in, so lean runs cannot be checked; \
             update {name} or turn lean runs off for this binding"
        )
    };
    let no_answer = |args: &[&str]| {
        with_detail(
            format!(
                "`{name} {}` gave no usable answer (exit code {})",
                args.join(" "),
                exit_code_text(lean.code)
            ),
            first_line(&lean.stderr),
        )
    };
    match kind {
        ExecutorKind::Claude => {
            let Some(lean) = claude_sign_in(&lean.stdout) else {
                return Err(no_answer(&CLAUDE_LEAN_LOGIN));
            };
            if !lean.logged_in {
                return Err(not_signed_in());
            }
            let usual = claude_sign_in(&usual.stdout).unwrap_or_default();
            let said = [&lean.method, &lean.provider, &usual.method, &usual.provider];
            if said.iter().any(|value| value.is_none()) {
                return Err(unchecked("auth status"));
            }
            if (&lean.method, &lean.provider) == (&usual.method, &usual.provider) {
                Ok(())
            } else {
                Err(differently(lean.describe(&usual), usual.describe(&lean)))
            }
        }
        ExecutorKind::Codex => {
            match codex_login_answer(lean.code, &lean.stdout, &lean.stderr) {
                None => return Err(no_answer(&CODEX_LEAN_LOGIN)),
                Some(false) => return Err(not_signed_in()),
                Some(true) => {}
            }
            let (Some(lean), Some(usual)) = (
                codex_sign_in_method(&lean.stdout, &lean.stderr),
                codex_sign_in_method(&usual.stdout, &usual.stderr),
            ) else {
                return Err(unchecked("login status"));
            };
            let (lean_shown, usual_shown) = (codex_shown(&lean), codex_shown(&usual));
            if lean == usual {
                Ok(())
            } else if lean_shown == usual_shown {
                Err(format!(
                    "in lean runs {name} would sign in with {lean_shown}, but not the one usual \
                     runs use; turn lean runs off for this binding"
                ))
            } else {
                Err(differently(lean_shown, usual_shown))
            }
        }
    }
}

/// What `claude auth status` says about a sign-in.
#[derive(Debug, Default)]
struct ClaudeSignIn {
    logged_in: bool,
    /// `authMethod`, such as `claude.ai` or `api_key`.
    method: Option<String>,
    /// `apiProvider`, such as `firstParty`.
    provider: Option<String>,
}

impl ClaudeSignIn {
    /// The sign-in as a problem names it next to `other`: its method, and its provider too when
    /// the two differ there.
    fn describe(&self, other: &ClaudeSignIn) -> String {
        let shown = |value: &Option<String>| clean(value.as_deref().unwrap_or("unknown"));
        if self.provider == other.provider {
            shown(&self.method)
        } else {
            format!("{} with {}", shown(&self.method), shown(&self.provider))
        }
    }
}

/// Reads `claude auth status`: a JSON object with a boolean `loggedIn`, and `authMethod` and
/// `apiProvider` when it says them. `None` when the output is not such an object.
fn claude_sign_in(stdout: &str) -> Option<ClaudeSignIn> {
    let stdout = strip_ansi(stdout);
    let value = serde_json::from_str::<serde_json::Value>(stdout.trim()).ok()?;
    let text = |key: &str| value[key].as_str().map(str::to_string);
    Some(ClaudeSignIn {
        logged_in: value["loggedIn"].as_bool()?,
        method: text("authMethod"),
        provider: text("apiProvider"),
    })
}

/// How `codex login status` says Codex is logged in: the text after "Logged in using" on its
/// line, lower-cased, with its spaces evened out (`chatgpt`, `an api key - <masked key>`). Both
/// streams are read, as `codex_login_answer` does. `None` when it does not say.
fn codex_sign_in_method(stdout: &str, stderr: &str) -> Option<String> {
    const LOGGED_IN_USING: &str = "logged in using";
    let text = format!("{}\n{}", strip_ansi(stdout), strip_ansi(stderr)).to_lowercase();
    let (_, rest) = text.split_once(LOGGED_IN_USING)?;
    let method = rest.lines().next().unwrap_or_default();
    Some(clean(
        &method.split_whitespace().collect::<Vec<_>>().join(" "),
    ))
}

/// A Codex sign-in as a problem shows it: the words it starts with, up to the first character
/// that is neither a letter nor a space (`an unknown method` when that leaves nothing). After an
/// API key login codex-cli 0.159.2 prints ` - ` and the key there, masked to its first eight and
/// last five characters (`codex-rs/cli/src/login.rs` at `rust-v0.159.2`, read 2026-10-20);
/// cutting at any such character, not only at ` - `, keeps the key out of the problem also if a
/// later version prints it some other way. Problems reach the panel, the CLI and MCP. The two
/// sign-ins are still compared whole, so another key is another sign-in.
fn codex_shown(method: &str) -> String {
    let words: String = method
        .chars()
        .take_while(|c| c.is_alphabetic() || *c == ' ')
        .collect();
    match words.trim() {
        "" => "an unknown method".to_string(),
        words => words.to_string(),
    }
}

/// Whether Codex's configuration file sets a model provider that lean runs, which ignore the
/// user configuration, would not use; `Err` holds the lean problem. No file sets none.
async fn codex_default_provider(config: Option<&Path>) -> Result<(), String> {
    const UNREADABLE: &str = "could not read the codex configuration to check its model provider";
    let Some(config) = config else {
        return Ok(());
    };
    let text = match tokio::fs::read_to_string(config).await {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{UNREADABLE}: {e}")),
    };
    match codex_custom_provider(&text) {
        Ok(None) => Ok(()),
        Ok(Some(provider)) => Err(format!(
            "codex uses the model provider `{}`; lean runs would use the default provider, so \
             turn lean runs off for this binding",
            clean(&provider)
        )),
        Err(_) => Err(format!("{UNREADABLE}: it is not valid TOML")),
    }
}

/// The variables of the `env` of Claude Code's user settings that send Claude Code through a
/// gateway or a proxy. Lean runs leave the user settings out, so they would connect without
/// them. A switch of the provider (Bedrock, Vertex) or of the sign-in shows in the lean sign-in
/// check instead. The list was decided on 2026-10-20 (the lean-run plan's leftovers): the base
/// URLs, and the proxy variables in their upper- and lower-case spellings.
const CLAUDE_ROUTING_VARS: [&str; 9] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_VERTEX_BASE_URL",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Checks Claude Code's user settings at `settings` for routing variables lean runs would not
/// apply (`claude_routing_env`); `Err` holds the lean problem. A missing file sets none; one that
/// cannot be read, or is not JSON, rules lean runs out, as what it sets cannot be known.
async fn claude_user_routing(settings: Option<&Path>) -> Result<(), String> {
    const UNREADABLE: &str = "could not read the Claude Code user settings to check their `env`";
    let Some(settings) = settings else {
        return Ok(());
    };
    let text = match tokio::fs::read_to_string(settings).await {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{UNREADABLE}: {e}")),
    };
    match claude_routing_env(&text) {
        Ok(vars) if vars.is_empty() => Ok(()),
        Ok(vars) => {
            let named: Vec<String> = vars.iter().map(|var| format!("`{var}`")).collect();
            Err(format!(
                "the Claude Code user settings set {} in `env`, which lean runs do not apply \
                 (they leave the user settings out), so turn lean runs off for this binding",
                named.join(", ")
            ))
        }
        Err(_) => Err(format!("{UNREADABLE}: they are not valid JSON")),
    }
}

/// The routing variables (`CLAUDE_ROUTING_VARS`, in that order) that the `env` of Claude Code's
/// user settings text sets to a non-empty string. `Err` when the text is not JSON, without the
/// parser's message.
pub fn claude_routing_env(settings_text: &str) -> Result<Vec<String>> {
    let settings: serde_json::Value =
        serde_json::from_str(settings_text).map_err(|_| anyhow::anyhow!("not valid JSON"))?;
    let env = &settings["env"];
    Ok(CLAUDE_ROUTING_VARS
        .iter()
        .filter(|var| env[**var].as_str().is_some_and(|value| !value.is_empty()))
        .map(|var| var.to_string())
        .collect())
}

/// The model provider Codex's configuration text chooses when it is not the default
/// (`openai`): the active profile's (`profile = "<name>"` with `[profiles.<name>]`) when that
/// sets one, else the top-level `model_provider`. `Err` when the text is not TOML, without the
/// parser's message: that quotes the file, which can hold a token.
pub fn codex_custom_provider(config_text: &str) -> Result<Option<String>> {
    let config: toml::Table = config_text
        .parse()
        .map_err(|_| anyhow::anyhow!("not valid TOML"))?;
    let provider_in = |table: &toml::Table| {
        table
            .get("model_provider")
            .and_then(toml::Value::as_str)
            .map(str::to_string)
    };
    let profile = config
        .get("profile")
        .and_then(toml::Value::as_str)
        .and_then(|name| config.get("profiles")?.get(name)?.as_table())
        .and_then(provider_in);
    Ok(profile
        .or_else(|| provider_in(&config))
        .filter(|provider| provider != "openai"))
}

/// What a finished login check says: `Some(true)` logged in, `Some(false)` logged out, `None`
/// when the output cannot be read.
fn login_answer(kind: ExecutorKind, output: &CmdOutput) -> Option<bool> {
    match kind {
        ExecutorKind::Claude => claude_login_answer(&output.stdout),
        ExecutorKind::Codex => codex_login_answer(output.code, &output.stdout, &output.stderr),
    }
}

/// `claude auth status` prints a JSON object with a boolean `loggedIn`; `None` for anything else.
fn claude_login_answer(stdout: &str) -> Option<bool> {
    let stdout = strip_ansi(stdout);
    serde_json::from_str::<serde_json::Value>(stdout.trim()).ok()?["loggedIn"].as_bool()
}

/// `codex login status` exits 0 and prints "Logged in using ..." when logged in. Both streams
/// are read because codex-cli 0.159.2 writes the status to stderr (M0 finding F4). `None` when
/// neither "logged in" nor "not logged in" appears (the second contains the first).
fn codex_login_answer(code: Option<i32>, stdout: &str, stderr: &str) -> Option<bool> {
    let text = format!("{}\n{}", strip_ansi(stdout), strip_ansi(stderr)).to_lowercase();
    text.contains("logged in")
        .then(|| code == Some(0) && !text.contains("not logged in"))
}

/// Reads `claude auth status` output, a JSON object with a `loggedIn` boolean. Anything else
/// counts as not logged in.
pub fn parse_claude_auth(stdout: &str) -> bool {
    claude_login_answer(stdout) == Some(true)
}

/// Reads `codex login status`: it exits 0 and prints "Logged in using ...". Both streams are
/// read because codex-cli 0.159.2 writes the status to stderr (M0 finding F4).
pub fn parse_codex_login(code: Option<i32>, stdout: &str, stderr: &str) -> bool {
    codex_login_answer(code, stdout, stderr) == Some(true)
}

/// Whether `mcp list` output shows the Linear server lean runs load (`executor::lean`) usable:
/// a line whose server name is exactly `CLAUDE_LINEAR_SERVER` / `CODEX_LINEAR_SERVER`, whose URL
/// is exactly `LINEAR_MCP_URL`, and whose status is usable as `mcp_list_has_linear` reads it. A
/// lean run reuses the sign-in stored under that name (measured: see the two constants); another
/// server, claude.ai's Linear connector among them, is not loaded. Codex's line must also show
/// `OAuth` as its sign-in (`codex_lean_sign_in_usable`).
pub fn mcp_list_has_lean_linear(kind: ExecutorKind, stdout: &str) -> bool {
    let stdout = strip_ansi(stdout);
    lean_linear_line(kind, &stdout).is_some_and(|(url, line)| {
        url == LINEAR_MCP_URL
            && line_usable(kind, line)
            && (kind != ExecutorKind::Codex || codex_lean_sign_in_usable(line))
    })
}

/// The sign-in a Codex `mcp list` line shows for an enabled HTTP server, in its last column: the
/// words after the URL, the bearer token variable (`-` when there is none) and the status. In
/// codex-cli 0.159.2 that is `OAuth`, `Bearer token`, `Not logged in`, `Unsupported` or
/// `Unknown` (its source, read 2026-10-21: `McpAuthStatus` and rmcp-client's `auth_status.rs`).
fn codex_sign_in(line: &str) -> String {
    line.split_whitespace()
        .skip_while(|token| !is_linear_url(token))
        .skip(3)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a lean Codex run can sign in to the server of this line. It leaves out the user's
/// configuration, so a bearer token variable or an `Authorization` header set there does not
/// reach it, and only a stored OAuth sign-in for that name and URL does. codex-cli 0.159.2 shows
/// `OAuth` exactly when one is stored and usable, and `Bearer token` whenever the configuration
/// sets a token, stored sign-in or not (its source, read 2026-10-21).
fn codex_lean_sign_in_usable(line: &str) -> bool {
    codex_sign_in(line) == "OAuth"
}

/// The server lean runs load in `kind`.
fn lean_server_name(kind: ExecutorKind) -> &'static str {
    match kind {
        ExecutorKind::Claude => CLAUDE_LINEAR_SERVER,
        ExecutorKind::Codex => CODEX_LINEAR_SERVER,
    }
}

/// The first `mcp list` line of the server lean runs load, with the URL (or command) it shows.
fn lean_linear_line(kind: ExecutorKind, stdout: &str) -> Option<(&str, &str)> {
    stdout.lines().find_map(|line| {
        let (server, target) = server_and_target(kind, line)?;
        (server == lean_server_name(kind)).then_some((target, line))
    })
}

/// The server name of an `mcp list` line and the first thing after it, its URL or command.
/// Claude prints `<name>: <url> ...` (a name may hold colons, as `plugin:a:b` does), Codex
/// `<name>  <url>  ...`, as in the M0 samples of Claude Code 2.1.285 and codex-cli 0.159.2
/// (docs/gates/samples/m0/executors/*-mcp-list.txt, 2026-10-10).
fn server_and_target(kind: ExecutorKind, line: &str) -> Option<(&str, &str)> {
    let (server, rest) = match kind {
        ExecutorKind::Claude => line.split_once(": ")?,
        ExecutorKind::Codex => line.trim_start().split_once(char::is_whitespace)?,
    };
    Some((server.trim(), rest.split_whitespace().next()?))
}

/// The lean problem when `mcp list` ran without showing the Linear server lean runs load
/// usable: it is missing, or its name points elsewhere (then adding it would fail, so only the
/// reason is given), or Codex's line shows a sign-in lean runs cannot use (with what to do about
/// it), or what its line says.
fn lean_linear_problem(kind: ExecutorKind, output: &CmdOutput) -> String {
    let name = kind.program_name();
    let server = lean_server_name(kind);
    let stdout = strip_ansi(&output.stdout);
    match lean_linear_line(kind, &stdout) {
        None => {
            let register = match kind {
                ExecutorKind::Claude => "register it for all directories with",
                ExecutorKind::Codex => "register it with",
            };
            format!(
                "lean runs use the Linear MCP server named `{server}` in {name} ({register}: {})",
                register_linear_command(kind)
            )
        }
        Some((target, _)) if target != LINEAR_MCP_URL => format!(
            "the Linear MCP server `{server}` in {name} points to {}, but lean runs need \
             {LINEAR_MCP_URL}; change it yourself or turn lean runs off for this binding",
            clean(target)
        ),
        // Usable for usual runs, which read the configuration that holds a token, but not for
        // lean ones.
        Some((_, line)) if kind == ExecutorKind::Codex && line_usable(kind, line) => {
            match clean(&codex_sign_in(line)) {
                // Signing in alone would not change the column: codex-cli 0.159.2 says `Bearer
                // token` whenever the configuration sets a token, before it looks for a stored
                // sign-in, and `mcp login` stores one all the same (its source, read 2026-10-21).
                sign_in if sign_in == "Bearer token" => format!(
                    "the Linear MCP server `{server}` in {name} signs in with a token from \
                     {name}'s configuration (`Bearer token`), which lean runs leave out; to use \
                     lean runs, remove the token from its entry (`bearer_token_env_var` or an \
                     `Authorization` header) and sign in with `{name} mcp login {server}`, or \
                     turn lean runs off for this binding"
                ),
                sign_in => {
                    let shown = if sign_in.is_empty() {
                        "no sign-in".to_string()
                    } else {
                        format!("`{sign_in}` as its sign-in")
                    };
                    format!(
                        "the Linear MCP server `{server}` in {name} shows {shown}, but lean runs \
                         leave out {name}'s configuration and can only use a stored OAuth \
                         sign-in; sign in with `{name} mcp login {server}`, or turn lean runs off \
                         for this binding"
                    )
                }
            }
        }
        Some((_, line)) => match clean(&line_status(kind, line)) {
            status if status.is_empty() => {
                format!("the Linear MCP server `{server}` in {name} is not connected")
            }
            status => format!("the Linear MCP server `{server}` in {name}: {status}"),
        },
    }
}

/// Whether `mcp list` output has any line that describes Linear's server.
fn has_linear_line(stdout: &str) -> bool {
    linear_lines(&strip_ansi(stdout)).next().is_some()
}

/// Whether `mcp list` output shows Linear's MCP server usable. A line describes Linear only
/// when one of its whitespace-separated tokens is an `http(s)` URL on exactly `mcp.linear.app`;
/// another host, a query parameter or a server name that merely contains that text does not
/// count. Claude then has to say `Connected` after the last ` - ` (a leading check mark or other
/// symbol is ignored); Codex has to show the `enabled` token and not `Not logged in`. Colour
/// codes are stripped first. The formats come from the M0 samples
/// (docs/gates/samples/m0/executors/*-mcp-list.txt, 2026-10-10).
pub fn mcp_list_has_linear(kind: ExecutorKind, stdout: &str) -> bool {
    let stdout = strip_ansi(stdout);
    linear_lines(&stdout).any(|line| line_usable(kind, line))
}

/// The lines of `mcp list` output that describe Linear's server.
fn linear_lines(stdout: &str) -> impl Iterator<Item = &str> {
    stdout
        .lines()
        .filter(|line| line.split_whitespace().any(is_linear_url))
}

fn is_linear_url(token: &str) -> bool {
    Url::parse(token).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https") && url.host_str() == Some(LINEAR_MCP_HOST)
    })
}

fn line_usable(kind: ExecutorKind, line: &str) -> bool {
    match kind {
        ExecutorKind::Claude => {
            claude_status(line).is_some_and(|status| status.eq_ignore_ascii_case("Connected"))
        }
        ExecutorKind::Codex => {
            line.split_whitespace()
                .any(|token| token.eq_ignore_ascii_case("enabled"))
                && !line.to_lowercase().contains("not logged in")
        }
    }
}

/// What a Claude `mcp list` line says about its server: the text after the last ` - `, without
/// the check mark or other symbol that leads it (the Windows console may print a different
/// one). `None` when the line has no such separator.
fn claude_status(line: &str) -> Option<&str> {
    let (_, status) = line.rsplit_once(" - ")?;
    Some(
        status
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim_end(),
    )
}

/// What a Codex `mcp list` row says after the server's URL (state and login), without the `-`
/// that stands for an empty column.
fn codex_status(line: &str) -> String {
    let mut tokens = line
        .split_whitespace()
        .skip_while(|token| !is_linear_url(token));
    tokens.next();
    tokens
        .filter(|token| *token != "-")
        .collect::<Vec<_>>()
        .join(" ")
}

fn line_status(kind: ExecutorKind, line: &str) -> String {
    match kind {
        ExecutorKind::Claude => claude_status(line).unwrap_or_default().to_string(),
        ExecutorKind::Codex => codex_status(line),
    }
}

/// The problem to report when `mcp list` ran without showing a usable Linear server, and whether
/// that is a usable answer. What a Linear line says, or that there is none after a clean exit,
/// is an answer about the setup; without a Linear line the problem also says how to register
/// the server. A command that exited non-zero without any Linear line crashed or was refused:
/// that says nothing about the setup, so it is reported as a failure and is not a usable answer.
fn linear_problem(kind: ExecutorKind, output: &CmdOutput) -> (String, bool) {
    let name = kind.program_name();
    let not_configured = format!("Linear MCP is not configured or not authorized in {name}");
    let stdout = strip_ansi(&output.stdout);
    match linear_lines(&stdout).next() {
        Some(line) => {
            let status = clean(&line_status(kind, line));
            if status.is_empty() {
                (not_configured, true)
            } else {
                (format!("Linear MCP in {name}: {status}"), true)
            }
        }
        None if output.code == Some(0) => (
            format!(
                "{not_configured} (register it for all directories with: {})",
                register_linear_command(kind)
            ),
            true,
        ),
        None => {
            let failed = format!(
                "`{name} mcp list` failed (exit code {})",
                exit_code_text(output.code)
            );
            (with_detail(failed, first_line(&output.stderr)), false)
        }
    }
}

/// The official command that registers Linear's MCP server for every directory (design section
/// 8). Runs happen in the bindings' folders while this check runs in the host's, so a server
/// registered for one directory only would pass here and be missing there; Claude's default
/// scope is the current project, hence `--scope user`; `codex mcp add` writes the user's own
/// configuration.
fn register_linear_command(kind: ExecutorKind) -> String {
    match kind {
        ExecutorKind::Claude => {
            format!("claude mcp add --scope user --transport http linear-server {LINEAR_MCP_URL}")
        }
        ExecutorKind::Codex => format!("codex mcp add linear --url {LINEAR_MCP_URL}"),
    }
}

/// An exit code as a problem shows it; `unknown` when a signal ended the process.
fn exit_code_text(code: Option<i32>) -> String {
    code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
}

/// Command output as a problem or the version may show it: colour codes, other control
/// characters and the characters that reorder or hide text (`text::is_invisible`) dropped, at
/// most `MAX_PROBLEM_TEXT` characters.
fn clean(text: &str) -> String {
    let text: String = strip_ansi(text)
        .chars()
        .filter(|c| !c.is_control() && !is_invisible(*c))
        .collect();
    text.trim().chars().take(MAX_PROBLEM_TEXT).collect()
}

/// The first line of command output that still shows something once `clean`ed, cleaned; empty
/// when there is none. A line that holds only colour codes, control or invisible characters
/// does not count. (No colour code spans two lines: a CSI sequence cannot hold a line break.)
fn first_line(text: &str) -> String {
    text.lines()
        .map(clean)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
}

fn with_detail(what: String, detail: String) -> String {
    if detail.is_empty() {
        what
    } else {
        format!("{what}: {detail}")
    }
}

/// What the Linear workspace is missing for IssueRelay's trigger names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReport {
    /// The trigger label exists.
    pub label_ok: bool,
    /// Entries like `Demo: In Testing` (team name and missing status).
    pub missing_states: Vec<String>,
    /// The names of the teams whose statuses were checked.
    #[serde(default)]
    pub teams_checked: Vec<String>,
}

impl WorkspaceReport {
    /// The label and every status are in place.
    pub fn ok(&self) -> bool {
        self.label_ok && self.missing_states.is_empty()
    }
}

/// Looks for the trigger label and, in the teams of the bound projects, the four statuses
/// IssueRelay moves issues through. A team that never uses IssueRelay need not have them. Every
/// team is looked at before any project is bound, and also when no bound project names a team
/// that Linear lists (Linear sent `teams` as null or empty, no longer lists the project, or no
/// longer lists its teams, as it leaves out archived ones): with nothing to narrow the check down
/// to, no team at all would report a workspace with missing statuses as fine. Names compare like
/// everywhere else (`same_name`). Errors when Linear cannot be queried.
pub async fn check_workspace(
    api: &dyn LinearApi,
    names: &TriggerNames,
    bindings: &[Binding],
) -> Result<WorkspaceReport> {
    let label_ok = api
        .labels()
        .await?
        .iter()
        .any(|l| same_name(l, &names.label));
    let mut teams = api.teams().await?;
    if !bindings.is_empty() {
        let bound: HashSet<&str> = bindings.iter().map(|b| b.project_id.as_str()).collect();
        let team_ids: HashSet<String> = api
            .projects()
            .await?
            .into_iter()
            .filter(|p| bound.contains(p.id.as_str()))
            .flat_map(|p| p.team_ids)
            .collect();
        // Narrowed only when that leaves a team to check.
        if teams.iter().any(|team| team_ids.contains(&team.id)) {
            teams.retain(|team| team_ids.contains(&team.id));
        }
    }
    let mut missing_states = Vec::new();
    for team in &teams {
        let statuses = api.statuses(&team.id).await?;
        for wanted in [
            &names.todo,
            &names.in_progress,
            &names.in_testing,
            &names.done,
        ] {
            if !statuses.iter().any(|s| same_name(s, wanted)) {
                missing_states.push(format!("{}: {wanted}", team.name));
            }
        }
    }
    Ok(WorkspaceReport {
        label_ok,
        missing_states,
        teams_checked: teams.into_iter().map(|team| team.name).collect(),
    })
}

/// Whether one binding can be used right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingReport {
    pub binding_id: String,
    pub project_name: String,
    pub directory_exists: bool,
    /// The executor the binding runs with (its own choice, else the default).
    pub executor: ExecutorKind,
    /// Ready for the binding's runs: lean ones when `lean` applies, else usual ones.
    pub executor_ready: bool,
    /// Not ready only because of the lean checks: the binding's runs are lean, and the executor
    /// is ready for usual runs. Its issues wait with `WaitReason::LeanNotReady`. A host from before
    /// the field sends none, which reads as `false`.
    #[serde(default)]
    pub lean_not_ready: bool,
    /// Whether the binding's runs are lean, or why not.
    pub lean: LeanStatus,
    /// The reasoning effort IssueRelay passes to its runs: its own, else the settings' default;
    /// `None` when neither sets one (the executor's own setting applies). In a Claude run that is
    /// not lean, `CLAUDE_CODE_EFFORT_LEVEL` in the user settings' `env` wins over it (lean-run
    /// design, section 5.6).
    pub effort: Option<Effort>,
    /// The binding's routing labels; empty for the project's default binding. A host from before
    /// routing labels sends none.
    #[serde(default)]
    pub labels: Vec<String>,
}

/// One report per binding, in configuration order. `ready` is what `ready_set` gives; `home` is
/// the user's home folder (a binding of that folder cannot be lean), `None` when not known.
pub fn check_bindings(cfg: &Config, ready: &Readiness, home: Option<&Path>) -> Vec<BindingReport> {
    cfg.bindings
        .iter()
        .map(|b| {
            let executor = cfg.executor_for(b);
            let lean = cfg.lean_for(b, home);
            BindingReport {
                binding_id: b.id.clone(),
                project_name: b.project_name.clone(),
                directory_exists: b.directory.is_dir(),
                executor,
                executor_ready: ready.ready(executor, lean),
                lean_not_ready: ready.held_back_by_lean(executor, lean),
                lean,
                effort: cfg.effort_for(b),
                labels: b.labels.clone(),
            }
        })
        .collect()
}

/// Everything `issuerelay doctor` shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    /// The Linear user the connection is authorized as; `None` when Linear could not be reached.
    pub viewer: Option<Viewer>,
    /// Why Linear could not be reached.
    pub linear_error: Option<String>,
    pub executors: Vec<ExecutorReport>,
    /// `None` when the workspace could not be checked.
    pub workspace: Option<WorkspaceReport>,
    pub bindings: Vec<BindingReport>,
    /// The PATH and proxy executors get, with any user name and password in a proxy URL masked
    /// (`ExecEnv::proxy_for_display`); `None` when the probe does not resolve one.
    #[serde(default)]
    pub env: Option<ExecEnv>,
}

/// Where the engine gets its executor reports. `SystemReadiness` is the real implementation;
/// tests use a fixed fake.
#[async_trait::async_trait]
pub trait ReadinessProbe: Send + Sync {
    /// One report per executor, always Claude first, then Codex. `fresh` asks for current
    /// results (the doctor command wants them): cached ones are skipped, except a result
    /// recorded after the call was made. A `fresh` caller that waited behind a running check
    /// therefore uses that check's result instead of running the commands a second time.
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport>;

    /// The environment executors are found and run with, when this probe resolves one. The proxy
    /// values are the real ones; `Engine::doctor` masks them for the report.
    fn env(&self) -> Option<ExecEnv> {
        None
    }
}

/// One executor's last check as `SystemReadiness` keeps it.
struct CachedCheck {
    report: ExecutorReport,
    /// Every check gave a usable answer (see `check_executor_detailed`).
    complete: bool,
    /// When the check finished.
    at: Instant,
    /// The number of the refresh that recorded it (see `SystemReadiness::refreshes`).
    refresh: u64,
}

/// Finds where an executor is installed; `None` when it is not.
type FindProgram = Arc<dyn Fn(ExecutorKind) -> Option<PathBuf> + Send + Sync>;

/// Reads the sources of the executors' environment again (`env::read_sources`).
type RefreshEnv = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Sources> + Send>> + Send + Sync>;

/// Checks the real `claude` / `codex` on PATH and caches what it finds, one entry per executor.
/// An entry whose checks all gave a usable answer is reused for `ttl`; one with a check that
/// timed out, did not start, crashed or could not be read for at most `FAILED_CHECK_TTL`, so it is
/// retried soon without re-running a slow `claude mcp list` on every poll. Only the executors
/// whose entry expired are checked again, together. One lock covers the whole refresh, so a
/// caller that arrives meanwhile waits and then uses what the refresh recorded. The lean checks
/// run only when `with_lean_checks` gave what they need; without them lean runs are not ready.
pub struct SystemReadiness {
    runner: Arc<dyn CommandRunner>,
    resolve: FindProgram,
    /// Shared with the job runner; executors are looked up on its PATH.
    env: SharedEnv,
    refresh_env: RefreshEnv,
    lean: Option<LeanCheckPaths>,
    ttl: Duration,
    failed_ttl: Duration,
    /// How many refreshes have been recorded. A `fresh` caller reads it before it waits for the
    /// lock, so an entry numbered above that value was recorded after the call, even when the
    /// clock did not move in between.
    refreshes: AtomicU64,
    cache: tokio::sync::Mutex<HashMap<ExecutorKind, CachedCheck>>,
}

impl SystemReadiness {
    /// `ttl` is how long `executor_reports(false)` reuses a check whose answers were all usable.
    /// `env` is shared with the job runner: executors are looked up on its PATH, check commands
    /// run with it, and every fresh check resolves it again first.
    pub fn new(ttl: Duration, env: SharedEnv) -> Self {
        Self::with_refresh(ttl, env, Arc::new(|| Box::pin(crate::env::read_sources())))
    }

    /// `new` with another way to read the sources of the environment again; the tests use it so
    /// that no login shell runs. The check commands and the lookup of the executables are wired
    /// as `new` does.
    fn with_refresh(ttl: Duration, env: SharedEnv, refresh_env: RefreshEnv) -> Self {
        let lookup = env.clone();
        Self::with_parts(
            ttl,
            FAILED_CHECK_TTL,
            Arc::new(SystemRunner::with_env(env.clone())),
            Arc::new(move |kind| {
                let path = lookup
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .path
                    .clone();
                executor::resolve_program_in(kind, path.as_deref()).ok()
            }),
            env,
            refresh_env,
        )
    }

    /// The seams the unit tests use: both lifetimes, the command runner, where to find the
    /// executables, the shared environment and how to resolve it again.
    fn with_parts(
        ttl: Duration,
        failed_ttl: Duration,
        runner: Arc<dyn CommandRunner>,
        resolve: FindProgram,
        env: SharedEnv,
        refresh_env: RefreshEnv,
    ) -> Self {
        Self {
            runner,
            resolve,
            env,
            refresh_env,
            lean: None,
            ttl,
            failed_ttl,
            refreshes: AtomicU64::new(0),
            cache: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Runs the lean checks too, with `paths` (the host's own folder and Codex's configuration
    /// file).
    pub fn with_lean_checks(mut self, paths: LeanCheckPaths) -> Self {
        self.lean = Some(paths);
        self
    }

    /// Whether `cached` may still be served to a caller that did not ask for `fresh`.
    fn within_ttl(&self, cached: &CachedCheck, now: Instant) -> bool {
        let life = if cached.complete {
            self.ttl
        } else {
            self.ttl.min(self.failed_ttl)
        };
        now.saturating_duration_since(cached.at) < life
    }

    /// Checks `kind` when `due`; `None` when its cache entry is still good.
    async fn check_if_due(
        &self,
        kind: ExecutorKind,
        due: bool,
    ) -> Option<(ExecutorReport, bool, Instant)> {
        if !due {
            return None;
        }
        let program = (self.resolve)(kind);
        let (report, complete) =
            check_executor_detailed(kind, program, self.runner.as_ref(), self.lean.as_ref()).await;
        Some((report, complete, Instant::now()))
    }
}

#[async_trait::async_trait]
impl ReadinessProbe for SystemReadiness {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        // Read before waiting for the lock: an entry recorded after this point is newer than
        // the call.
        let asked_after = self.refreshes.load(Ordering::SeqCst);
        // The lock stays held while checking, so a caller that arrives meanwhile waits for the
        // refresh instead of starting a second round of commands.
        let mut cache = self.cache.lock().await;
        // A fresh check looks at the PATH and proxy again too, before the executors are found:
        // installing `claude` or changing the proxy shows up without restarting the host (Unix:
        // the login shell is read again; Windows: the registry). A source that cannot be read
        // this time keeps what it gave before (`env::combine`), so a slow rc file does not make
        // a good environment worse.
        if fresh {
            let sources = (self.refresh_env)().await;
            let mut env = self.env.write().unwrap_or_else(PoisonError::into_inner);
            let resolved = crate::env::combine(&sources, Some(&env));
            *env = resolved;
        }
        let now = Instant::now();
        let due = |kind: ExecutorKind| match cache.get(&kind) {
            None => true,
            Some(cached) if fresh => cached.refresh <= asked_after,
            Some(cached) => !self.within_ttl(cached, now),
        };
        let (due_claude, due_codex) = (due(ExecutorKind::Claude), due(ExecutorKind::Codex));
        if due_claude || due_codex {
            // Checked together; an entry is only replaced once its new check has finished, so a
            // cancelled call leaves the cache as it was.
            let (claude, codex) = tokio::join!(
                self.check_if_due(ExecutorKind::Claude, due_claude),
                self.check_if_due(ExecutorKind::Codex, due_codex),
            );
            let refresh = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
            for (kind, checked) in [(ExecutorKind::Claude, claude), (ExecutorKind::Codex, codex)] {
                if let Some((report, complete, at)) = checked {
                    if let Some(before) = cache.get(&kind) {
                        log_readiness_change(&before.report, &report);
                    }
                    cache.insert(
                        kind,
                        CachedCheck {
                            report,
                            complete,
                            at,
                            refresh,
                        },
                    );
                }
            }
        }
        [ExecutorKind::Claude, ExecutorKind::Codex]
            .iter()
            .map(|kind| cache[kind].report.clone())
            .collect()
    }

    fn env(&self) -> Option<ExecEnv> {
        Some(
            self.env
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
    }
}

/// What an executor's check lets it take: no runs, usual runs only, or lean runs too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    NotReady,
    UsualOnly,
    Ready,
}

impl Standing {
    fn of(report: &ExecutorReport) -> Self {
        match (report.ready(), report.ready_lean()) {
            (false, _) => Standing::NotReady,
            (true, false) => Standing::UsualOnly,
            (true, true) => Standing::Ready,
        }
    }
}

/// Writes into the host's log what a check changed in the runs an executor can take, against
/// the check before it: a warning naming the problems when it can take fewer, a line when it can
/// take more. A check that changes nothing writes nothing, so checking every poll does not fill
/// the log. Without this, a check that failed once left its reasons only in `doctor`
/// (2026-10-21, while looking into N5 of the lean-run acceptance).
fn log_readiness_change(before: &ExecutorReport, after: &ExecutorReport) {
    let executor = after.kind.as_str();
    let problems = |problems: &[String]| problems.join("; ");
    match (Standing::of(before), Standing::of(after)) {
        (was, now) if was == now => {}
        (_, Standing::NotReady) => tracing::warn!(
            executor,
            problems = %problems(&after.problems),
            "executor no longer ready"
        ),
        (Standing::Ready, Standing::UsualOnly) => tracing::warn!(
            executor,
            problems = %problems(&after.lean_problems),
            "executor no longer ready for lean runs"
        ),
        (_, Standing::UsualOnly) => tracing::info!(
            executor,
            problems = %problems(&after.lean_problems),
            "executor ready now, but not for lean runs"
        ),
        (_, Standing::Ready) => tracing::info!(executor, "executor ready now"),
    }
}

/// The executors whose report says they are ready, for usual runs and for lean ones.
pub fn ready_set(reports: &[ExecutorReport]) -> Readiness {
    let ready_by = |ready: fn(&ExecutorReport) -> bool| {
        reports
            .iter()
            .filter(|r| ready(r))
            .map(|r| r.kind)
            .collect()
    };
    Readiness {
        full: ready_by(ExecutorReport::ready),
        lean: ready_by(ExecutorReport::ready_lean),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Answers from a table of replies and records every command it is asked to run.
    ///
    /// A key is `<program> <args>` (`claude mcp list`) or only the args, which answer for any
    /// program. A command without a reply fails, like a timeout would. Each command takes
    /// `delay` of (paused) time.
    struct FakeRunner {
        replies: Mutex<HashMap<String, CmdOutput>>,
        calls: Mutex<Vec<String>>,
        /// The folders `run_in` was asked to run in, in order.
        dirs: Mutex<Vec<PathBuf>>,
        delay: Duration,
    }

    impl FakeRunner {
        fn with_delay(mut self, delay: Duration) -> Self {
            self.delay = delay;
            self
        }

        fn without(self, command: &str) -> Self {
            self.replies.lock().unwrap().remove(command);
            self
        }

        fn set(&self, command: &str, reply: CmdOutput) {
            self.replies.lock().unwrap().insert(command.into(), reply);
        }

        /// The commands run since the last call, sorted.
        fn take_calls(&self) -> Vec<String> {
            let mut calls = std::mem::take(&mut *self.calls.lock().unwrap());
            calls.sort();
            calls
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
            let args = args.join(" ");
            let command = format!("{} {args}", program.display());
            self.calls.lock().unwrap().push(command.clone());
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let replies = self.replies.lock().unwrap();
            replies
                .get(&command)
                .or_else(|| replies.get(&args))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unexpected command {command:?}"))
        }

        async fn run_in(&self, program: &Path, args: &[&str], cwd: &Path) -> Result<CmdOutput> {
            self.dirs.lock().unwrap().push(cwd.to_path_buf());
            self.run(program, args).await
        }
    }

    /// Holds every command until `parties` of them are running at once, so a check can only
    /// finish if its commands really run together.
    struct BarrierRunner {
        barrier: tokio::sync::Barrier,
        inner: FakeRunner,
    }

    impl BarrierRunner {
        fn new(parties: usize, inner: FakeRunner) -> Self {
            Self {
                barrier: tokio::sync::Barrier::new(parties),
                inner,
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for BarrierRunner {
        async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
            self.barrier.wait().await;
            self.inner.run(program, args).await
        }
    }

    fn out(code: i32, stdout: &str) -> CmdOutput {
        CmdOutput {
            code: Some(code),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    fn runner(pairs: &[(&str, CmdOutput)]) -> FakeRunner {
        FakeRunner {
            replies: Mutex::new(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
            dirs: Mutex::new(Vec::new()),
            delay: Duration::ZERO,
        }
    }

    const CLAUDE_OK: &str = "linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected";
    const CLAUDE_AUTH: &str =
        "linear-server: https://mcp.linear.app/mcp (HTTP) - ! Needs authentication";
    const CODEX_OK: &str = "linear  https://mcp.linear.app/mcp  -  enabled  OAuth";

    /// Both executors installed, logged in and connected to Linear.
    fn both_ready() -> FakeRunner {
        runner(&[
            ("claude --version", out(0, "2.1.285 (Claude Code)")),
            ("claude auth status", out(0, r#"{"loggedIn": true}"#)),
            ("claude mcp list", out(0, CLAUDE_OK)),
            ("codex --version", out(0, "codex-cli 0.159.2")),
            ("codex login status", out(0, "Logged in using ChatGPT")),
            ("codex mcp list", out(0, CODEX_OK)),
        ])
    }

    const CLAUDE_THREE: [&str; 3] = ["claude --version", "claude auth status", "claude mcp list"];
    const CODEX_THREE: [&str; 3] = ["codex --version", "codex login status", "codex mcp list"];
    const ALL_SIX: [&str; 6] = [
        "claude --version",
        "claude auth status",
        "claude mcp list",
        "codex --version",
        "codex login status",
        "codex mcp list",
    ];

    /// `check_executor_detailed` on a program named like the executor.
    async fn detailed(kind: ExecutorKind, replies: &[(&str, CmdOutput)]) -> (ExecutorReport, bool) {
        let program = Some(PathBuf::from(kind.program_name()));
        check_executor_detailed(kind, program, &runner(replies), None).await
    }

    #[tokio::test]
    async fn claude_ready_when_logged_in_and_linear_connected() {
        let r = runner(&[
            (
                "--version",
                out(0, "  2.1.285 (Claude Code)  \nUpdate available\n"),
            ),
            (
                "auth status",
                out(0, r#"{"loggedIn": true, "authMethod": "claude.ai"}"#),
            ),
            ("mcp list", out(0, CLAUDE_OK)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(report.ready(), "{report:?}");
        assert_eq!(report.version.as_deref(), Some("2.1.285 (Claude Code)"));
        assert!(report.problems.is_empty());
    }

    // The version reaches IPC, `--json` and the panel like the problems do, so it is cleaned the
    // same way: a control character or a direction override must not come through.
    #[tokio::test]
    async fn the_version_is_cleaned_like_the_problems() {
        let replies = |version: &str| {
            [
                ("--version", out(0, version)),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", out(0, CLAUDE_OK)),
            ]
        };
        let (report, _) = detailed(
            ExecutorKind::Claude,
            &replies("2.1.285\u{7}\u{202e} (Claude\u{200b} Code)\nsecond line\n"),
        )
        .await;
        assert_eq!(report.version.as_deref(), Some("2.1.285 (Claude Code)"));
        // The first line that shows anything is the version; nothing at all is no version.
        let (report, _) = detailed(ExecutorKind::Claude, &replies("\n\u{1b}[0m\n2.1.285\n")).await;
        assert_eq!(report.version.as_deref(), Some("2.1.285"));
        // A line that shows nothing once cleaned is passed over too.
        let (report, _) = detailed(ExecutorKind::Claude, &replies("\u{202e}\n2.1.285\n")).await;
        assert_eq!(report.version.as_deref(), Some("2.1.285"));
        let (report, _) = detailed(ExecutorKind::Claude, &replies("\u{202e}\n")).await;
        assert_eq!(report.version, None);
    }

    #[tokio::test]
    async fn claude_needing_linear_auth_is_not_ready() {
        let r = runner(&[
            ("--version", out(0, "2.1.285")),
            ("auth status", out(0, r#"{"loggedIn": true}"#)),
            ("mcp list", out(0, CLAUDE_AUTH)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(!report.ready());
        assert!(report.problems.iter().any(|p| p.contains("Linear MCP")));
    }

    #[tokio::test]
    async fn logged_out_claude_is_not_ready() {
        let r = runner(&[
            ("--version", out(0, "2.1.285")),
            (
                "auth status",
                out(0, r#"{"loggedIn": false, "authMethod": "none"}"#),
            ),
            ("mcp list", out(0, CLAUDE_OK)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(!report.logged_in && !report.ready());
    }

    #[tokio::test]
    async fn codex_ready_and_logged_out_cases() {
        let ok = runner(&[
            ("--version", out(0, "codex-cli 0.159.2")),
            ("login status", out(0, "Logged in using ChatGPT")),
            (
                "mcp list",
                out(
                    0,
                    "Name    Url                         Status\nlinear  https://mcp.linear.app/mcp  enabled",
                ),
            ),
        ]);
        assert!(
            check_executor(ExecutorKind::Codex, Some(PathBuf::from("codex")), &ok)
                .await
                .ready()
        );
        // Real codex-cli 0.159.2 prints the login status on stderr (M0 finding F4).
        let stderr_only = runner(&[
            ("--version", out(0, "codex-cli 0.159.2")),
            (
                "login status",
                CmdOutput {
                    code: Some(0),
                    stdout: String::new(),
                    stderr: "Logged in using ChatGPT\n".into(),
                },
            ),
            (
                "mcp list",
                out(0, "linear  https://mcp.linear.app/mcp  -  enabled  OAuth"),
            ),
        ]);
        assert!(
            check_executor(
                ExecutorKind::Codex,
                Some(PathBuf::from("codex")),
                &stderr_only
            )
            .await
            .ready()
        );
        let logged_out = runner(&[
            ("--version", out(0, "codex-cli 0.159.2")),
            ("login status", out(1, "Not logged in")),
            (
                "mcp list",
                out(0, "linear  https://mcp.linear.app/mcp  enabled"),
            ),
        ]);
        assert!(
            !check_executor(
                ExecutorKind::Codex,
                Some(PathBuf::from("codex")),
                &logged_out
            )
            .await
            .ready()
        );
    }

    #[tokio::test]
    async fn missing_program_is_reported_without_running_anything() {
        let report = check_executor(ExecutorKind::Codex, None, &runner(&[])).await;
        assert!(!report.ready());
        assert!(report.problems[0].contains("not installed"));
    }

    #[tokio::test]
    async fn a_check_that_cannot_run_is_not_reported_as_missing_setup() {
        // FakeRunner fails every command it was not given, like a timeout would.
        let r = runner(&[("--version", out(0, "2.1.285"))]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert!(!report.ready());
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.starts_with("login check failed")),
            "{report:?}"
        );
        assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("mcp list` failed")),
            "{report:?}"
        );
        assert!(
            !report
                .problems
                .iter()
                .any(|p| p.contains("is not logged in")),
            "{report:?}"
        );
        assert!(
            !report.problems.iter().any(|p| p.contains("not configured")),
            "{report:?}"
        );
    }

    #[tokio::test]
    async fn a_failing_version_command_adds_a_problem_and_no_version() {
        let r = runner(&[
            (
                "--version",
                CmdOutput {
                    code: Some(1),
                    stdout: String::new(),
                    stderr: "boom\nsecond line\n".into(),
                },
            ),
            ("auth status", out(0, r#"{"loggedIn": true}"#)),
            ("mcp list", out(0, CLAUDE_OK)),
        ]);
        let report = check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &r).await;
        assert_eq!(report.version, None);
        // The problem quotes the first line of stderr.
        assert_eq!(report.problems, ["`claude --version` failed: boom"]);
        // The version is informational: only login and the Linear server decide readiness.
        assert!(report.ready(), "{report:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn check_executor_runs_its_three_commands_together() {
        // Every command waits for the other two, so a one-after-the-other version never ends.
        let runner = BarrierRunner::new(3, both_ready());
        let checked = tokio::time::timeout(
            Duration::from_secs(5),
            check_executor(ExecutorKind::Claude, Some(PathBuf::from("claude")), &runner),
        )
        .await;
        let report = checked.expect("the three commands must run at the same time");
        assert!(report.ready(), "{report:?}");
    }

    #[tokio::test]
    async fn a_linear_server_that_is_not_connected_reports_its_status() {
        let line =
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✘ Failed to connect — HTTP 500";
        let (report, complete) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", out(0, line)),
            ],
        )
        .await;
        assert!(!report.linear_mcp && !report.ready());
        assert_eq!(
            report.problems,
            ["Linear MCP in claude: Failed to connect — HTTP 500"]
        );
        // A status that can be read is a usable answer, even a bad one.
        assert!(complete);
    }

    #[tokio::test]
    async fn a_codex_linear_server_that_is_not_usable_reports_its_state() {
        for (row, problem) in [
            (
                "linear  https://mcp.linear.app/mcp  -  enabled  Not logged in",
                "Linear MCP in codex: enabled Not logged in",
            ),
            (
                "linear  https://mcp.linear.app/mcp  -  disabled  OAuth",
                "Linear MCP in codex: disabled OAuth",
            ),
        ] {
            let (report, complete) = detailed(
                ExecutorKind::Codex,
                &[
                    ("--version", out(0, "codex-cli 0.159.2")),
                    ("login status", out(0, "Logged in using ChatGPT")),
                    ("mcp list", out(0, row)),
                ],
            )
            .await;
            assert!(!report.linear_mcp && !report.ready(), "{row}");
            assert_eq!(report.problems, [problem], "{row}");
            assert!(complete, "{row}");
        }
    }

    #[tokio::test]
    async fn without_a_linear_line_the_problem_is_not_configured() {
        let slack = "claude.ai Slack: https://mcp.slack.com/mcp - ! Needs authentication";
        let lookalike = "notes  https://mcp.linear.app.example.com/mcp  -  enabled  OAuth";
        // A Linear line that shows no status gives nothing to quote either.
        let no_status = "linear-server: https://mcp.linear.app/mcp (HTTP)";
        for (kind, login, mcp_list) in [
            (
                ExecutorKind::Claude,
                ("auth status", r#"{"loggedIn": true}"#),
                slack,
            ),
            (
                ExecutorKind::Claude,
                ("auth status", r#"{"loggedIn": true}"#),
                "",
            ),
            (
                ExecutorKind::Claude,
                ("auth status", r#"{"loggedIn": true}"#),
                no_status,
            ),
            (
                ExecutorKind::Codex,
                ("login status", "Logged in using ChatGPT"),
                lookalike,
            ),
        ] {
            let (report, complete) = detailed(
                kind,
                &[
                    ("--version", out(0, "1.0.0")),
                    (login.0, out(0, login.1)),
                    ("mcp list", out(0, mcp_list)),
                ],
            )
            .await;
            let name = kind.program_name();
            let not_configured =
                format!("Linear MCP is not configured or not authorized in {name}");
            // Without a Linear line the problem says how to register the server for every
            // folder; a Linear line without a status means it is registered already.
            let expected = if mcp_list == no_status {
                not_configured
            } else {
                format!(
                    "{not_configured} (register it for all directories with: {})",
                    register_linear_command(kind)
                )
            };
            assert_eq!(report.problems, [expected]);
            assert!(complete);
        }
    }

    #[test]
    fn the_registration_hint_is_the_official_command_for_every_directory() {
        assert_eq!(
            register_linear_command(ExecutorKind::Claude),
            "claude mcp add --scope user --transport http linear-server https://mcp.linear.app/mcp"
        );
        assert_eq!(
            register_linear_command(ExecutorKind::Codex),
            "codex mcp add linear --url https://mcp.linear.app/mcp"
        );
    }

    #[test]
    fn the_registration_hint_is_the_setup_command_for_every_directory() {
        // The hint is the command a user types by hand, and the one `setup` runs: they must agree.
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let args = crate::setup::arguments(crate::setup::SetupAction::AddLinearMcp(kind), None)
                .unwrap();
            assert_eq!(
                register_linear_command(kind),
                format!("{} {}", kind.program_name(), args.join(" "))
            );
        }
    }

    #[tokio::test]
    async fn a_status_in_a_problem_is_one_clean_short_line() {
        // A bell and a tab are control characters; the rest is far longer than the limit.
        let long = format!("Failed to connect \u{7}\t{}", "y".repeat(300));
        let line = format!("linear-server: https://mcp.linear.app/mcp (HTTP) - ✘ {long}");
        let (report, _) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", out(0, &line)),
            ],
        )
        .await;
        let expected: String = "Failed to connect "
            .chars()
            .chain(std::iter::repeat('y'))
            .take(200)
            .collect();
        assert_eq!(
            report.problems,
            [format!("Linear MCP in claude: {expected}")]
        );
    }

    #[test]
    fn command_output_in_problems_is_trimmed_clean_and_limited() {
        // A colour code goes away whole; any other control character is dropped on its own.
        assert_eq!(clean("  a\u{1b}[31mb\tc\r  "), "abc");
        assert_eq!(clean("a\u{1b}xb"), "axb");
        // So are the characters that reorder or hide text (`text::is_invisible`).
        assert_eq!(clean("a\u{202e}b\u{200b}c\u{2066}d"), "abcd");
        assert_eq!(clean(&"x".repeat(300)).chars().count(), 200);
        // The limit counts characters, not bytes.
        assert_eq!(clean(&"—".repeat(300)).chars().count(), 200);
        assert_eq!(first_line("\n  \n  first\nsecond"), "first");
        assert_eq!(first_line("a\u{7}b"), "ab");
        assert_eq!(first_line(" \n "), "");
        // A line that is only colour codes is empty, so the next line is the first one.
        assert_eq!(
            first_line("\u{1b}[0m\n\u{1b}[31mboom\u{1b}[0m\nsecond"),
            "boom"
        );
        // So is a line of control or invisible characters only.
        assert_eq!(first_line("\u{7}\t\nboom"), "boom");
        assert_eq!(first_line("\u{202e}\u{200b}\n2.1.285"), "2.1.285");
    }

    #[tokio::test]
    async fn coloured_output_is_read_like_plain_output() {
        // What the CLIs print into a pipe when the environment forces colour.
        let (claude, complete) = detailed(
            ExecutorKind::Claude,
            &[
                (
                    "--version",
                    out(0, "\u{1b}[1m2.1.285\u{1b}[22m (Claude Code)\n"),
                ),
                (
                    "auth status",
                    out(0, "\u{1b}[32m{\"loggedIn\": true}\u{1b}[0m"),
                ),
                (
                    "mcp list",
                    out(
                        0,
                        "\u{1b}[2mChecking MCP server health…\u{1b}[22m\n\nlinear-server: \u{1b}[36mhttps://mcp.linear.app/mcp\u{1b}[39m (HTTP) - \u{1b}[32m✔ Connected\u{1b}[39m\n",
                    ),
                ),
            ],
        )
        .await;
        assert!(claude.ready() && complete, "{claude:?}");
        assert_eq!(claude.version.as_deref(), Some("2.1.285 (Claude Code)"));
        assert!(claude.problems.is_empty(), "{claude:?}");

        let coloured_login = |text: &str, code| CmdOutput {
            code: Some(code),
            stdout: String::new(),
            stderr: text.into(),
        };
        let row = "linear  https://mcp.linear.app/mcp  -  \u{1b}[32menabled\u{1b}[0m  OAuth";
        // The colour codes sit inside the phrase, so it is only found once they are gone. The
        // status is on stderr in codex-cli 0.159.2, but both streams are read.
        let logged_in = "\u{1b}[32mLogged\u{1b}[0m \u{1b}[1min\u{1b}[22m using ChatGPT\n";
        for login in [
            coloured_login(logged_in, 0),
            CmdOutput {
                code: Some(0),
                stdout: logged_in.into(),
                stderr: String::new(),
            },
        ] {
            let (codex, complete) = detailed(
                ExecutorKind::Codex,
                &[
                    ("--version", out(0, "codex-cli 0.159.2")),
                    ("login status", login),
                    ("mcp list", out(0, row)),
                ],
            )
            .await;
            assert!(codex.ready() && complete, "{codex:?}");
        }

        // A coloured "logged out" answer is still a logged-out answer, not an unreadable one.
        let (codex, complete) = detailed(
            ExecutorKind::Codex,
            &[
                ("--version", out(0, "codex-cli 0.159.2")),
                (
                    "login status",
                    coloured_login("\u{1b}[31mNot\u{1b}[0m logged \u{1b}[1min\u{1b}[22m\n", 1),
                ),
                ("mcp list", out(0, row)),
            ],
        )
        .await;
        assert_eq!(codex.problems, ["codex is not logged in"]);
        assert!(complete);
    }

    #[tokio::test]
    async fn colour_codes_do_not_reach_the_text_of_a_problem() {
        let claude_line = "linear-server: \u{1b}[36mhttps://mcp.linear.app/mcp\u{1b}[39m (HTTP) - \u{1b}[31m✘ Failed to connect — HTTP 500\u{1b}[39m";
        let (claude, _) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", out(0, claude_line)),
            ],
        )
        .await;
        assert_eq!(
            claude.problems,
            ["Linear MCP in claude: Failed to connect — HTTP 500"]
        );

        let codex_row = "linear  https://mcp.linear.app/mcp  -  \u{1b}[32menabled\u{1b}[0m  \u{1b}[31mNot logged in\u{1b}[0m";
        let (codex, _) = detailed(
            ExecutorKind::Codex,
            &[
                ("--version", out(0, "codex-cli 0.159.2")),
                ("login status", out(0, "Logged in using ChatGPT")),
                ("mcp list", out(0, codex_row)),
            ],
        )
        .await;
        assert_eq!(
            codex.problems,
            ["Linear MCP in codex: enabled Not logged in"]
        );

        // The stderr line a problem quotes is cleaned the same way.
        let (claude, _) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                (
                    "auth status",
                    CmdOutput {
                        code: Some(1),
                        stdout: "weird".into(),
                        stderr: "\u{1b}[0m\n\u{1b}[31mboom\u{1b}[0m\n".into(),
                    },
                ),
                ("mcp list", out(0, CLAUDE_OK)),
            ],
        )
        .await;
        assert_eq!(
            claude.problems,
            ["`claude auth status` gave no usable answer (exit code 1): boom"]
        );
    }

    #[tokio::test]
    async fn a_crashed_mcp_list_is_a_failure_not_a_missing_server() {
        let crashed = |code, stderr: &str| CmdOutput {
            code,
            stdout: String::new(),
            stderr: stderr.into(),
        };
        let cases = [
            (
                ExecutorKind::Claude,
                ("auth status", r#"{"loggedIn": true}"#),
                crashed(Some(1), "boom\nsecond line\n"),
                "`claude mcp list` failed (exit code 1): boom",
            ),
            (
                ExecutorKind::Codex,
                ("login status", "Logged in using ChatGPT"),
                crashed(Some(2), "\u{1b}[31mboom\u{1b}[0m"),
                "`codex mcp list` failed (exit code 2): boom",
            ),
            (
                // Ended by a signal, nothing on stderr: there is no line to quote.
                ExecutorKind::Claude,
                ("auth status", r#"{"loggedIn": true}"#),
                crashed(None, ""),
                "`claude mcp list` failed (exit code unknown)",
            ),
        ];
        for (kind, login, mcp_list, problem) in cases {
            let (report, complete) = detailed(
                kind,
                &[
                    ("--version", out(0, "1.0.0")),
                    (login.0, out(0, login.1)),
                    ("mcp list", mcp_list),
                ],
            )
            .await;
            assert!(!report.linear_mcp && !report.ready(), "{problem}");
            assert_eq!(report.problems, [problem]);
            // It says nothing about the setup, so the check is retried soon.
            assert!(!complete, "{problem}");
        }

        // A clean exit with no Linear line is a real answer: the server is not set up.
        for (kind, login, name) in [
            (
                ExecutorKind::Claude,
                ("auth status", r#"{"loggedIn": true}"#),
                "claude",
            ),
            (
                ExecutorKind::Codex,
                ("login status", "Logged in using ChatGPT"),
                "codex",
            ),
        ] {
            let (report, complete) = detailed(
                kind,
                &[
                    ("--version", out(0, "1.0.0")),
                    (login.0, out(0, login.1)),
                    ("mcp list", out(0, "")),
                ],
            )
            .await;
            assert_eq!(
                report.problems,
                [format!(
                    "Linear MCP is not configured or not authorized in {name} (register it for \
                     all directories with: {name} mcp add {})",
                    if name == "claude" {
                        "--scope user --transport http linear-server https://mcp.linear.app/mcp"
                    } else {
                        "linear --url https://mcp.linear.app/mcp"
                    }
                )]
            );
            assert!(complete);
        }

        // The exit code does not outweigh what the output shows: a usable Linear line stays
        // usable, and a Linear line with a bad status keeps that status.
        let mut exit_1 = out(1, CLAUDE_OK);
        let (report, complete) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", exit_1.clone()),
            ],
        )
        .await;
        assert!(report.ready() && complete, "{report:?}");
        assert!(report.problems.is_empty());
        exit_1.stdout =
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✘ Failed to connect — HTTP 500"
                .into();
        let (report, complete) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", exit_1),
            ],
        )
        .await;
        assert_eq!(
            report.problems,
            ["Linear MCP in claude: Failed to connect — HTTP 500"]
        );
        assert!(complete);
    }

    #[tokio::test]
    async fn an_unreadable_login_answer_is_not_reported_as_logged_out() {
        let version = ("--version", out(0, "1.0.0"));
        let cases = [
            (
                ExecutorKind::Claude,
                (
                    "auth status",
                    CmdOutput {
                        code: Some(1),
                        stdout: "not json".into(),
                        stderr: "boom\nsecond line\n".into(),
                    },
                ),
                ("mcp list", out(0, CLAUDE_OK)),
                "`claude auth status` gave no usable answer (exit code 1): boom",
            ),
            (
                // `loggedIn` is there but is not a boolean.
                ExecutorKind::Claude,
                ("auth status", out(0, r#"{"loggedIn": "yes"}"#)),
                ("mcp list", out(0, CLAUDE_OK)),
                "`claude auth status` gave no usable answer (exit code 0)",
            ),
            (
                ExecutorKind::Codex,
                (
                    "login status",
                    CmdOutput {
                        code: Some(2),
                        stdout: String::new(),
                        stderr: "error: unexpected argument\n".into(),
                    },
                ),
                ("mcp list", out(0, CODEX_OK)),
                "`codex login status` gave no usable answer (exit code 2): error: unexpected argument",
            ),
            (
                ExecutorKind::Codex,
                (
                    "login status",
                    CmdOutput {
                        code: None,
                        stdout: "weird".into(),
                        stderr: String::new(),
                    },
                ),
                ("mcp list", out(0, CODEX_OK)),
                "`codex login status` gave no usable answer (exit code unknown)",
            ),
        ];
        for (kind, login, mcp_list, problem) in cases {
            let (report, complete) = detailed(kind, &[version.clone(), login, mcp_list]).await;
            assert!(!report.logged_in && !report.ready(), "{problem}");
            assert_eq!(report.problems, [problem]);
            assert!(!complete, "{problem}");
        }
    }

    #[tokio::test]
    async fn a_readable_logged_out_answer_stays_a_logged_out_problem() {
        let (claude, claude_complete) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": false}"#)),
                ("mcp list", out(0, CLAUDE_OK)),
            ],
        )
        .await;
        assert_eq!(claude.problems, ["claude is not logged in"]);
        let (codex, codex_complete) = detailed(
            ExecutorKind::Codex,
            &[
                ("--version", out(0, "codex-cli 0.159.2")),
                ("login status", out(1, "Not logged in")),
                ("mcp list", out(0, CODEX_OK)),
            ],
        )
        .await;
        assert_eq!(codex.problems, ["codex is not logged in"]);
        assert!(claude_complete && codex_complete);
    }

    #[tokio::test]
    async fn only_commands_that_could_not_run_or_be_read_leave_a_check_incomplete() {
        let replies = [
            ("--version", out(0, "2.1.285")),
            ("auth status", out(0, r#"{"loggedIn": true}"#)),
            ("mcp list", out(0, CLAUDE_OK)),
        ];
        let (report, complete) = detailed(ExecutorKind::Claude, &replies).await;
        assert!(report.ready() && complete);
        // The fake fails a command it has no reply for, like a timeout would.
        for missing in ["--version", "auth status", "mcp list"] {
            let replies: Vec<_> = replies
                .iter()
                .filter(|(command, _)| *command != missing)
                .cloned()
                .collect();
            let (_, complete) = detailed(ExecutorKind::Claude, &replies).await;
            assert!(!complete, "{missing}");
        }
        // A check that cannot run stays incomplete even when `mcp list` gave a real answer
        // (here: no `auth status` reply, and `mcp list` ran cleanly without a Linear line).
        let (_, complete) = detailed(
            ExecutorKind::Claude,
            &[("--version", out(0, "2.1.285")), ("mcp list", out(0, ""))],
        )
        .await;
        assert!(!complete);
        // Readable negative answers are complete.
        let (report, complete) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(0, "2.1.285")),
                ("auth status", out(0, r#"{"loggedIn": false}"#)),
                ("mcp list", out(0, "")),
            ],
        )
        .await;
        assert!(!report.ready() && complete);
        // Not installed is a definite answer: no command was run that could have failed.
        let (report, complete) =
            check_executor_detailed(ExecutorKind::Codex, None, &runner(&[]), None).await;
        assert!(!report.ready() && complete);
        // A `--version` that fails is an answer too; the version is only informational.
        let (report, complete) = detailed(
            ExecutorKind::Claude,
            &[
                ("--version", out(1, "")),
                ("auth status", out(0, r#"{"loggedIn": true}"#)),
                ("mcp list", out(0, CLAUDE_OK)),
            ],
        )
        .await;
        assert!(report.ready() && complete);
    }

    #[test]
    fn mcp_list_parsing() {
        assert!(mcp_list_has_linear(ExecutorKind::Claude, CLAUDE_OK));
        assert!(!mcp_list_has_linear(ExecutorKind::Claude, CLAUDE_AUTH));
        assert!(!mcp_list_has_linear(
            ExecutorKind::Claude,
            "other: https://example.com/mcp - ✓ Connected"
        ));
        assert!(!mcp_list_has_linear(
            ExecutorKind::Codex,
            "linear  https://mcp.linear.app/mcp  disabled"
        ));
        for line in [
            "other: https://mcp.linear.app.example.com/mcp (HTTP) - ✔ Connected",
            "proxy: https://proxy.example/mcp?upstream=mcp.linear.app (HTTP) - ✔ Connected",
            "mcp.linear.app-notes: https://notes.example/mcp (HTTP) - ✔ Connected",
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✗ Not connected",
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✘ Failed to connect — HTTP 500",
        ] {
            assert!(!mcp_list_has_linear(ExecutorKind::Claude, line), "{line}");
        }
        assert!(mcp_list_has_linear(
            ExecutorKind::Claude,
            "linear-server: https://mcp.linear.app/mcp (HTTP) - √ Connected"
        ));
        for line in [
            "linear  https://mcp.linear.app/mcp  -  enabled  Not logged in",
            "linear  https://mcp.linear.app/mcp  -  error  OAuth",
            "notes  https://mcp.linear.app.example.com/mcp  -  enabled  OAuth",
        ] {
            assert!(!mcp_list_has_linear(ExecutorKind::Codex, line), "{line}");
        }
        // Colour codes (an environment with FORCE_COLOR can make the CLIs print them into a
        // pipe) neither hide a connected line nor make a bad one pass.
        assert!(mcp_list_has_linear(
            ExecutorKind::Claude,
            "linear-server: \u{1b}[36mhttps://mcp.linear.app/mcp\u{1b}[39m (HTTP) - \u{1b}[32m✔ Connected\u{1b}[39m"
        ));
        assert!(mcp_list_has_linear(
            ExecutorKind::Codex,
            "linear  https://mcp.linear.app/mcp  -  \u{1b}[32menabled\u{1b}[0m  OAuth"
        ));
        assert!(!mcp_list_has_linear(
            ExecutorKind::Claude,
            "linear-server: \u{1b}[36mhttps://mcp.linear.app/mcp\u{1b}[39m (HTTP) - \u{1b}[33m! Needs authentication\u{1b}[39m"
        ));
        assert!(!mcp_list_has_linear(
            ExecutorKind::Codex,
            "linear  https://mcp.linear.app/mcp  -  \u{1b}[32menabled\u{1b}[0m  \u{1b}[31mNot logged in\u{1b}[0m"
        ));
    }

    #[test]
    fn mcp_list_lines_are_read_exactly() {
        // Accepted: either scheme, a server name with ` - ` in it, any case, trailing blanks.
        for line in [
            "linear-server: http://mcp.linear.app/mcp (HTTP) - ✔ Connected",
            "Team - Linear: https://mcp.linear.app/mcp (HTTP) - ✔ Connected",
            "linear-server: HTTPS://MCP.LINEAR.APP/mcp (HTTP) - ✔ CONNECTED",
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected   ",
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ? Connected",
        ] {
            assert!(mcp_list_has_linear(ExecutorKind::Claude, line), "{line}");
        }
        // Refused: another scheme, a userinfo trick, a trailing dot, other hosts that merely
        // end with the name, no status, other words.
        for line in [
            "linear-server: ftp://mcp.linear.app/mcp (HTTP) - ✔ Connected",
            "linear-server: https://mcp.linear.app@evil.example/mcp (HTTP) - ✔ Connected",
            "linear-server: https://mcp.linear.app./mcp (HTTP) - ✔ Connected",
            "linear-server: https://evilmcp.linear.app/mcp (HTTP) - ✔ Connected",
            "linear-server: https://sub.mcp.linear.app/mcp (HTTP) - ✔ Connected",
            "linear-server: https://mcp.linear.app/mcp (HTTP)",
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected (but slow)",
            "linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Disconnected",
        ] {
            assert!(!mcp_list_has_linear(ExecutorKind::Claude, line), "{line}");
        }
        // Only the line with Linear's URL counts, wherever it is in the output.
        let output = format!(
            "Checking MCP server health…\n\nplugin:x:x: https://example.com/mcp - ✘ Failed to connect\n{CLAUDE_OK}\n"
        );
        assert!(mcp_list_has_linear(ExecutorKind::Claude, &output));
        assert!(mcp_list_has_linear(
            ExecutorKind::Codex,
            "Name  Url  Status  Auth\nlinear  https://mcp.linear.app/mcp  -  enabled  OAuth"
        ));
        assert!(!mcp_list_has_linear(ExecutorKind::Codex, ""));
        // Codex needs the token `enabled`, not just text that contains it.
        assert!(!mcp_list_has_linear(
            ExecutorKind::Codex,
            "linear  https://mcp.linear.app/mcp  -  not-enabled  OAuth"
        ));
    }

    #[test]
    fn the_matched_host_is_the_host_of_the_mcp_url() {
        let url = Url::parse(crate::linear::auth::LINEAR_MCP_URL).unwrap();
        assert_eq!(url.host_str(), Some(LINEAR_MCP_HOST));
    }

    #[test]
    fn real_m0_mcp_list_samples_show_linear() {
        assert!(mcp_list_has_linear(
            ExecutorKind::Claude,
            include_str!("../tests/fixtures/claude-mcp-list.txt")
        ));
        assert!(mcp_list_has_linear(
            ExecutorKind::Codex,
            include_str!("../tests/fixtures/codex-mcp-list.txt")
        ));
    }

    #[test]
    fn login_parsing() {
        assert!(parse_claude_auth(r#"{"loggedIn": true}"#));
        assert!(!parse_claude_auth(r#"{"loggedIn": false}"#));
        assert!(!parse_claude_auth("not json"));
        assert!(parse_codex_login(Some(0), "Logged in using ChatGPT", ""));
        // Real codex-cli 0.159.2 writes the status to stderr (M0 finding F4).
        assert!(parse_codex_login(Some(0), "", "Logged in using ChatGPT"));
        assert!(!parse_codex_login(Some(1), "Not logged in", ""));
        assert!(!parse_codex_login(Some(0), "Not logged in", ""));
        // A failing exit code outweighs the text.
        assert!(!parse_codex_login(Some(1), "Logged in using ChatGPT", ""));
        assert!(!parse_codex_login(None, "Logged in using ChatGPT", ""));
        // Colour codes do not change the answer, even when they sit inside it, on either stream.
        assert!(parse_claude_auth("\u{1b}[32m{\"loggedIn\": true}\u{1b}[0m"));
        assert!(parse_claude_auth(
            "{\"loggedIn\": \u{1b}[33mtrue\u{1b}[39m}"
        ));
        assert!(!parse_claude_auth(
            "\u{1b}[31m{\"loggedIn\": false}\u{1b}[0m"
        ));
        let split = "\u{1b}[32mLogged\u{1b}[0m \u{1b}[1min\u{1b}[22m using ChatGPT";
        assert!(parse_codex_login(Some(0), "", split));
        assert!(parse_codex_login(Some(0), split, ""));
        assert!(!parse_codex_login(
            Some(1),
            "\u{1b}[31mNot logged in\u{1b}[0m",
            ""
        ));
    }

    #[test]
    fn ready_set_holds_only_the_ready_executors() {
        let report = |kind, logged_in| ExecutorReport {
            kind,
            program: Some(PathBuf::from(kind.program_name())),
            version: None,
            logged_in,
            linear_mcp: true,
            problems: Vec::new(),
            lean_linear_mcp: true,
            lean_logged_in: true,
            lean_problems: Vec::new(),
        };
        let reports = [
            report(ExecutorKind::Claude, false),
            report(ExecutorKind::Codex, true),
        ];
        let ready: HashSet<_> = [ExecutorKind::Codex].into_iter().collect();
        assert_eq!(ready_set(&reports).full, ready);
        assert_eq!(ready_set(&[]), Readiness::default());
    }

    #[test]
    fn ready_set_keeps_full_and_lean_apart() {
        let report = |kind, lean_linear_mcp, lean_logged_in| ExecutorReport {
            kind,
            program: Some(PathBuf::from(kind.program_name())),
            version: None,
            logged_in: true,
            linear_mcp: true,
            problems: Vec::new(),
            lean_linear_mcp,
            lean_logged_in,
            lean_problems: Vec::new(),
        };
        let both: HashSet<_> = [ExecutorKind::Claude, ExecutorKind::Codex]
            .into_iter()
            .collect();
        let claude: HashSet<_> = [ExecutorKind::Claude].into_iter().collect();
        assert_eq!(
            ready_set(&[
                report(ExecutorKind::Claude, true, true),
                report(ExecutorKind::Codex, true, false),
            ]),
            Readiness {
                full: both.clone(),
                lean: claude.clone(),
            }
        );
        assert_eq!(
            ready_set(&[
                report(ExecutorKind::Claude, true, true),
                report(ExecutorKind::Codex, false, true),
            ]),
            Readiness {
                full: both,
                lean: claude,
            }
        );
    }

    #[test]
    fn binding_checks() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        for (project, path, executor) in [
            ("proj-1", dir.path().to_path_buf(), None),
            (
                "proj-2",
                PathBuf::from("/definitely/not/here"),
                Some(ExecutorKind::Codex),
            ),
        ] {
            cfg.bindings.push(crate::config::Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: path,
                executor,
                permission: None,
                model: None,
                prompt_override: None,
                lean: true,
                effort: None,
                labels: Vec::new(),
            });
        }
        let claude: HashSet<_> = [ExecutorKind::Claude].into_iter().collect();
        let ready = Readiness {
            full: claude.clone(),
            lean: claude,
        };
        let reports = check_bindings(&cfg, &ready, None);
        assert!(reports[0].directory_exists && reports[0].executor_ready);
        assert!(!reports[1].directory_exists && !reports[1].executor_ready);
    }

    #[test]
    fn binding_reports_say_whether_lean_runs_apply() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        for (project, dir, permission) in [
            ("proj-1", elsewhere.path(), None),
            (
                "proj-2",
                elsewhere.path(),
                Some(crate::config::PermissionProfile::Personal),
            ),
            ("proj-3", home.path(), None),
        ] {
            cfg.bindings.push(crate::config::Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: dir.to_path_buf(),
                executor: None,
                permission,
                model: None,
                prompt_override: None,
                lean: true,
                effort: None,
                labels: Vec::new(),
            });
        }
        let ready = Readiness {
            full: [ExecutorKind::Claude].into_iter().collect(),
            lean: HashSet::new(),
        };
        let reports = check_bindings(&cfg, &ready, Some(home.path()));
        let seen: Vec<_> = reports.iter().map(|r| (r.lean, r.executor_ready)).collect();
        assert_eq!(
            seen,
            [
                (LeanStatus::On, false),
                (LeanStatus::Personal, true),
                (LeanStatus::HomeDirectory, true),
            ]
        );
    }

    #[test]
    fn binding_reports_carry_the_effort_issuerelay_passes() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.settings.default_effort = Some(Effort::High);
        for (project, effort) in [("proj-1", Some(Effort::Max)), ("proj-2", None)] {
            cfg.bindings.push(crate::config::Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: dir.path().to_path_buf(),
                executor: None,
                permission: None,
                model: None,
                prompt_override: None,
                lean: true,
                effort,
                labels: Vec::new(),
            });
        }
        let effort = |cfg: &Config, index: usize| {
            let reports = check_bindings(cfg, &Readiness::default(), None);
            serde_json::to_value(&reports[index]).unwrap()["effort"].clone()
        };
        // The binding's own level, else the default from the settings, named as the settings
        // and `bind list` name it.
        assert_eq!(effort(&cfg, 0), serde_json::json!("max"));
        assert_eq!(effort(&cfg, 1), serde_json::json!("high"));
        // Neither set: the executor's own setting applies, and the report names no level.
        cfg.settings.default_effort = None;
        assert_eq!(effort(&cfg, 1), serde_json::Value::Null);
    }

    #[test]
    fn a_binding_held_back_only_by_the_lean_checks_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        for (project, executor, lean) in [
            ("proj-1", ExecutorKind::Codex, true),
            ("proj-2", ExecutorKind::Codex, false),
            ("proj-3", ExecutorKind::Claude, true),
        ] {
            cfg.bindings.push(crate::config::Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: dir.path().to_path_buf(),
                executor: Some(executor),
                permission: None,
                model: None,
                prompt_override: None,
                lean,
                effort: None,
                labels: Vec::new(),
            });
        }
        // What the command line and the panel read: `executor_ready`, and `lean_not_ready`.
        let seen = |ready: &Readiness| -> Vec<(serde_json::Value, serde_json::Value)> {
            check_bindings(&cfg, ready, None)
                .iter()
                .map(|report| {
                    let json = serde_json::to_value(report).unwrap();
                    (
                        json["executor_ready"].clone(),
                        json["lean_not_ready"].clone(),
                    )
                })
                .collect()
        };
        let (yes, no) = (serde_json::json!(true), serde_json::json!(false));
        // Codex passes its usual checks but not the lean ones; Claude passes both. Only the lean
        // binding of Codex waits, and only for the lean checks.
        let lean_missing = Readiness {
            full: [ExecutorKind::Claude, ExecutorKind::Codex]
                .into_iter()
                .collect(),
            lean: [ExecutorKind::Claude].into_iter().collect(),
        };
        assert_eq!(
            seen(&lean_missing),
            [
                (no.clone(), yes.clone()),
                (yes.clone(), no.clone()),
                (yes.clone(), no.clone())
            ]
        );
        // Codex fails its usual checks too: then it is not only the lean checks.
        let codex_down = Readiness {
            full: [ExecutorKind::Claude].into_iter().collect(),
            lean: [ExecutorKind::Claude].into_iter().collect(),
        };
        assert_eq!(
            seen(&codex_down),
            [
                (no.clone(), no.clone()),
                (no.clone(), no.clone()),
                (yes, no)
            ]
        );
    }

    /// The lean checks of the tests: a folder in `dir` (the check makes it; the fake runner never
    /// enters it) and no Codex configuration file.
    fn lean_paths(dir: &tempfile::TempDir) -> LeanCheckPaths {
        LeanCheckPaths {
            empty_dir: dir.path().join("lean-check"),
            codex_config: None,
            claude_settings: None,
        }
    }

    /// `check_executor_detailed` with the lean checks.
    async fn lean_detailed(
        kind: ExecutorKind,
        replies: &[(&str, CmdOutput)],
        lean: &LeanCheckPaths,
    ) -> (ExecutorReport, bool) {
        let program = Some(PathBuf::from(kind.program_name()));
        check_executor_detailed(kind, program, &runner(replies), Some(lean)).await
    }

    const CLAUDE_USUAL: &str =
        r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#;

    /// A Claude whose usual checks pass; `lean` answers the lean sign-in check.
    fn claude_replies(lean: CmdOutput, mcp: &str) -> Vec<(&'static str, CmdOutput)> {
        vec![
            ("--version", out(0, "2.1.285 (Claude Code)")),
            ("auth status", out(0, CLAUDE_USUAL)),
            ("--setting-sources project,local auth status", lean),
            ("mcp list", out(0, mcp)),
        ]
    }

    /// A Codex whose usual checks pass; `lean` answers the lean sign-in check.
    fn codex_replies(lean: CmdOutput) -> Vec<(&'static str, CmdOutput)> {
        vec![
            ("--version", out(0, "codex-cli 0.159.2")),
            ("login status", out(0, "Logged in using ChatGPT")),
            ("-c cli_auth_credentials_store=file login status", lean),
            ("mcp list", out(0, CODEX_OK)),
        ]
    }

    #[test]
    fn lean_linear_needs_the_exact_name_and_url() {
        assert!(mcp_list_has_lean_linear(ExecutorKind::Claude, CLAUDE_OK));
        for line in [
            "linear: https://mcp.linear.app/mcp (HTTP) - ✔ Connected",
            "claude.ai Linear: https://mcp.linear.app/mcp - ✔ Connected",
            "linear-server: https://mcp.linear.app/sse (HTTP) - ✔ Connected",
            "linear-server: https://mcp.linear.app/mcp?x=1 (HTTP) - ✔ Connected",
            // A plugin's server of that name: lean runs load no plugin's servers.
            "plugin:demo:linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected",
            CLAUDE_AUTH,
        ] {
            assert!(
                !mcp_list_has_lean_linear(ExecutorKind::Claude, line),
                "{line}"
            );
        }
        let mixed =
            format!("claude.ai Linear: https://mcp.linear.app/mcp - ✔ Connected\n{CLAUDE_OK}\n");
        assert!(mcp_list_has_lean_linear(ExecutorKind::Claude, &mixed));
        assert!(mcp_list_has_lean_linear(ExecutorKind::Codex, CODEX_OK));
        for line in [
            "linear-alt  https://mcp.linear.app/mcp  -  enabled  OAuth",
            "linear  https://mcp.linear.app/mcp  -  enabled  Not logged in",
            "linear  https://mcp.linear.app/sse  -  enabled  OAuth",
        ] {
            assert!(
                !mcp_list_has_lean_linear(ExecutorKind::Codex, line),
                "{line}"
            );
        }
    }

    #[test]
    fn lean_runs_take_codexs_linear_server_only_with_a_stored_oauth_sign_in() {
        // As codex-cli 0.159.2 prints the table of HTTP servers: every cell padded to the widest
        // in its column (docs/gates/samples/m0/executors/codex-mcp-list.txt).
        let table = |token: &str, auth: &str| {
            format!(
                "Name    Url                         Bearer Token Env Var  Status   Auth\n\
                 linear  https://mcp.linear.app/mcp  {token:<20}  enabled  {auth:<12}\n"
            )
        };
        assert!(mcp_list_has_lean_linear(
            ExecutorKind::Codex,
            &table("-", "OAuth")
        ));
        for (token, auth) in [
            ("DEMO_TOKEN", "Bearer token"),
            ("-", "Bearer token"),
            ("-", "Unsupported"),
            ("-", "Unknown"),
            // A column after it: the sign-in is no longer the whole of what follows the status.
            ("-", "OAuth  extra"),
        ] {
            let listed = table(token, auth);
            assert!(
                !mcp_list_has_lean_linear(ExecutorKind::Codex, &listed),
                "{listed}"
            );
            // Usual runs read the configuration that holds the token: they stay ready.
            assert!(
                mcp_list_has_linear(ExecutorKind::Codex, &listed),
                "{listed}"
            );
        }
    }

    #[tokio::test]
    async fn a_codex_linear_server_without_an_oauth_sign_in_says_what_lean_runs_need() {
        let dir = tempfile::tempdir().unwrap();
        let mut replies = codex_replies(out(0, "Logged in using ChatGPT"));
        replies[3] = (
            "mcp list",
            out(
                0,
                "linear  https://mcp.linear.app/mcp  DEMO_TOKEN  enabled  Bearer token",
            ),
        );
        let (report, complete) =
            lean_detailed(ExecutorKind::Codex, &replies, &lean_paths(&dir)).await;
        assert!(
            complete && report.ready() && !report.ready_lean(),
            "{report:?}"
        );
        // Signing in alone would not do: the column says `Bearer token` while the configuration
        // sets a token, stored sign-in or not.
        assert_eq!(
            report.lean_problems,
            [
                "the Linear MCP server `linear` in codex signs in with a token from codex's \
                 configuration (`Bearer token`), which lean runs leave out; to use lean runs, \
                 remove the token from its entry (`bearer_token_env_var` or an `Authorization` \
                 header) and sign in with `codex mcp login linear`, or turn lean runs off for \
                 this binding"
            ]
        );
        replies[3] = (
            "mcp list",
            out(
                0,
                "linear  https://mcp.linear.app/mcp  -  enabled  Unsupported",
            ),
        );
        let (report, _) = lean_detailed(ExecutorKind::Codex, &replies, &lean_paths(&dir)).await;
        assert_eq!(
            report.lean_problems,
            [
                "the Linear MCP server `linear` in codex shows `Unsupported` as its sign-in, but \
                 lean runs leave out codex's configuration and can only use a stored OAuth \
                 sign-in; sign in with `codex mcp login linear`, or turn lean runs off for this \
                 binding"
            ]
        );
        // A Codex that stopped printing the column is not taken at its word either.
        replies[3] = (
            "mcp list",
            out(0, "linear  https://mcp.linear.app/mcp  -  enabled"),
        );
        let (report, _) = lean_detailed(ExecutorKind::Codex, &replies, &lean_paths(&dir)).await;
        assert!(report.ready() && !report.ready_lean(), "{report:?}");
        assert!(
            report.lean_problems[0].starts_with(
                "the Linear MCP server `linear` in codex shows no sign-in, but lean runs leave out"
            ),
            "{report:?}"
        );
    }

    #[tokio::test]
    async fn lean_problems_name_the_server_lean_runs_need() {
        let dir = tempfile::tempdir().unwrap();
        let connector = "claude.ai Linear: https://mcp.linear.app/mcp - ✔ Connected";
        let (report, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_replies(out(0, CLAUDE_USUAL), connector),
            &lean_paths(&dir),
        )
        .await;
        assert!(report.ready() && !report.ready_lean(), "{report:?}");
        assert!(
            report
                .lean_problems
                .iter()
                .any(|p| p.contains("`linear-server`")
                    && p.contains("claude mcp add --scope user --transport http linear-server")),
            "{report:?}"
        );
        let elsewhere = "linear-server: https://mcp.linear.app/sse (HTTP) - ✔ Connected";
        let (report, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_replies(out(0, CLAUDE_USUAL), elsewhere),
            &lean_paths(&dir),
        )
        .await;
        assert!(!report.ready_lean());
        assert!(
            report
                .lean_problems
                .iter()
                .any(|p| p.contains("points to https://mcp.linear.app/sse")),
            "{report:?}"
        );
    }

    #[tokio::test]
    async fn a_crashed_mcp_list_adds_no_lean_problem() {
        let dir = tempfile::tempdir().unwrap();
        let crashed = CmdOutput {
            code: Some(1),
            stdout: String::new(),
            stderr: "demo crash".into(),
        };
        let mut replies = claude_replies(out(0, CLAUDE_USUAL), "");
        replies[3] = ("mcp list", crashed);
        let (report, complete) =
            lean_detailed(ExecutorKind::Claude, &replies, &lean_paths(&dir)).await;
        assert!(!complete && !report.ready_lean(), "{report:?}");
        assert!(
            report
                .problems
                .iter()
                .any(|p| p == "`claude mcp list` failed (exit code 1): demo crash"),
            "{report:?}"
        );
        // That problem says it all: a crash says nothing about the server lean runs load.
        assert!(report.lean_problems.is_empty(), "{report:?}");
    }

    #[tokio::test]
    async fn a_usual_login_that_says_logged_out_is_not_compared_for_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let logged_out = r#"{"loggedIn":false}"#;
        let (report, complete) = lean_detailed(
            ExecutorKind::Claude,
            &claude_signing_in(logged_out, out(0, logged_out)),
            &lean_paths(&dir),
        )
        .await;
        assert!(complete, "{report:?}");
        assert!(!report.logged_in && !report.lean_logged_in, "{report:?}");
        assert!(
            report
                .problems
                .iter()
                .any(|p| p == "claude is not logged in"),
            "{report:?}"
        );
        // "not logged in" says it all: the lean sign-in check adds nothing.
        assert!(report.lean_problems.is_empty(), "{report:?}");
    }

    #[tokio::test]
    async fn claude_lean_login_must_match_the_usual_one() {
        let dir = tempfile::tempdir().unwrap();
        let (same, complete) = lean_detailed(
            ExecutorKind::Claude,
            &claude_replies(out(0, CLAUDE_USUAL), CLAUDE_OK),
            &lean_paths(&dir),
        )
        .await;
        assert!(complete);
        assert!(same.ready_lean(), "{same:?}");
        assert!(same.lean_problems.is_empty(), "{same:?}");
        let api_key = r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#;
        let (other, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_replies(out(0, api_key), CLAUDE_OK),
            &lean_paths(&dir),
        )
        .await;
        assert!(other.ready() && !other.ready_lean(), "{other:?}");
        assert!(
            other
                .lean_problems
                .iter()
                .any(|p| p.contains("api_key") && p.contains("claude.ai")),
            "{other:?}"
        );
        let (signed_out, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_replies(out(0, r#"{"loggedIn":false}"#), CLAUDE_OK),
            &lean_paths(&dir),
        )
        .await;
        assert!(!signed_out.ready_lean());
        assert!(
            signed_out
                .lean_problems
                .iter()
                .any(|p| p.contains("not signed in")),
            "{signed_out:?}"
        );
    }

    #[tokio::test]
    async fn the_claude_lean_login_check_runs_in_the_empty_folder() {
        let dir = tempfile::tempdir().unwrap();
        let fake = runner(&claude_replies(out(0, CLAUDE_USUAL), CLAUDE_OK));
        let (report, _) = check_executor_detailed(
            ExecutorKind::Claude,
            Some(PathBuf::from("claude")),
            &fake,
            Some(&lean_paths(&dir)),
        )
        .await;
        assert!(report.ready_lean(), "{report:?}");
        let empty = dir.path().join("lean-check");
        assert_eq!(*fake.dirs.lock().unwrap(), std::slice::from_ref(&empty));
        assert!(empty.is_dir(), "the check makes the folder");
    }

    #[tokio::test]
    async fn a_lean_check_folder_that_cannot_be_made_is_a_lean_problem() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let blocked = LeanCheckPaths {
            empty_dir: file.join("lean-check"),
            codex_config: None,
            claude_settings: None,
        };
        let fake = runner(&claude_replies(out(0, CLAUDE_USUAL), CLAUDE_OK));
        let (report, complete) = check_executor_detailed(
            ExecutorKind::Claude,
            Some(PathBuf::from("claude")),
            &fake,
            Some(&blocked),
        )
        .await;
        assert!(
            complete && report.ready() && !report.ready_lean(),
            "{report:?}"
        );
        assert!(
            report.lean_problems.iter().any(|p| p
                .starts_with("could not prepare the folder for the lean sign-in check: ")),
            "{report:?}"
        );
        assert!(
            fake.dirs.lock().unwrap().is_empty(),
            "the check did not run"
        );
    }

    #[test]
    fn codex_config_follows_codex_home() {
        let home = PathBuf::from("demo-home");
        assert_eq!(
            codex_config_path_from(Some("demo-codex".into()), Some(home.clone())),
            Some(PathBuf::from("demo-codex").join("config.toml"))
        );
        let usual = Some(home.join(".codex").join("config.toml"));
        assert_eq!(codex_config_path_from(None, Some(home.clone())), usual);
        assert_eq!(codex_config_path_from(Some("".into()), Some(home)), usual);
        assert_eq!(codex_config_path_from(None, None), None);
    }

    #[test]
    fn claude_user_settings_follow_claude_config_dir() {
        let home = PathBuf::from("demo-home");
        assert_eq!(
            claude_settings_path_from(Some("demo-claude".into()), Some(home.clone())),
            Some(PathBuf::from("demo-claude").join("settings.json"))
        );
        let usual = Some(home.join(".claude").join("settings.json"));
        assert_eq!(claude_settings_path_from(None, Some(home.clone())), usual);
        assert_eq!(
            claude_settings_path_from(Some("".into()), Some(home)),
            usual
        );
        assert_eq!(claude_settings_path_from(None, None), None);
    }

    #[test]
    fn routing_variables_are_found_in_the_claude_user_settings() {
        let found = |text: &str| claude_routing_env(text).unwrap();
        assert_eq!(
            found(r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example.com"}}"#),
            ["ANTHROPIC_BASE_URL"]
        );
        assert_eq!(
            found(r#"{"env":{"https_proxy":"http://proxy.example.com:8080","HTTPS_PROXY":"x"}}"#),
            ["HTTPS_PROXY", "https_proxy"]
        );
        // Lean runs may leave the rest out: other variables, empty values, no `env` at all.
        assert!(found(r#"{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"max"}}"#).is_empty());
        assert!(found(r#"{"env":{"ANTHROPIC_BASE_URL":""}}"#).is_empty());
        assert!(found(r#"{"model":"opus"}"#).is_empty());
        assert!(found("{}").is_empty());
        assert!(claude_routing_env("{ not json").is_err());
    }

    #[tokio::test]
    async fn a_gateway_or_proxy_in_the_claude_user_settings_rules_out_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example.com/demo-token-value"}}"#,
        )
        .unwrap();
        let paths = LeanCheckPaths {
            claude_settings: Some(settings.clone()),
            ..lean_paths(&dir)
        };
        let replies = claude_replies(out(0, CLAUDE_USUAL), CLAUDE_OK);
        let (report, _) = lean_detailed(ExecutorKind::Claude, &replies, &paths).await;
        assert!(report.ready() && !report.ready_lean(), "{report:?}");
        assert!(
            report
                .lean_problems
                .iter()
                .any(|p| p.contains("`ANTHROPIC_BASE_URL`")),
            "{report:?}"
        );
        // The value, which can hold a token, is not shown.
        assert!(
            !report
                .lean_problems
                .iter()
                .any(|p| p.contains("demo-token-value")),
            "{report:?}"
        );
        // Other variables, or no file at all, leave lean runs ready.
        std::fs::write(&settings, r#"{"env":{"CLAUDE_CODE_EFFORT_LEVEL":"max"}}"#).unwrap();
        let (report, _) = lean_detailed(ExecutorKind::Claude, &replies, &paths).await;
        assert!(report.ready_lean(), "{report:?}");
        let missing = LeanCheckPaths {
            claude_settings: Some(dir.path().join("missing.json")),
            ..lean_paths(&dir)
        };
        let (report, _) = lean_detailed(ExecutorKind::Claude, &replies, &missing).await;
        assert!(report.ready_lean(), "{report:?}");
    }

    #[tokio::test]
    async fn claude_user_settings_that_cannot_be_read_rule_out_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_BASE_URL":"demo-token-value""#,
        )
        .unwrap();
        // A folder where the file should be cannot be read either, on both platforms.
        for (path, says) in [
            (settings, "they are not valid JSON"),
            (
                dir.path().to_path_buf(),
                "could not read the Claude Code user settings",
            ),
        ] {
            let paths = LeanCheckPaths {
                claude_settings: Some(path),
                ..lean_paths(&dir)
            };
            let replies = claude_replies(out(0, CLAUDE_USUAL), CLAUDE_OK);
            let (report, _) = lean_detailed(ExecutorKind::Claude, &replies, &paths).await;
            assert!(report.ready() && !report.ready_lean(), "{report:?}");
            assert!(
                report.lean_problems.iter().any(|p| p.contains(says)),
                "{report:?}"
            );
            assert!(
                !report
                    .lean_problems
                    .iter()
                    .any(|p| p.contains("demo-token-value")),
                "{report:?}"
            );
        }
    }

    #[tokio::test]
    async fn codex_lean_login_reads_the_file_store_and_must_match() {
        let dir = tempfile::tempdir().unwrap();
        let (same, _) = lean_detailed(
            ExecutorKind::Codex,
            &codex_replies(out(0, "Logged in using ChatGPT")),
            &lean_paths(&dir),
        )
        .await;
        assert!(same.ready_lean(), "{same:?}");
        let (signed_out, _) = lean_detailed(
            ExecutorKind::Codex,
            &codex_replies(out(1, "Not logged in")),
            &lean_paths(&dir),
        )
        .await;
        assert!(signed_out.ready() && !signed_out.ready_lean());
        assert!(
            signed_out
                .lean_problems
                .iter()
                .any(|p| p.contains("not signed in")),
            "{signed_out:?}"
        );
        let (other, _) = lean_detailed(
            ExecutorKind::Codex,
            &codex_replies(out(0, "Logged in using an API key")),
            &lean_paths(&dir),
        )
        .await;
        assert!(!other.ready_lean());
        assert!(
            other.lean_problems.iter().any(|p| p.contains("an api key")),
            "{other:?}"
        );
    }

    /// `codex login status` after an API key login as codex-cli 0.159.2 prints it: on stderr, the
    /// key masked to its first eight and last five characters.
    fn codex_api_key(masked: &str) -> CmdOutput {
        CmdOutput {
            code: Some(0),
            stdout: String::new(),
            stderr: format!("Logged in using an API key - {masked}\n"),
        }
    }

    #[test]
    fn a_codex_sign_in_is_shown_by_the_words_it_starts_with() {
        for (method, shown) in [
            ("chatgpt", "chatgpt"),
            ("an api key", "an api key"),
            ("an api key - demo-key***12345", "an api key"),
            // Other ways a later codex-cli may print the key: none of it is shown either.
            ("an api key: demo-key***12345", "an api key"),
            ("an api key (demo-key***12345)", "an api key"),
            ("", "an unknown method"),
        ] {
            assert_eq!(codex_shown(method), shown, "{method:?}");
        }
    }

    #[tokio::test]
    async fn a_lean_problem_names_a_codex_api_key_login_without_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let usual = codex_api_key("demo-key***12345");
        let api_key_runs = |lean: CmdOutput| {
            let mut replies = codex_replies(lean);
            replies[1] = ("login status", usual.clone());
            replies
        };
        let (same, _) = lean_detailed(
            ExecutorKind::Codex,
            &api_key_runs(usual.clone()),
            &lean_paths(&dir),
        )
        .await;
        assert!(same.ready_lean(), "{same:?}");
        // Another key is another sign-in all the same, and the problem shows neither key.
        let (other_key, _) = lean_detailed(
            ExecutorKind::Codex,
            &api_key_runs(codex_api_key("demo-key***67890")),
            &lean_paths(&dir),
        )
        .await;
        let (instead_of_chatgpt, _) = lean_detailed(
            ExecutorKind::Codex,
            &codex_replies(usual),
            &lean_paths(&dir),
        )
        .await;
        for (report, says) in [
            (
                other_key,
                "would sign in with an api key, but not the one usual runs use",
            ),
            (
                instead_of_chatgpt,
                "would sign in differently (an api key instead of chatgpt)",
            ),
        ] {
            assert!(report.ready() && !report.ready_lean(), "{report:?}");
            assert!(
                report.lean_problems.iter().any(|p| p.contains(says)),
                "{report:?}"
            );
            let shown = format!("{report:?}");
            assert!(
                !shown.contains("***") && !shown.contains("demo-key"),
                "{shown}"
            );
        }
    }

    /// A Claude whose usual and lean `auth status` answer `usual` and `lean`.
    fn claude_signing_in(usual: &str, lean: CmdOutput) -> Vec<(&'static str, CmdOutput)> {
        let mut replies = claude_replies(lean, CLAUDE_OK);
        replies[1] = ("auth status", out(0, usual));
        replies
    }

    #[tokio::test]
    async fn claude_lean_login_compares_the_api_provider_too() {
        let dir = tempfile::tempdir().unwrap();
        let usual = r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"bedrock"}"#;
        let lean = r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#;
        let (report, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_signing_in(usual, out(0, lean)),
            &lean_paths(&dir),
        )
        .await;
        assert!(report.ready() && !report.ready_lean(), "{report:?}");
        assert!(
            report
                .lean_problems
                .iter()
                .any(|p| p.contains("firstParty") && p.contains("bedrock")),
            "{report:?}"
        );
    }

    #[tokio::test]
    async fn a_sign_in_that_does_not_say_how_cannot_be_checked_for_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let bare = r#"{"loggedIn": true}"#;
        let (claude, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_signing_in(bare, out(0, bare)),
            &lean_paths(&dir),
        )
        .await;
        let mut replies = codex_replies(out(0, "Logged in"));
        replies[1] = ("login status", out(0, "Logged in"));
        let (codex, _) = lean_detailed(ExecutorKind::Codex, &replies, &lean_paths(&dir)).await;
        for report in [claude, codex] {
            assert!(report.ready() && !report.ready_lean(), "{report:?}");
            assert!(
                report
                    .lean_problems
                    .iter()
                    .any(|p| p.contains("lean runs cannot be checked")),
                "{report:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_lean_sign_in_check_that_cannot_be_read_is_not_lean_ready() {
        let dir = tempfile::tempdir().unwrap();
        let (claude, _) = lean_detailed(
            ExecutorKind::Claude,
            &claude_replies(out(0, "Welcome to Claude Code"), CLAUDE_OK),
            &lean_paths(&dir),
        )
        .await;
        let (codex, _) = lean_detailed(
            ExecutorKind::Codex,
            &codex_replies(out(0, "Something else entirely")),
            &lean_paths(&dir),
        )
        .await;
        for report in [claude, codex] {
            assert!(report.ready() && !report.ready_lean(), "{report:?}");
            assert!(
                report
                    .lean_problems
                    .iter()
                    .any(|p| p.contains("gave no usable answer")),
                "{report:?}"
            );
        }
    }

    #[test]
    fn a_configuration_that_is_not_toml_is_refused_without_its_content() {
        let error = codex_custom_provider("model_provider = \"demo-token-value\n")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("demo-token-value"), "{error}");
    }

    #[tokio::test]
    async fn a_codex_configuration_that_cannot_be_read_rules_out_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "model_provider = \"demo-token-value\n").unwrap();
        // A folder where the file should be cannot be read either, on both platforms.
        for (path, says) in [
            (config, "it is not valid TOML"),
            (
                dir.path().to_path_buf(),
                "could not read the codex configuration",
            ),
        ] {
            let paths = LeanCheckPaths {
                codex_config: Some(path),
                ..lean_paths(&dir)
            };
            let replies = codex_replies(out(0, "Logged in using ChatGPT"));
            let (report, _) = lean_detailed(ExecutorKind::Codex, &replies, &paths).await;
            assert!(report.ready() && !report.ready_lean(), "{report:?}");
            assert!(
                report.lean_problems.iter().any(|p| p.contains(says)),
                "{report:?}"
            );
            assert!(
                !report
                    .lean_problems
                    .iter()
                    .any(|p| p.contains("demo-token-value")),
                "{report:?}"
            );
        }
    }

    #[test]
    fn codex_custom_providers_are_found() {
        assert_eq!(
            codex_custom_provider("model_provider = \"azure\"\n").unwrap(),
            Some("azure".into())
        );
        assert_eq!(
            codex_custom_provider("model_provider = \"openai\"\n").unwrap(),
            None
        );
        assert_eq!(
            codex_custom_provider(
                "profile = \"work\"\n[profiles.work]\nmodel_provider = \"demo\"\n"
            )
            .unwrap(),
            Some("demo".into())
        );
        assert_eq!(
            codex_custom_provider("model = \"gpt-5.5\"\n").unwrap(),
            None
        );
        // Both set: the active profile's wins, also when it is the default; a profile that sets
        // none leaves the top-level one.
        let top_level = "model_provider = \"azure\"\nprofile = \"work\"\n[profiles.work]\n";
        for (profile, provider) in [
            ("model_provider = \"demo\"\n", Some("demo")),
            ("model_provider = \"openai\"\n", None),
            ("model = \"gpt-5.5\"\n", Some("azure")),
        ] {
            assert_eq!(
                codex_custom_provider(&format!("{top_level}{profile}")).unwrap(),
                provider.map(str::to_string),
                "{profile}"
            );
        }
        assert_eq!(codex_custom_provider("").unwrap(), None);
        assert!(codex_custom_provider("model_provider = ").is_err());
    }

    #[tokio::test]
    async fn a_custom_codex_model_provider_rules_out_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(&config, "model_provider = \"azure\"\n").unwrap();
        let custom = LeanCheckPaths {
            codex_config: Some(config),
            ..lean_paths(&dir)
        };
        let replies = codex_replies(out(0, "Logged in using ChatGPT"));
        let (report, _) = lean_detailed(ExecutorKind::Codex, &replies, &custom).await;
        assert!(report.ready() && !report.ready_lean(), "{report:?}");
        assert!(
            report.lean_problems.iter().any(|p| p.contains("azure")),
            "{report:?}"
        );
        // A missing file means the default provider.
        let missing = LeanCheckPaths {
            codex_config: Some(dir.path().join("missing.toml")),
            ..lean_paths(&dir)
        };
        let (report, _) = lean_detailed(ExecutorKind::Codex, &replies, &missing).await;
        assert!(report.ready_lean(), "{report:?}");
    }

    #[tokio::test]
    async fn without_the_lean_checks_lean_runs_are_not_ready() {
        let report = check_executor(
            ExecutorKind::Claude,
            Some(PathBuf::from("claude")),
            &both_ready(),
        )
        .await;
        assert!(report.ready() && !report.ready_lean(), "{report:?}");
        assert!(!report.lean_problems.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_lean_sign_in_check_runs_together_with_the_others() {
        let dir = tempfile::tempdir().unwrap();
        // Four parties: a check that ran the lean sign-in check after the others never ends.
        let runner =
            BarrierRunner::new(4, runner(&claude_replies(out(0, CLAUDE_USUAL), CLAUDE_OK)));
        let checked = tokio::time::timeout(
            Duration::from_secs(5),
            check_executor_detailed(
                ExecutorKind::Claude,
                Some(PathBuf::from("claude")),
                &runner,
                Some(&lean_paths(&dir)),
            ),
        )
        .await;
        let (report, _) = checked.expect("the four commands must run at the same time");
        assert!(report.ready_lean(), "{report:?}");
    }

    #[test]
    fn mcp_list_gets_its_own_limit_and_everything_else_the_check_limit() {
        let runner = SystemRunner::with_limits(Duration::from_secs(1), Duration::from_secs(2));
        assert_eq!(runner.limit_for(&["mcp", "list"]), Duration::from_secs(2));
        for args in [
            &["--version"][..],
            &["auth", "status"],
            &["login", "status"],
        ] {
            assert_eq!(runner.limit_for(args), Duration::from_secs(1), "{args:?}");
        }
    }

    // The far end stays open, as when a process that escaped the check's process group holds the
    // pipe: at the drain deadline the reader stops and what it read is kept.
    #[tokio::test]
    async fn a_pipe_still_open_at_the_drain_deadline_gives_what_was_read_by_then() {
        use tokio::io::AsyncWriteExt;

        let (mut far_end, near_end) = tokio::io::duplex(64);
        far_end.write_all(b"partial").await.unwrap();
        let stop = CancellationToken::new();
        let reader = read_pipe(Some(near_end), stop.clone());
        let deadline = Instant::now() + Duration::from_millis(200);
        let text = collected(reader, &stop, deadline, "stdout").await.unwrap();
        assert_eq!(text, "partial");
        assert!(stop.is_cancelled(), "the reader was not told to stop");
        drop(far_end);
    }

    #[tokio::test]
    async fn a_pipe_that_ends_before_the_drain_deadline_is_read_to_its_end() {
        use tokio::io::AsyncWriteExt;

        let (mut far_end, near_end) = tokio::io::duplex(64);
        let stop = CancellationToken::new();
        let reader = read_pipe(Some(near_end), stop.clone());
        // More than the duplex buffer holds: the reader has to keep reading while it is written.
        let text = "line\n".repeat(100);
        far_end.write_all(text.as_bytes()).await.unwrap();
        drop(far_end);
        let deadline = Instant::now() + Duration::from_secs(10);
        assert_eq!(
            collected(reader, &stop, deadline, "stdout").await.unwrap(),
            text
        );
        assert!(!stop.is_cancelled());
        let absent = read_pipe(None::<tokio::io::DuplexStream>, stop.clone());
        assert!(absent.await.unwrap().unwrap().is_empty());
    }

    /// A pipe that hands out one scripted step per read: bytes or an error, and the end of input
    /// once the steps run out.
    struct ScriptedPipe(std::collections::VecDeque<io::Result<Vec<u8>>>);

    impl AsyncRead for ScriptedPipe {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(match self.0.pop_front() {
                Some(Ok(bytes)) => {
                    buf.put_slice(&bytes);
                    Ok(())
                }
                Some(Err(e)) => Err(e),
                None => Ok(()),
            })
        }
    }

    // Half of an `mcp list` that exited 0 would read as "Linear is not configured" and be cached
    // for the full TTL; as an error the check counts as incomplete and is retried soon.
    #[tokio::test]
    async fn a_pipe_that_fails_half_way_is_an_error_not_the_end_of_the_output() {
        let pipe = ScriptedPipe(
            [
                Ok(b"half of the list".to_vec()),
                Err(io::Error::other("boom")),
            ]
            .into(),
        );
        let stop = CancellationToken::new();
        let reader = read_pipe(Some(pipe), stop.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        let err = collected(reader, &stop, deadline, "stdout")
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "cannot read its stdout: boom");
    }

    #[tokio::test]
    async fn an_interrupted_read_is_tried_again() {
        let pipe = ScriptedPipe(
            [
                Ok(b"one ".to_vec()),
                Err(io::ErrorKind::Interrupted.into()),
                Ok(b"two".to_vec()),
            ]
            .into(),
        );
        let stop = CancellationToken::new();
        let reader = read_pipe(Some(pipe), stop.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        let text = collected(reader, &stop, deadline, "stdout").await.unwrap();
        assert_eq!(text, "one two");
    }

    #[tokio::test]
    async fn a_reader_task_that_failed_is_an_error() {
        let reader: JoinHandle<io::Result<Vec<u8>>> = tokio::spawn(std::future::pending());
        reader.abort();
        let stop = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        let err = collected(reader, &stop, deadline, "stderr")
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("the reader of its stderr failed: "),
            "{err}"
        );
    }

    #[test]
    fn the_default_limits_are_20_and_60_seconds() {
        let runner = SystemRunner::default();
        assert_eq!(runner.limit_for(&["--version"]), Duration::from_secs(20));
        assert_eq!(
            runner.limit_for(&["auth", "status"]),
            Duration::from_secs(20)
        );
        assert_eq!(runner.limit_for(&["mcp", "list"]), Duration::from_secs(60));
    }

    const TTL: Duration = Duration::from_secs(300);
    const MILLI: Duration = Duration::from_millis(1);

    /// A `SystemReadiness` over `runner` that finds an executable named like the executor.
    fn readiness<R: CommandRunner + 'static>(runner: &Arc<R>, ttl: Duration) -> SystemReadiness {
        SystemReadiness::with_parts(
            ttl,
            FAILED_CHECK_TTL,
            runner.clone(),
            Arc::new(|kind| Some(PathBuf::from(kind.program_name()))),
            ExecEnv::inherit().shared(),
            inherited(),
        )
    }

    /// What a read of the environment's sources finds when the login shell gives `shell` and
    /// nothing else is set.
    fn read_with_shell(shell: Result<crate::env::Vars, String>) -> Sources {
        Sources {
            host: crate::env::Vars::new(),
            shell,
            system_proxy: Ok(Vec::new()),
            system_path: Ok((None, None)),
        }
    }

    /// A `refresh_env` that finds nothing beyond the host's own environment.
    fn inherited() -> RefreshEnv {
        Arc::new(|| Box::pin(async { read_with_shell(Ok(crate::env::Vars::new())) }))
    }

    fn kinds(reports: &[ExecutorReport]) -> Vec<ExecutorKind> {
        reports.iter().map(|r| r.kind).collect()
    }

    #[test]
    fn the_real_probe_keeps_the_ttl_and_retries_failed_checks_after_a_minute() {
        let probe = SystemReadiness::new(TTL, ExecEnv::inherit().shared());
        assert_eq!(probe.ttl, TTL);
        assert_eq!(probe.failed_ttl, Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_that_could_not_run_is_retried_soon_and_only_for_that_executor() {
        // Codex's `mcp list` has no reply, so it fails like a timeout would.
        let fake = Arc::new(both_ready().without("codex mcp list"));
        let probe = readiness(&fake, TTL);

        let first = probe.executor_reports(false).await;
        assert_eq!(kinds(&first), [ExecutorKind::Claude, ExecutorKind::Codex]);
        assert!(first[0].ready());
        assert!(!first[1].ready());
        assert_eq!(fake.take_calls(), ALL_SIX);

        // An immediate second call is answered from the cache.
        assert_eq!(probe.executor_reports(false).await, first);
        assert!(fake.take_calls().is_empty());

        // Codex's failed entry is kept for FAILED_CHECK_TTL ...
        tokio::time::advance(FAILED_CHECK_TTL - MILLI).await;
        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());

        // ... then only Codex's three commands run again; Claude's complete entry is still good.
        tokio::time::advance(MILLI).await;
        let retried = probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), CODEX_THREE);
        assert_eq!(kinds(&retried), [ExecutorKind::Claude, ExecutorKind::Codex]);
        assert_eq!(retried[0], first[0]);

        // Once ttl has passed since Claude's check, both are checked again.
        tokio::time::advance(TTL - FAILED_CHECK_TTL).await;
        let later = probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), ALL_SIX);
        assert_eq!(kinds(&later), [ExecutorKind::Claude, ExecutorKind::Codex]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_recovered_check_is_kept_for_the_full_ttl_and_each_executor_ages_on_its_own() {
        let fake = Arc::new(both_ready().without("codex mcp list"));
        let probe = readiness(&fake, TTL);
        probe.executor_reports(false).await;
        fake.take_calls();

        // t = 60 s: Codex is retried and now answers.
        fake.set("codex mcp list", out(0, CODEX_OK));
        tokio::time::advance(FAILED_CHECK_TTL).await;
        let reports = probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), CODEX_THREE);
        assert!(reports.iter().all(|r| r.ready()), "{reports:?}");

        // Its entry is complete now, so it is not retried after FAILED_CHECK_TTL.
        tokio::time::advance(TTL - FAILED_CHECK_TTL - MILLI).await;
        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());

        // t = 300 s: Claude's entry (t = 0) expires, Codex's (t = 60 s) does not.
        tokio::time::advance(MILLI).await;
        probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), CLAUDE_THREE);

        // t = 360 s: Codex's entry expires, Claude's (t = 300 s) does not.
        tokio::time::advance(FAILED_CHECK_TTL - MILLI).await;
        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());
        tokio::time::advance(MILLI).await;
        probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), CODEX_THREE);
    }

    #[tokio::test(start_paused = true)]
    async fn complete_checks_are_reused_for_exactly_the_ttl() {
        let fake = Arc::new(both_ready());
        let probe = readiness(&fake, TTL);
        probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), ALL_SIX);

        tokio::time::advance(TTL - MILLI).await;
        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());

        tokio::time::advance(MILLI).await;
        probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), ALL_SIX);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_check_is_never_kept_longer_than_a_short_ttl() {
        let fake = Arc::new(both_ready().without("codex mcp list"));
        let ttl = Duration::from_secs(10);
        let probe = readiness(&fake, ttl);
        probe.executor_reports(false).await;
        fake.take_calls();

        tokio::time::advance(ttl - MILLI).await;
        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());

        // Codex's failed entry would otherwise live for FAILED_CHECK_TTL (60 s).
        tokio::time::advance(MILLI).await;
        probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), ALL_SIX);
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_skips_the_cache_even_in_the_same_instant() {
        let fake = Arc::new(both_ready());
        let probe = readiness(&fake, TTL);
        probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), ALL_SIX);

        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());

        probe.executor_reports(true).await;
        assert_eq!(fake.take_calls(), ALL_SIX);

        // What the fresh call found is cached for the next ordinary call.
        probe.executor_reports(false).await;
        assert!(fake.take_calls().is_empty());
    }

    /// Every event at `INFO` or above that the current thread logs while `f` runs, one string per
    /// event: the level, the message, then `name=value` for each field.
    fn logged<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
        use std::fmt::{Debug, Write as _};
        use tracing::{Event, Level, Metadata, Subscriber, field::Visit, span};

        struct Capture(Arc<Mutex<Vec<String>>>);

        struct Fields(String);

        impl Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
                if field.name() == "message" {
                    write!(self.0, " {value:?}").unwrap();
                } else {
                    write!(self.0, " {}={value:?}", field.name()).unwrap();
                }
            }
        }

        impl Subscriber for Capture {
            fn enabled(&self, metadata: &Metadata<'_>) -> bool {
                *metadata.level() <= Level::INFO
            }

            fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
                span::Id::from_u64(1)
            }

            fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

            fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

            fn event(&self, event: &Event<'_>) {
                let mut fields = Fields(event.metadata().level().to_string());
                event.record(&mut fields);
                self.0.lock().unwrap().push(fields.0);
            }

            fn enter(&self, _: &span::Id) {}

            fn exit(&self, _: &span::Id) {}
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        // As in `tests/engine.rs`: with a second, idle subscriber registered, tracing asks every
        // registered one whether a log call is wanted, instead of caching the answer of the first
        // thread that reaches it (in a parallel test, a thread without a subscriber).
        let _idle = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let result = tracing::subscriber::with_default(Capture(events.clone()), f);
        let events = events.lock().unwrap().clone();
        (result, events)
    }

    fn paused_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
    }

    /// Both executors ready for usual runs and for lean ones, checked as the host checks them.
    fn ready_for_lean_runs(dir: &tempfile::TempDir) -> (Arc<FakeRunner>, SystemReadiness) {
        let fake = Arc::new(both_ready());
        fake.set("claude auth status", out(0, CLAUDE_USUAL));
        fake.set(
            "claude --setting-sources project,local auth status",
            out(0, CLAUDE_USUAL),
        );
        fake.set(
            "codex -c cli_auth_credentials_store=file login status",
            out(0, "Logged in using ChatGPT"),
        );
        let probe = readiness(&fake, TTL).with_lean_checks(lean_paths(dir));
        (fake, probe)
    }

    #[test]
    fn an_executor_that_stops_being_ready_is_logged_once_with_its_problems() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, probe) = ready_for_lean_runs(&dir);
        let (first, events) = logged(|| {
            paused_runtime().block_on(async {
                let first = probe.executor_reports(true).await;
                // Claude's Linear server needs authentication for two checks, then it is back.
                fake.set("claude mcp list", out(0, CLAUDE_AUTH));
                probe.executor_reports(true).await;
                probe.executor_reports(true).await;
                fake.set("claude mcp list", out(0, CLAUDE_OK));
                probe.executor_reports(true).await;
                probe.executor_reports(true).await;
                first
            })
        });
        assert!(
            first.iter().all(|r| r.ready() && r.ready_lean()),
            "{first:?}"
        );
        // The first check has nothing to compare with, and a check that changes nothing logs
        // nothing: one line when Claude falls, one when it is back.
        let [fell, back] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(
            fell.starts_with(r#"WARN executor no longer ready executor="claude" problems="#),
            "{fell}"
        );
        // The usual problem, not the lean one ("the Linear MCP server `linear-server` in …").
        assert!(fell.contains("Linear MCP in claude"), "{fell}");
        assert!(
            back.starts_with(r#"INFO executor ready now executor="claude""#),
            "{back}"
        );
    }

    #[test]
    fn an_executor_that_stops_being_ready_for_lean_runs_says_only_that() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, probe) = ready_for_lean_runs(&dir);
        let lean_login = "claude --setting-sources project,local auth status";
        let ((), events) = logged(|| {
            paused_runtime().block_on(async {
                probe.executor_reports(true).await;
                // A lean run would not be signed in; usual runs still are.
                fake.set(lean_login, out(0, r#"{"loggedIn": false}"#));
                probe.executor_reports(true).await;
                fake.set(lean_login, out(0, CLAUDE_USUAL));
                probe.executor_reports(true).await;
            })
        });
        let [fell, back] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(
            fell.starts_with(
                r#"WARN executor no longer ready for lean runs executor="claude" problems="#
            ),
            "{fell}"
        );
        assert!(fell.contains("not signed in"), "{fell}");
        assert!(
            back.starts_with(r#"INFO executor ready now executor="claude""#),
            "{back}"
        );
    }

    #[test]
    fn an_executor_back_for_usual_runs_only_names_what_still_holds_lean_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, probe) = ready_for_lean_runs(&dir);
        fake.set("codex mcp list", out(1, ""));
        let ((), events) = logged(|| {
            paused_runtime().block_on(async {
                // Not ready from the first check, which logs nothing.
                probe.executor_reports(true).await;
                // Codex's usual runs are fine again, its lean runs would not be signed in.
                fake.set("codex mcp list", out(0, CODEX_OK));
                fake.set(
                    "codex -c cli_auth_credentials_store=file login status",
                    out(1, "Not logged in"),
                );
                probe.executor_reports(true).await;
            })
        });
        let [back] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert!(
            back.starts_with(
                r#"INFO executor ready now, but not for lean runs executor="codex" problems="#
            ),
            "{back}"
        );
        assert!(back.contains("not signed in"), "{back}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_call_that_waited_behind_a_running_check_reuses_its_result() {
        let fake = Arc::new(both_ready().with_delay(Duration::from_secs(1)));
        let probe = Arc::new(readiness(&fake, TTL));
        let running = tokio::spawn({
            let probe = probe.clone();
            async move { probe.executor_reports(false).await }
        });
        // The spawned call takes the lock and starts all six commands, then waits for them.
        tokio::task::yield_now().await;
        assert_eq!(fake.take_calls(), ALL_SIX);

        // This call is made while that check is running, so the check's result is newer than it.
        let fresh = probe.executor_reports(true).await;
        let checked = running.await.unwrap();
        assert_eq!(fresh, checked);
        assert!(
            fake.take_calls().is_empty(),
            "the fresh call ran the commands again"
        );

        // A fresh call made after that check finished does run them again.
        probe.executor_reports(true).await;
        assert_eq!(fake.take_calls(), ALL_SIX);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_refresh_records_nothing_and_releases_the_lock() {
        let fake = Arc::new(both_ready().with_delay(Duration::from_secs(10)));
        let probe = readiness(&fake, TTL);
        let cancelled =
            tokio::time::timeout(Duration::from_secs(1), probe.executor_reports(false)).await;
        assert!(cancelled.is_err());
        assert_eq!(fake.take_calls(), ALL_SIX);

        // Nothing was cached and the lock is free: the next call checks again.
        let reports = probe.executor_reports(false).await;
        assert_eq!(fake.take_calls(), ALL_SIX);
        assert!(reports.iter().all(|r| r.ready()), "{reports:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn both_executors_are_checked_together() {
        // Six commands wait for each other: all of them must be running at the same time.
        let runner = Arc::new(BarrierRunner::new(6, both_ready()));
        let probe = readiness(&runner, TTL);
        let checked =
            tokio::time::timeout(Duration::from_secs(5), probe.executor_reports(false)).await;
        let reports = checked.expect("both executors must be checked at the same time");
        assert_eq!(kinds(&reports), [ExecutorKind::Claude, ExecutorKind::Codex]);
        assert!(reports.iter().all(|r| r.ready()), "{reports:?}");
    }

    #[tokio::test]
    async fn a_fresh_check_resolves_the_environment_and_reports_it() {
        use crate::env::EnvSource;

        let env = ExecEnv::inherit().shared();
        let readiness = SystemReadiness::with_parts(
            TTL,
            FAILED_CHECK_TTL,
            Arc::new(runner(&[])),
            Arc::new(|_| None),
            env.clone(),
            Arc::new(|| {
                Box::pin(async {
                    read_with_shell(Ok([("PATH".to_string(), "/opt/demo/bin".to_string())]
                        .into_iter()
                        .collect()))
                })
            }),
        );
        assert_eq!(
            readiness.env().unwrap().path,
            None,
            "nothing resolved before the first fresh check"
        );
        readiness.executor_reports(false).await;
        assert_eq!(
            readiness.env().unwrap().path,
            None,
            "a cached check does not re-resolve"
        );
        readiness.executor_reports(true).await;
        assert_eq!(
            readiness.env().unwrap().path.as_deref(),
            Some("/opt/demo/bin")
        );
        assert_eq!(readiness.env().unwrap().path_source, EnvSource::LoginShell);
        assert_eq!(
            env.read().unwrap().path.as_deref(),
            Some("/opt/demo/bin"),
            "the job runner sees it too"
        );
    }

    // One slow rc file during a re-check must not swap a good login-shell PATH for the host's
    // own (launchd's minimal one for an app started from Finder): the executors would read as not
    // installed, and that complete answer would be cached for the whole TTL.
    #[tokio::test]
    async fn a_source_that_fails_on_a_later_check_keeps_what_it_gave_before() {
        use crate::env::EnvSource;
        use std::sync::atomic::AtomicUsize;

        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let readiness = SystemReadiness::with_parts(
            TTL,
            FAILED_CHECK_TTL,
            Arc::new(runner(&[])),
            Arc::new(|_| None),
            ExecEnv::inherit().shared(),
            Arc::new(move || {
                let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
                Box::pin(async move {
                    read_with_shell(if first {
                        Ok([
                            ("PATH".to_string(), "/opt/demo/bin".to_string()),
                            ("HTTPS_PROXY".to_string(), "http://127.0.0.1:9".to_string()),
                        ]
                        .into_iter()
                        .collect())
                    } else {
                        Err("/bin/zsh did not finish within 10s".to_string())
                    })
                })
            }),
        );
        readiness.executor_reports(true).await;
        let first = readiness.env().unwrap();
        assert_eq!(first.path.as_deref(), Some("/opt/demo/bin"));
        assert!(first.problems.is_empty(), "{:?}", first.problems);

        readiness.executor_reports(true).await;
        assert_eq!(
            reads.load(Ordering::SeqCst),
            2,
            "each fresh check reads again"
        );
        let second = readiness.env().unwrap();
        assert_eq!(second.path, first.path, "the earlier PATH is kept");
        assert_eq!(second.path_source, EnvSource::LoginShell);
        assert_eq!(second.proxy, first.proxy, "and the earlier proxy");
        assert!(
            second
                .problems
                .iter()
                .any(|p| p.contains("login shell") && p.contains("did not finish within 10s")),
            "the failure is still reported: {:?}",
            second.problems
        );
    }

    #[tokio::test]
    async fn executors_are_looked_up_on_the_shared_environments_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };
        let program = dir.path().join(name);
        std::fs::write(&program, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = ExecEnv::inherit().shared();
        let probe = SystemReadiness::new(TTL, env.clone());
        // The path is set after the probe exists and replaces the host's PATH, so a `claude` or
        // `codex` installed on the machine running the test cannot be the one that is found.
        env.write().unwrap().path = Some(dir.path().to_string_lossy().into_owned());
        let reports = probe.executor_reports(false).await;
        assert_eq!(reports[0].program, Some(program));
        assert_eq!(reports[1].program, None);
    }

    // H1 depends on this wiring: `claude` is a node launcher that needs the resolved PATH, and it
    // reaches Linear only with the resolved proxy. A fresh check resolves the environment first;
    // `with_refresh` replaces only that step, so no login shell runs and everything else is wired
    // as in `new`. Unix only because the fake `claude` is a shell script; the same hand-over of the
    // shared environment to a `.cmd` check command is tested in tests/doctor.rs.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_fresh_check_runs_the_executor_found_on_the_resolved_path_with_the_resolved_proxy() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        // `--version` shows what reached the command; the other commands print nothing.
        std::fs::write(
            &claude,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"fake-claude $HTTPS_PROXY\"; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        // What the login shell gives: the folder with the fake `claude` and a proxy. The host's
        // own variables are left out, so a proxy the machine running the test sets cannot win.
        let shell: crate::env::Vars = [
            (
                "PATH".to_string(),
                dir.path().to_string_lossy().into_owned(),
            ),
            ("HTTPS_PROXY".to_string(), "http://127.0.0.1:9".to_string()),
        ]
        .into_iter()
        .collect();
        let probe = SystemReadiness::with_refresh(
            TTL,
            ExecEnv::inherit().shared(),
            Arc::new(move || {
                let shell = shell.clone();
                Box::pin(async move { read_with_shell(Ok(shell)) })
            }),
        );
        // Executing a script right after writing it can fail with `ETXTBSY` when another test
        // thread forks in between (`tests/doctor.rs::run_script` does the same): try again.
        let mut reports = Vec::new();
        for _ in 0..5 {
            reports = probe.executor_reports(true).await;
            if reports[0].version.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            reports[0].version.as_deref(),
            Some("fake-claude http://127.0.0.1:9"),
            "{reports:?}"
        );
        assert_eq!(reports[0].program, Some(claude));
        assert_eq!(reports[1].program, None, "{reports:?}");
    }

    #[test]
    fn a_doctor_report_does_not_print_proxy_credentials_in_its_debug_output() {
        let report = DoctorReport {
            viewer: None,
            linear_error: None,
            executors: Vec::new(),
            workspace: None,
            bindings: Vec::new(),
            env: Some(ExecEnv {
                proxy: vec![(
                    "HTTPS_PROXY".into(),
                    "http://jane:s3cret@proxy.example:8080".into(),
                )],
                ..ExecEnv::inherit()
            }),
        };
        let text = format!("{report:?}");
        assert!(!text.contains("s3cret"), "{text}");
        assert!(text.contains("http://***@proxy.example:8080"), "{text}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_executor_that_is_not_installed_is_reported_without_running_commands() {
        let fake = Arc::new(both_ready());
        let probe = SystemReadiness::with_parts(
            TTL,
            FAILED_CHECK_TTL,
            fake.clone(),
            Arc::new(|kind| (kind == ExecutorKind::Claude).then(|| PathBuf::from("claude"))),
            ExecEnv::inherit().shared(),
            inherited(),
        );
        let reports = probe.executor_reports(false).await;
        assert_eq!(kinds(&reports), [ExecutorKind::Claude, ExecutorKind::Codex]);
        assert!(reports[0].ready());
        assert!(!reports[1].ready());
        assert!(reports[1].problems[0].contains("not installed"));
        assert_eq!(fake.take_calls(), CLAUDE_THREE);
    }
}
