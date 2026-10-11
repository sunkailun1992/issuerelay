//! Environment checks: are Claude Code / Codex installed, logged in and connected to Linear;
//! does the Linear workspace have the label and statuses IssueRelay relies on; are bindings usable.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use url::Url;

use crate::config::{Config, ExecutorKind, TriggerNames};
use crate::executor;
use crate::linear::{LinearApi, Viewer};
use crate::trigger::same_name;

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
    /// `Err` means the command could not run to completion (it did not start, or timed out); a
    /// command that ran and exited non-zero is an `Ok`.
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput>;
}

/// Default limit for `mcp list`. `claude mcp list` health-checks every configured MCP server,
/// so its time grows with their number: measured on 2026-10-11 with about 35 servers it took
/// 11-14 s, while `codex mcp list` took 0.3 s.
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// Default limit for every other check (`--version`, `auth status`, `login status`), which
/// only read local state.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// How long `SystemReadiness` keeps an executor's check that timed out or whose output could
/// not be read, instead of the full cache TTL: soon enough to recover from a transient
/// failure, late enough that the engine's poll loop does not re-run a slow `claude mcp list`
/// on every poll.
const FAILED_CHECK_TTL: Duration = Duration::from_secs(60);

/// The most characters of command output that a problem carries.
const MAX_PROBLEM_TEXT: usize = 200;

/// The host of Linear's MCP server (the host of `LINEAR_MCP_URL`). An `mcp list` line only
/// describes Linear when it names exactly this host.
const LINEAR_MCP_HOST: &str = "mcp.linear.app";

/// Runs the real check commands, each under its own time limit: by default 60 s for
/// `mcp list` and 20 s for the rest (`MCP_LIST_TIMEOUT`, `CHECK_TIMEOUT`). A command still
/// running at its limit has its process killed and the call fails with `timed out after 20s`
/// (the limit as `Duration` prints it); processes it started itself may outlive it.
pub struct SystemRunner {
    check_timeout: Duration,
    mcp_list_timeout: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::with_limits(CHECK_TIMEOUT, MCP_LIST_TIMEOUT)
    }
}

impl SystemRunner {
    /// For tests: `check` limits every command except `mcp list`, `mcp_list` limits that one.
    pub fn with_limits(check: Duration, mcp_list: Duration) -> Self {
        Self {
            check_timeout: check,
            mcp_list_timeout: mcp_list,
        }
    }

    fn limit_for(&self, args: &[&str]) -> Duration {
        if matches!(args, ["mcp", "list"]) {
            self.mcp_list_timeout
        } else {
            self.check_timeout
        }
    }
}

#[async_trait::async_trait]
impl CommandRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
        let limit = self.limit_for(args);
        let child = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(limit, child)
            .await
            .map_err(|_| anyhow!("timed out after {limit:?}"))??;
        Ok(CmdOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Whether one executor can take jobs, and what is wrong when it cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutorReport {
    pub kind: ExecutorKind,
    /// Where the executable was found; `None` when it is not installed.
    pub program: Option<PathBuf>,
    /// First line of `--version`, when that ran.
    pub version: Option<String>,
    pub logged_in: bool,
    /// Linear's MCP server is configured, authorized and connected in this executor.
    pub linear_mcp: bool,
    /// Human-readable reasons for the checks that failed or could not run.
    pub problems: Vec<String>,
}

impl ExecutorReport {
    /// Installed, logged in and connected to Linear. A check that could not run leaves this
    /// `false`: readiness was not confirmed.
    pub fn ready(&self) -> bool {
        self.program.is_some() && self.logged_in && self.linear_mcp
    }
}

/// Checks that one executor is installed, logged in and connected to Linear's MCP server.
/// `program` is where the executable was found, `None` when it was not found. The three
/// commands (`--version`, the login check, `mcp list`) run together, since `claude mcp list`
/// alone can take over ten seconds. A check that cannot run adds a problem saying so and leaves
/// the executor not ready, without also reporting the setup as missing.
pub async fn check_executor(
    kind: ExecutorKind,
    program: Option<PathBuf>,
    runner: &dyn CommandRunner,
) -> ExecutorReport {
    check_executor_detailed(kind, program, runner).await.0
}

/// `check_executor`, plus whether every check gave a usable answer. A check gives none when its
/// command could not run (it timed out or did not start) or, for the login check, when its
/// output cannot be read. A negative answer that can be read ("not logged in", no Linear
/// server) is usable. `SystemReadiness` retries an incomplete result sooner.
async fn check_executor_detailed(
    kind: ExecutorKind,
    program: Option<PathBuf>,
    runner: &dyn CommandRunner,
) -> (ExecutorReport, bool) {
    let name = kind.program_name();
    let mut report = ExecutorReport {
        kind,
        program: program.clone(),
        version: None,
        logged_in: false,
        linear_mcp: false,
        problems: Vec::new(),
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
    let (version, login, mcp) = tokio::join!(
        runner.run(&program, &["--version"]),
        runner.run(&program, login_args),
        runner.run(&program, &["mcp", "list"]),
    );
    let mut complete = true;
    match version {
        Ok(o) if o.code == Some(0) => {
            report.version = o.stdout.lines().next().map(|l| l.trim().to_string())
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
    match login {
        Ok(o) => match login_answer(kind, &o) {
            Some(logged_in) => {
                report.logged_in = logged_in;
                if !logged_in {
                    report.problems.push(format!("{name} is not logged in"));
                }
            }
            None => {
                let code = o
                    .code
                    .map_or_else(|| "unknown".to_string(), |code| code.to_string());
                report.problems.push(with_detail(
                    format!(
                        "`{name} {}` gave no usable answer (exit code {code})",
                        login_args.join(" ")
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
                report.problems.push(linear_problem(kind, &o.stdout));
            }
        }
        Err(e) => {
            report
                .problems
                .push(format!("`{name} mcp list` failed: {e}"));
            complete = false;
        }
    }
    (report, complete)
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
    serde_json::from_str::<serde_json::Value>(stdout.trim()).ok()?["loggedIn"].as_bool()
}

/// `codex login status` exits 0 and prints "Logged in using ..." when logged in. Both streams
/// are read because codex-cli 0.159.2 writes the status to stderr (M0 finding F4). `None` when
/// neither "logged in" nor "not logged in" appears (the second contains the first).
fn codex_login_answer(code: Option<i32>, stdout: &str, stderr: &str) -> Option<bool> {
    let text = format!("{stdout}\n{stderr}").to_lowercase();
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

/// Whether `mcp list` output shows Linear's MCP server usable. A line describes Linear only
/// when one of its whitespace-separated tokens is an `http(s)` URL on exactly `mcp.linear.app`;
/// another host, a query parameter or a server name that merely contains that text does not
/// count. Claude then has to say `Connected` after the last ` - ` (a leading check mark or other
/// symbol is ignored); Codex has to show the `enabled` token and not `Not logged in`. The
/// formats come from the M0 samples (docs/gates/samples/m0/executors/*-mcp-list.txt,
/// 2026-10-10).
pub fn mcp_list_has_linear(kind: ExecutorKind, stdout: &str) -> bool {
    linear_lines(stdout).any(|line| line_usable(kind, line))
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

/// Why `mcp list` shows no usable Linear server: what its Linear line says, or that there is no
/// such line.
fn linear_problem(kind: ExecutorKind, stdout: &str) -> String {
    let name = kind.program_name();
    let status = linear_lines(stdout)
        .next()
        .map(|line| clean(&line_status(kind, line)))
        .filter(|status| !status.is_empty());
    match status {
        Some(status) => format!("Linear MCP in {name}: {status}"),
        None => format!("Linear MCP is not configured or not authorized in {name}"),
    }
}

/// Command output as a problem may show it: control characters dropped, at most
/// `MAX_PROBLEM_TEXT` characters.
fn clean(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    text.trim().chars().take(MAX_PROBLEM_TEXT).collect()
}

/// The first non-empty line of command output, `clean`ed; empty when there is none.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(clean)
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
}

impl WorkspaceReport {
    /// The label and every status are in place.
    pub fn ok(&self) -> bool {
        self.label_ok && self.missing_states.is_empty()
    }
}

/// Looks for the trigger label and, in every team, the four statuses IssueRelay moves issues
/// through. Names compare like everywhere else (`same_name`). Errors when Linear cannot be
/// queried.
pub async fn check_workspace(api: &dyn LinearApi, names: &TriggerNames) -> Result<WorkspaceReport> {
    let label_ok = api
        .labels()
        .await?
        .iter()
        .any(|l| same_name(l, &names.label));
    let mut missing_states = Vec::new();
    for team in api.teams().await? {
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
    pub executor_ready: bool,
}

/// One report per binding, in configuration order. `ready` is the set from `ready_set`.
pub fn check_bindings(cfg: &Config, ready: &HashSet<ExecutorKind>) -> Vec<BindingReport> {
    cfg.bindings
        .iter()
        .map(|b| {
            let executor = cfg.executor_for(b);
            BindingReport {
                binding_id: b.id.clone(),
                project_name: b.project_name.clone(),
                directory_exists: b.directory.is_dir(),
                executor,
                executor_ready: ready.contains(&executor),
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

/// Checks the real `claude` / `codex` on PATH and caches what it finds, one entry per executor.
/// An entry whose checks all gave a usable answer is reused for `ttl`; one with a check that
/// timed out, did not start or could not be read for at most `FAILED_CHECK_TTL`, so it is
/// retried soon without re-running a slow `claude mcp list` on every poll. Only the executors
/// whose entry expired are checked again, together. One lock covers the whole refresh, so a
/// caller that arrives meanwhile waits and then uses what the refresh recorded.
pub struct SystemReadiness {
    runner: Arc<dyn CommandRunner>,
    resolve: fn(ExecutorKind) -> Option<PathBuf>,
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
    pub fn new(ttl: Duration) -> Self {
        Self::with_parts(
            ttl,
            FAILED_CHECK_TTL,
            Arc::new(SystemRunner::default()),
            |kind| executor::resolve_program(kind).ok(),
        )
    }

    /// The seams the unit tests use: both lifetimes, the command runner and where to find the
    /// executables.
    fn with_parts(
        ttl: Duration,
        failed_ttl: Duration,
        runner: Arc<dyn CommandRunner>,
        resolve: fn(ExecutorKind) -> Option<PathBuf>,
    ) -> Self {
        Self {
            runner,
            resolve,
            ttl,
            failed_ttl,
            refreshes: AtomicU64::new(0),
            cache: tokio::sync::Mutex::new(HashMap::new()),
        }
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
        let (report, complete) = check_executor_detailed(kind, program, self.runner.as_ref()).await;
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
}

/// The executors whose report says they are ready.
pub fn ready_set(reports: &[ExecutorReport]) -> HashSet<ExecutorKind> {
    reports
        .iter()
        .filter(|r| r.ready())
        .map(|r| r.kind)
        .collect()
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
        check_executor_detailed(kind, program, &runner(replies)).await
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
            assert_eq!(
                report.problems,
                [format!(
                    "Linear MCP is not configured or not authorized in {name}"
                )]
            );
            assert!(complete);
        }
    }

    #[tokio::test]
    async fn a_status_in_a_problem_is_one_clean_short_line() {
        let long = format!("Failed to connect \u{1b}[0m\t{}", "y".repeat(300));
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
        let expected: String = "Failed to connect [0m"
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
        assert_eq!(clean("  a\u{1b}[31mb\tc\r  "), "a[31mbc");
        assert_eq!(clean(&"x".repeat(300)).chars().count(), 200);
        // The limit counts characters, not bytes.
        assert_eq!(clean(&"—".repeat(300)).chars().count(), 200);
        assert_eq!(first_line("\n  \n  first\nsecond"), "first");
        assert_eq!(first_line("a\u{7}b"), "ab");
        assert_eq!(first_line(" \n "), "");
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
            check_executor_detailed(ExecutorKind::Codex, None, &runner(&[])).await;
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
        };
        let reports = [
            report(ExecutorKind::Claude, false),
            report(ExecutorKind::Codex, true),
        ];
        let ready: HashSet<_> = [ExecutorKind::Codex].into_iter().collect();
        assert_eq!(ready_set(&reports), ready);
        assert!(ready_set(&[]).is_empty());
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
            });
        }
        let ready: HashSet<_> = [ExecutorKind::Claude].into_iter().collect();
        let reports = check_bindings(&cfg, &ready);
        assert!(reports[0].directory_exists && reports[0].executor_ready);
        assert!(!reports[1].directory_exists && !reports[1].executor_ready);
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
        SystemReadiness::with_parts(ttl, FAILED_CHECK_TTL, runner.clone(), |kind| {
            Some(PathBuf::from(kind.program_name()))
        })
    }

    fn kinds(reports: &[ExecutorReport]) -> Vec<ExecutorKind> {
        reports.iter().map(|r| r.kind).collect()
    }

    #[test]
    fn the_real_probe_keeps_the_ttl_and_retries_failed_checks_after_a_minute() {
        let probe = SystemReadiness::new(TTL);
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

    #[tokio::test(start_paused = true)]
    async fn an_executor_that_is_not_installed_is_reported_without_running_commands() {
        let fake = Arc::new(both_ready());
        let probe = SystemReadiness::with_parts(TTL, FAILED_CHECK_TTL, fake.clone(), |kind| {
            (kind == ExecutorKind::Claude).then(|| PathBuf::from("claude"))
        });
        let reports = probe.executor_reports(false).await;
        assert_eq!(kinds(&reports), [ExecutorKind::Claude, ExecutorKind::Codex]);
        assert!(reports[0].ready());
        assert!(!reports[1].ready());
        assert!(reports[1].problems[0].contains("not installed"));
        assert_eq!(fake.take_calls(), CLAUDE_THREE);
    }
}
