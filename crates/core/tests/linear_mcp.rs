mod common;

use common::fake_server::{FakeLinearServer, connect_full, connect_partial};
use issuerelay_core::linear::{
    LinearApi,
    mcp::{LinearError, McpLinear},
};

#[tokio::test]
async fn new_rejects_missing_tools() {
    let err = McpLinear::new(connect_partial().await)
        .await
        .err()
        .expect("should fail");
    match err.downcast_ref::<LinearError>() {
        Some(LinearError::MissingTools(missing)) => {
            assert!(missing.contains(&"get_issue".to_string()));
            assert!(!missing.contains(&"list_issues".to_string()));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn viewer_and_workspace_metadata() {
    let linear = McpLinear::new(connect_full(FakeLinearServer::default()).await)
        .await
        .unwrap();
    assert_eq!(linear.viewer().await.unwrap().id, "user-1");
    assert_eq!(linear.projects().await.unwrap()[0].id, "proj-1");
    assert_eq!(linear.labels().await.unwrap(), ["ai-ready", "Bug"]);
    let teams = linear.teams().await.unwrap();
    assert_eq!(teams[0].id, "team-1");
    assert_eq!(
        linear.statuses("team-1").await.unwrap(),
        ["Todo", "In Progress", "In Testing", "Done"]
    );
    linear.close().await.unwrap();
}

#[tokio::test]
async fn candidate_issues_follows_pagination() {
    let server = FakeLinearServer::default();
    let linear = McpLinear::new(connect_full(server.clone()).await)
        .await
        .unwrap();
    let issues = linear.candidate_issues("ai-ready", "Todo").await.unwrap();
    assert_eq!(
        issues
            .iter()
            .map(|i| i.identifier.as_str())
            .collect::<Vec<_>>(),
        ["DEMO-1", "DEMO-2"]
    );
    let calls = server.calls_to("list_issues");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["assignee"], "me");
    assert_eq!(calls[0]["label"], "ai-ready");
    assert_eq!(calls[0]["state"], "Todo");
    assert!(
        calls[0]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "uuid")
    );
    assert!(calls[0]["cursor"].is_null());
    assert_eq!(calls[1]["cursor"], "c2");
}

#[tokio::test]
async fn issue_lookup_and_missing_issue() {
    let linear = McpLinear::new(connect_full(FakeLinearServer::default()).await)
        .await
        .unwrap();
    let issue = linear.issue("DEMO-1").await.unwrap().unwrap();
    assert_eq!(issue.priority, 2);
    assert_eq!(issue.project_id.as_deref(), Some("proj-1"));
    assert_eq!(issue.labels, ["ai-ready"]);
    assert!(linear.issue("DEMO-404").await.unwrap().is_none());
}

#[tokio::test]
async fn set_state_calls_save_issue() {
    let server = FakeLinearServer::default();
    let linear = McpLinear::new(connect_full(server.clone()).await)
        .await
        .unwrap();
    linear.set_state("uuid-9", "Done").await.unwrap();
    let calls = server.calls_to("save_issue");
    assert_eq!(
        calls,
        [serde_json::json!({"id": "uuid-9", "state": "Done"})]
    );
}
