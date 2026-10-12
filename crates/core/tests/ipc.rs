mod common;

use common::{engine_kit::Harness, issue};
use issuerelay_core::{
    ipc::{
        client::{self, NotRunning},
        server::{AlreadyRunning, IpcServer},
    },
    paths::{AppPaths, IpcEndpoint},
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

async fn start(h: &Harness) -> (IpcEndpoint, CancellationToken) {
    let endpoint = AppPaths::under(&h.root.path().join("app")).ipc_endpoint();
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    (endpoint, token)
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

#[tokio::test]
async fn unknown_and_panel_only_methods_are_rejected() {
    let h = Harness::new(2, false);
    let (endpoint, token) = start(&h).await;
    for method in ["mark_done", "retry_job", "cancel_job", "nope"] {
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
async fn a_long_data_dir_falls_back_to_a_short_socket_that_works() {
    use std::os::unix::fs::PermissionsExt;
    let h = Harness::new(2, false);
    let deep = h.root.path().join("a".repeat(60)).join("b".repeat(60));
    let endpoint = AppPaths::under(&deep).ipc_endpoint();
    let IpcEndpoint::Unix(path) = &endpoint else {
        panic!("expected a unix socket")
    };
    assert!(path.as_os_str().len() <= 103, "{}", path.display());
    let server = IpcServer::bind(&endpoint).await.unwrap();
    let token = CancellationToken::new();
    tokio::spawn(server.run(h.engine.clone(), token.clone()));
    assert_eq!(
        client::call(&endpoint, "status", json!({})).await.unwrap()["paused"],
        false
    );
    let dir = path.parent().unwrap();
    assert_eq!(
        std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    token.cancel();
    let _ = std::fs::remove_dir_all(dir);
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
