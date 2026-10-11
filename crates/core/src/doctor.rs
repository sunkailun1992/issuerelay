//! Environment checks: are Claude Code / Codex installed, logged in and connected to Linear;
//! does the Linear workspace have the label and statuses IssueRelay relies on; are bindings usable.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

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

/// Limit for `mcp list`. `claude mcp list` health-checks every configured MCP server, so its
/// time grows with their number: measured on 2026-10-11 with about 35 servers it took 11-14 s,
/// while `codex mcp list` took 0.3 s.
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// Limit for every other check (`--version`, `auth status`, `login status`), which only read
/// local state.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs the real check commands, each under its own time limit: `MCP_LIST_TIMEOUT` for
/// `mcp list`, `CHECK_TIMEOUT` for the rest. A command still running at its limit has its
/// process killed and the call fails with `timed out after N s`; processes it started itself
/// may outlive it.
pub struct SystemRunner;

#[async_trait::async_trait]
impl CommandRunner for SystemRunner {
    async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
        let limit = if matches!(args, ["mcp", "list"]) {
            MCP_LIST_TIMEOUT
        } else {
            CHECK_TIMEOUT
        };
        let child = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(limit, child)
            .await
            .map_err(|_| anyhow!("timed out after {} s", limit.as_secs()))??;
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
        return report;
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
    match version {
        Ok(o) if o.code == Some(0) => {
            report.version = o.stdout.lines().next().map(|l| l.trim().to_string())
        }
        Ok(o) => report
            .problems
            .push(format!("`{name} --version` failed: {}", o.stderr.trim())),
        Err(e) => report
            .problems
            .push(format!("`{name} --version` failed: {e}")),
    }
    match login {
        Ok(o) => {
            report.logged_in = match kind {
                ExecutorKind::Claude => parse_claude_auth(&o.stdout),
                ExecutorKind::Codex => parse_codex_login(o.code, &o.stdout, &o.stderr),
            };
            if !report.logged_in {
                report.problems.push(format!("{name} is not logged in"));
            }
        }
        Err(e) => report.problems.push(format!("login check failed: {e}")),
    }
    match mcp {
        Ok(o) => {
            report.linear_mcp = mcp_list_has_linear(kind, &o.stdout);
            if !report.linear_mcp {
                report.problems.push(format!(
                    "Linear MCP is not configured or not authorized in {name}"
                ));
            }
        }
        Err(e) => report
            .problems
            .push(format!("`{name} mcp list` failed: {e}")),
    }
    report
}

/// Reads `claude auth status` output, a JSON object with a `loggedIn` boolean. Anything else
/// counts as not logged in.
pub fn parse_claude_auth(stdout: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .and_then(|v| v["loggedIn"].as_bool())
        .unwrap_or(false)
}

/// Reads `codex login status`: it exits 0 and prints "Logged in using ...". Both streams are
/// read because codex-cli 0.159.2 writes the status to stderr (M0 finding F4).
pub fn parse_codex_login(code: Option<i32>, stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_lowercase();
    code == Some(0) && text.contains("logged in") && !text.contains("not logged in")
}

/// Whether `mcp list` output shows Linear's MCP server usable: for Claude a "Connected" line,
/// for Codex a table row that is neither disabled nor logged out. Formats confirmed with Claude
/// Code 2.1.285 and codex-cli 0.159.2 on 2026-10-10
/// (docs/gates/samples/m0/executors/*-mcp-list.txt).
pub fn mcp_list_has_linear(kind: ExecutorKind, stdout: &str) -> bool {
    stdout.lines().any(|line| {
        let line = line.to_lowercase();
        line.contains("mcp.linear.app")
            && match kind {
                ExecutorKind::Claude => {
                    line.contains("connected")
                        && !line.contains("disconnected")
                        && !line.contains("needs authentication")
                        && !line.contains("failed")
                }
                ExecutorKind::Codex => {
                    !line.contains("disabled") && !line.contains("not logged in")
                }
            }
    })
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
    /// `fresh` bypasses any cache (the doctor command wants current results).
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport>;
}

/// Checks the real `claude` / `codex` on PATH and caches the result for `ttl`.
pub struct SystemReadiness {
    runner: SystemRunner,
    ttl: Duration,
    cache: tokio::sync::Mutex<Option<(Instant, Vec<ExecutorReport>)>>,
}

impl SystemReadiness {
    /// `ttl` is how long `executor_reports(false)` reuses a result.
    pub fn new(ttl: Duration) -> Self {
        Self {
            runner: SystemRunner,
            ttl,
            cache: tokio::sync::Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl ReadinessProbe for SystemReadiness {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        // The lock stays held while checking: a caller that arrives meanwhile waits for these
        // results (unless it asks for `fresh`) instead of starting a second round of commands.
        let mut cache = self.cache.lock().await;
        if !fresh
            && let Some((at, reports)) = cache.as_ref()
            && at.elapsed() < self.ttl
        {
            return reports.clone();
        }
        // Checked together; the vector is always Claude first, then Codex.
        let (claude, codex) = tokio::join!(
            check_executor(
                ExecutorKind::Claude,
                executor::resolve_program(ExecutorKind::Claude).ok(),
                &self.runner,
            ),
            check_executor(
                ExecutorKind::Codex,
                executor::resolve_program(ExecutorKind::Codex).ok(),
                &self.runner,
            ),
        );
        let reports = vec![claude, codex];
        *cache = Some((Instant::now(), reports.clone()));
        reports
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

    struct FakeRunner(HashMap<String, CmdOutput>);

    #[async_trait::async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(&self, _program: &Path, args: &[&str]) -> Result<CmdOutput> {
            self.0
                .get(&args.join(" "))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unexpected command {args:?}"))
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
        FakeRunner(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    const CLAUDE_OK: &str = "linear-server: https://mcp.linear.app/mcp (HTTP) - ✔ Connected";
    const CLAUDE_AUTH: &str =
        "linear-server: https://mcp.linear.app/mcp (HTTP) - ! Needs authentication";

    #[tokio::test]
    async fn claude_ready_when_logged_in_and_linear_connected() {
        let r = runner(&[
            ("--version", out(0, "2.1.285 (Claude Code)\n")),
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
        assert!(!parse_claude_auth("not json"));
        assert!(parse_codex_login(Some(0), "Logged in using ChatGPT", ""));
        // Real codex-cli 0.159.2 writes the status to stderr (M0 finding F4).
        assert!(parse_codex_login(Some(0), "", "Logged in using ChatGPT"));
        assert!(!parse_codex_login(Some(1), "Not logged in", ""));
        assert!(!parse_codex_login(Some(0), "Not logged in", ""));
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
}
