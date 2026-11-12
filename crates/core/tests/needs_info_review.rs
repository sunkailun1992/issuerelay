mod common;

use std::time::Duration;

use common::{
    engine_kit::{Harness, Script},
    issue,
};
use issuerelay_core::{
    config::ExecutorKind,
    model::JobState,
    store::Store,
    workflow::{ContinuationStrategy, IssueContext},
};

fn local_brief(history: &str) -> IssueContext {
    IssueContext {
        request: "Adjust the demo fixture".into(),
        conversation_history: Some(history.into()),
        scope:
            "Only local implementation and verification. Do not write to Linear, push, or deploy."
                .into(),
        acceptance: vec!["Existing demo output stays compatible".into()],
        ..IssueContext::default()
    }
}

#[tokio::test]
async fn local_information_request_does_not_require_a_linear_status_write() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let h = Harness::new(1, true);
        h.bind("proj-1", "repo");
        h.linear.upsert(issue("1", "proj-1"));
        h.runner.script(
            "DEMO-1",
            Script {
                set_state: None,
                ..Script::needs_info()
            },
        );
        h.engine
            .set_issue_context("DEMO-1", 0, local_brief("Keep the demo output stable."))
            .await
            .unwrap();
        let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
        h.engine
            .approve_dispatch("DEMO-1", &preview.token)
            .await
            .unwrap();
        h.engine.tick().await.unwrap();
        assert_eq!(h.latest("1").state, JobState::Running);

        // The issue can advance independently while local work is running.
        h.linear
            .update("1", |issue| issue.state = "In Progress".into());
        h.runner.gate.add_permits(1);
        h.settle().await;

        let job = h.latest("1");
        assert_eq!(job.report.as_deref(), Some("needs_info"));
        assert_eq!(job.state, JobState::NeedsInfo, "{:?}", job.reason);
        assert!(h.linear.set_state_calls().is_empty());
    })
    .await
    .expect("the local information request must finish without waiting on Linear changes");
}

#[tokio::test]
async fn reviewed_information_request_without_a_native_session_can_use_handoff() {
    for executor in [ExecutorKind::Codex, ExecutorKind::Claude] {
        tokio::time::timeout(Duration::from_secs(5), async {
            let h = Harness::with_dispatch_review();
            h.bind("proj-1", "repo");
            h.linear.upsert(issue("1", "proj-1"));
            h.runner.script("DEMO-1", Script::needs_info());
            let original = h
                .engine
                .set_issue_context("DEMO-1", 0, local_brief("Keep the demo output stable."))
                .await
                .unwrap();
            let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
            h.engine
                .approve_dispatch("DEMO-1", &preview.token)
                .await
                .unwrap();
            h.engine.tick().await.unwrap();
            h.settle().await;
            let source = h.latest("1");
            assert_eq!(source.state, JobState::NeedsInfo);
            assert_eq!(source.session_id, None);
            h.linear.update("1", |issue| issue.labels.clear());

            h.engine
                .set_issue_context(
                    "DEMO-1",
                    original.revision,
                    local_brief(
                        "Keep the demo output stable. The demo fixture is on the second page.",
                    ),
                )
                .await
                .unwrap();
            let preview = h
                .engine
                .get_continuation_preview(source.id, executor)
                .await
                .expect(
                    "new reviewed information must remain usable when the native session is absent",
                );
            assert_eq!(preview.strategy, ContinuationStrategy::Handoff);
            assert_eq!(preview.issue_uuid, source.issue_uuid);
            assert_eq!(h.runner.started(), ["DEMO-1"]);
            assert_eq!(h.latest("1").id, source.id);
            assert!(h.linear.set_state_calls().is_empty());
            h.runner.script("DEMO-1", Script::success());
            let child_id = h
                .engine
                .approve_continuation(source.id, &preview.token)
                .await
                .unwrap();
            h.engine.tick().await.unwrap();
            h.settle().await;
            let child = h.latest("1");
            assert_eq!(child.id, child_id);
            assert_eq!(child.resume_job_id, Some(source.id));
            assert_eq!(child.executor, Some(executor));
            assert_eq!(child.state, JobState::AwaitingAcceptance);
            let specs = h.runner.specs.lock().unwrap();
            assert_eq!(specs.len(), 2);
            assert!(!specs[1].args.iter().any(|arg| arg == "resume"));
            assert!(
                specs[1]
                    .stdin
                    .contains("The demo fixture is on the second page.")
            );
            assert!(h.linear.set_state_calls().is_empty());
        })
        .await
        .expect("handoff preview must be bounded and must not start another run");
    }
}

#[tokio::test]
async fn desktop_supplements_preserve_the_brief_and_require_fresh_confirmation() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let database = tempfile::tempdir().unwrap();
        let path = database.path().join("state.db");
        let h = Harness::with_dispatch_review_store(Store::open(&path).unwrap());
        h.bind("proj-1", "repo");
        h.linear.upsert(issue("1", "proj-1"));
        h.runner.script("DEMO-1", Script::needs_info());
        let original = h
            .engine
            .set_issue_context(
                "DEMO-1",
                0,
                IssueContext {
                    origin_executor: Some(ExecutorKind::Codex),
                    preferred_executor: Some(ExecutorKind::Claude),
                    source_refs: vec!["wiki/demo.md#fixture".into()],
                    ..local_brief("Keep the demo output stable.")
                },
            )
            .await
            .unwrap();
        let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
        h.engine
            .approve_dispatch("DEMO-1", &preview.token)
            .await
            .unwrap();
        h.engine.tick().await.unwrap();
        h.settle().await;
        let source = h.latest("1");
        h.linear.update("1", |issue| issue.labels.clear());

        assert_eq!(
            h.engine.continuation_context(source.id).await.unwrap(),
            Some(original.clone())
        );
        let first = h
            .engine
            .supplement_continuation_context(
                source.id,
                original.revision,
                "The demo fixture is on the second page.",
            )
            .await
            .unwrap();
        assert_eq!(first.request, original.request);
        assert_eq!(first.scope, original.scope);
        assert_eq!(first.acceptance, original.acceptance);
        assert_eq!(first.source_refs, original.source_refs);
        assert_eq!(first.origin_executor, original.origin_executor);
        assert_eq!(first.preferred_executor, original.preferred_executor);
        assert_eq!(first.revision, original.revision + 1);
        assert_eq!(
            first.conversation_history.as_deref(),
            Some("Keep the demo output stable.\n\nThe demo fixture is on the second page.")
        );
        assert!(
            h.engine
                .supplement_continuation_context(
                    source.id,
                    original.revision,
                    "A stale editor must not replace the saved supplement."
                )
                .await
                .is_err()
        );
        let first_preview = h
            .engine
            .get_continuation_preview(source.id, ExecutorKind::Codex)
            .await
            .unwrap();
        assert_eq!(first_preview.context, Some(first.clone()));

        let second = h
            .engine
            .supplement_continuation_context(
                source.id,
                first.revision,
                "Preserve the existing demo configuration.",
            )
            .await
            .unwrap();
        assert_eq!(second.revision, first.revision + 1);
        assert!(
            h.engine
                .approve_continuation(source.id, &first_preview.token)
                .await
                .is_err()
        );
        assert_eq!(
            h.engine.continuation_context(source.id).await.unwrap(),
            Some(second.clone())
        );
        assert_eq!(h.latest("1").id, source.id);
        assert_eq!(h.runner.started(), ["DEMO-1"]);

        // Staging only changes the review in memory. The original brief remains on disk.
        let connection = rusqlite::Connection::open(&path).unwrap();
        let payload: String = connection
            .query_row(
                "SELECT payload FROM workflow_context WHERE issue_key = ?1",
                ["1"],
                |row| row.get(0),
            )
            .unwrap();
        let stored: IssueContext = serde_json::from_str(&payload).unwrap();
        assert_eq!(stored, original);
        let preview = h
            .engine
            .get_continuation_preview(source.id, ExecutorKind::Codex)
            .await
            .unwrap();
        assert_eq!(preview.context, Some(second));
        assert!(h.linear.set_state_calls().is_empty());
    })
    .await
    .expect("desktop supplements must remain local and require explicit dispatch approval");
}
