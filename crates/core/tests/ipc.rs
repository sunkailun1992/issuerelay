mod common;

use std::{path::Path, sync::Arc, time::Duration};

use common::{
    engine_kit::{Harness, ScriptedRunner},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{Engine, EngineDeps},
    ipc::{
        client::{self, NotRunning},
        protocol::{METHOD_NOT_FOUND, METHODS, PARSE_ERROR, Request, Response, SERVER_ERROR},
        server::{AlreadyRunning, IpcServer, handle},
    },
    paths::{AppPaths, IpcEndpoint},
    store::Store,
};
use serde_json::json;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf,
};
use tokio_util::sync::CancellationToken;

/// Serves `engine` on an endpoint under `root`.
async fn serve(engine: &Engine, root: &Path) -> (IpcEndpoint, CancellationToken) {
    let endpoint = AppPaths::under(&root.join("app")).ipc_endpoint();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(engine.clone(), token.clone()));
    (endpoint, token)
}

async fn start(h: &Harness) -> (IpcEndpoint, CancellationToken) {
    serve(&h.engine, h.root.path()).await
}

/// A connection spoken to by hand, for what the client library never does: send lines it would
/// not write, or stay connected.
trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

struct Wire {
    reader: BufReader<ReadHalf<Box<dyn Duplex>>>,
    writer: WriteHalf<Box<dyn Duplex>>,
}

impl Wire {
    async fn connect(endpoint: &IpcEndpoint) -> Self {
        let stream: Box<dyn Duplex> = match endpoint {
            #[cfg(unix)]
            IpcEndpoint::Unix(path) => {
                Box::new(tokio::net::UnixStream::connect(path).await.unwrap())
            }
            #[cfg(windows)]
            IpcEndpoint::Pipe(name) => Box::new(open_pipe(name).await),
            other => panic!("unsupported endpoint {other:?}"),
        };
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: BufReader::new(reader),
            writer,
        }
    }

    async fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes).await
    }

    /// The next response, or `None` once the server has closed the connection (a reset counts
    /// as closed).
    async fn response(&mut self) -> Option<Response> {
        let mut line = String::new();
        let read = tokio::time::timeout(Duration::from_secs(10), self.reader.read_line(&mut line));
        match read.await.expect("no answer and no hang-up within 10 s") {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(
                serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("not a response ({e}): {line:?}")),
            ),
        }
    }
}

#[cfg(windows)]
async fn open_pipe(name: &str) -> tokio::net::windows::named_pipe::NamedPipeClient {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..100 {
        match ClientOptions::new().open(name) {
            Ok(client) => return client,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("cannot open {name}: {e}"),
        }
    }
    panic!("{name} stayed busy")
}

/// A request line, newline included.
fn request_line(id: u64, method: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&Request::new(id, method, json!({}))).unwrap();
    bytes.push(b'\n');
    bytes
}

/// Removes a directory when dropped, so a test that fails on the way leaves nothing behind.
#[cfg(unix)]
struct RemoveDirOnDrop(std::path::PathBuf);

#[cfg(unix)]
impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn status_pause_and_bindings_round_trip() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    client::call(&endpoint, "pause", json!({})).await.unwrap();
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        true
    );
    let dir = h.root.path().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    let binding = json!({"id": "proj-9", "project_id": "proj-9", "project_name": "Demo Project", "directory": dir});
    client::call(&endpoint, "bindings.add", json!({"binding": binding}))
        .await
        .unwrap();
    let list = client::call(&endpoint, "bindings.list", json!({}))
        .await
        .unwrap();
    assert_eq!(list[0]["project_id"], "proj-9");
    let removed = client::call(&endpoint, "bindings.remove", json!({"id": "proj-9"}))
        .await
        .unwrap();
    assert_eq!(removed["removed"], true);
    token.cancel();
}

#[tokio::test]
async fn jobs_and_logs_over_ipc() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    h.bind("proj-1", "a");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let jobs = client::call(&endpoint, "list_jobs", json!({"limit": 5}))
        .await
        .unwrap();
    assert_eq!(jobs[0]["state"], "awaiting_acceptance");
    let logs = client::call(
        &endpoint,
        "logs",
        json!({"job_id": jobs[0]["id"], "lines": 10}),
    )
    .await
    .unwrap();
    assert_eq!(logs, json!(["{\"type\":\"result\"}"]));
    token.cancel();
}

// Which requests are answered at all: the method table decides, not the dispatcher.

#[tokio::test]
async fn unknown_and_panel_only_methods_are_rejected() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    for method in [
        "mark_done",
        "retry_job",
        "cancel_job",
        "job.cancel",
        "jobs.retry",
        "jobs.mark_done",
        "nope",
    ] {
        let err = client::call(&endpoint, method, json!({"job_id": 1}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("unknown method"),
            "{method}: {err}"
        );
    }
    token.cancel();
}

#[tokio::test]
async fn every_listed_method_is_dispatched() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    for method in METHODS {
        // Empty parameters: a method that needs some answers with a parameter error, which is
        // fine. "Unknown" would mean the table lists a method the dispatcher has no arm for.
        if let Err(err) = client::call(&endpoint, method, json!({})).await {
            assert!(
                !err.to_string().contains("unknown method"),
                "{method} is listed but not dispatched: {err}"
            );
        }
    }
    token.cancel();
}

#[tokio::test]
async fn answers_carry_json_rpc_codes_and_the_request_id() {
    let h = Harness::new(2, false);
    let unknown = handle(
        &h.engine,
        Request::new(7, "mark_done", json!({"job_id": 1})),
    )
    .await;
    assert_eq!(unknown.id, 7);
    assert_eq!(unknown.error.expect("an error").code, METHOD_NOT_FOUND);
    // Missing or unreadable parameters are errors, never defaults.
    for (method, params, message) in [
        ("logs", json!({}), "missing `job_id`"),
        ("bindings.remove", json!({}), "missing `id`"),
        ("bindings.add", json!({}), "invalid `binding`"),
    ] {
        let reply = handle(&h.engine, Request::new(8, method, params)).await;
        assert!(reply.result.is_none(), "{method}");
        let error = reply.error.unwrap_or_else(|| panic!("{method} must fail"));
        assert_eq!(error.code, SERVER_ERROR, "{method}");
        assert!(
            error.message.contains(message),
            "{method}: {}",
            error.message
        );
    }
    let ok = handle(&h.engine, Request::new(9, "status", json!({}))).await;
    assert_eq!(ok.id, 9);
    assert!(ok.error.is_none());
    assert_eq!(ok.result.expect("a result")["paused"], false);
}

// Parameters: what is accepted, what is capped, what is refused.

#[tokio::test]
async fn unreadable_parameters_are_errors_not_defaults() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    for (method, params, message) in [
        ("list_jobs", json!({"limit": -1}), "invalid `limit`"),
        ("list_jobs", json!({"limit": "5"}), "invalid `limit`"),
        ("list_jobs", json!({"limit": [5]}), "invalid `limit`"),
        ("logs", json!({"job_id": 1, "lines": -1}), "invalid `lines`"),
        ("logs", json!({"job_id": "1"}), "invalid `job_id`"),
        ("logs", json!({"job_id": [1]}), "invalid `job_id`"),
        ("logs", json!({}), "missing `job_id`"),
        ("bindings.remove", json!({"id": 5}), "invalid `id`"),
    ] {
        let err = client::call(&endpoint, method, params).await.unwrap_err();
        assert!(err.to_string().contains(message), "{method}: {err}");
    }
    token.cancel();
}

#[tokio::test]
async fn counts_are_honored_and_capped() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    h.bind("proj-1", "a");
    h.bind("proj-2", "b");
    h.linear.upsert(issue("1", "proj-1"));
    h.linear.upsert(issue("2", "proj-2"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let all = client::call(&endpoint, "list_jobs", json!({}))
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 2);
    let newest = client::call(&endpoint, "list_jobs", json!({"limit": 1}))
        .await
        .unwrap();
    assert_eq!(newest.as_array().unwrap().len(), 1);
    // A log of 6,000 lines: the default is 50 of them, a larger request is cut at 5,000.
    let job = h.engine.list_jobs(1).unwrap().remove(0);
    let log = job.log_path.expect("a finished run has a log");
    let text: String = (0..6_000).map(|n| format!("line {n}\n")).collect();
    std::fs::write(&log, text).unwrap();
    let tail = |params| client::call(&endpoint, "logs", params);
    let default = tail(json!({"job_id": job.id})).await.unwrap();
    assert_eq!(default.as_array().unwrap().len(), 50);
    assert_eq!(default[49], "line 5999");
    let few = tail(json!({"job_id": job.id, "lines": 3})).await.unwrap();
    assert_eq!(few, json!(["line 5997", "line 5998", "line 5999"]));
    let capped = tail(json!({"job_id": job.id, "lines": 100_000}))
        .await
        .unwrap();
    assert_eq!(capped.as_array().unwrap().len(), 5_000);
    assert_eq!(capped[0], "line 1000");
    assert_eq!(capped[4_999], "line 5999");
    token.cancel();
}

#[tokio::test]
async fn bindings_add_refuses_a_relative_directory() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let binding = json!({"id": "proj-9", "project_id": "proj-9", "project_name": "Demo Project", "directory": "repo"});
    let err = client::call(&endpoint, "bindings.add", json!({"binding": binding}))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("absolute"), "{err}");
    assert!(h.engine.bindings().is_empty());
    token.cancel();
}

// The client: what it reports when it cannot reach the host, and how long it waits.

#[tokio::test]
async fn client_reports_not_running() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = AppPaths::under(root.path()).ipc_endpoint();
    let err = client::call(&endpoint, "status", json!({}))
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<NotRunning>().is_some(), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_leftover_socket_nobody_listens_on_means_not_running() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = AppPaths::under(root.path()).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    // What a host that crashed leaves behind: a socket file whose listener is gone.
    drop(std::os::unix::net::UnixListener::bind(path).unwrap());
    let err = client::call(&endpoint, "status", json!({}))
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<NotRunning>().is_some(), "{err:#}");
}

#[tokio::test(start_paused = true)]
async fn status_gives_up_after_30_seconds_and_doctor_after_180() {
    for (method, seconds) in [("status", 30), ("doctor", 180)] {
        // Every call gets a silent server of its own. A pipe instance serves one client, and one
        // that nobody disconnects stays occupied after that client has timed out and gone, so a
        // second call on the same server would find the pipe busy instead of waiting.
        let root = tempfile::tempdir().unwrap();
        let endpoint = AppPaths::under(root.path()).ipc_endpoint();
        // Bound but never served: a client can connect and send its request, and nobody answers.
        let _silent = IpcServer::bind(&endpoint).await.unwrap();
        let started = tokio::time::Instant::now();
        let err = client::call(&endpoint, method, json!({}))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("did not answer in time"),
            "{method}: {err}"
        );
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_secs(seconds) && waited < Duration::from_secs(seconds + 1),
            "{method} waited {waited:?}"
        );
    }
}

// The endpoint: one host at a time, a private directory, leftovers cleared away.

#[tokio::test]
async fn second_server_refuses_to_start() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let err = IpcServer::bind(&endpoint)
        .await
        .err()
        .expect("second bind must fail");
    assert!(err.downcast_ref::<AlreadyRunning>().is_some(), "{err}");
    token.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn of_many_racing_hosts_exactly_one_binds() {
    const HOSTS: usize = 16;
    for round in 0..50 {
        let root = tempfile::tempdir().unwrap();
        let endpoint = AppPaths::under(root.path()).ipc_endpoint();
        #[cfg(unix)]
        {
            let IpcEndpoint::Unix(path) = &endpoint else {
                panic!("expected a unix socket")
            };
            assert!(
                path.starts_with(root.path()),
                "the temp dir is too long for this test"
            );
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            // A socket file whose host is gone: bind one, drop the listener, the file stays.
            drop(std::os::unix::net::UnixListener::bind(path).unwrap());
            assert!(path.symlink_metadata().is_ok());
        }
        let start_together = Arc::new(tokio::sync::Barrier::new(HOSTS));
        let hosts: Vec<_> = (0..HOSTS)
            .map(|_| {
                let (endpoint, start_together) = (endpoint.clone(), start_together.clone());
                tokio::spawn(async move {
                    start_together.wait().await;
                    IpcServer::bind(&endpoint).await
                })
            })
            .collect();
        // The winner is kept until every result is in: dropping it early would free the
        // endpoint for a latecomer.
        let (mut owners, mut refused) = (Vec::new(), 0);
        for host in hosts {
            match host.await.unwrap() {
                Ok(server) => owners.push(server),
                Err(e) if e.downcast_ref::<AlreadyRunning>().is_some() => refused += 1,
                Err(e) => panic!("round {round}: a host failed some other way: {e:#}"),
            }
        }
        assert_eq!(owners.len(), 1, "round {round}: owners of the endpoint");
        assert_eq!(refused, HOSTS - 1, "round {round}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn stale_socket_file_is_replaced() {
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"stale").unwrap();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    token.cancel();
}

#[cfg(unix)]
#[tokio::test]
async fn a_dangling_symlink_at_the_socket_path_is_replaced_by_a_working_socket() {
    use std::os::unix::fs::FileTypeExt;
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    let nowhere = h.root.path().join("nowhere");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&nowhere, path).unwrap();
    assert!(!path.exists() && path.symlink_metadata().is_ok());
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    assert!(
        path.symlink_metadata().unwrap().file_type().is_socket(),
        "the socket takes the link's place"
    );
    assert!(!nowhere.exists(), "nothing may be bound through the link");
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    token.cancel();
}

#[cfg(unix)]
#[tokio::test]
async fn a_live_socket_nobody_may_connect_to_is_not_taken_for_a_stale_one() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    // The host is up and serving, not merely spawned.
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    // A probe that connects to see whether the host is alive would fail here and call the
    // socket stale. The lock decides instead.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let err = IpcServer::bind(&endpoint)
        .await
        .err()
        .expect("the endpoint is owned");
    assert!(err.downcast_ref::<AlreadyRunning>().is_some(), "{err:#}");
    assert!(
        path.symlink_metadata().is_ok(),
        "the live socket was deleted"
    );
    // The client says what went wrong, and where, instead of "not running". (Permission bits
    // do not apply to root, for whom the call simply works.)
    if let Err(err) = client::call(&endpoint, "status", json!({})).await {
        assert!(err.downcast_ref::<NotRunning>().is_none(), "{err:#}");
        assert!(
            format!("{err:#}").contains(&path.display().to_string()),
            "{err:#}"
        );
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    token.cancel();
}

#[cfg(unix)]
#[tokio::test]
async fn the_lock_file_is_private_and_outlives_the_server() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    let lock = path.parent().unwrap().join("issuerelay.lock");
    let server = IpcServer::bind(&endpoint).await.unwrap();
    assert_eq!(
        std::fs::metadata(&lock).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let token = CancellationToken::new();
    let running = tokio::spawn(server.run(h.engine.clone(), token.clone()));
    token.cancel();
    running.await.unwrap().unwrap();
    // A lock file that is deleted could be created and locked anew while an old host still
    // holds the deleted one.
    assert!(lock.exists(), "the lock file must stay");
    IpcServer::bind(&endpoint)
        .await
        .expect("the lock is free once the server has ended");
}

#[cfg(unix)]
#[tokio::test]
async fn a_long_data_dir_falls_back_to_a_short_socket_that_works() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(2, false);
    let deep = h.root.path().join("a".repeat(60)).join("b".repeat(60));
    let endpoint = AppPaths::under(&deep).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    assert!(path.as_os_str().len() <= 103, "{}", path.display());
    let dir = path.parent().unwrap();
    // The fallback directory lies outside the test's own temp dir.
    let _cleanup = RemoveDirOnDrop(dir.to_path_buf());
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    assert_eq!(
        std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    token.cancel();
}

#[cfg(unix)]
#[tokio::test]
async fn an_existing_socket_directory_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _server = IpcServer::bind(&endpoint).await.unwrap();
    assert_eq!(
        std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

// The end of a server: the host disappears, and takes its connections with it.

#[tokio::test]
async fn the_host_is_gone_after_shutdown() {
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    let running = tokio::spawn(server.run(h.engine.clone(), token.clone()));
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    token.cancel();
    running.await.unwrap().unwrap();
    let err = client::call(&endpoint, "status", json!({}))
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<NotRunning>().is_some(), "{err}");
    #[cfg(unix)]
    {
        let IpcEndpoint::Unix(path) = &endpoint else {
            panic!("expected a unix socket")
        };
        assert!(!path.exists(), "the socket file must go with the host");
    }
}

#[tokio::test]
async fn connections_end_with_the_server() {
    let h = Harness::new(2, false);
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    let running = tokio::spawn(server.run(h.engine.clone(), token.clone()));
    let mut wire = Wire::connect(&endpoint).await;
    wire.send(&request_line(1, "status")).await.unwrap();
    assert!(wire.response().await.is_some(), "the connection is served");
    token.cancel();
    running.await.unwrap().unwrap();
    // `run` has returned: the connection is closed, and a request sent on it now is not served.
    let _ = wire.send(&request_line(2, "pause")).await;
    assert!(wire.response().await.is_none());
    assert!(
        !h.engine.status().paused,
        "a request after the shutdown reached the engine"
    );
}

// How the server treats a connection: lines it cannot use, lines that are too long, silence.

#[tokio::test]
async fn a_garbage_line_gets_a_parse_error_and_the_connection_stays_usable() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let mut wire = Wire::connect(&endpoint).await;
    // A blank line between the two is skipped without an answer.
    let lines = b"this is not json\n\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"status\"}\n";
    wire.send(lines).await.unwrap();
    let first = wire.response().await.expect("an answer to the garbage");
    assert_eq!(first.error.expect("a parse error").code, PARSE_ERROR);
    let second = wire.response().await.expect("an answer to the request");
    assert_eq!(second.id, 3);
    assert!(second.error.is_none());
    token.cancel();
}

#[tokio::test]
async fn invalid_utf8_gets_a_parse_error_and_the_connection_stays_usable() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let mut wire = Wire::connect(&endpoint).await;
    wire.send(b"{\"method\":\"\xff\xfe\"}\n").await.unwrap();
    let error = wire
        .response()
        .await
        .expect("an answer")
        .error
        .expect("a parse error");
    assert_eq!(error.code, PARSE_ERROR);
    assert!(error.message.contains("UTF-8"), "{}", error.message);
    wire.send(&request_line(2, "status")).await.unwrap();
    assert_eq!(wire.response().await.expect("an answer").id, 2);
    token.cancel();
}

#[tokio::test]
async fn an_over_long_line_gets_a_parse_error_and_the_connection_closes() {
    const LIMIT: usize = 1024 * 1024;
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let mut wire = Wire::connect(&endpoint).await;
    // A request of exactly the limit is still served.
    let head = r#"{"jsonrpc":"2.0","id":5,"method":"status","params":{"pad":""#;
    let tail = r#""}}"#;
    let pad = "p".repeat(LIMIT - head.len() - tail.len());
    wire.send(format!("{head}{pad}{tail}\n").as_bytes())
        .await
        .unwrap();
    let served = wire.response().await.expect("an answer");
    assert_eq!(served.id, 5);
    assert!(served.error.is_none());
    // One byte more, and no newline in sight, is not.
    wire.send(&vec![b'a'; LIMIT + 1]).await.unwrap();
    let error = wire
        .response()
        .await
        .expect("an answer")
        .error
        .expect("a parse error");
    assert_eq!(error.code, PARSE_ERROR);
    assert!(error.message.contains("too large"), "{}", error.message);
    assert!(wire.response().await.is_none(), "the connection must close");
    token.cancel();
}

/// The next response while the clock is frozen. A frozen clock jumps to the next timer whenever
/// the runtime goes idle, even with socket data on its way, so this waits by yielding: the
/// runtime never goes idle, and the server's side of the exchange still gets to run.
async fn response_without_idling(wire: &mut Wire) -> Option<Response> {
    let mut response = Box::pin(wire.response());
    loop {
        tokio::select! {
            result = &mut response => return result,
            _ = tokio::task::yield_now() => {}
        }
    }
}

#[tokio::test]
async fn a_connection_is_closed_after_a_minute_without_a_request() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    let mut wire = Wire::connect(&endpoint).await;
    wire.send(&request_line(1, "status")).await.unwrap();
    assert!(wire.response().await.is_some(), "the connection is served");
    // The clock is frozen only now: a clock that runs ahead whenever the runtime is idle would
    // cut the real socket exchange above short.
    tokio::time::pause();
    // A request after 59 seconds starts the minute afresh.
    tokio::time::advance(Duration::from_secs(59)).await;
    wire.send(&request_line(2, "status")).await.unwrap();
    let second = response_without_idling(&mut wire).await;
    assert_eq!(second.expect("a request is served").id, 2);
    // Silence from here on: open for another 59 seconds, closed once 60 have passed.
    tokio::time::advance(Duration::from_secs(59)).await;
    let early = tokio::time::timeout(Duration::from_millis(100), wire.response()).await;
    assert!(early.is_err(), "closed before a minute of silence");
    tokio::time::advance(Duration::from_secs(2)).await;
    let late = tokio::time::timeout(Duration::from_secs(1), wire.response()).await;
    assert!(
        matches!(late, Ok(None)),
        "still open after a minute of silence"
    );
    token.cancel();
}

/// A readiness probe that takes 90 seconds, as a real `doctor` can: longer than the idle timeout.
struct SlowReadiness {
    asked: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl ReadinessProbe for SlowReadiness {
    async fn executor_reports(&self, _fresh: bool) -> Vec<ExecutorReport> {
        self.asked.notify_one();
        tokio::time::sleep(Duration::from_secs(90)).await;
        Vec::new()
    }
}

#[tokio::test]
async fn a_request_slower_than_the_idle_timeout_is_answered_not_cut_off() {
    let root = tempfile::tempdir().unwrap();
    let linear = FakeLinear::new("user-1");
    let asked = Arc::new(tokio::sync::Notify::new());
    let engine = Engine::new(EngineDeps {
        config_path: root.path().join("config.toml"),
        store: Store::open_in_memory().unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(SlowReadiness {
            asked: asked.clone(),
        }),
        job_runner: Arc::new(ScriptedRunner::new(linear, false)),
        logs_dir: root.path().join("logs"),
    })
    .unwrap();
    let (endpoint, token) = serve(&engine, root.path()).await;
    let mut wire = Wire::connect(&endpoint).await;
    wire.send(&request_line(1, "doctor")).await.unwrap();
    // Real time until the server is inside the request; the clock is frozen only then.
    asked.notified().await;
    tokio::time::pause();
    // Past the idle limit the request is still being handled: the connection stays open and
    // nothing is answered yet. (Handling is not reading, so no idle timer runs meanwhile.)
    tokio::time::advance(Duration::from_secs(61)).await;
    let early = tokio::time::timeout(Duration::from_millis(100), wire.response()).await;
    assert!(early.is_err(), "the connection was cut off mid-request");
    // The probe's 90 seconds are up: the answer comes.
    tokio::time::advance(Duration::from_secs(40)).await;
    let answer = response_without_idling(&mut wire)
        .await
        .expect("the slow request is answered");
    assert_eq!(answer.id, 1);
    assert!(answer.error.is_none(), "{:?}", answer.error);
    token.cancel();
}
