//! IPC client used by the CLI and the MCP control server.
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use super::protocol::{Request, Response};
use crate::paths::IpcEndpoint;

/// No host owns the endpoint.
#[derive(Debug, thiserror::Error)]
#[error("IssueRelay is not running")]
pub struct NotRunning;

/// How long a call may take: a quick read, or a change to the configuration.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// `doctor` runs both executors' checks (`mcp list` alone may take up to 60 s) and several Linear
/// calls, so it gets far longer than any other call.
const DOCTOR_TIMEOUT: Duration = Duration::from_secs(180);

fn timeout_for(method: &str) -> Duration {
    if method == "doctor" {
        DOCTOR_TIMEOUT
    } else {
        CALL_TIMEOUT
    }
}

/// Sends one request and returns its `result`, or the error message the engine reported.
/// `NotRunning` means no host owns the endpoint; any other failure to reach it says why.
pub async fn call(endpoint: &IpcEndpoint, method: &str, params: Value) -> Result<Value> {
    let mut line = serde_json::to_string(&Request::new(1, method, params))?;
    line.push('\n');
    let reply = tokio::time::timeout(timeout_for(method), exchange(endpoint, line))
        .await
        .context("IssueRelay did not answer in time")??;
    let response: Response =
        serde_json::from_str(&reply).context("invalid response from IssueRelay")?;
    if let Some(error) = response.error {
        bail!("{}", error.message);
    }
    Ok(response.result.unwrap_or(Value::Null))
}

async fn exchange(endpoint: &IpcEndpoint, line: String) -> Result<String> {
    match endpoint {
        #[cfg(unix)]
        IpcEndpoint::Unix(path) => {
            use std::io::ErrorKind::{ConnectionRefused, NotFound};
            let stream = match tokio::net::UnixStream::connect(path).await {
                Ok(stream) => stream,
                // No socket file, or a leftover one nobody listens on: no host is running.
                Err(e) if matches!(e.kind(), NotFound | ConnectionRefused) => {
                    return Err(NotRunning.into());
                }
                // Anything else (no permission, no file descriptors, ...) is not "not running".
                Err(e) => {
                    return Err(anyhow::Error::new(e)
                        .context(format!("cannot connect to {}", path.display())));
                }
            };
            roundtrip(stream, line).await
        }
        #[cfg(windows)]
        IpcEndpoint::Pipe(name) => roundtrip(open_pipe(name).await?, line).await,
        other => bail!("IPC endpoint {other:?} is not supported on this platform"),
    }
}

#[cfg(windows)]
async fn open_pipe(name: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..50 {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(NotRunning.into()),
            Err(e) => return Err(e.into()),
        }
    }
    bail!("IssueRelay's pipe stayed busy")
}

async fn roundtrip<S>(stream: S, line: String) -> Result<String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    write.write_all(line.as_bytes()).await?;
    let mut lines = BufReader::new(read).lines();
    lines
        .next_line()
        .await?
        .context("IssueRelay closed the connection without answering")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_gets_the_long_timeout() {
        assert_eq!(timeout_for("doctor"), Duration::from_secs(180));
        assert_eq!(timeout_for("status"), Duration::from_secs(30));
        assert_eq!(timeout_for("bindings.add"), Duration::from_secs(30));
    }
}
