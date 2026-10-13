//! Development-only headless host for the IssueRelay engine. Not shipped; the M2 tray app runs
//! the same `core::host::Host`.
use std::{
    io::{self, IsTerminal, Read, Write},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use issuerelay_core::{
    host::{Connector, Host, HostParts, LinearSetup, StopMode, SystemConnector},
    linear::{
        LinearApi, auth,
        mcp::{LinearClient, McpLinear},
    },
    paths::AppPaths,
    text::{clean_text, is_invisible},
};
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};

#[derive(Parser)]
#[command(
    name = "issuerelay-devhost",
    about = "Development-only headless host for IssueRelay (not shipped)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authorize with Linear in the browser and keep the token in the system keychain.
    Login,
    /// Read a Linear personal API key from stdin and keep it in the system keychain. A terminal
    /// is refused; pipe the key in, for example: `pbpaste | issuerelay-devhost api-key`.
    ApiKey,
    /// Forget stored Linear credentials.
    Logout,
    /// List Linear projects (id and name) for `issuerelay bind add`.
    Projects,
    /// Run the engine and the IPC server until Ctrl+C.
    Run,
}

/// Connects with the credentials in the system keychain: the OAuth credentials that `login`
/// stored first, then a saved personal API key. Stored OAuth credentials that cannot be read are
/// an error that says to log in again (from `auth::connect_stored`), not a reason to try the API
/// key.
async fn connect() -> Result<LinearClient> {
    if let Some(client) = auth::connect_stored(auth::LINEAR_MCP_URL).await? {
        return Ok(client);
    }
    if let Some(key) = auth::load_api_key()? {
        return auth::connect_api_key(auth::LINEAR_MCP_URL, &key).await;
    }
    bail!("no Linear credentials; run `issuerelay-devhost login` (or `api-key`) first")
}

/// `text` with every control character turned into a space and every invisible character
/// (zero-width and direction-changing ones) removed. This is `clean_text` without the trimming
/// and the length cap, for text that must keep its end, such as the root cause of an error.
fn strip_control(text: &str) -> String {
    text.chars()
        .filter_map(|c| {
            if c.is_control() {
                Some(' ')
            } else if is_invisible(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect()
}

/// Passes what is written to `W` with `strip_control` applied, keeping only the line ending that
/// closes the text (the last byte, when it is a newline). A log line can then neither move the
/// cursor, change the terminal's state nor forge a second line. The tracing subscriber hands
/// over each line in a single `write_all`; a character split across two writes would show as
/// U+FFFD.
struct CleanLines<W>(W);

impl<W: Write> Write for CleanLines<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(bytes);
        let (body, ending) = match text.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (&*text, ""),
        };
        let mut clean = strip_control(body);
        clean.push_str(ending);
        self.0.write_all(clean.as_bytes())?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Where the logs go: stderr, through `CleanLines`. The subscriber escapes only the message and
/// error values of an event; named fields are written as they are, and the engine and the IPC
/// server log fields (`error = %…`) that can hold Linear's text.
struct CleanStderr;

impl<'a> MakeWriter<'a> for CleanStderr {
    type Writer = CleanLines<io::Stderr>;

    fn make_writer(&'a self) -> Self::Writer {
        CleanLines(io::stderr())
    }
}

/// What is logged unless `RUST_LOG` says otherwise. `run` is the long-lived host and logs what the
/// engine does; the other commands print their result and stay quiet unless something is wrong
/// (rmcp logs every connection at `info`).
fn default_filter(command: &Command) -> &'static str {
    match command {
        Command::Run => "info,issuerelay_core=debug",
        Command::Login | Command::ApiKey | Command::Logout | Command::Projects => "warn",
    }
}

/// Logs to stderr, so stdout carries only what a subcommand prints as its result. Colors are off
/// so the writer never has to let an escape sequence through. `RUST_LOG` overrides
/// `default_filter`.
fn init_logging(default_filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter)),
        )
        .with_ansi(false)
        .with_writer(CleanStderr)
        // The subscriber reports a failed log write with `eprintln!`, which panics when stderr is
        // gone, as it is once the terminal has closed (SIGHUP). The host must keep shutting down.
        .log_internal_errors(false)
        .init();
}

/// Writes `text` and a line break. A reader that went away (`projects | head -1`) is not a
/// failure: the output was only wanted up to there.
fn write_line(out: &mut impl Write, text: &str) -> io::Result<()> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Prints `text` and a line break to stdout (see `write_line`).
fn print_line(text: &str) -> io::Result<()> {
    write_line(&mut io::stdout().lock(), text)
}

/// Resolves when the host is asked to stop: on Ctrl+C, and on SIGTERM (`kill`, a closed task) and
/// SIGHUP (the terminal was closed). The executors lead process groups of their own, so a host
/// that simply died would leave full-permission runs working as orphans.
#[cfg(unix)]
async fn shutdown_signal() -> io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    tokio::select! {
        interrupt = tokio::signal::ctrl_c() => interrupt,
        _ = terminate.recv() => Ok(()),
        _ = hangup.recv() => Ok(()),
    }
}

/// Resolves when the host is asked to stop: on Ctrl+C, Ctrl+Break, the console being closed and
/// the system shutting down. (Windows ends the process shortly after the last two; the job
/// object of each run then ends its process tree.)
#[cfg(windows)]
async fn shutdown_signal() -> io::Result<()> {
    use tokio::signal::windows;

    let mut ctrl_break = windows::ctrl_break()?;
    let mut ctrl_close = windows::ctrl_close()?;
    let mut ctrl_shutdown = windows::ctrl_shutdown()?;
    tokio::select! {
        interrupt = tokio::signal::ctrl_c() => interrupt,
        _ = ctrl_break.recv() => Ok(()),
        _ = ctrl_close.recv() => Ok(()),
        _ = ctrl_shutdown.recv() => Ok(()),
    }
}

/// How often `wait_for_first_connection` looks at the host.
const CONNECTION_POLL: Duration = Duration::from_millis(100);

/// How long the first connection to Linear may take before the devhost says it is still going.
const STILL_CONNECTING_AFTER: Duration = Duration::from_secs(10);

/// Waits until `setup` stops saying `Connecting` (`Host::start` does not wait for Linear) and
/// returns what it says then. `still_connecting` is called once if that takes longer than
/// `slow_after`.
async fn wait_for_first_connection(
    setup: impl Fn() -> LinearSetup,
    poll: Duration,
    slow_after: Duration,
    still_connecting: impl FnOnce(),
) -> LinearSetup {
    let began = tokio::time::Instant::now();
    let mut still_connecting = Some(still_connecting);
    loop {
        let now = setup();
        if now != LinearSetup::Connecting {
            return now;
        }
        if began.elapsed() >= slow_after
            && let Some(say) = still_connecting.take()
        {
            say();
        }
        tokio::time::sleep(poll).await;
    }
}

/// Logs how the host's first connection to Linear ended: the devhost is already running by then.
async fn report_first_connection(host: Host) {
    let ended = wait_for_first_connection(
        || host.linear_setup(),
        CONNECTION_POLL,
        STILL_CONNECTING_AFTER,
        || tracing::info!("still connecting to Linear"),
    )
    .await;
    match ended {
        LinearSetup::Connected { method } => tracing::info!(?method, "connected to Linear"),
        LinearSetup::NotConfigured => tracing::warn!(
            "not logged in to Linear: run `issuerelay-devhost login` (or `api-key`) and start the devhost again; checks fail until then"
        ),
        LinearSetup::InterfaceChanged { message } | LinearSetup::ConnectFailed { message } => {
            tracing::warn!(%message, "not connected to Linear")
        }
        // `wait_for_first_connection` returns only once the connection is no longer under way.
        LinearSetup::Connecting => {}
    }
}

/// What the devhost ends with once the host has stopped: a failure if the engine's task died
/// (nothing polled Linear or started runs after that), as it was before the host was shared.
fn engine_outcome(failure: Option<String>) -> Result<()> {
    match failure {
        Some(why) => Err(anyhow!("the engine task failed: {why}")),
        None => Ok(()),
    }
}

/// Runs one subcommand. The viewer's name and the projects' ids and names come from Linear, whose
/// content is untrusted and may carry terminal escape sequences, so they are printed through
/// `clean_text`.
async fn run(command: Command) -> Result<()> {
    match command {
        // `login` and `api-key` go through the connector, like the host's own: it deletes the
        // other kind of saved credential, which would otherwise be left in the keychain (the
        // OAuth credentials are tried first at every start, so an old login would win over a key).
        Command::Login => {
            let linear = SystemConnector
                .login(Box::new(|url| {
                    // Not passed through `clean_text`: rmcp serializes the URL with `url::Url`, so
                    // it holds no control characters, and `clean_text` would cut a long one.
                    let _ = print_line(&format!("Open this URL to authorize IssueRelay:\n{url}"));
                    let _ = open::that_detached(url);
                }))
                .await?;
            let viewer = linear.viewer().await?;
            print_line(&format!(
                "Logged in to Linear as {}",
                clean_text(&viewer.name)
            ))?;
            Ok(())
        }
        Command::ApiKey => {
            if io::stdin().is_terminal() {
                bail!("pipe the key in, for example: pbpaste | issuerelay-devhost api-key");
            }
            let mut key = String::new();
            io::stdin().read_to_string(&mut key)?;
            let key = key.trim();
            if key.is_empty() {
                bail!("no API key on stdin");
            }
            let linear = SystemConnector.api_key(key).await?;
            let viewer = linear.viewer().await?;
            print_line(&format!("API key works for {}", clean_text(&viewer.name)))?;
            Ok(())
        }
        Command::Logout => {
            auth::clear_credentials()?;
            print_line("Stored Linear credentials removed.")?;
            Ok(())
        }
        Command::Projects => {
            let linear = McpLinear::new(connect().await?).await?;
            let lines: Vec<String> = linear
                .projects()
                .await?
                .iter()
                .map(|project| {
                    format!("{}\t{}", clean_text(&project.id), clean_text(&project.name))
                })
                .collect();
            if !lines.is_empty() {
                print_line(&lines.join("\n"))?;
            }
            linear.close().await
        }
        Command::Run => {
            let host = Host::start(
                AppPaths::system()?,
                Arc::new(SystemConnector),
                HostParts::default(),
            )
            .await?;
            tracing::info!("IssueRelay devhost is running; press Ctrl+C to stop");
            // `start` does not wait for Linear: the outcome of the first connection is logged
            // when it comes.
            tokio::spawn(report_first_connection(host.clone()));
            shutdown_signal()
                .await
                .context("cannot listen for stop signals")?;
            tracing::info!(
                "stopping: waiting for running jobs to end (press Ctrl+C again to stop at once)"
            );
            // The first signal stops the runs and waits until their results are recorded. A second
            // one returns at once: the runtime shuts down after `main` returns, and the guard of
            // each run ends its process tree.
            tokio::select! {
                () = host.stop(StopMode::StopRuns) => engine_outcome(host.engine_failure()),
                second = shutdown_signal() => {
                    second.context("cannot listen for a second stop signal")?;
                    tracing::warn!("second stop signal: ending the remaining runs now");
                    Ok(())
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(default_filter(&cli.command));
    match run(cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // One line with the whole cause chain. The chain can carry Linear's text, so it is
            // cleaned, but not cut: the root cause is its last part. If stderr is gone there is
            // nobody to tell, and the exit code still says the command failed.
            let _ = writeln!(
                io::stderr(),
                "Error: {}",
                strip_control(&format!("{error:#}"))
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use issuerelay_core::host::AuthMethod;

    use super::*;

    /// What `CleanLines` passes on when `text` is written to it in one go.
    fn written(text: &str) -> String {
        let mut out = CleanLines(Vec::new());
        out.write_all(text.as_bytes()).unwrap();
        String::from_utf8(out.0).unwrap()
    }

    #[test]
    fn a_log_line_carries_no_terminal_escapes() {
        // Clear screen, OSC 52 (write the clipboard, ended by BEL), a lone BEL, a carriage return
        // that would overwrite the line, and a right-to-left override.
        let hostile =
            "WARN poll: error=boom\u{1b}[2J\u{1b}]52;c;ZXZpbA==\u{7} more\rrewritten \u{202e}end\n";
        assert_eq!(
            written(hostile),
            "WARN poll: error=boom [2J ]52;c;ZXZpbA==  more rewritten end\n"
        );
    }

    #[test]
    fn only_the_closing_newline_survives() {
        // A newline inside the text would let a field forge a second log line.
        assert_eq!(
            written("INFO a: x=one\nINFO forged: y=two\n"),
            "INFO a: x=one INFO forged: y=two\n"
        );
        assert_eq!(written("\n"), "\n");
        assert_eq!(written("no ending"), "no ending");
        // Text that already ends in a newline before the closing one keeps just one.
        assert_eq!(written("tail\n\n"), "tail \n");
        // Other scripts and punctuation are untouched; so is the C1 range's neighbour U+00A0.
        assert_eq!(written("日本語 é \u{a0}…\n"), "日本語 é \u{a0}…\n");
        // The C1 controls (here NEL and CSI) are controls too.
        assert_eq!(written("a\u{85}b\u{9b}c\n"), "a b c\n");
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

    #[test]
    fn a_write_reports_the_bytes_it_was_given_and_passes_errors_on() {
        let mut out = CleanLines(Vec::new());
        // `write_all` relies on this: the cleaned text is shorter than what came in.
        assert_eq!(out.write(b"a\x1b[2Jb\n").unwrap(), 7);
        assert_eq!(out.0, b"a [2Jb\n");

        let error = CleanLines(Failing(io::ErrorKind::BrokenPipe))
            .write_all(b"gone\n")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn a_closed_pipe_is_not_a_failure_but_other_write_errors_are() {
        assert!(write_line(&mut Failing(io::ErrorKind::BrokenPipe), "text").is_ok());
        let error = write_line(&mut Failing(io::ErrorKind::PermissionDenied), "text").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let mut out = Vec::new();
        write_line(&mut out, "first\nsecond").unwrap();
        assert_eq!(out, b"first\nsecond\n");
    }

    #[test]
    fn only_run_logs_below_warnings_by_default() {
        assert_eq!(default_filter(&Command::Run), "info,issuerelay_core=debug");
        for command in [
            Command::Login,
            Command::ApiKey,
            Command::Logout,
            Command::Projects,
        ] {
            assert_eq!(default_filter(&command), "warn");
        }
    }

    #[tokio::test]
    async fn the_first_connection_is_waited_for_and_only_a_slow_one_is_announced() {
        use std::cell::Cell;

        let tick = Duration::from_millis(1);
        // Ends after three looks.
        let looks = Cell::new(0);
        let quick = || {
            looks.set(looks.get() + 1);
            if looks.get() <= 3 {
                LinearSetup::Connecting
            } else {
                LinearSetup::NotConfigured
            }
        };
        let announced = Cell::new(0);
        let ended = wait_for_first_connection(quick, tick, Duration::from_secs(60), || {
            announced.set(announced.get() + 1)
        })
        .await;
        assert_eq!(ended, LinearSetup::NotConfigured);
        assert_eq!(announced.get(), 0, "a quick connection is not announced");

        // Ends after sixty looks, much later than the announcement is due.
        let looks = Cell::new(0);
        let slow = || {
            looks.set(looks.get() + 1);
            if looks.get() <= 60 {
                LinearSetup::Connecting
            } else {
                LinearSetup::Connected {
                    method: AuthMethod::OAuth,
                }
            }
        };
        let ended = wait_for_first_connection(slow, tick, Duration::from_millis(5), || {
            announced.set(announced.get() + 1)
        })
        .await;
        assert_eq!(
            ended,
            LinearSetup::Connected {
                method: AuthMethod::OAuth
            }
        );
        assert_eq!(announced.get(), 1, "a slow one is announced once");
    }

    #[test]
    fn a_dead_engine_task_makes_the_devhost_fail() {
        assert!(engine_outcome(None).is_ok());
        let error = engine_outcome(Some("task 3 panicked".into())).unwrap_err();
        assert_eq!(error.to_string(), "the engine task failed: task 3 panicked");
    }

    // `login` and `api-key` must go through `SystemConnector`, which deletes the other kind of
    // saved credential. Saving the key or the OAuth credentials directly would leave the old ones
    // in the keychain, where the OAuth credentials win at the next start.
    #[test]
    fn the_login_commands_go_through_the_connector() {
        let source = include_str!("main.rs");
        let production = source.split("\n#[cfg(test)]\n").next().unwrap();
        // However rustfmt breaks the lines.
        let production: String = production.split_whitespace().collect();
        assert!(production.contains("SystemConnector.login("));
        assert!(production.contains("SystemConnector.api_key("));
        for call in ["auth::login(", "auth::save_api_key("] {
            assert!(
                !production.contains(call),
                "the devhost must not call `{call}` itself"
            );
        }
    }

    #[test]
    fn the_error_line_keeps_the_root_cause_however_long_the_chain() {
        let error = anyhow!("root cause\u{1b}[2J")
            .context("x".repeat(600))
            .context("outer\n");
        let line = strip_control(&format!("{error:#}"));
        assert!(line.starts_with("outer : xxx"), "{line}");
        assert!(line.ends_with(": root cause [2J"), "{line}");
        assert_eq!(
            line.chars().count(),
            "outer : ".len() + 600 + ": root cause [2J".len()
        );
    }
}
