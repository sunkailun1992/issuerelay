//! IPC server: binds the endpoint (refusing to start when another host owns it) and answers
//! requests by calling the engine.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::protocol::{METHOD_NOT_FOUND, PARSE_ERROR, Request, Response, SERVER_ERROR};
use crate::config::Binding;
use crate::engine::Engine;
use crate::paths::IpcEndpoint;

/// How long `run` waits after a failed accept before it accepts again, so a failure that lasts
/// (no free file descriptors, say) cannot spin the loop.
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// Another host already owns the endpoint.
#[derive(Debug, thiserror::Error)]
#[error("another IssueRelay instance is already running")]
pub struct AlreadyRunning;

#[derive(Debug, thiserror::Error)]
#[error("unknown method `{0}`")]
struct UnknownMethod(String);

/// Answers one request by calling the engine. A failure comes back as an error response.
pub async fn handle(engine: &Engine, request: Request) -> Response {
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
        "list_jobs" => {
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .min(500) as usize;
            serde_json::to_value(engine.list_jobs(limit)?)?
        }
        "doctor" => serde_json::to_value(engine.doctor().await)?,
        "bindings.list" => serde_json::to_value(engine.bindings())?,
        "bindings.add" => {
            let raw = params.get("binding").cloned().unwrap_or(Value::Null);
            let binding: Binding = serde_json::from_value(raw).context("invalid `binding`")?;
            engine.add_binding(binding)?;
            json!({"ok": true})
        }
        "bindings.remove" => {
            let id = params
                .get("id")
                .and_then(Value::as_str)
                .context("missing `id`")?;
            json!({"removed": engine.remove_binding(id)?})
        }
        "logs" => {
            let job_id = params
                .get("job_id")
                .and_then(Value::as_i64)
                .context("missing `job_id`")?;
            let lines = params
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(50)
                .min(5_000) as usize;
            serde_json::to_value(engine.log_tail(job_id, lines)?)?
        }
        other => return Err(UnknownMethod(other.to_string()).into()),
    })
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
    },
    #[cfg(windows)]
    Pipe {
        first: tokio::net::windows::named_pipe::NamedPipeServer,
        name: String,
    },
}

impl IpcServer {
    /// Binds the endpoint; fails with `AlreadyRunning` when another host owns it.
    pub async fn bind(endpoint: &IpcEndpoint) -> Result<Self> {
        match endpoint {
            #[cfg(unix)]
            IpcEndpoint::Unix(path) => Ok(Self {
                listener: Listener::Unix {
                    listener: bind_unix(path).await?,
                    path: path.clone(),
                },
            }),
            #[cfg(windows)]
            IpcEndpoint::Pipe(name) => Ok(Self {
                listener: Listener::Pipe {
                    first: bind_pipe(name)?,
                    name: name.clone(),
                },
            }),
            other => bail!("IPC endpoint {other:?} is not supported on this platform"),
        }
    }

    /// Serves connections until `shutdown` is cancelled. A failed accept (or pipe connect) is
    /// logged and retried after a short pause instead of ending the server: the CLI and MCP
    /// would otherwise lose the host until the app restarts. On Windows only a pipe instance
    /// that cannot be created ends it with an error.
    pub async fn run(self, engine: Engine, shutdown: CancellationToken) -> Result<()> {
        match self.listener {
            #[cfg(unix)]
            Listener::Unix { listener, path } => {
                loop {
                    let accepted = tokio::select! {
                        _ = shutdown.cancelled() => None,
                        accepted = listener.accept() => Some(accepted),
                    };
                    let Some(accepted) = accepted else { break };
                    match accepted {
                        Ok((stream, _)) => {
                            let engine = engine.clone();
                            tokio::spawn(async move {
                                let _ = serve_connection(stream, engine).await;
                            });
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "IPC accept failed; still serving");
                            if !pause_before_retry(&shutdown).await {
                                break;
                            }
                        }
                    }
                }
                let _ = std::fs::remove_file(&path);
                Ok(())
            }
            #[cfg(windows)]
            Listener::Pipe { first, name } => {
                let mut server = first;
                loop {
                    let connected = tokio::select! {
                        _ = shutdown.cancelled() => None,
                        result = server.connect() => Some(result),
                    };
                    let Some(result) = connected else { break };
                    // Create the next instance first: after a successful connect it listens while
                    // the connected one is served, after a failed connect it replaces the failed
                    // one. Either way the pipe name always has a listening instance.
                    let next = new_pipe_instance(&name)?;
                    match result {
                        Ok(()) => {
                            let client = std::mem::replace(&mut server, next);
                            let engine = engine.clone();
                            tokio::spawn(async move {
                                let _ = serve_connection(client, engine).await;
                            });
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "IPC connect failed; still serving");
                            server = next;
                            if !pause_before_retry(&shutdown).await {
                                break;
                            }
                        }
                    }
                }
                Ok(())
            }
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

#[cfg(unix)]
async fn bind_unix(path: &std::path::Path) -> Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
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
        .context("socket path has no parent directory")?;
    std::fs::create_dir_all(dir)?;
    // Also tightens a data directory that another part of the app created earlier.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    if path.exists() {
        if tokio::net::UnixStream::connect(path).await.is_ok() {
            return Err(AlreadyRunning.into());
        }
        std::fs::remove_file(path)?;
    }
    Ok(tokio::net::UnixListener::bind(path)?)
}

#[cfg(windows)]
fn bind_pipe(name: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    const ERROR_ACCESS_DENIED: i32 = 5;
    match ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(name)
    {
        Ok(server) => Ok(server),
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => Err(AlreadyRunning.into()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(windows)]
fn new_pipe_instance(
    name: &str,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    ServerOptions::new()
        .reject_remote_clients(true)
        .create(name)
}

async fn serve_connection<S>(stream: S, engine: Engine) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(&engine, request).await,
            Err(e) => Response::err(0, PARSE_ERROR, format!("invalid request: {e}")),
        };
        let mut text = serde_json::to_string(&response)?;
        text.push('\n');
        write.write_all(text.as_bytes()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[tokio::test]
    async fn an_over_long_socket_path_is_named_in_the_error() {
        use super::*;
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
}
