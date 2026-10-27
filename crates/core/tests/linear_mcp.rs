mod common;

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use chrono::{DateTime, Utc};
use common::fake_server::{
    FakeLinearServer, Reply, connect_full, connect_partial, connect_with_hanging_close,
    connect_with_hanging_tool_list, connect_without_argument, connect_without_field_value,
    issue_json, validate_arguments,
};
use issuerelay_core::{
    linear::{
        ISSUE_FIELDS, LinearApi, PROJECT_FIELDS, REQUIRED_ARGUMENTS, REQUIRED_FIELDS,
        mcp::{LinearClient, LinearError, McpLinear},
    },
    model::Issue,
};
use rmcp::model::CallToolRequestParams;

/// The issue `issue_json(n)` describes, built from its JSON one field at a time. A parsed issue
/// that equals it proves every field IssueRelay asks Linear for was asked for and read: the
/// fake leaves out what is not requested, and a field left out no longer matches.
fn expected_issue(n: u32) -> Issue {
    let json = issue_json(n);
    let text = |key: &str| json[key].as_str().unwrap().to_string();
    let time = |key: &str| text(key).parse::<DateTime<Utc>>().unwrap();
    Issue {
        uuid: text("uuid"),
        identifier: text("id"),
        title: text("title"),
        url: text("url"),
        state: text("status"),
        labels: json["labels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|label| label.as_str().unwrap().to_string())
            .collect(),
        project_id: Some(text("projectId")),
        project_name: Some(text("project")),
        assignee_id: Some(text("assigneeId")),
        parent_identifier: json["parentId"].as_str().map(str::to_string),
        priority: json["priority"]["value"].as_u64().unwrap() as u8,
        created_at: time("createdAt"),
        updated_at: time("updatedAt"),
        state_type: Some(text("statusType")),
        blocked_by: None,
    }
}

/// Every call in `calls` must have asked for every field of `ISSUE_FIELDS`. Without `fields`
/// the fake (like Linear) would answer with a default selection, and the whole-issue
/// comparisons with `expected_issue` would still pass.
fn assert_every_issue_field_requested(calls: &[serde_json::Value]) {
    assert!(!calls.is_empty(), "no call was made");
    for call in calls {
        let requested: Vec<&str> = call["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("the call sent no `fields`: {call}"))
            .iter()
            .map(|field| field.as_str().unwrap())
            .collect();
        for field in ISSUE_FIELDS {
            assert!(
                requested.contains(&field),
                "`{field}` was not requested: {requested:?}"
            );
        }
    }
}

#[test]
fn the_fake_issues_carry_a_value_for_every_requested_field() {
    // A field the fake leaves empty everywhere could be dropped from `ISSUE_FIELDS` unnoticed.
    for field in ISSUE_FIELDS {
        assert!(
            [1, 2].iter().any(|&n| !issue_json(n)[field].is_null()),
            "no fake issue has a value for `{field}`"
        );
    }
}

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
    let projects = linear.projects().await.unwrap();
    assert_eq!(projects[0].id, "proj-1");
    assert_eq!(
        projects[0].team_ids,
        ["team-1"],
        "`list_projects` was asked for `teams`"
    );
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
    assert_eq!(issues, [expected_issue(1), expected_issue(2)]);
    let calls = server.calls_to("list_issues");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["assignee"], "me");
    assert_eq!(calls[0]["label"], "ai-ready");
    assert_eq!(calls[0]["state"], "Todo");
    assert_every_issue_field_requested(&calls);
    assert!(calls[0]["cursor"].is_null());
    assert_eq!(calls[1]["cursor"], "c2");
    // Every page in creation order: under Linear's default (`updatedAt`) an issue edited during
    // the walk moves to another page and can be missed.
    for call in &calls {
        assert_eq!(call["orderBy"], "createdAt", "{call}");
    }
}

#[tokio::test]
async fn labeled_issues_reads_one_page_of_every_status() {
    let server = FakeLinearServer::default();
    let mut in_testing = issue_json(2);
    in_testing["status"] = serde_json::json!("In Testing");
    // The page says more follow: the issue list shows the newest page only.
    let page = serde_json::json!({"issues": [issue_json(1), in_testing], "hasNextPage": true, "cursor": "c2"});
    server.reply("list_issues", Reply::Text(page.to_string()));
    let linear = linear_with(&server).await;
    let issues = linear.labeled_issues("ai-ready").await.unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0], expected_issue(1));
    assert_eq!(issues[1].state, "In Testing");
    let calls = server.calls_to("list_issues");
    assert_eq!(calls.len(), 1, "one page only");
    let call = &calls[0];
    assert_eq!(call["assignee"], "me");
    assert_eq!(call["label"], "ai-ready");
    assert_eq!(call["orderBy"], "updatedAt");
    assert_eq!(call["limit"], 50);
    // The fake records an argument the caller left out as `null`.
    assert!(call["state"].is_null(), "every status");
    assert!(call["cursor"].is_null(), "one page only");
    assert_every_issue_field_requested(&calls);
}

#[tokio::test]
async fn issue_lookup_and_missing_issue() {
    let server = FakeLinearServer::default();
    let linear = McpLinear::new(connect_full(server.clone()).await)
        .await
        .unwrap();
    // The first fake issue has no parent and the second one has: both shapes are pinned.
    assert_eq!(
        linear.issue("DEMO-1").await.unwrap(),
        Some(expected_issue(1))
    );
    assert_eq!(
        linear.issue("DEMO-2").await.unwrap(),
        Some(expected_issue(2))
    );
    assert!(linear.issue("DEMO-404").await.unwrap().is_none());
    assert_every_issue_field_requested(&server.calls_to("get_issue"));
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
    linear.labeled_issues("ai-ready").await.unwrap();
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
    let requested: Vec<&str> = projects["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field.as_str().unwrap())
        .collect();
    for field in PROJECT_FIELDS {
        assert!(
            requested.contains(&field),
            "`{field}` was not requested: {requested:?}"
        );
    }
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
    // `orderBy` takes one of the values its schema lists.
    for order in ["createdAt", "updatedAt"] {
        let args = serde_json::json!({"orderBy": order});
        assert!(validate_arguments("list_issues", &args).is_ok(), "{order}");
    }
    let err =
        validate_arguments("list_issues", &serde_json::json!({"orderBy": "priority"})).unwrap_err();
    assert!(err.contains("orderBy"), "{err}");
}

/// Every read method of `LinearApi`, called once; `None` for a call that succeeded.
async fn read_errors(linear: &McpLinear) -> [(&'static str, Option<anyhow::Error>); 7] {
    [
        ("viewer", linear.viewer().await.err()),
        (
            "candidate_issues",
            linear.candidate_issues("ai-ready", "Todo").await.err(),
        ),
        (
            "labeled_issues",
            linear.labeled_issues("ai-ready").await.err(),
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
            // The argument, then every value it was to be asked for: with the argument gone
            // none of them can be.
            assert_eq!(missing[0], "list_issues.fields");
            assert!(
                missing[1..]
                    .iter()
                    .all(|entry| entry.starts_with("list_issues.fields: ")),
                "{missing:?}"
            );
            assert_eq!(missing.len(), 1 + ISSUE_FIELDS.len(), "{missing:?}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn new_rejects_a_linear_that_no_longer_offers_a_field_it_is_asked_for() {
    let err = McpLinear::new(connect_without_field_value("list_projects", "teams").await)
        .await
        .err()
        .expect("should fail");
    match err.downcast_ref::<LinearError>() {
        Some(LinearError::MissingArguments(missing)) => {
            assert_eq!(missing, &["list_projects.fields: teams"])
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let message = err.to_string();
    assert!(message.contains("list_projects.fields: teams"), "{message}");
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

#[tokio::test(start_paused = true)]
async fn closing_a_connection_that_never_finishes_closing_gives_up() {
    let linear = McpLinear::new(connect_with_hanging_close().await)
        .await
        .unwrap();
    // Paused time: the 30 s limit passes as soon as every task is waiting. The outer limit
    // makes an unbounded `close` fail this test instead of hanging it.
    let err = tokio::time::timeout(Duration::from_secs(120), linear.close())
        .await
        .expect("close waited for ever on a connection that does not close")
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("did not close the connection"),
        "{err:#}"
    );
}

/// A two-page reply for a list tool: the first call gets a page with a cursor, the second (the
/// one that carries it) the last page.
fn two_pages(key: &str, entry: serde_json::Value) -> Reply {
    Reply::Pages(vec![
        serde_json::json!({key: [entry.clone()], "hasNextPage": true, "cursor": "c2"}).to_string(),
        serde_json::json!({key: [entry], "hasNextPage": false}).to_string(),
    ])
}

#[tokio::test]
async fn the_arguments_sent_per_tool_are_exactly_the_ones_in_required_arguments() {
    // `REQUIRED_ARGUMENTS` is what connect time checks against Linear's tool list. It only
    // protects anything while it is the list of what the code really sends: a name missing from
    // it would never be checked, a name too many would turn a harmless change into a refusal.
    let server = FakeLinearServer::default();
    server.reply(
        "list_projects",
        two_pages(
            "projects",
            serde_json::json!({"id": "P-DEMO-1", "uuid": "proj-1", "name": "Demo Project"}),
        ),
    );
    server.reply(
        "list_issue_labels",
        two_pages("labels", serde_json::json!({"name": "ai-ready"})),
    );
    server.reply(
        "list_teams",
        two_pages("teams", serde_json::json!({"id": "team-1", "name": "Demo"})),
    );
    let linear = linear_with(&server).await;
    linear.viewer().await.unwrap();
    linear.candidate_issues("ai-ready", "Todo").await.unwrap();
    linear.labeled_issues("ai-ready").await.unwrap();
    linear.issue("DEMO-1").await.unwrap();
    linear.set_state("uuid-9", "Done").await.unwrap();
    linear.projects().await.unwrap();
    linear.labels().await.unwrap();
    linear.teams().await.unwrap();
    linear.statuses("team-1").await.unwrap();

    let mut sent: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (tool, arguments) in server.calls.lock().unwrap().iter() {
        let names = sent.entry(tool.clone()).or_default();
        for (name, value) in arguments.as_object().unwrap() {
            if !value.is_null() {
                names.insert(name.clone());
            }
        }
    }
    let listed: BTreeMap<String, BTreeSet<String>> = REQUIRED_ARGUMENTS
        .iter()
        .map(|(tool, arguments)| {
            (
                tool.to_string(),
                arguments.iter().map(|a| a.to_string()).collect(),
            )
        })
        .collect();
    assert_eq!(sent, listed);
    // The paged tools really were asked for a second page, or `cursor` would not show up above
    // (`list_issues` once more: `labeled_issues` reads a single page).
    for (tool, calls) in [
        ("list_issues", 3),
        ("list_projects", 2),
        ("list_issue_labels", 2),
        ("list_teams", 2),
    ] {
        assert_eq!(server.calls_to(tool).len(), calls, "{tool}");
    }
}

#[tokio::test]
async fn the_fields_sent_per_tool_are_exactly_the_ones_in_required_fields() {
    // `REQUIRED_FIELDS` is what connect time looks for in Linear's `fields` lists. Like
    // `REQUIRED_ARGUMENTS` it only protects anything while it is what the code really asks for.
    let server = FakeLinearServer::default();
    let linear = linear_with(&server).await;
    linear.candidate_issues("ai-ready", "Todo").await.unwrap();
    linear.labeled_issues("ai-ready").await.unwrap();
    linear.issue("DEMO-1").await.unwrap();
    linear.projects().await.unwrap();
    for (tool, fields) in REQUIRED_FIELDS {
        let calls = server.calls_to(tool);
        assert!(!calls.is_empty(), "{tool} was not called");
        let listed: BTreeSet<&str> = fields.iter().copied().collect();
        for call in &calls {
            let sent: BTreeSet<&str> = call["fields"]
                .as_array()
                .unwrap_or_else(|| panic!("{tool} sent no `fields`: {call}"))
                .iter()
                .map(|field| field.as_str().unwrap())
                .collect();
            assert_eq!(sent, listed, "{tool}");
        }
    }
    // And no tool is sent `fields` that connect time does not look for.
    for (tool, arguments) in server.calls.lock().unwrap().iter() {
        if !arguments["fields"].is_null() {
            assert!(
                REQUIRED_FIELDS.iter().any(|(name, _)| name == tool),
                "{tool} is sent `fields` that `REQUIRED_FIELDS` does not list"
            );
        }
    }
}

#[tokio::test]
async fn a_page_cursor_that_repeats_is_an_error_not_the_end_of_the_list() {
    // The fake answers every call with the same page, so the second request gets the cursor it
    // has just sent: Linear would be sending the walk round in a circle.
    let server = FakeLinearServer::default();
    let page = serde_json::json!({"issues": [issue_json(1)], "hasNextPage": true, "cursor": "c2"});
    server.reply("list_issues", Reply::Text(page.to_string()));
    let err = linear_with(&server)
        .await
        .candidate_issues("ai-ready", "Todo")
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("repeated"), "{err:#}");
    assert_eq!(
        server.calls_to("list_issues").len(),
        2,
        "stopped at the repeat"
    );
    // The same goes for the lists that are read whole.
    let server = FakeLinearServer::default();
    let page = serde_json::json!({"projects": [{"uuid": "proj-1", "name": "Demo Project"}], "hasNextPage": true, "cursor": "c2"});
    server.reply("list_projects", Reply::Text(page.to_string()));
    let err = linear_with(&server).await.projects().await.unwrap_err();
    assert!(format!("{err:#}").contains("repeated"), "{err:#}");
}

#[tokio::test]
async fn an_empty_page_with_a_new_cursor_does_not_end_the_walk() {
    // Later pages may still hold issues, so the empty one is not taken for the last.
    let server = FakeLinearServer::default();
    server.reply(
        "list_issues",
        Reply::Pages(vec![
            serde_json::json!({"issues": [], "hasNextPage": true, "cursor": "c2"}).to_string(),
            serde_json::json!({"issues": [issue_json(2)], "hasNextPage": true, "cursor": "c3"})
                .to_string(),
            serde_json::json!({"issues": [], "hasNextPage": false}).to_string(),
        ]),
    );
    let issues = linear_with(&server)
        .await
        .candidate_issues("ai-ready", "Todo")
        .await
        .unwrap();
    assert_eq!(issues, [expected_issue(2)]);
    let calls = server.calls_to("list_issues");
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[1]["cursor"], "c2");
    assert_eq!(calls[2]["cursor"], "c3");
}

async fn get_issue_unchecked(
    client: &LinearClient,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let arguments = arguments.as_object().unwrap().clone();
    let result = client
        .call_tool(CallToolRequestParams::new("get_issue").with_arguments(arguments))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect();
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn the_fake_returns_only_the_requested_fields() {
    // The tests above lean on the fake leaving out what a call does not ask for.
    let client = connect_full(FakeLinearServer::default()).await;
    let issue = get_issue_unchecked(
        &client,
        serde_json::json!({"id": "DEMO-1", "fields": ["id", "title"]}),
    )
    .await;
    let mut keys: Vec<&str> = issue
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["id", "title"]);
    // Like Linear, it adds `id` unasked, and a requested field without a value is `null`.
    let issue = get_issue_unchecked(
        &client,
        serde_json::json!({"id": "DEMO-1", "fields": ["title", "parentId"]}),
    )
    .await;
    assert_eq!(
        issue,
        serde_json::json!({"id": "DEMO-1", "title": "Demo task 1", "parentId": null})
    );
    // Without `fields` the whole issue comes back.
    let whole = get_issue_unchecked(&client, serde_json::json!({"id": "DEMO-1"})).await;
    assert_eq!(whole, issue_json(1));
}
