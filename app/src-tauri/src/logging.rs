//! Logs go to `<data dir>/logs/app.<date>.log` (a new file each day, by the UTC date; seven files
//! kept) and, in debug builds, to stderr, every line through core's scrub
//! (`logscrub::CleanWriter`): event fields can hold Linear's or an executor's text. `RUST_LOG`
//! overrides the default filter, but rmcp is never logged below `info`: at `debug` it writes the
//! raw OAuth authorization code.
use std::{fmt::Display, path::Path};

use issuerelay_core::{logscrub::CleanWriter, text::strip_control};
use tracing::{Level, Subscriber};
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{EnvFilter, filter::filter_fn, fmt, prelude::*};

const DEFAULT_FILTER: &str = "info,issuerelay_core=debug,issuerelay_app=debug";

/// How many daily log files are kept. Only files named `app.*.log` count and are deleted: the
/// run logs (`<job id>.jsonl`) in the same folder are not the appender's.
const KEPT_LOG_FILES: usize = 7;

/// Whether an event of `level` from `target` may be logged at all.
pub fn allowed(target: &str, level: &Level) -> bool {
    let rmcp = target == "rmcp" || target.starts_with("rmcp::");
    !(rmcp && *level > Level::INFO)
}

/// Logs a tray, window or store call that failed; the app goes on without it.
pub fn warn_failed<T, E: Display>(result: Result<T, E>, what: &str) {
    if let Err(e) = result {
        tracing::warn!(error = %e, "{what} failed");
    }
}

/// Logs an event the panel could not be sent; its webview may be gone, as while the app quits.
pub fn debug_failed<T, E: Display>(result: Result<T, E>, what: &str) {
    if let Err(e) = result {
        tracing::debug!(error = %e, "{what} failed");
    }
}

/// Logs every panic through tracing, so that it reaches the log file scrubbed like any other line
/// (a release build on Windows has no console, and a panicking task would otherwise end without a
/// trace), then hands it to the hook that was there before, which prints it as before.
pub fn install_panic_hook() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(panic = %info, "IssueRelay panicked");
        before(info);
    }));
}

/// Starts logging; keep the returned guard alive for as long as the app runs and drop it before
/// the process ends, or the last lines are lost. Without a log directory only stderr is used.
pub fn init(logs_dir: Option<&Path>) -> Option<WorkerGuard> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    let (subscriber, guard) = subscriber(logs_dir, filter, cfg!(debug_assertions));
    if let Err(e) = subscriber.try_init() {
        eprintln!("IssueRelay could not start logging: {e}");
    }
    guard
}

/// The subscriber `init` installs: `filter`, the rmcp cap, the daily file in `logs_dir` when it
/// can be opened, and stderr when `stderr` is set; with the guard of the file's background writer.
fn subscriber(
    logs_dir: Option<&Path>,
    filter: EnvFilter,
    stderr: bool,
) -> (impl Subscriber + Send + Sync + 'static, Option<WorkerGuard>) {
    let cap = filter_fn(|meta| allowed(meta.target(), meta.level()));
    let appender = logs_dir.and_then(|dir| {
        let built = tracing_appender::rolling::Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix("app")
            .filename_suffix("log")
            .max_log_files(KEPT_LOG_FILES)
            .build(dir);
        match built {
            Ok(appender) => Some(appender),
            Err(e) => {
                // Logging is not up yet, so stderr is the only place to say it.
                eprintln!(
                    "IssueRelay cannot write its log file: {}",
                    strip_control(&e.to_string())
                );
                None
            }
        }
    });
    let (file_layer, guard) = match appender {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer()
                .with_ansi(false)
                .with_writer(CleanWriter(writer));
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    // No colours: the scrub would turn their escape sequences into stray text. A failed write to
    // stderr is not reported: the subscriber would report it with `eprintln!`, which panics when
    // stderr is a closed pipe, as it is once the terminal of a development run has gone.
    let stderr_layer = stderr.then(|| {
        fmt::layer()
            .with_ansi(false)
            .with_writer(CleanWriter(std::io::stderr))
            .log_internal_errors(false)
    });
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(cap)
        .with(file_layer)
        .with(stderr_layer);
    (subscriber, guard)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use chrono::Utc;

    use super::*;

    #[test]
    fn rmcp_never_logs_below_info() {
        // rmcp logs the raw OAuth authorization code at debug; a user's own RUST_LOG must not
        // bring that into the log file.
        assert!(!allowed("rmcp::transport::auth", &Level::DEBUG));
        assert!(!allowed("rmcp", &Level::TRACE));
        assert!(allowed("rmcp::service", &Level::INFO));
        assert!(allowed("issuerelay_core::engine", &Level::DEBUG));
        assert!(
            allowed("rmcpx", &Level::DEBUG),
            "only the rmcp crate itself"
        );
    }

    /// The names of the files in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_log_file_is_named_by_day_and_every_line_is_scrubbed() {
        let dir = tempfile::tempdir().unwrap();
        let day = || Utc::now().format("app.%Y-%m-%d.log").to_string();
        let before = day();
        // Everything at trace, as a user's RUST_LOG could ask: the rmcp cap still holds.
        let (subscriber, guard) = subscriber(Some(dir.path()), EnvFilter::new("trace"), false);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(error = %"boom\u{1b}[2J\nINFO forged: x=1", "poll failed");
            tracing::debug!(target: "rmcp::transport::auth", "authorization code abc123");
            tracing::debug!(target: "issuerelay_core::engine", "checked Linear");
        });
        // Dropping the guard writes out what the background writer still holds.
        drop(guard);
        let after = day();
        let files = names(dir.path());
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(
            files[0] == before || files[0] == after,
            "app.<day in UTC>.log: {files:?}"
        );
        let text = fs::read_to_string(dir.path().join(&files[0])).unwrap();
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert_eq!(
            text.lines().count(),
            2,
            "one line per event, none forged: {text:?}"
        );
        assert!(text.contains("poll failed") && text.contains("boom [2J INFO forged"));
        assert!(text.contains("checked Linear"));
        assert!(!text.contains("authorization code"), "{text:?}");
    }

    #[test]
    fn a_panic_is_logged_scrubbed_and_still_handed_to_the_hook_before() {
        let dir = tempfile::tempdir().unwrap();
        let (subscriber, guard) = subscriber(Some(dir.path()), EnvFilter::new("info"), false);
        let before = std::panic::take_hook();
        // Stands in for the default hook (which prints the panic): the new hook must hand it on.
        let handed_on = Arc::new(AtomicBool::new(false));
        let flag = handed_on.clone();
        std::panic::set_hook(Box::new(move |_| flag.store(true, Ordering::SeqCst)));
        install_panic_hook();
        // A thread whose default subscriber is the log file's, as the app's are once logging is up.
        let outcome = std::thread::spawn(move || {
            tracing::subscriber::with_default(subscriber, || {
                panic!("the forwarder broke\u{1b}[2J\nINFO forged: x=1")
            })
        })
        .join();
        std::panic::set_hook(before);
        assert!(outcome.is_err(), "the thread panicked");
        assert!(
            handed_on.load(Ordering::SeqCst),
            "the hook before still saw it"
        );
        drop(guard);
        let file = names(dir.path()).pop().expect("a log file");
        let text = fs::read_to_string(dir.path().join(file)).unwrap();
        assert!(text.contains("IssueRelay panicked"), "{text:?}");
        assert!(
            text.contains("the forwarder broke [2J INFO forged: x=1"),
            "{text:?}"
        );
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert_eq!(text.lines().count(), 1, "one scrubbed line: {text:?}");
    }

    #[test]
    fn seven_log_files_are_kept_and_the_run_logs_beside_them_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=8 {
            fs::write(
                dir.path().join(format!("app.2000-01-{day:02}.log")),
                "old\n",
            )
            .unwrap();
        }
        fs::write(dir.path().join("1.jsonl"), "{}\n").unwrap();
        let (_subscriber, guard) = subscriber(Some(dir.path()), EnvFilter::new("info"), false);
        drop(guard);
        let files = names(dir.path());
        let logs = files
            .iter()
            .filter(|name| name.starts_with("app.") && name.ends_with(".log"))
            .count();
        assert_eq!(logs, 7, "{files:?}");
        assert!(files.contains(&"1.jsonl".to_string()), "{files:?}");
    }

    #[test]
    fn without_a_usable_log_folder_logging_goes_on_without_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("data");
        fs::write(&blocker, "a file where the folder would go").unwrap();
        let (_subscriber, guard) =
            subscriber(Some(&blocker.join("logs")), EnvFilter::new("info"), false);
        assert!(guard.is_none());
        let (_subscriber, guard) = subscriber(None, EnvFilter::new("info"), false);
        assert!(guard.is_none());
    }
}
