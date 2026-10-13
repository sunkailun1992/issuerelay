//! One-click setup commands offered by the environment check (design section 8) and the
//! settings page (design section 10.3): adding Linear's MCP server to an executor, starting the
//! executor's own browser login for it, and registering or removing IssueRelay's MCP control
//! server. Every command is a fixed list of arguments; the only variable part is the path of
//! IssueRelay's own command-line program.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::config::ExecutorKind;
use crate::doctor::{CmdOutput, CommandRunner};
use crate::linear::auth::LINEAR_MCP_URL;
use crate::text::clean_text;

/// The name IssueRelay's control server is registered under.
pub const CONTROL_SERVER_NAME: &str = "issuerelay";

/// The names the official instructions give Linear's server (design section 8).
const CLAUDE_LINEAR_NAME: &str = "linear-server";
const CODEX_LINEAR_NAME: &str = "linear";

/// A setup step the user can start with one click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "executor", rename_all = "snake_case")]
pub enum SetupAction {
    /// Add Linear's MCP server to the executor (user scope for Claude Code).
    AddLinearMcp(ExecutorKind),
    /// Start the executor's own browser login for Linear's MCP server.
    LoginLinearMcp(ExecutorKind),
    /// Register IssueRelay's control server (`issuerelay mcp`) with the executor.
    RegisterControlServer(ExecutorKind),
    /// Remove IssueRelay's control server from the executor.
    RemoveControlServer(ExecutorKind),
}

fn owned(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// The arguments for `action`, after the executor's program. `cli` is IssueRelay's
/// command-line program, which registering the control server needs.
pub fn arguments(action: SetupAction, cli: Option<&Path>) -> Result<Vec<String>> {
    use ExecutorKind::{Claude, Codex};
    Ok(match action {
        SetupAction::AddLinearMcp(Claude) => owned(&[
            "mcp",
            "add",
            "--scope",
            "user",
            "--transport",
            "http",
            CLAUDE_LINEAR_NAME,
            LINEAR_MCP_URL,
        ]),
        SetupAction::AddLinearMcp(Codex) => {
            owned(&["mcp", "add", CODEX_LINEAR_NAME, "--url", LINEAR_MCP_URL])
        }
        SetupAction::LoginLinearMcp(Claude) => owned(&["mcp", "login", CLAUDE_LINEAR_NAME]),
        SetupAction::LoginLinearMcp(Codex) => owned(&["mcp", "login", CODEX_LINEAR_NAME]),
        SetupAction::RegisterControlServer(kind) => {
            let cli =
                cli.ok_or_else(|| anyhow!("IssueRelay's command-line program was not found"))?;
            let cli = cli.to_str().ok_or_else(|| {
                anyhow!("the path of IssueRelay's command-line program is not valid Unicode")
            })?;
            match kind {
                Claude => owned(&[
                    "mcp",
                    "add",
                    "--scope",
                    "user",
                    CONTROL_SERVER_NAME,
                    "--",
                    cli,
                    "mcp",
                ]),
                Codex => owned(&["mcp", "add", CONTROL_SERVER_NAME, "--", cli, "mcp"]),
            }
        }
        SetupAction::RemoveControlServer(Claude) => {
            owned(&["mcp", "remove", "--scope", "user", CONTROL_SERVER_NAME])
        }
        SetupAction::RemoveControlServer(Codex) => owned(&["mcp", "remove", CONTROL_SERVER_NAME]),
    })
}

/// How long `action` may take: a login waits for the user to finish in the browser.
pub fn time_limit(action: SetupAction) -> Duration {
    match action {
        SetupAction::LoginLinearMcp(_) => Duration::from_secs(5 * 60),
        _ => Duration::from_secs(60),
    }
}

/// The most characters of a failed command's output an error carries.
const MAX_OUTPUT_CHARS: usize = 400;

/// Runs `action` with `program`, the executor as the environment check found it. A command that
/// exits non-zero is an error carrying its (cleaned, shortened) output.
pub async fn run(
    action: SetupAction,
    program: &Path,
    cli: Option<&Path>,
    runner: &dyn CommandRunner,
) -> Result<CmdOutput> {
    let args = arguments(action, cli)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = runner
        .run_limited(program, &refs, time_limit(action))
        .await?;
    if output.code != Some(0) {
        let text = if output.stderr.trim().is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        let shown: String = clean_text(text).chars().take(MAX_OUTPUT_CHARS).collect();
        let code = output
            .code
            .map_or_else(|| "none".to_string(), |c| c.to_string());
        bail!(
            "`{} {}` failed (exit code {code}): {shown}",
            program.display(),
            args.join(" ")
        );
    }
    Ok(output)
}

/// Whether IssueRelay's control server is registered with the executor at `program`:
/// `mcp get issuerelay` exits 0 only then (measured with Claude Code 2.1.285 and codex-cli
/// 0.159.2).
pub async fn control_server_registered(program: &Path, runner: &dyn CommandRunner) -> Result<bool> {
    let output = runner
        .run_limited(
            program,
            &["mcp", "get", CONTROL_SERVER_NAME],
            Duration::from_secs(60),
        )
        .await?;
    Ok(output.code == Some(0))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;

    /// Records every command and answers with `reply`.
    struct Recorder {
        reply: CmdOutput,
        calls: Mutex<Vec<(PathBuf, Vec<String>, Duration)>>,
    }

    impl Recorder {
        fn new(code: i32, stdout: &str, stderr: &str) -> Self {
            Self {
                reply: CmdOutput {
                    code: Some(code),
                    stdout: stdout.into(),
                    stderr: stderr.into(),
                },
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for Recorder {
        async fn run(&self, program: &Path, args: &[&str]) -> Result<CmdOutput> {
            self.run_limited(program, args, Duration::ZERO).await
        }

        async fn run_limited(
            &self,
            program: &Path,
            args: &[&str],
            limit: Duration,
        ) -> Result<CmdOutput> {
            self.calls.lock().unwrap().push((
                program.to_path_buf(),
                args.iter().map(|a| a.to_string()).collect(),
                limit,
            ));
            Ok(self.reply.clone())
        }
    }

    fn args(action: SetupAction, cli: Option<&Path>) -> Vec<String> {
        arguments(action, cli).unwrap()
    }

    #[test]
    fn every_action_runs_a_fixed_official_command() {
        use ExecutorKind::{Claude, Codex};
        let cli = Path::new("/Applications/IssueRelay.app/Contents/MacOS/issuerelay");
        assert_eq!(
            args(SetupAction::AddLinearMcp(Claude), None),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "http",
                "linear-server",
                "https://mcp.linear.app/mcp"
            ]
        );
        assert_eq!(
            args(SetupAction::AddLinearMcp(Codex), None),
            [
                "mcp",
                "add",
                "linear",
                "--url",
                "https://mcp.linear.app/mcp"
            ]
        );
        assert_eq!(
            args(SetupAction::LoginLinearMcp(Claude), None),
            ["mcp", "login", "linear-server"]
        );
        assert_eq!(
            args(SetupAction::LoginLinearMcp(Codex), None),
            ["mcp", "login", "linear"]
        );
        assert_eq!(
            args(SetupAction::RegisterControlServer(Claude), Some(cli)),
            [
                "mcp",
                "add",
                "--scope",
                "user",
                "issuerelay",
                "--",
                cli.to_str().unwrap(),
                "mcp"
            ]
        );
        assert_eq!(
            args(SetupAction::RegisterControlServer(Codex), Some(cli)),
            [
                "mcp",
                "add",
                "issuerelay",
                "--",
                cli.to_str().unwrap(),
                "mcp"
            ]
        );
        assert_eq!(
            args(SetupAction::RemoveControlServer(Claude), None),
            ["mcp", "remove", "--scope", "user", "issuerelay"]
        );
        assert_eq!(
            args(SetupAction::RemoveControlServer(Codex), None),
            ["mcp", "remove", "issuerelay"]
        );
        assert!(
            arguments(SetupAction::RegisterControlServer(Claude), None).is_err(),
            "registering needs the CLI"
        );
    }

    #[test]
    fn a_login_may_wait_for_the_browser() {
        assert_eq!(
            time_limit(SetupAction::LoginLinearMcp(ExecutorKind::Claude)),
            Duration::from_secs(300)
        );
        assert_eq!(
            time_limit(SetupAction::AddLinearMcp(ExecutorKind::Codex)),
            Duration::from_secs(60)
        );
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_cleaned_output() {
        let runner = Recorder::new(1, "", "error: server exists\u{1b}[2J already\n");
        let error = run(
            SetupAction::AddLinearMcp(ExecutorKind::Claude),
            Path::new("/fake/claude"),
            None,
            &runner,
        )
        .await
        .unwrap_err();
        let text = format!("{error:#}");
        assert!(
            text.contains("server exists") && text.contains("exit code 1"),
            "{text}"
        );
        assert!(!text.contains('\u{1b}'), "{text:?}");
        {
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls[0].0, PathBuf::from("/fake/claude"));
            assert_eq!(calls[0].2, Duration::from_secs(60));
        }
        let ok = Recorder::new(0, "Added", "");
        assert!(
            run(
                SetupAction::RemoveControlServer(ExecutorKind::Codex),
                Path::new("/fake/codex"),
                None,
                &ok
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn registration_is_read_from_mcp_get() {
        let found = Recorder::new(0, "issuerelay:\n  Type: stdio", "");
        assert!(
            control_server_registered(Path::new("/fake/claude"), &found)
                .await
                .unwrap()
        );
        assert_eq!(
            found.calls.lock().unwrap()[0].1,
            ["mcp", "get", "issuerelay"]
        );
        let missing = Recorder::new(1, "", "No MCP server named \"issuerelay\"");
        assert!(
            !control_server_registered(Path::new("/fake/claude"), &missing)
                .await
                .unwrap()
        );
    }
}
