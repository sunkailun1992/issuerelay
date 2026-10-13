//! One-click setup commands offered by the environment check (design section 8) and the
//! settings page (design section 10.3): adding Linear's MCP server to an executor, starting the
//! executor's own browser login for it, and registering or removing IssueRelay's MCP control
//! server. Every command is a fixed list of arguments; the only variable part is the path of
//! IssueRelay's own command-line program.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::config::ExecutorKind;
use crate::doctor::{CmdOutput, CommandRunner};
use crate::linear::auth::LINEAR_MCP_URL;
use crate::text::{clean_text, strip_ansi};

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

/// The arguments for `action`, after the executor's program: fixed literals, except that
/// registering the control server puts `cli`, IssueRelay's own command-line program, after `--`.
/// `cli` must be an absolute path, valid Unicode (the executor stores it and starts it from any
/// folder); anything else, or no `cli` when registering, is an error. The other actions ignore
/// `cli`.
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
            if !cli.is_absolute() {
                bail!(
                    "IssueRelay's command-line program must be given by an absolute path: {}",
                    cli.display()
                );
            }
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

/// The time limit of every setup command but a login, and of `mcp get`.
const COMMAND_LIMIT: Duration = Duration::from_secs(60);

/// The time limit of a login: the user finishes it in the browser.
const LOGIN_LIMIT: Duration = Duration::from_secs(5 * 60);

/// How long `action` may take: a login waits for the user to finish in the browser.
pub fn time_limit(action: SetupAction) -> Duration {
    match action {
        SetupAction::LoginLinearMcp(_) => LOGIN_LIMIT,
        _ => COMMAND_LIMIT,
    }
}

/// The most characters of a failed command's output an error carries, all streams together: when
/// both streams print, each gets half. A cut is marked with `…`.
const MAX_OUTPUT_CHARS: usize = 400;

/// `text` as one plain line: escape sequences stripped first, then control and invisible
/// characters cleaned (`clean_text`).
fn plain(text: &str) -> String {
    clean_text(&strip_ansi(text))
}

/// The first `limit` characters of `text`, with `…` added when it was longer.
fn cut(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// What a failed command printed, as its error shows it: its stderr, or its stdout when the
/// stderr is empty, or both when both print.
fn shown_output(output: &CmdOutput) -> String {
    let stderr = plain(&output.stderr);
    let stdout = plain(&output.stdout);
    match (stderr.is_empty(), stdout.is_empty()) {
        (false, false) => format!(
            "stderr: {}; stdout: {}",
            cut(&stderr, MAX_OUTPUT_CHARS / 2),
            cut(&stdout, MAX_OUTPUT_CHARS / 2)
        ),
        (false, true) => cut(&stderr, MAX_OUTPUT_CHARS),
        (true, false) => cut(&stdout, MAX_OUTPUT_CHARS),
        (true, true) => "no output".to_string(),
    }
}

/// Runs `action` with `program`, the executor as the environment check found it. A command that
/// exits non-zero, is ended by a signal, or cannot run to completion is an error that names the
/// command; the error of a command that finished carries its output (`shown_output`). On success
/// the output is returned as the command printed it: callers clean it before they show it.
pub async fn run(
    action: SetupAction,
    program: &Path,
    cli: Option<&Path>,
    runner: &dyn CommandRunner,
) -> Result<CmdOutput> {
    let args = arguments(action, cli)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = format!("{} {}", program.display(), args.join(" "));
    let output = runner
        .run_limited(program, &refs, time_limit(action))
        .await
        .with_context(|| format!("`{command}` did not run to completion"))?;
    match output.code {
        Some(0) => Ok(output),
        Some(code) => bail!(
            "`{command}` failed (exit code {code}): {}",
            shown_output(&output)
        ),
        None => bail!(
            "`{command}` was ended by a signal: {}",
            shown_output(&output)
        ),
    }
}

/// Whether IssueRelay's control server is registered with the executor at `program`:
/// `mcp get issuerelay` exits 0 only then, and exits non-zero when it is not (Claude Code
/// 2.1.285 and codex-cli 0.159.2, as recorded in the M2 core plan (2026-10-13)). A command that
/// was ended by a signal, or that cannot run to completion, says nothing about the registration:
/// it is an error.
pub async fn control_server_registered(program: &Path, runner: &dyn CommandRunner) -> Result<bool> {
    let args = ["mcp", "get", CONTROL_SERVER_NAME];
    let command = format!("{} {}", program.display(), args.join(" "));
    let output = runner
        .run_limited(program, &args, COMMAND_LIMIT)
        .await
        .with_context(|| format!("`{command}` did not run to completion"))?;
    match output.code {
        Some(0) => Ok(true),
        Some(_) => Ok(false),
        None => bail!("`{command}` was ended by a signal"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;

    /// Records every command and answers with `reply`, or fails with `fail` when it is set.
    struct Recorder {
        reply: CmdOutput,
        fail: Option<String>,
        calls: Mutex<Vec<(PathBuf, Vec<String>, Duration)>>,
    }

    impl Recorder {
        fn new(code: i32, stdout: &str, stderr: &str) -> Self {
            Self::replying(Some(code), stdout, stderr)
        }

        fn replying(code: Option<i32>, stdout: &str, stderr: &str) -> Self {
            Self {
                reply: CmdOutput {
                    code,
                    stdout: stdout.into(),
                    stderr: stderr.into(),
                },
                fail: None,
                calls: Mutex::new(Vec::new()),
            }
        }

        /// A command that was ended by a signal, with no output.
        fn signalled() -> Self {
            Self::replying(None, "", "")
        }

        /// A command that cannot run to completion; the runner reports `message`.
        fn unrunnable(message: &str) -> Self {
            Self {
                fail: Some(message.into()),
                ..Self::replying(Some(0), "", "")
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
            match &self.fail {
                Some(message) => Err(anyhow!("{message}")),
                None => Ok(self.reply.clone()),
            }
        }
    }

    /// An absolute path to IssueRelay's command-line program, as this platform spells one.
    #[cfg(windows)]
    const CLI: &str = r"C:\Program Files\IssueRelay\issuerelay.exe";
    #[cfg(not(windows))]
    const CLI: &str = "/Applications/IssueRelay.app/Contents/MacOS/issuerelay";

    fn args(action: SetupAction, cli: Option<&Path>) -> Vec<String> {
        arguments(action, cli).unwrap()
    }

    #[test]
    fn every_action_runs_a_fixed_official_command() {
        use ExecutorKind::{Claude, Codex};
        let cli = Path::new(CLI);
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
        assert_eq!(found.calls.lock().unwrap()[0].2, Duration::from_secs(60));
        let missing = Recorder::new(1, "", "No MCP server named \"issuerelay\"");
        assert!(
            !control_server_registered(Path::new("/fake/claude"), &missing)
                .await
                .unwrap()
        );
    }

    /// What `run` reports when adding Linear's server fails, given the command's `reply`.
    async fn failure_of(reply: Recorder) -> anyhow::Error {
        run(
            SetupAction::AddLinearMcp(ExecutorKind::Claude),
            Path::new("/fake/claude"),
            None,
            &reply,
        )
        .await
        .unwrap_err()
    }

    /// The output part of a failure: what follows the command and its status.
    fn shown(error: &anyhow::Error) -> String {
        let text = format!("{error:#}");
        text.split_once(": ")
            .expect("a failure names the command and its status first")
            .1
            .to_string()
    }

    #[tokio::test]
    async fn a_failure_shows_stderr_without_escape_residue() {
        let error = failure_of(Recorder::new(
            1,
            "",
            "error: server exists\u{1b}[2J already\n",
        ))
        .await;
        assert_eq!(shown(&error), "error: server exists already");
    }

    #[tokio::test]
    async fn a_long_output_is_cut_to_400_characters_and_marked() {
        let error = failure_of(Recorder::new(1, "", &"x".repeat(1000))).await;
        assert_eq!(shown(&error), format!("{}…", "x".repeat(400)));
    }

    #[tokio::test]
    async fn a_failure_shows_both_streams_when_both_print() {
        let error = failure_of(Recorder::new(1, "out text", "err text")).await;
        assert_eq!(shown(&error), "stderr: err text; stdout: out text");
    }

    #[tokio::test]
    async fn two_long_streams_are_each_cut_to_half_the_limit_and_marked() {
        let error = failure_of(Recorder::new(1, &"o".repeat(1000), &"e".repeat(1000))).await;
        assert_eq!(
            shown(&error),
            format!("stderr: {}…; stdout: {}…", "e".repeat(200), "o".repeat(200))
        );
        // A short stream next to a long one keeps all of its text and gets no mark.
        let error = failure_of(Recorder::new(1, "out text", &"e".repeat(1000))).await;
        assert_eq!(
            shown(&error),
            format!("stderr: {}…; stdout: out text", "e".repeat(200))
        );
    }

    #[tokio::test]
    async fn a_failure_shows_stdout_when_stderr_is_empty() {
        let error = failure_of(Recorder::new(1, "only stdout", "")).await;
        assert_eq!(shown(&error), "only stdout");
    }

    #[tokio::test]
    async fn a_failure_without_output_says_so() {
        let error = failure_of(Recorder::new(1, "", "")).await;
        assert_eq!(shown(&error), "no output");
    }

    #[tokio::test]
    async fn a_command_ended_by_a_signal_is_a_failure_of_its_own() {
        let error = failure_of(Recorder::signalled()).await;
        let text = format!("{error:#}");
        assert!(text.contains("was ended by a signal"), "{text}");
        assert!(!text.contains("exit code"), "{text}");
    }

    #[tokio::test]
    async fn a_command_that_cannot_run_to_completion_is_named() {
        let error = failure_of(Recorder::unrunnable(
            "No such file or directory (os error 2)",
        ))
        .await;
        assert_eq!(
            format!("{error:#}"),
            "`/fake/claude mcp add --scope user --transport http linear-server https://mcp.linear.app/mcp` did not run to completion: No such file or directory (os error 2)"
        );
    }

    #[tokio::test]
    async fn a_registration_check_that_cannot_run_names_its_command() {
        let runner = Recorder::unrunnable("timed out after 60s");
        let error = control_server_registered(Path::new("/fake/claude"), &runner)
            .await
            .unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "`/fake/claude mcp get issuerelay` did not run to completion: timed out after 60s"
        );
    }

    #[tokio::test]
    async fn a_signal_death_is_no_registration_verdict() {
        let runner = Recorder::signalled();
        let error = control_server_registered(Path::new("/fake/claude"), &runner)
            .await
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("was ended by a signal"),
            "{error:#}"
        );
    }

    #[test]
    fn a_relative_command_line_program_is_refused() {
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let error = arguments(
                SetupAction::RegisterControlServer(kind),
                Some(Path::new("issuerelay")),
            )
            .unwrap_err();
            assert!(format!("{error:#}").contains("absolute path"), "{error:#}");
        }
    }

    #[test]
    fn setup_actions_keep_their_json_shape() {
        let action = SetupAction::AddLinearMcp(ExecutorKind::Claude);
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(json, r#"{"kind":"add_linear_mcp","executor":"claude"}"#);
        assert_eq!(serde_json::from_str::<SetupAction>(&json).unwrap(), action);
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            for action in [
                SetupAction::AddLinearMcp(kind),
                SetupAction::LoginLinearMcp(kind),
                SetupAction::RegisterControlServer(kind),
                SetupAction::RemoveControlServer(kind),
            ] {
                let json = serde_json::to_string(&action).unwrap();
                let back: SetupAction = serde_json::from_str(&json).unwrap();
                assert_eq!(back, action, "{json}");
            }
        }
    }
}
