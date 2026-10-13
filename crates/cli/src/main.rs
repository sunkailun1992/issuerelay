//! The `issuerelay` command: sends each subcommand to the running engine host over local IPC,
//! or serves the MCP control tools with `issuerelay mcp`.
use std::{
    future::Future,
    io::{self, IsTerminal, Write},
    path::{Component, Path, PathBuf},
    process::ExitCode,
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use issuerelay_cli::{format, mcp};
use issuerelay_core::{
    config::{Binding, ExecutorKind, PermissionProfile},
    ipc::client,
    paths::AppPaths,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "issuerelay",
    version,
    about = "Control the running IssueRelay engine"
)]
struct Cli {
    /// Print the raw JSON result.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show monitoring status.
    Status,
    /// Pause monitoring (runs in progress continue).
    Pause,
    /// Resume monitoring.
    Resume,
    /// Check Linear now.
    Poll,
    /// Check executors, the Linear workspace and bindings.
    Doctor,
    /// List recent runs.
    Jobs {
        /// How many runs to list, newest first (at most 500).
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// Show the last lines of a run's log.
    Logs {
        /// The run's number, as in the `#` column of `issuerelay jobs`.
        job_id: i64,
        /// How many lines to show from the end of the log (at most 5000).
        #[arg(long, default_value_t = 50)]
        lines: u64,
    },
    /// Manage Linear project bindings.
    #[command(subcommand)]
    Bind(BindCommand),
    /// Run the MCP control server on stdio (for Claude Code / Codex).
    Mcp,
}

#[derive(Subcommand)]
enum BindCommand {
    /// List the saved bindings.
    List,
    /// Bind a Linear project to a local directory (replaces a binding with the same id).
    Add {
        /// The Linear project's uuid, not its `P-<TEAM>-N` identifier.
        #[arg(long)]
        project_id: String,
        /// The project's name, shown in lists and checks.
        #[arg(long)]
        project_name: String,
        /// The directory runs start in; it must exist.
        #[arg(long)]
        dir: PathBuf,
        /// `claude` or `codex`; omit it to use the default executor from the settings.
        #[arg(long, value_parser = parse_executor)]
        executor: Option<ExecutorKind>,
        /// `full`, `auto` or `personal`; omit it to use the default from the settings.
        #[arg(long, value_parser = parse_permission)]
        permission: Option<PermissionProfile>,
    },
    /// Remove a binding by its id (the project id it was added with).
    Remove {
        /// The binding's id, as `issuerelay bind list` shows it.
        id: String,
    },
}

fn parse_executor(text: &str) -> Result<ExecutorKind, String> {
    ExecutorKind::parse(text).ok_or_else(|| format!("expected `claude` or `codex`, got `{text}`"))
}

fn parse_permission(text: &str) -> Result<PermissionProfile, String> {
    PermissionProfile::parse(text)
        .ok_or_else(|| format!("expected `full`, `auto` or `personal`, got `{text}`"))
}

/// The folder to bind, absolute and without `.` or `..` segments, without resolving symlinks
/// (the engine compares folders by their canonical path itself). It must exist.
fn normalize_folder(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut normal = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::CurDir => {}
            // Above the root there is nothing to leave: `/..` is `/`, and so is `C:\..`.
            Component::ParentDir => {
                normal.pop();
            }
            // The prefix and root (`C:` and `\` on Windows) are kept as they are.
            other => normal.push(other),
        }
    }
    anyhow::ensure!(normal.is_dir(), "{} is not a folder", normal.display());
    Ok(normal)
}

/// The IPC method and parameters a subcommand stands for.
fn request(command: &Command) -> Result<(&'static str, Value)> {
    Ok(match command {
        Command::Status => ("status", json!({})),
        Command::Pause => ("pause", json!({})),
        Command::Resume => ("resume", json!({})),
        Command::Poll => ("poll_now", json!({})),
        Command::Doctor => ("doctor", json!({})),
        Command::Jobs { limit } => ("list_jobs", json!({"limit": limit})),
        Command::Logs { job_id, lines } => ("logs", json!({"job_id": job_id, "lines": lines})),
        Command::Bind(BindCommand::List) => ("bindings.list", json!({})),
        Command::Bind(BindCommand::Remove { id }) => ("bindings.remove", json!({"id": id})),
        Command::Bind(BindCommand::Add {
            project_id,
            project_name,
            dir,
            executor,
            permission,
        }) => {
            // `normalize_folder` also checks that the folder exists: the engine does not look at
            // it when saving, so a typo here would otherwise give a binding whose jobs wait
            // forever for a missing directory.
            let binding = Binding {
                id: project_id.clone(),
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                directory: normalize_folder(dir)?,
                executor: *executor,
                permission: *permission,
                model: None,
                prompt_override: None,
            };
            ("bindings.add", json!({"binding": binding}))
        }
        Command::Mcp => unreachable!("handled before sending a request"),
    })
}

/// Writes `text` and a line break. A reader that went away (`issuerelay logs 3 | head -1`) is
/// not a failure: the output was only wanted up to there.
fn write_line(out: &mut impl Write, text: &str) -> io::Result<()> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Sends the subcommand's request through `send` and renders the answer: the raw JSON with
/// `--json`, otherwise the summary. Nothing is sent when the request cannot be built.
async fn execute<Fut>(cli: &Cli, send: impl FnOnce(&'static str, Value) -> Fut) -> Result<String>
where
    Fut: Future<Output = Result<Value>>,
{
    let (method, params) = request(&cli.command)?;
    let value = send(method, params).await?;
    Ok(format::render(method, &value, cli.json))
}

/// Tells the user why the command failed and returns the exit code for it. The line is cleaned
/// like all other output. If `err` is gone too there is nobody to tell; the exit code still says
/// the command failed.
fn fail(error: &anyhow::Error, err: &mut impl Write) -> ExitCode {
    let _ = writeln!(err, "{}", format::error_line(error));
    ExitCode::FAILURE
}

/// Tells the person at the terminal what `command` is about to wait for, if it is slow:
/// `doctor` runs both executors' checks, which can take up to a minute, and a silent command
/// looks hung. The line goes to `to` (the caller passes stderr) and only when `is_terminal`, so
/// neither a captured stdout (the `--json` result included) nor a captured stderr ever holds it.
/// If `to` is gone there is nobody to tell, and the command goes on.
fn announce_wait(command: &Command, to: &mut impl Write, is_terminal: bool) {
    if is_terminal && matches!(command, Command::Doctor) {
        let notice = "Checking Linear, Claude Code and Codex… (this can take up to a minute)";
        let _ = write_line(to, notice);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let endpoint = AppPaths::system()?.ipc_endpoint();
    if matches!(cli.command, Command::Mcp) {
        return mcp::serve(endpoint).await;
    }
    let stderr_is_terminal = io::stderr().is_terminal();
    announce_wait(&cli.command, &mut io::stderr().lock(), stderr_is_terminal);
    let text = execute(&cli, |method, params| {
        client::call(&endpoint, method, params)
    })
    .await?;
    write_line(&mut io::stdout().lock(), &text)?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&error, &mut io::stderr().lock()),
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, sync::Mutex};

    use anyhow::anyhow;
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Cli {
        let argv = std::iter::once("issuerelay").chain(args.iter().copied());
        Cli::try_parse_from(argv).unwrap_or_else(|e| panic!("{args:?} should parse: {e}"))
    }

    fn bind_add(dir: &Path) -> Command {
        Command::Bind(BindCommand::Add {
            project_id: "proj-1".into(),
            project_name: "Demo Project".into(),
            dir: dir.to_path_buf(),
            executor: Some(ExecutorKind::Codex),
            permission: None,
        })
    }

    fn collect_names(command: &clap::Command, names: &mut Vec<String>) {
        for sub in command.get_subcommands() {
            names.push(sub.get_name().to_ascii_lowercase());
            names.extend(sub.get_all_aliases().map(str::to_ascii_lowercase));
            collect_names(sub, names);
        }
    }

    /// A writer whose every write fails with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    /// Runs `issuerelay <args>` against a host that answers `answer`; returns what was printed
    /// and every `(method, params)` that was sent.
    async fn run_command(
        args: &[&str],
        answer: Value,
    ) -> (Result<String>, Vec<(&'static str, Value)>) {
        let sent = Mutex::new(Vec::new());
        let printed = execute(&parse(args), |method, params| {
            sent.lock().unwrap().push((method, params));
            std::future::ready(Ok(answer))
        })
        .await;
        (printed, sent.into_inner().unwrap())
    }

    #[test]
    fn every_subcommand_sends_its_method_and_params() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().to_str().unwrap();
        let cases: Vec<(Vec<&str>, &str, Value)> = vec![
            (vec!["status"], "status", json!({})),
            (vec!["pause"], "pause", json!({})),
            (vec!["resume"], "resume", json!({})),
            (vec!["poll"], "poll_now", json!({})),
            (vec!["doctor"], "doctor", json!({})),
            (vec!["jobs"], "list_jobs", json!({"limit": 20})),
            (
                vec!["jobs", "--limit", "5"],
                "list_jobs",
                json!({"limit": 5}),
            ),
            (vec!["logs", "7"], "logs", json!({"job_id": 7, "lines": 50})),
            (
                vec!["logs", "7", "--lines", "3"],
                "logs",
                json!({"job_id": 7, "lines": 3}),
            ),
            (vec!["bind", "list"], "bindings.list", json!({})),
            (
                vec!["bind", "remove", "proj-1"],
                "bindings.remove",
                json!({"id": "proj-1"}),
            ),
            // Nothing is sent for an executor or permission that was not named.
            (
                vec![
                    "bind",
                    "add",
                    "--project-id",
                    "proj-1",
                    "--project-name",
                    "Demo Project",
                    "--dir",
                    dir,
                ],
                "bindings.add",
                json!({"binding": {"id": "proj-1", "project_id": "proj-1",
                    "project_name": "Demo Project", "directory": root.path()}}),
            ),
            (
                vec![
                    "bind",
                    "add",
                    "--project-id",
                    "proj-1",
                    "--project-name",
                    "Demo Project",
                    "--dir",
                    dir,
                    "--executor",
                    "Codex",
                    "--permission",
                    "auto",
                ],
                "bindings.add",
                json!({"binding": {"id": "proj-1", "project_id": "proj-1",
                    "project_name": "Demo Project", "directory": root.path(),
                    "executor": "codex", "permission": "auto"}}),
            ),
        ];
        for (args, method, params) in cases {
            let (sent_method, sent_params) = request(&parse(&args).command).unwrap();
            assert_eq!(
                (sent_method, &sent_params),
                (method, &params),
                "issuerelay {args:?}"
            );
        }
    }

    #[test]
    fn the_json_flag_works_before_and_after_the_subcommand_and_is_off_by_default() {
        assert!(parse(&["--json", "status"]).json);
        assert!(parse(&["status", "--json"]).json);
        assert!(parse(&["bind", "list", "--json"]).json);
        assert!(!parse(&["status"]).json);
        assert!(!parse(&["bind", "list"]).json);
    }

    #[test]
    fn unknown_executors_and_permissions_are_refused() {
        let add = |option: &str, value: &str| {
            let argv = ["issuerelay", "bind", "add", "--project-id", "p"]
                .into_iter()
                .chain(["--project-name", "n", "--dir", ".", option, value]);
            Cli::try_parse_from(argv)
        };
        assert!(add("--executor", "gemini").is_err());
        assert!(add("--permission", "everything").is_err());
        assert!(add("--executor", "claude").is_ok());
        assert!(add("--permission", "personal").is_ok());
    }

    #[test]
    fn bind_add_refuses_a_directory_that_does_not_exist() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("typo");
        let error = request(&bind_add(&missing)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("{} is not a folder", missing.display())
        );
        // A file is no directory either.
        let file = root.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(
            request(&bind_add(&file))
                .unwrap_err()
                .to_string()
                .ends_with("is not a folder")
        );
    }

    #[test]
    fn bind_add_sends_the_existing_directory_as_an_absolute_path() {
        // `.` exists wherever the tests run and is relative, which `absolute` must resolve.
        let (method, params) = request(&bind_add(Path::new("."))).unwrap();
        assert_eq!(method, "bindings.add");
        let binding = &params["binding"];
        assert_eq!(binding["id"], "proj-1");
        assert_eq!(binding["project_id"], "proj-1");
        assert_eq!(binding["executor"], "codex");
        let sent = PathBuf::from(binding["directory"].as_str().unwrap());
        assert_eq!(sent, std::path::absolute(".").unwrap());
        assert!(sent.is_absolute());
    }

    #[test]
    fn bound_folders_are_stored_without_dot_segments() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a");
        std::fs::create_dir(&nested).unwrap();
        let messy = nested.join("..").join("a").join(".");
        assert_eq!(
            normalize_folder(&messy).unwrap(),
            std::path::absolute(&nested).unwrap()
        );
        assert!(normalize_folder(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn the_stored_folder_text_has_no_dot_segments_or_trailing_separator() {
        // `Path` equality ignores `.` and a trailing separator, but the text that is stored and
        // shown does not, so compare the text.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a");
        std::fs::create_dir(&nested).unwrap();
        let sep = std::path::MAIN_SEPARATOR;
        let spellings = [
            format!("{}{sep}", nested.display()),
            format!("{}{sep}.", nested.display()),
            format!("{}{sep}..{sep}a{sep}.{sep}", nested.display()),
            format!("{}{sep}.{sep}", dir.path().join("a").display()),
        ];
        for spelling in spellings {
            let normal = normalize_folder(Path::new(&spelling)).unwrap();
            assert_eq!(normal.to_str(), nested.to_str(), "{spelling:?}");
        }
    }

    #[test]
    fn dot_dot_cannot_climb_above_the_root() {
        let temp = std::env::temp_dir();
        let root = temp.ancestors().last().unwrap();
        // More `..` than the path has folders.
        let too_far = temp
            .components()
            .fold(temp.clone(), |path, _| path.join(".."))
            .join("..");
        assert_eq!(normalize_folder(&too_far).unwrap(), root);
    }

    #[test]
    fn a_file_is_not_a_folder_to_bind() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let error = normalize_folder(&file).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("{} is not a folder", file.display())
        );
    }

    #[test]
    fn bind_add_sends_the_folder_without_dot_segments() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a");
        std::fs::create_dir(&nested).unwrap();
        let messy = nested.join("..").join("a").join(".");
        let (method, params) = request(&bind_add(&messy)).unwrap();
        assert_eq!(method, "bindings.add");
        assert_eq!(params["binding"]["directory"].as_str(), nested.to_str());
    }

    #[test]
    fn doctor_tells_a_terminal_it_is_waiting_and_nothing_else_does() {
        let said = |command: &Command, is_terminal: bool| {
            let mut shown = Vec::new();
            announce_wait(command, &mut shown, is_terminal);
            String::from_utf8(shown).unwrap()
        };
        // On a terminal the line goes to the writer it is given (the caller passes stderr).
        assert_eq!(
            said(&Command::Doctor, true),
            "Checking Linear, Claude Code and Codex… (this can take up to a minute)\n"
        );
        // A script that captures the stream gets nothing extra.
        assert_eq!(said(&Command::Doctor, false), "");
        let quick = [
            Command::Status,
            Command::Pause,
            Command::Resume,
            Command::Poll,
            Command::Jobs { limit: 5 },
            Command::Logs {
                job_id: 1,
                lines: 5,
            },
            Command::Bind(BindCommand::List),
            Command::Mcp,
        ];
        for command in &quick {
            for is_terminal in [true, false] {
                assert_eq!(said(command, is_terminal), "");
            }
        }
        // A stderr that is gone leaves nobody to tell, and the check goes on.
        for kind in [io::ErrorKind::BrokenPipe, io::ErrorKind::PermissionDenied] {
            announce_wait(&Command::Doctor, &mut Failing(kind), true);
        }
    }

    #[test]
    fn a_closed_pipe_is_not_a_failure_but_other_write_errors_are() {
        assert!(write_line(&mut Failing(io::ErrorKind::BrokenPipe), "text").is_ok());
        let error = write_line(&mut Failing(io::ErrorKind::PermissionDenied), "text").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let mut written = Vec::new();
        write_line(&mut written, "first\nsecond").unwrap();
        assert_eq!(written, b"first\nsecond\n");
    }

    #[tokio::test]
    async fn the_answer_is_printed_as_a_summary_or_with_json_as_the_raw_result() {
        let answer = json!({"paused": true});
        let (summary, sent) = run_command(&["pause"], answer.clone()).await;
        assert_eq!(
            summary.unwrap(),
            "Monitoring paused (running jobs continue)."
        );
        assert_eq!(sent, [("pause", json!({}))]);
        let (raw, sent) = run_command(&["--json", "pause"], answer.clone()).await;
        assert_eq!(
            serde_json::from_str::<Value>(&raw.unwrap()).unwrap(),
            answer
        );
        assert_eq!(sent, [("pause", json!({}))]);
        // What is sent includes the parameters, and the summary is the one for that method.
        let (listed, sent) = run_command(&["jobs", "--limit", "5"], json!([])).await;
        assert_eq!(listed.unwrap(), "No runs yet.");
        assert_eq!(sent, [("list_jobs", json!({"limit": 5}))]);
    }

    #[tokio::test]
    async fn a_failed_send_is_the_commands_error_and_a_bad_request_sends_nothing() {
        let failed = execute(&parse(&["status"]), |_, _| {
            std::future::ready(Err(anyhow!("inner").context("outer")))
        })
        .await;
        assert_eq!(format!("{:#}", failed.unwrap_err()), "outer: inner");
        // `bind add` for a missing directory fails before anything is sent.
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("typo");
        let args = ["bind", "add", "--project-id", "p", "--project-name", "n"];
        let args: Vec<&str> = args
            .into_iter()
            .chain(["--dir", missing.to_str().unwrap()])
            .collect();
        let (printed, sent) = run_command(&args, json!({})).await;
        assert!(printed.unwrap_err().to_string().contains("is not a folder"));
        assert!(sent.is_empty());
    }

    #[test]
    fn a_failure_is_one_clean_line_with_its_causes_and_exit_code_1() {
        let mut shown = Vec::new();
        let error = anyhow!("bad\u{1b}]52;c;ZXZpbA==\u{7}end").context("outer");
        let code = fail(&error, &mut shown);
        assert_eq!(
            String::from_utf8(shown).unwrap(),
            "Error: outer: bad ]52;c;ZXZpbA== end\n"
        );
        assert_eq!(code, ExitCode::FAILURE);
        // A stderr that is gone changes nothing about the exit code.
        let closed = &mut Failing(io::ErrorKind::BrokenPipe);
        assert_eq!(fail(&anyhow!("x"), closed), ExitCode::FAILURE);
    }

    #[test]
    fn no_subcommand_marks_done_retries_or_cancels() {
        // Only the desktop panel may do these (AGENTS.md); the CLI must not offer them in any
        // spelling, at any depth of its command tree, under its own name or an alias.
        let mut names = Vec::new();
        collect_names(&Cli::command(), &mut names);
        assert!(
            names.iter().any(|n| n == "status") && names.iter().any(|n| n == "add"),
            "the walk should reach top-level and nested subcommands: {names:?}"
        );
        for name in &names {
            for word in ["done", "retry", "cancel"] {
                assert!(
                    !name.contains(word),
                    "`{name}` looks like a panel-only action"
                );
            }
        }
    }
}
