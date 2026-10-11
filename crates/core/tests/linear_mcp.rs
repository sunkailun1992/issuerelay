mod common;

use std::time::Duration;

use common::fake_server::{
    FakeLinearServer, Reply, connect_full, connect_partial, connect_with_hanging_tool_list,
    connect_without_argument, issue_json, validate_arguments,
};
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

async fn linear_with(server: &FakeLinearServer) -> McpLinear {
    McpLinear::new(connect_full(server.clone()).await)
        .await
        .unwrap()
}

fn without(mut issue: serde_json::Value, key: &str) -> serde_json::Value {
    issue.as_object_mut().unwrap().remove(key);
    issue
}

#[tokio::test]
async fn an_unreadable_issue_reply_is_an_error_not_a_missing_issue() {
    let mut bad_time = issue_json(1);
    bad_time["updatedAt"] = serde_json::json!("2026-10-01 09:00:00");
    for reply in [
        "{}".to_string(),
        "null".to_string(),
        r#"{"issue":null}"#.to_string(),
        without(issue_json(1), "uuid").to_string(),
        without(issue_json(1), "status").to_string(),
        bad_time.to_string(),
    ] {
        let server = FakeLinearServer::default();
        server.reply("get_issue", Reply::Text(reply.clone()));
        assert!(
            linear_with(&server).await.issue("DEMO-1").await.is_err(),
            "{reply}"
        );
    }
}

#[tokio::test]
async fn other_tool_errors_are_errors() {
    for message in ["Rate limit exceeded", "Team not found"] {
        let server = FakeLinearServer::default();
        server.reply("get_issue", Reply::ToolError(message.into()));
        let err = linear_with(&server)
            .await
            .issue("DEMO-1")
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains(message), "{err:#}");
    }
}

#[tokio::test]
async fn a_failed_status_change_is_an_error() {
    let server = FakeLinearServer::default();
    server.reply("save_issue", Reply::ToolError("Status not found".into()));
    assert!(
        linear_with(&server)
            .await
            .set_state("uuid-9", "Done")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_unreadable_issue_fails_the_whole_poll() {
    let server = FakeLinearServer::default();
    let page = serde_json::json!({"issues": [without(issue_json(1), "uuid")], "hasNextPage": true, "cursor": "c2"});
    server.reply("list_issues", Reply::Text(page.to_string()));
    let err = linear_with(&server)
        .await
        .candidate_issues("ai-ready", "Todo")
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("item 0"), "{err:#}");
}

#[tokio::test]
async fn more_pages_without_a_cursor_is_an_error() {
    let server = FakeLinearServer::default();
    let page = serde_json::json!({"issues": [issue_json(1)], "hasNextPage": true});
    server.reply("list_issues", Reply::Text(page.to_string()));
    assert!(
        linear_with(&server)
            .await
            .candidate_issues("ai-ready", "Todo")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_call_that_never_answers_times_out() {
    let server = FakeLinearServer::default();
    server.reply("get_issue", Reply::Hang);
    let linear = linear_with(&server)
        .await
        .with_call_timeout(Duration::from_millis(200));
    let err = tokio::time::timeout(Duration::from_secs(5), linear.issue("DEMO-1"))
        .await
        .expect("the call timeout did not fire")
        .unwrap_err();
    assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
}

#[tokio::test]
async fn every_call_sends_arguments_linear_accepts() {
    // The fake checks every call against Linear's own schemas, so each call below fails on a
    // rejected argument; the assertions pin the values that matter.
    let server = FakeLinearServer::default();
    let linear = linear_with(&server).await;
    linear.viewer().await.unwrap();
    linear.candidate_issues("ai-ready", "Todo").await.unwrap();
    linear.issue("DEMO-1").await.unwrap();
    linear.set_state("uuid-9", "In Testing").await.unwrap();
    linear.projects().await.unwrap();
    linear.labels().await.unwrap();
    linear.teams().await.unwrap();
    linear.statuses("team-1").await.unwrap();
    assert_eq!(server.calls_to("get_user")[0]["query"], "me");
    assert_eq!(server.calls_to("list_issue_statuses")[0]["team"], "team-1");
    let projects = &server.calls_to("list_projects")[0];
    assert!(projects["limit"].as_u64().unwrap() <= 50);
    assert!(
        projects["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "uuid")
    );
}

#[test]
fn the_fake_rejects_what_linear_rejects() {
    assert!(validate_arguments("list_projects", &serde_json::json!({"limit": 50})).is_ok());
    assert!(validate_arguments("list_projects", &serde_json::json!({"limit": 100})).is_err());
    assert!(validate_arguments("list_teams", &serde_json::json!({"fields": ["id"]})).is_err());
    assert!(validate_arguments("get_issue", &serde_json::json!({"fields": ["id"]})).is_err());
    assert!(
        validate_arguments("list_issues", &serde_json::json!({"fields": ["notAField"]})).is_err()
    );
}

/// Every read method of `LinearApi`, called once; `None` for a call that succeeded.
async fn read_errors(linear: &McpLinear) -> [(&'static str, Option<anyhow::Error>); 6] {
    [
        ("viewer", linear.viewer().await.err()),
        (
            "candidate_issues",
            linear.candidate_issues("ai-ready", "Todo").await.err(),
        ),
        ("projects", linear.projects().await.err()),
        ("labels", linear.labels().await.err()),
        ("teams", linear.teams().await.err()),
        ("statuses", linear.statuses("team-1").await.err()),
    ]
}

const READ_TOOLS: [&str; 6] = [
    "get_user",
    "list_issues",
    "list_projects",
    "list_issue_labels",
    "list_teams",
    "list_issue_statuses",
];

#[tokio::test]
async fn a_tool_error_on_any_read_is_an_error() {
    let server = FakeLinearServer::default();
    for tool in READ_TOOLS {
        server.reply(tool, Reply::ToolError("Rate limit exceeded".into()));
    }
    for (method, err) in read_errors(&linear_with(&server).await).await {
        let err = err.unwrap_or_else(|| panic!("{method} swallowed the tool error"));
        assert!(
            format!("{err:#}").contains("Rate limit exceeded"),
            "{method}: {err:#}"
        );
    }
}

#[tokio::test]
async fn a_reply_in_the_wrong_shape_is_an_error_on_any_read() {
    // (A bare `[]` is not garbage: a top-level array is a valid list, which is how
    // `list_issue_statuses` answers, so it reads as an empty list.)
    for garbage in [
        "Internal error",
        r#"{"unexpected":true}"#,
        "null",
        "42",
        r#""text""#,
    ] {
        let server = FakeLinearServer::default();
        for tool in READ_TOOLS {
            server.reply(tool, Reply::Text(garbage.into()));
        }
        for (method, err) in read_errors(&linear_with(&server).await).await {
            assert!(err.is_some(), "{method} accepted the reply {garbage:?}");
        }
    }
}

#[tokio::test]
async fn a_poll_that_never_reaches_the_last_page_is_an_error() {
    let server = FakeLinearServer::default();
    server.endless_issue_pages();
    let err = linear_with(&server)
        .await
        .candidate_issues("ai-ready", "Todo")
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("more than"), "{err:#}");
    // `MAX_PAGES` in `mcp.rs`: the walk stops after exactly this many requests.
    assert_eq!(server.calls_to("list_issues").len(), 20);
}

#[tokio::test]
async fn a_timed_out_call_leaves_the_connection_usable() {
    let server = FakeLinearServer::default();
    server.reply("get_issue", Reply::Hang);
    let linear = linear_with(&server)
        .await
        .with_call_timeout(Duration::from_millis(200));
    // The outer limit turns a call limit that does not work into a failure instead of a hang.
    let first = tokio::time::timeout(Duration::from_secs(5), linear.issue("DEMO-1"))
        .await
        .expect("the call timeout did not fire");
    assert!(first.is_err());
    // A generous limit for the call that must work, so a slow machine cannot fail it.
    let linear = linear.with_call_timeout(Duration::from_secs(30));
    assert_eq!(linear.viewer().await.unwrap().id, "user-1");
}

#[tokio::test]
async fn new_rejects_a_linear_that_no_longer_accepts_an_argument() {
    let err = McpLinear::new(connect_without_argument("list_issues", "fields").await)
        .await
        .err()
        .expect("should fail");
    match err.downcast_ref::<LinearError>() {
        Some(LinearError::MissingArguments(missing)) => {
            assert_eq!(missing, &["list_issues.fields"])
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn connecting_to_a_linear_that_never_lists_its_tools_times_out() {
    // Paused time: the 30 s call limit passes as soon as every task is waiting. The outer limit
    // makes a missing call limit fail this test instead of hanging it.
    let connecting = McpLinear::new(connect_with_hanging_tool_list().await);
    let err = tokio::time::timeout(Duration::from_secs(120), connecting)
        .await
        .expect("connecting never gave up on a Linear that does not answer")
        .err()
        .expect("should fail");
    assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
}
