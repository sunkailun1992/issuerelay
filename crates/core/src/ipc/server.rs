//! IPC server: binds the endpoint (refusing to start when another host owns it) and answers
//! requests by calling the engine.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::protocol::{METHOD_NOT_FOUND, METHODS, PARSE_ERROR, Request, Response, SERVER_ERROR};
use crate::config::Binding;
use crate::engine::Engine;
use crate::paths::IpcEndpoint;

/// How long `run` waits after a failed accept before it accepts again, so a failure that lasts
/// (no free file descriptors, say) cannot spin the loop.
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// The longest request line a client may send, newline not counted. A longer one is answered with
/// a parse error and the connection is closed.
const MAX_LINE_BYTES: usize = 1024 * 1024;

/// A connection that sends nothing for this long is closed, so clients that went away without
/// closing cannot pile up.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// `list_jobs` asks the store for this many jobs when the caller names no `limit`, and never for
/// more than `MAX_JOBS`.
const DEFAULT_JOBS: u64 = 20;
const MAX_JOBS: u64 = 500;

/// `logs` returns this many lines when the caller names no `lines`, and never more than
/// `MAX_LOG_LINES`.
const DEFAULT_LOG_LINES: u64 = 50;
const MAX_LOG_LINES: u64 = 5_000;

/// The file next to the socket whose lock makes one host the only owner of the endpoint.
#[cfg(unix)]
const LOCK_FILE: &str = "issuerelay.lock";

/// Another host already owns the endpoint.
#[derive(Debug, thiserror::Error)]
#[error("another IssueRelay instance is already running")]
pub struct AlreadyRunning;

#[derive(Debug, thiserror::Error)]
#[error("unknown method `{0}`")]
struct UnknownMethod(String);

/// Answers one request by calling the engine. A failure comes back as an error response.
///
/// The method table is the gate: a method that `METHODS` does not list is refused here, before
/// `dispatch` is asked, so an arm added to `dispatch` alone can never make an action the table
/// leaves out (marking Done, retrying, cancelling) reachable.
pub async fn handle(engine: &Engine, request: Request) -> Response {
    if !METHODS.contains(&request.method.as_str()) {
        let unknown = UnknownMethod(request.method);
        return Response::err(request.id, METHOD_NOT_FOUND, unknown.to_string());
    }
    match dispatch(engine, &request.method, &request.params).await {
        Ok(value) => Response::ok(request.id, value),
        Err(e) if e.is::<UnknownMethod>() => {
            Response::err(request.id, METHOD_NOT_FOUND, e.to_string())
        }
        Err(e) => Response::err(request.id, SERVER_ERROR, format!("{e:#}")),
    }
}

async fn dispatch(engine: &Engine, method: &str, params: &Value) -> Result<Value> {
    Ok(match method {
        "status" => serde_json::to_value(engine.status())?,
        "pause" => {
            engine.pause()?;
            json!({"paused": true})
        }
        "resume" => {
            engine.resume()?;
            json!({"paused": false})
        }
        "poll_now" => {
            engine.poll_now();
            json!({"ok": true})
        }
        "list_jobs" => serde_json::to_value(engine.list_jobs(jobs_limit(params)?)?)?,
        "doctor" => serde_json::to_value(engine.doctor().await)?,
        "bindings.list" => serde_json::to_value(engine.bindings())?,
        "bindings.add" => {
            let raw = params.get("binding").cloned().unwrap_or(Value::Null);
            let binding: Binding = serde_json::from_value(raw).context("invalid `binding`")?;
            engine.add_binding(binding)?;
            json!({"ok": true})
        }
        "bindings.remove" => {
            json!({"removed": engine.remove_binding(string_param(params, "id")?)?})
        }
        "logs" => {
            let job_id = job_id_param(params)?;
            serde_json::to_value(engine.log_tail(job_id, log_lines(params)?)?)?
        }
        // Only reached when `METHODS` lists a method that has no arm above; a test dispatches
        // every listed method, so the table and this match stay in step.
        other => return Err(UnknownMethod(other.to_string()).into()),
    })
}

/// How many jobs a `list_jobs` call asks the store for.
fn jobs_limit(params: &Value) -> Result<usize> {
    count_param(params, "limit", DEFAULT_JOBS, MAX_JOBS)
}

/// How many log lines a `logs` call asks for.
fn log_lines(params: &Value) -> Result<usize> {
    count_param(params, "lines", DEFAULT_LOG_LINES, MAX_LOG_LINES)
}

/// An optional count. Absent (or `null`) gives `default`; a non-negative whole number is capped
/// at `max`; anything else is an error, not a silent default.
fn count_param(params: &Value, name: &str, default: u64, max: u64) -> Result<usize> {
    let count = match params.get(name) {
        None | Some(Value::Null) => default,
        Some(value) => value
            .as_u64()
            .with_context(|| format!("invalid `{name}`: expected a non-negative whole number"))?,
    };
    Ok(count.min(max) as usize)
}

/// The required `job_id`: a missing one and an unreadable one are different mistakes.
fn job_id_param(params: &Value) -> Result<i64> {
    match params.get("job_id") {
        None | Some(Value::Null) => bail!("missing `job_id`"),
        Some(value) => value
            .as_i64()
            .context("invalid `job_id`: expected a whole number"),
    }
}

/// A required string parameter.
fn string_param<'a>(params: &'a Value, name: &str) -> Result<&'a str> {
    match params.get(name) {
        None | Some(Value::Null) => bail!("missing `{name}`"),
        Some(value) => value
            .as_str()
            .with_context(|| format!("invalid `{name}`: expected a string")),
    }
}

/// A bound endpoint; `run` serves it.
pub struct IpcServer {
    listener: Listener,
}

enum Listener {
    #[cfg(unix)]
    Unix {
        listener: tokio::net::UnixListener,
        path: std::path::PathBuf,
        /// The exclusive lock that makes this the only host for the endpoint. It is released
        /// when the server is dropped, and not before.
        _lock: std::fs::File,
    },
    #[cfg(windows)]
    Pipe {
        first: tokio::net::windows::named_pipe::NamedPipeServer,
        name: String,
        /// The security descriptor every instance of the pipe is created with, kept for as long
        /// as the server creates instances.
        security: crate::ipc::pipe_security::PipeSecurity,
    },
}

impl IpcServer {
    /// Binds the endpoint; fails with `AlreadyRunning` when another host owns it.
    ///
    /// On Unix the owner is whoever holds the lock on `issuerelay.lock` in the socket's
    /// directory, for as long as the server lives. Windows needs no lock, see `bind_pipe`.
    pub async fn bind(endpoint: &IpcEndpoint) -> Result<Self> {
        match endpoint {
            #[cfg(unix)]
            IpcEndpoint::Unix(path) => {
                let (listener, lock) = bind_unix(path)?;
                Ok(Self {
                    listener: Listener::Unix {
                        listener,
                        path: path.clone(),
                        _lock: lock,
                    },
                })
            }
            #[cfg(windows)]
            IpcEndpoint::Pipe(name) => {
                let (first, security) = bind_pipe(name)?;
                Ok(Self {
                    listener: Listener::Pipe {
                        first,
                        name: name.clone(),
                        security,
                    },
                })
            }
            other => bail!("IPC endpoint {other:?} is not supported on this platform"),
        }
    }

    /// Serves connections until `shutdown` is cancelled, then ends the ones still open: no
    /// connection outlives the server. A failed accept (or pipe connect, or pipe instance) is
    /// logged and retried after a short pause instead of ending the server: the CLI and MCP
    /// would otherwise lose the host until the app restarts.
    pub async fn run(self, engine: Engine, shutdown: CancellationToken) -> Result<()> {
        match self.listener {
            #[cfg(unix)]
            Listener::Unix {
                listener,
                path,
                _lock,
            } => {
                let mut connections = JoinSet::new();
                loop {
                    reap_finished(&mut connections);
                    let accepted = tokio::select! {
                        _ = shutdown.cancelled() => None,
                        accepted = listener.accept() => Some(accepted),
                    };
                    let Some(accepted) = accepted else { break };
                    match accepted {
                        Ok((stream, _)) => {
                            connections.spawn(serve_connection(stream, engine.clone()));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "IPC accept failed; still serving");
                            if !pause_before_retry(&shutdown).await {
                                break;
                            }
                        }
                    }
                }
                // New clients first find nobody home, then the open connections go; the lock is
                // released last, when this arm ends, so no second host starts before that.
                let _ = std::fs::remove_file(&path);
                drop(listener);
                connections.shutdown().await;
                Ok(())
            }
            #[cfg(windows)]
            Listener::Pipe {
                first,
                name,
                security,
            } => {
                let mut connections = JoinSet::new();
                // The instance that waits for the next client; `None` only while a replacement
                // could not be created.
                let mut waiting = Some(first);
                loop {
                    reap_finished(&mut connections);
                    let server = match waiting.take() {
                        Some(server) => server,
                        None => match new_pipe_instance(&name, &security) {
                            Ok(server) => server,
                            Err(e) => {
                                tracing::warn!(error = %e, "IPC pipe instance not created; still trying");
                                if !pause_before_retry(&shutdown).await {
                                    break;
                                }
                                continue;
                            }
                        },
                    };
                    let connected = tokio::select! {
                        _ = shutdown.cancelled() => None,
                        result = server.connect() => Some(result),
                    };
                    let Some(result) = connected else { break };
                    // The replacement is created before the connected instance is handed to its
                    // task, or the failed one is dropped. A pipe name without any instance shows
                    // clients nobody home ("not running") and lets a second host's
                    // `first_pipe_instance` succeed. Only when the replacement cannot be created
                    // does the name go without a listener, until the retry at the top works.
                    let replacement = new_pipe_instance(&name, &security);
                    let mut trouble = false;
                    match result {
                        Ok(()) => {
                            connections.spawn(serve_connection(server, engine.clone()));
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "IPC connect failed; still serving");
                            drop(server);
                            trouble = true;
                        }
                    }
                    match replacement {
                        Ok(next) => waiting = Some(next),
                        Err(e) => {
                            tracing::warn!(error = %e, "IPC pipe instance not created; still trying");
                            trouble = true;
                        }
                    }
                    if trouble && !pause_before_retry(&shutdown).await {
                        break;
                    }
                }
                connections.shutdown().await;
                Ok(())
            }
        }
    }
}

/// Collects the connection tasks that have ended, so the set does not grow with every client.
fn reap_finished(connections: &mut JoinSet<Result<()>>) {
    while let Some(ended) = connections.try_join_next() {
        match ended {
            Ok(Ok(())) => {}
            // Clients hang up all the time.
            Ok(Err(e)) => tracing::debug!(error = %format!("{e:#}"), "IPC connection ended"),
            Err(e) => tracing::warn!(error = %e, "IPC connection task failed"),
        }
    }
}

/// Waits out `RETRY_DELAY`; `false` when the shutdown came first.
async fn pause_before_retry(shutdown: &CancellationToken) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => false,
        _ = tokio::time::sleep(RETRY_DELAY) => true,
    }
}

/// Binds the socket and returns the lock that makes this host its only owner.
///
/// The lock comes first: an advisory exclusive lock on `LOCK_FILE` in the socket's directory,
/// which a second host cannot get, so it is told `AlreadyRunning` however the two starts
/// interleave. Probing the socket instead ("connect to see whether it is alive") races with the
/// other host and mistakes any failed connect, such as running out of file descriptors, for a
/// dead socket whose file it then deletes. The lock file is never deleted: a deleted one could
/// be created and locked anew while another host still holds the old one.
#[cfg(unix)]
fn bind_unix(path: &std::path::Path) -> Result<(tokio::net::UnixListener, std::fs::File)> {
    use std::fs::{OpenOptions, Permissions, TryLockError};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let len = path.as_os_str().len();
    let max = crate::paths::UNIX_SOCKET_MAX;
    if len > max {
        bail!(
            "IPC socket path is too long ({len} bytes, at most {max}): {}",
            path.display()
        );
    }
    let dir = path
        .parent()
        .with_context(|| format!("socket path {} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    // Also tightens a data directory that another part of the app created earlier.
    std::fs::set_permissions(dir, Permissions::from_mode(0o700))
        .with_context(|| format!("cannot restrict {} to the current user", dir.display()))?;

    let lock_path = dir.join(LOCK_FILE);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock_path)
        .with_context(|| format!("cannot open {}", lock_path.display()))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(AlreadyRunning.into()),
        Err(TryLockError::Error(e)) => {
            return Err(
                anyhow::Error::new(e).context(format!("cannot lock {}", lock_path.display()))
            );
        }
    }

    // Holding the lock, whatever lies at the socket path is a leftover. `remove_file` does not
    // follow symlinks, so a dangling link goes too (on macOS `bind` would otherwise create the
    // socket at the link's target).
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow::Error::new(e)
                .context(format!("cannot remove the leftover {}", path.display())));
        }
    }
    let listener = tokio::net::UnixListener::bind(path).map_err(|e| bind_error(e, path))?;
    Ok((listener, lock))
}

/// A failed `bind` of the socket. Whatever lay at the path was removed while holding the lock, so
/// a path that is taken again by now was taken by a process that ignores the lock (a build
/// without one, say): "address in use" means someone else owns the endpoint.
#[cfg(unix)]
fn bind_error(error: std::io::Error, path: &std::path::Path) -> anyhow::Error {
    use std::io::ErrorKind::{AddrInUse, AlreadyExists};
    if matches!(error.kind(), AddrInUse | AlreadyExists) {
        AlreadyRunning.into()
    } else {
        anyhow::Error::new(error).context(format!("cannot bind {}", path.display()))
    }
}

/// Creates the first pipe instance and returns the security descriptor that every instance gets:
/// only the user the host runs as, and SYSTEM, may open the pipe (`pipe_security`). There is no
/// lock file on Windows: asking for the first instance makes the system refuse (access denied)
/// when any instance of the name exists, and it does so atomically, so racing hosts are told
/// apart by the system itself. The name goes away with its last instance, so nothing is left
/// over to clean up either.
#[cfg(windows)]
fn bind_pipe(
    name: &str,
) -> Result<(
    tokio::net::windows::named_pipe::NamedPipeServer,
    crate::ipc::pipe_security::PipeSecurity,
)> {
    use tokio::net::windows::named_pipe::ServerOptions;
    const ERROR_ACCESS_DENIED: i32 = 5;
    let security = crate::ipc::pipe_security::PipeSecurity::new()
        .context("cannot build the security descriptor of IssueRelay's pipe")?;
    // SAFETY: the attributes point at a live descriptor owned by `security`, which the server
    // keeps for as long as it creates instances.
    let created = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, security.attributes())
    };
    match created {
        Ok(server) => Ok((server, security)),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Err(AlreadyRunning.into()),
        Err(e) => Err(e.into()),
    }
}

/// Creates another instance of the pipe, with the same security descriptor as the first.
#[cfg(windows)]
fn new_pipe_instance(
    name: &str,
    security: &crate::ipc::pipe_security::PipeSecurity,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    // SAFETY: the attributes point at the live descriptor `security` owns, which outlives this
    // call.
    unsafe {
        ServerOptions::new()
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, security.attributes())
    }
}

/// Answers the request lines of one connection, one response per line, until the client hangs
/// up, sends a line that is too long, or stays silent for `IDLE_TIMEOUT`.
async fn serve_connection<S>(stream: S, engine: Engine) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut line = Vec::new();
    loop {
        line.clear();
        // One byte more than the limit is read, so a line of exactly `MAX_LINE_BYTES` (plus its
        // newline) can be told from a longer one.
        let mut capped = (&mut reader).take(MAX_LINE_BYTES as u64 + 1);
        let Ok(got) = tokio::time::timeout(IDLE_TIMEOUT, capped.read_until(b'\n', &mut line)).await
        else {
            return Ok(());
        };
        if got? == 0 {
            return Ok(());
        }
        if line.last() != Some(&b'\n') && line.len() > MAX_LINE_BYTES {
            let too_large = Response::err(0, PARSE_ERROR, "request too large");
            send(&mut write, &too_large).await?;
            return Ok(());
        }
        let response = match std::str::from_utf8(&line) {
            Err(_) => Response::err(0, PARSE_ERROR, "invalid request: not valid UTF-8"),
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => match serde_json::from_str::<Request>(text) {
                Ok(request) => handle(&engine, request).await,
                Err(e) => Response::err(0, PARSE_ERROR, format!("invalid request: {e}")),
            },
        };
        send(&mut write, &response).await?;
    }
}

async fn send<W>(write: &mut W, response: &Response) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut text = serde_json::to_string(response)?;
    text.push('\n');
    write.write_all(text.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The method table gates what can be asked, but the hard rule is about what the server can
    // do: nothing reachable over IPC may call the engine's panel-only operations, under any name.
    #[test]
    fn the_server_never_calls_a_panel_only_engine_method() {
        let source = include_str!("server.rs");
        let production = source.split("\n#[cfg(test)]\n").next().unwrap();
        assert!(
            production.contains("engine.pause()"),
            "the scan must reach the dispatcher"
        );
        for call in ["cancel_job", "retry_job", "mark_done"] {
            assert!(
                !production.contains(call),
                "the IPC server must not call `{call}`"
            );
        }
    }

    #[test]
    fn counts_default_when_absent_and_are_capped() {
        assert_eq!(jobs_limit(&json!({})).unwrap(), 20);
        assert_eq!(jobs_limit(&Value::Null).unwrap(), 20, "no params at all");
        assert_eq!(jobs_limit(&json!({"limit": null})).unwrap(), 20);
        assert_eq!(jobs_limit(&json!({"limit": 0})).unwrap(), 0);
        assert_eq!(jobs_limit(&json!({"limit": 500})).unwrap(), 500);
        assert_eq!(jobs_limit(&json!({"limit": 501})).unwrap(), 500);
        assert_eq!(jobs_limit(&json!({"limit": 10_000})).unwrap(), 500);
        assert_eq!(jobs_limit(&json!({"limit": u64::MAX})).unwrap(), 500);
        assert_eq!(log_lines(&json!({})).unwrap(), 50);
        assert_eq!(log_lines(&json!({"lines": 5_000})).unwrap(), 5_000);
        assert_eq!(log_lines(&json!({"lines": 100_000})).unwrap(), 5_000);
        assert_eq!(log_lines(&json!({"lines": 3})).unwrap(), 3);
    }

    #[test]
    fn counts_that_cannot_be_read_are_errors_not_defaults() {
        for bad in [
            json!(-1),
            json!("5"),
            json!([5]),
            json!({"n": 5}),
            json!(1.5),
            json!(true),
        ] {
            let err = jobs_limit(&json!({"limit": bad})).unwrap_err();
            assert!(err.to_string().contains("invalid `limit`"), "{bad}: {err}");
            let err = log_lines(&json!({"lines": bad})).unwrap_err();
            assert!(err.to_string().contains("invalid `lines`"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_job_id_is_required_and_must_be_a_whole_number() {
        assert_eq!(job_id_param(&json!({"job_id": 7})).unwrap(), 7);
        for missing in [json!({}), json!({"job_id": null}), Value::Null] {
            let err = job_id_param(&missing).unwrap_err();
            assert!(err.to_string().contains("missing `job_id`"), "{err}");
        }
        for bad in [json!("7"), json!([7]), json!(7.5), json!(true)] {
            let err = job_id_param(&json!({"job_id": bad})).unwrap_err();
            assert!(err.to_string().contains("invalid `job_id`"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_string_parameter_is_required_and_must_be_a_string() {
        assert_eq!(string_param(&json!({"id": "a"}), "id").unwrap(), "a");
        let err = string_param(&json!({}), "id").unwrap_err();
        assert!(err.to_string().contains("missing `id`"), "{err}");
        let err = string_param(&json!({"id": 5}), "id").unwrap_err();
        assert!(err.to_string().contains("invalid `id`"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn address_in_use_means_the_endpoint_is_owned() {
        use std::io::{Error, ErrorKind};
        let path = std::path::Path::new("/nowhere/issuerelay.sock");
        for kind in [ErrorKind::AddrInUse, ErrorKind::AlreadyExists] {
            let err = bind_error(Error::from(kind), path);
            assert!(err.downcast_ref::<AlreadyRunning>().is_some(), "{kind:?}");
        }
        let err = bind_error(Error::from(ErrorKind::PermissionDenied), path);
        assert!(err.downcast_ref::<AlreadyRunning>().is_none());
        assert!(
            err.to_string().contains("/nowhere/issuerelay.sock"),
            "other failures name the socket: {err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_over_long_socket_path_is_named_in_the_error() {
        let path = std::path::PathBuf::from(format!("/{}/issuerelay.sock", "a".repeat(120)));
        let len = path.as_os_str().len();
        let err = IpcServer::bind(&IpcEndpoint::Unix(path.clone()))
            .await
            .err()
            .expect("an over-long path must not bind");
        assert_eq!(
            err.to_string(),
            format!(
                "IPC socket path is too long ({len} bytes, at most 103): {}",
                path.display()
            )
        );
        assert!(err.downcast_ref::<AlreadyRunning>().is_none());
    }

    // 103 bytes is the last length that fits macOS's 104-byte `sun_path` with its NUL.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_socket_path_of_103_bytes_binds_and_one_of_104_does_not() {
        let root = tempfile::tempdir().unwrap();
        let tail = "issuerelay.sock";
        let fixed = root.path().as_os_str().len() + 1 + 1 + tail.len();
        assert!(fixed < 103, "the temp dir is too long for this test");
        // <root>/<padding>/issuerelay.sock, `total` bytes in all.
        let path_of = |total: usize| root.path().join("p".repeat(total - fixed)).join(tail);
        let fits = path_of(103);
        assert_eq!(fits.as_os_str().len(), 103);
        IpcServer::bind(&IpcEndpoint::Unix(fits))
            .await
            .expect("a 103-byte path fits");
        let err = IpcServer::bind(&IpcEndpoint::Unix(path_of(104)))
            .await
            .err()
            .expect("a 104-byte path does not");
        assert!(
            err.to_string()
                .starts_with("IPC socket path is too long (104 bytes, at most 103): "),
            "{err}"
        );
    }
}
