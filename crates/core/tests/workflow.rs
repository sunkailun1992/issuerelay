mod common;

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use common::{
    engine_kit::{FakeReadiness, Harness, Homes, Script},
    fake_linear::FakeLinear,
    issue,
};
use issuerelay_core::{
    config::{Binding, Config, ExecutorKind},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{Engine, EngineDeps, IssueStatus, JobRunner},
    env::ExecEnv,
    executor::CommandSpec,
    model::{Job, JobState, RunMode},
    runner::{self, RunHooks, RunLimits, RunOutput, SpawnHook},
    store::Store,
    trigger::WaitReasonView,
    workflow::{ContinuationStrategy, ExecutorSource, IssueContext},
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const SESSION: &str = "00000000-0000-4000-8000-000000000001";

fn brief(scope: &str) -> IssueContext {
    IssueContext {
        request: "Adjust the demo fixture".into(),
        scope: scope.into(),
        acceptance: vec!["Existing output stays compatible".into()],
        source_refs: vec!["docs/demo.md".into()],
        origin_executor: Some(ExecutorKind::Codex),
        ..IssueContext::default()
    }
}

async fn interrupted_implementation(h: &Harness) -> (Job, IssueContext) {
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.queue(
        "DEMO-1",
        vec![Script {
            set_state: Some("In Progress"),
            final_text: Some("Changed the demo fixture before the interruption".into()),
            session_id: Some(SESSION.into()),
            ..Script::exit_code(2)
        }],
    );
    let context = h
        .engine
        .set_issue_context(
            "DEMO-1",
            0,
            IssueContext {
                preferred_executor: Some(ExecutorKind::Claude),
                conversation_history: Some(
                    "Original decision: keep the demo output stable.".into(),
                ),
                ..brief("Only the demo fixture")
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
    assert_eq!(h.linear.get("1").state, "In Progress");
    assert_eq!(source.state, JobState::Failed);
    assert_eq!(source.executor, Some(ExecutorKind::Claude));
    assert_eq!(source.session_id.as_deref(), Some(SESSION));
    (source, context)
}

#[tokio::test]
async fn explicit_continuation_resumes_same_executor_while_linear_is_in_progress() {
    let h = Harness::with_dispatch_review();
    let (source, context) = interrupted_implementation(&h).await;
    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Claude)
        .await
        .unwrap();
    assert_eq!(preview.strategy, ContinuationStrategy::ResumeSession);
    assert_eq!(preview.context.as_ref(), Some(&context));
    assert!(
        preview
            .evidence
            .contains("Changed the demo fixture before the interruption")
    );
    h.engine.tick().await.unwrap();
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    assert_eq!(h.latest("1"), source);

    let child_id = h
        .engine
        .approve_continuation(source.id, &preview.token)
        .await
        .unwrap();
    assert_ne!(child_id, source.id);
    h.engine.tick().await.unwrap();
    h.settle().await;
    let child = h.latest("1");
    assert_eq!(child.id, child_id);
    assert_eq!(child.resume_job_id, Some(source.id));
    assert_eq!(child.state, JobState::AwaitingAcceptance);
    let specs = h.runner.specs.lock().unwrap();
    assert_eq!(specs.len(), 2);
    assert!(specs[1].args.windows(2).any(|w| w == ["--resume", SESSION]));
    assert!(specs[1].stdin.contains(&preview.evidence));
    assert!(h.linear.set_state_calls().is_empty());
}

#[tokio::test]
async fn explicit_cross_executor_continuation_gets_reviewed_history_in_the_same_worktree() {
    let h = Harness::with_dispatch_review();
    let (source, first) = interrupted_implementation(&h).await;
    let confirmed = h
        .engine
        .set_issue_context(
            "DEMO-1",
            first.revision,
            IssueContext {
                conversation_history: Some("New decision: verify the changed fixture.".into()),
                ..first
            },
        )
        .await
        .unwrap();
    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    assert_eq!(preview.strategy, ContinuationStrategy::Handoff);
    assert_eq!(preview.executor, ExecutorKind::Codex);
    assert_eq!(preview.source_executor, ExecutorKind::Claude);
    assert_eq!(preview.context.as_ref(), Some(&confirmed));
    assert!(
        preview
            .evidence
            .contains("Changed the demo fixture before the interruption")
    );
    assert!(
        !preview
            .evidence
            .contains("Original decision: keep the demo output stable.")
    );
    let child_id = h
        .engine
        .approve_continuation(source.id, &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").id, child_id);
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(
        *h.runner.kinds.lock().unwrap(),
        [ExecutorKind::Claude, ExecutorKind::Codex]
    );
    let specs = h.runner.specs.lock().unwrap();
    assert_eq!(specs[0].cwd, specs[1].cwd);
    assert!(
        !specs[1]
            .args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--resume" | "resume" | SESSION))
    );
    assert!(specs[1].stdin.contains(&preview.evidence));
    assert!(
        specs[1]
            .stdin
            .contains("New decision: verify the changed fixture.")
    );
    assert!(
        !specs[1]
            .stdin
            .contains("Original decision: keep the demo output stable.")
    );
    assert_eq!(h.engine.handoff(child_id).unwrap().context, Some(confirmed));
    assert!(h.linear.set_state_calls().is_empty());
}

#[tokio::test]
async fn new_task_context_requires_fresh_continuation_approval() {
    let h = Harness::with_dispatch_review();
    let (source, first) = interrupted_implementation(&h).await;
    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    let child_id = h
        .engine
        .approve_continuation(source.id, &preview.token)
        .await
        .unwrap();
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);

    let updated = h
        .engine
        .set_issue_context("DEMO-1", first.revision, brief("Only the updated demo"))
        .await
        .unwrap();
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").id, child_id);
    assert_eq!(h.latest("1").state, JobState::WaitingBinding);
    assert_eq!(h.runner.started(), ["DEMO-1"]);

    let renewed = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    assert_eq!(renewed.context, Some(updated));
    assert_eq!(
        h.engine
            .approve_continuation(source.id, &renewed.token)
            .await
            .unwrap(),
        child_id
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
}

#[tokio::test]
async fn explicit_continuation_rechecks_linear_revision_and_ownership() {
    let h = Harness::with_dispatch_review();
    let (source, _) = interrupted_implementation(&h).await;
    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    h.linear
        .update("1", |i| i.title = "Updated demo task".into());
    assert!(
        h.engine
            .approve_continuation(source.id, &preview.token)
            .await
            .is_err()
    );
    assert_eq!(h.latest("1"), source);

    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    let child_id = h
        .engine
        .approve_continuation(source.id, &preview.token)
        .await
        .unwrap();
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
    h.linear.update("1", |i| {
        i.title = "Clarified after continuation approval".into()
    });
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").state, JobState::WaitingBinding);
    assert_eq!(h.runner.started(), ["DEMO-1"]);

    let renewed = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    assert_eq!(
        h.engine
            .approve_continuation(source.id, &renewed.token)
            .await
            .unwrap(),
        child_id
    );
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);
    h.linear
        .update("1", |i| i.assignee_id = Some("other-user".into()));
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").id, child_id);
    assert_eq!(h.latest("1").state, JobState::Canceled);
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    assert!(h.linear.set_state_calls().is_empty());
}

#[tokio::test]
async fn explicit_native_resume_failure_does_not_start_a_fresh_session() {
    let h = Harness::with_dispatch_review();
    let (source, _) = interrupted_implementation(&h).await;
    h.runner
        .queue("DEMO-1", vec![Script::exit_code(1), Script::success()]);
    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Claude)
        .await
        .unwrap();
    assert_eq!(preview.strategy, ContinuationStrategy::ResumeSession);
    let child_id = h
        .engine
        .approve_continuation(source.id, &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    h.engine.tick().await.unwrap();
    assert_eq!(h.latest("1").id, child_id);
    assert_eq!(h.latest("1").state, JobState::Failed);
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-1"]);
    assert_eq!(h.linear.get("1").state, "In Progress");
    assert!(h.linear.set_state_calls().is_empty());
}

#[tokio::test]
async fn manual_rerun_of_failed_continuation_keeps_history_and_needs_dispatch_review() {
    let h = Harness::with_dispatch_review();
    let (source, _) = interrupted_implementation(&h).await;
    h.runner
        .queue("DEMO-1", vec![Script::exit_code(1), Script::success()]);
    let preview = h
        .engine
        .get_continuation_preview(source.id, ExecutorKind::Codex)
        .await
        .unwrap();
    let child_id = h
        .engine
        .approve_continuation(source.id, &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    let failed = h.latest("1");
    assert_eq!(failed.id, child_id);
    assert_eq!(failed.state, JobState::Failed);
    let log_path = failed.log_path.as_ref().unwrap();
    let saved_log = std::fs::read(log_path).unwrap();

    h.linear.update("1", |i| i.state = "Todo".into());
    h.engine.retry_job(failed.id).unwrap();
    h.engine.tick().await.unwrap();
    let fresh = h.latest("1");
    assert_ne!(fresh.id, failed.id);
    assert_eq!(fresh.state, JobState::WaitingBinding);
    assert_eq!(fresh.resume_job_id, None);
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-1"]);
    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;

    assert_eq!(h.latest("1").id, fresh.id);
    assert_eq!(h.latest("1").state, JobState::AwaitingAcceptance);
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-1", "DEMO-1"]);
    let prior = h
        .engine
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|job| job.id == failed.id);
    assert_eq!(std::fs::read(log_path).unwrap(), saved_log);
    assert_eq!(prior, Some(failed));
    let spec = &h.runner.specs.lock().unwrap()[2];
    assert!(
        !spec
            .args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--resume" | "resume"))
    );
    assert!(!spec.stdin.contains("recorded_runs_newest_first"));
}

#[tokio::test]
async fn dispatch_only_sends_the_exact_reviewed_context_and_keeps_it_in_the_handoff() {
    let h = Harness::with_dispatch_review();
    h.bind("proj-1", "repo");
    let mut candidate = issue("1", "proj-1");
    candidate.labels = vec!["ai-plan".into()];
    h.linear.upsert(candidate);
    h.runner.script(
        "DEMO-1",
        Script {
            set_state: None,
            ..Script::needs_info()
        },
    );
    let staged = h
        .engine
        .set_issue_context("DEMO-1", 0, IssueContext {
            conversation_history: Some("Previous decision: keep the demo output stable.\nLatest evidence: the focused check failed.".into()),
            ..brief("Only the approved demo fixture")
        })
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    let pending = h.latest("1");
    assert_eq!(pending.state, JobState::WaitingBinding);
    assert_eq!(
        h.engine.issues().await.unwrap()[0].status,
        IssueStatus::Waiting {
            reason: WaitReasonView::DispatchReviewRequired,
        }
    );
    assert!(h.runner.started().is_empty());
    assert!(h.engine.handoff(pending.id).unwrap().context.is_none());

    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    assert_eq!(preview.context.as_ref(), Some(&staged));
    assert_eq!(preview.executor, ExecutorKind::Codex);
    assert_eq!(preview.mode, RunMode::Plan);
    assert!(
        h.engine
            .approve_dispatch("DEMO-1", "wrong-token")
            .await
            .is_err()
    );
    h.engine.tick().await.unwrap();
    assert!(h.runner.started().is_empty());
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    let spec = h.runner.specs.lock().unwrap()[0].clone();
    assert!(spec.stdin.contains("This is a planning run"));
    assert!(spec.stdin.contains("Before planning, editing, or testing"));
    let injected = spec
        .stdin
        .lines()
        .find_map(|line| serde_json::from_str::<IssueContext>(line).ok());
    assert_eq!(injected.as_ref(), Some(&staged));

    let later = h
        .engine
        .set_issue_context("DEMO-1", staged.revision, brief("Unreviewed follow-up"))
        .await
        .unwrap();
    assert_eq!(h.engine.issue_context("DEMO-1").await.unwrap(), Some(later));
    let handoff = h.engine.handoff(pending.id).unwrap();
    assert!(
        handoff
            .review_prompt
            .contains("Previous decision: keep the demo output stable.")
    );
    assert_eq!(handoff.context, Some(staged));
    assert!(!handoff.prompt.contains("Unreviewed follow-up"));
    assert!(!handoff.review_prompt.contains("Unreviewed follow-up"));
}

#[tokio::test]
async fn dispatch_preview_expires_when_the_draft_linear_issue_or_destination_changes() {
    let h = Harness::with_dispatch_review();
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    let first = h
        .engine
        .set_issue_context("DEMO-1", 0, brief("First draft"))
        .await
        .unwrap();
    let old_preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    let edited = h
        .engine
        .set_issue_context("DEMO-1", first.revision, brief("Reviewed edit"))
        .await
        .unwrap();
    assert!(
        h.engine
            .approve_dispatch("DEMO-1", &old_preview.token)
            .await
            .is_err()
    );

    let old_preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    h.linear
        .update("1", |issue| issue.title = "Edited demo".into());
    assert!(
        h.engine
            .approve_dispatch("DEMO-1", &old_preview.token)
            .await
            .is_err()
    );

    let old_preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    let mut binding = h.engine.bindings().pop().unwrap();
    binding.executor = Some(ExecutorKind::Codex);
    binding.model = Some("demo-model-v2".into());
    h.engine.add_binding(binding).unwrap();
    assert!(
        h.engine
            .approve_dispatch("DEMO-1", &old_preview.token)
            .await
            .is_err()
    );
    h.engine.tick().await.unwrap();
    assert!(h.runner.started().is_empty());

    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    assert_eq!(preview.context.as_ref(), Some(&edited));
    assert_eq!(preview.model.as_deref(), Some("demo-model-v2"));
    assert_eq!(preview.title, "Edited demo");
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    assert_eq!(
        h.engine.handoff(h.latest("1").id).unwrap().context,
        Some(edited)
    );
}

#[tokio::test]
async fn dispatch_rechecks_approval_when_a_queued_issue_changes_before_start() {
    let h = Harness::with_dispatch_review();
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);

    h.linear
        .update("1", |issue| issue.title = "Edited demo".into());
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert!(h.runner.started().is_empty());
    assert_eq!(h.latest("1").state, JobState::WaitingBinding);

    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.poll_once().await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Queued);

    let mut binding = h.engine.bindings().pop().unwrap();
    binding.model = Some("demo-model-v2".into());
    h.engine.add_binding(binding).unwrap();
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert!(h.runner.started().is_empty());
    assert_eq!(h.latest("1").state, JobState::WaitingBinding);
}

#[tokio::test]
async fn linear_only_dispatch_and_manual_retry_each_require_approval() {
    let h = Harness::with_dispatch_review();
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    h.runner
        .queue("DEMO-1", vec![Script::exit_code(1), Script::success()]);
    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    assert!(preview.context.is_none());
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    let failed = h.latest("1");
    assert_eq!(failed.state, JobState::Failed);
    assert_eq!(h.runner.started(), ["DEMO-1"]);

    h.engine.retry_job(failed.id).unwrap();
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert_eq!(h.latest("1").state, JobState::WaitingBinding);
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    assert!(
        h.engine
            .approve_dispatch("DEMO-1", &preview.token)
            .await
            .is_err()
    );
    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    assert!(preview.context.is_none());
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-1"]);
    assert_eq!(h.latest("1").id, failed.id);
    assert!(h.engine.handoff(failed.id).unwrap().context.is_none());
}

#[tokio::test]
async fn claude_plan_and_codex_implementation_receive_the_same_approved_history() {
    let h = Harness::with_dispatch_review();
    h.bind("proj-1", "repo");
    let wiki_root = h.root.path().join("wiki");
    std::fs::create_dir(&wiki_root).unwrap();
    let mut settings = h.engine.settings();
    settings.wiki_root = Some(wiki_root.clone());
    h.engine.update_settings(settings).unwrap();

    let mut candidate = issue("1", "proj-1");
    let url = candidate.url.clone();
    candidate.labels = vec!["ai-plan".into()];
    h.linear.upsert(candidate);
    h.runner.script(
        "DEMO-1",
        Script {
            set_state: None,
            final_text: Some(r#"{"result":"planned","summary":"The plan is ready"}"#.into()),
            ..Script::success()
        },
    );

    let history = "The previous session verified the example.\nThe current session requires a read-only test.";
    let brief = IssueContext {
        conversation_history: Some(history.into()),
        preferred_executor: Some(ExecutorKind::Claude),
        ..brief("Only the demo fixture")
    };
    let first = h
        .engine
        .set_issue_context("DEMO-1", 0, brief.clone())
        .await
        .unwrap();
    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    assert_eq!(preview.wiki_root.as_ref(), Some(&wiki_root));
    assert_eq!(preview.executor, ExecutorKind::Claude);
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    let plan = h.latest("1");
    assert_eq!(h.engine.development_issue_url(plan.id).await.unwrap(), url);

    h.linear
        .update("1", |issue| issue.labels = vec!["ai-ready".into()]);
    h.runner.script("DEMO-1", Script::success());
    let second = h
        .engine
        .set_issue_context(
            "DEMO-1",
            first.revision,
            IssueContext {
                preferred_executor: Some(ExecutorKind::Codex),
                ..brief
            },
        )
        .await
        .unwrap();
    let preview = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    assert_eq!(preview.mode, RunMode::Implement);
    assert_eq!(preview.executor, ExecutorKind::Codex);
    h.engine
        .approve_dispatch("DEMO-1", &preview.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_eq!(
        *h.runner.kinds.lock().unwrap(),
        [ExecutorKind::Claude, ExecutorKind::Codex]
    );

    for (spec, approved) in h.runner.specs.lock().unwrap().iter().zip([&first, &second]) {
        let context = spec
            .stdin
            .lines()
            .find_map(|line| serde_json::from_str::<IssueContext>(line).ok());
        assert_eq!(context.as_ref(), Some(approved));
        assert!(
            spec.stdin
                .contains(&serde_json::to_string(&wiki_root).unwrap())
        );
    }
    assert_eq!(h.engine.handoff(plan.id).unwrap().context, Some(first));
    let implementation = h.engine.handoff(h.latest("1").id).unwrap();
    assert_eq!(implementation.context, Some(second));
    assert!(
        implementation
            .review_prompt
            .contains("The current session requires a read-only test.")
    );
}

struct CodexOnly;

#[async_trait::async_trait]
impl ReadinessProbe for CodexOnly {
    async fn executor_reports(&self, fresh: bool) -> Vec<ExecutorReport> {
        FakeReadiness
            .executor_reports(fresh)
            .await
            .into_iter()
            .filter(|report| report.kind == ExecutorKind::Codex)
            .collect()
    }
}

#[tokio::test]
async fn origin_routes_readiness_and_execution_and_run_keeps_approved_scope() {
    let h = Harness::with_parts(
        Config::default(),
        true,
        Homes::default(),
        Arc::new(CodexOnly),
    );
    assert_eq!(h.engine.settings().default_executor, ExecutorKind::Claude);
    h.bind("proj-1", "repo");
    let mut candidate = issue("1", "proj-1");
    candidate.labels.clear();
    h.linear.upsert(candidate);
    let approved = h
        .engine
        .set_issue_context("demo-1", 0, brief("Only the approved demo fixture"))
        .await
        .unwrap();
    assert!(h.engine.list_jobs(10).unwrap().is_empty());
    assert!(h.linear.get("1").labels.is_empty());

    h.linear
        .update("1", |issue| issue.labels.push("ai-ready".into()));
    h.engine.poll_once().await.unwrap();
    let queued = h.latest("1");
    assert_eq!(queued.state, JobState::Queued);
    assert_eq!(queued.executor, Some(ExecutorKind::Codex));
    assert_eq!(h.engine.dispatch().await.unwrap(), [queued.id]);

    let later = h
        .engine
        .set_issue_context(
            "DEMO-1",
            approved.revision,
            brief("Proposed follow-up scope"),
        )
        .await
        .unwrap();
    assert_eq!(h.engine.issue_context("DEMO-1").await.unwrap(), Some(later));
    assert_eq!(
        h.engine.handoff(queued.id).unwrap().context,
        Some(approved.clone())
    );

    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(*h.runner.kinds.lock().unwrap(), [ExecutorKind::Codex]);
    let spec = h.runner.specs.lock().unwrap()[0].clone();
    assert!(spec.stdin.contains("Only the approved demo fixture"));
    assert!(!spec.stdin.contains("Proposed follow-up scope"));
    let finished = h.engine.handoff(queued.id).unwrap();
    assert_eq!(finished.context, Some(approved));
    assert_eq!(
        finished.memory.unwrap().decision.unwrap().source,
        ExecutorSource::Origin
    );
    assert!(h.linear.set_state_calls().is_empty());
}

#[tokio::test]
async fn project_binding_overrides_origin_and_explicit_issue_selection_overrides_binding() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo");
    let mut binding = h.engine.bindings().pop().unwrap();
    binding.executor = Some(ExecutorKind::Claude);
    h.engine.add_binding(binding).unwrap();
    for (number, explicit, expected, source) in [
        ("1", None, ExecutorKind::Claude, ExecutorSource::Binding),
        (
            "2",
            Some(ExecutorKind::Codex),
            ExecutorKind::Codex,
            ExecutorSource::Explicit,
        ),
    ] {
        let candidate = issue(number, "proj-1");
        h.linear.upsert(candidate.clone());
        let context = IssueContext {
            preferred_executor: explicit,
            ..brief("Only the demo fixture")
        };
        h.engine
            .set_issue_context(&candidate.identifier, 0, context)
            .await
            .unwrap();
        h.engine.tick().await.unwrap();
        h.settle().await;
        let job = h.latest(number);
        assert_eq!(job.executor, Some(expected));
        let decision = h
            .engine
            .handoff(job.id)
            .unwrap()
            .memory
            .unwrap()
            .decision
            .unwrap();
        assert_eq!(decision.executor, expected);
        assert_eq!(decision.source, source);
    }
    assert_eq!(
        *h.runner.kinds.lock().unwrap(),
        [ExecutorKind::Claude, ExecutorKind::Codex]
    );
}

/// Captures the actual provider invocation, then exercises the native process runner on the
/// workspace's fake executor binary. Only the fake receives the script path.
struct NativeFakeRunner {
    scripts: Mutex<VecDeque<PathBuf>>,
    specs: Mutex<Vec<CommandSpec>>,
}

#[async_trait::async_trait]
impl JobRunner for NativeFakeRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        on_spawn: SpawnHook,
    ) -> RunOutput {
        self.run_with_output(
            spec,
            kind,
            log_path,
            limits,
            cancel,
            RunHooks {
                on_spawn: Some(on_spawn),
                on_stdout: None,
            },
        )
        .await
    }

    async fn run_with_output(
        &self,
        mut spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        hooks: RunHooks,
    ) -> RunOutput {
        self.specs.lock().unwrap().push(spec.clone());
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected executor run");
        spec.program = PathBuf::from(env!("CARGO_BIN_EXE_fake-agent"));
        spec.args = vec![script.to_str().unwrap().into()];
        spec.env.clear();
        runner::run_with_output(
            spec,
            kind,
            &log_path,
            limits,
            cancel,
            &ExecEnv::inherit(),
            hooks,
        )
        .await
    }
}

fn write_script(root: &Path, filename: &str, steps: Value) -> PathBuf {
    let path = root.join(filename);
    std::fs::write(&path, steps.to_string()).unwrap();
    path
}

fn latest(engine: &Engine) -> Job {
    engine.list_jobs(10).unwrap().into_iter().next().unwrap()
}

#[tokio::test]
async fn native_checkpoint_is_readable_while_running_and_quota_does_not_restart_resume() {
    let root = tempfile::tempdir().unwrap();
    let gate = root.path().join("release-first");
    let checkpoint = json!({"issuerelay_checkpoint":{
        "summary":"Read the demo fixture",
        "done":["Inspected fixture"],
        "next":["Run the focused test"],
        "verification":[],
        "blockers":[]
    }});
    let first = write_script(
        root.path(),
        "first.json",
        json!([
            {"out":json!({"type":"thread.started","thread_id":SESSION}).to_string()},
            {"out":json!({"type":"item.completed","item":{"type":"agent_message","text":checkpoint.to_string()}}).to_string()},
            {"wait_for":gate},
            {"exit":2}
        ]),
    );
    let quota = write_script(
        root.path(),
        "quota.json",
        json!([
            {"out":json!({"type":"turn.failed","error":{"code":"usage_limit_reached"}}).to_string()},
            {"exit":1}
        ]),
    );
    let runner = Arc::new(NativeFakeRunner {
        scripts: Mutex::new(VecDeque::from([first, quota])),
        specs: Mutex::new(Vec::new()),
    });
    let linear = FakeLinear::new("user-1");
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let mut cfg = Config::default();
    cfg.settings.default_executor = ExecutorKind::Codex;
    cfg.bindings.push(Binding {
        id: "proj-1".into(),
        project_id: "proj-1".into(),
        project_name: "Demo Project".into(),
        directory: repo,
        executor: None,
        permission: None,
        model: None,
        prompt_override: None,
        lean: false,
        effort: None,
        labels: Vec::new(),
    });
    let config_path = root.path().join("config.toml");
    cfg.save(&config_path).unwrap();
    let engine = Engine::new_for_scheduler_tests(EngineDeps {
        config_path,
        store: Store::open(&root.path().join("test.sqlite3")).unwrap(),
        linear: Arc::new(linear.clone()),
        readiness: Arc::new(FakeReadiness),
        job_runner: runner.clone(),
        logs_dir: root.path().join("logs"),
        user_home: None,
        claude_config_dir: None,
    })
    .unwrap();
    linear.upsert(issue("1", "proj-1"));
    engine.tick().await.unwrap();
    let first_id = latest(&engine).id;
    let live = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let handoff = engine.handoff(first_id).unwrap();
            if handoff
                .memory
                .as_ref()
                .is_some_and(|memory| memory.checkpoint.is_some())
            {
                break handoff;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("native stdout checkpoint was not saved");
    assert!(live.active);
    assert_eq!(
        live.memory.unwrap().checkpoint.unwrap().summary,
        "Read the demo fixture"
    );
    assert_eq!(latest(&engine).state, JobState::Running);
    std::fs::write(&gate, "continue").unwrap();
    engine.wait_idle().await;
    assert_eq!(latest(&engine).session_id.as_deref(), Some(SESSION));
    assert_eq!(latest(&engine).state, JobState::Failed);
    let memory = engine.handoff(first_id).unwrap().memory.unwrap();
    assert!(memory.checkpoint.is_some());
    assert!(memory.interruption.is_none());

    linear.update("1", |issue| issue.title = "Clarify the demo".into());
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    let resumed = latest(&engine);
    assert_eq!(resumed.resume_job_id, Some(first_id));
    assert_eq!(resumed.state, JobState::Failed);
    assert_eq!(resumed.session_id, None);
    let quota = engine
        .handoff(resumed.id)
        .unwrap()
        .memory
        .unwrap()
        .interruption
        .unwrap();
    assert_eq!(quota.code, "usage_limit_reached");
    assert_eq!(quota.reset_at, None);
    {
        let specs = runner.specs.lock().unwrap();
        assert_eq!(
            specs.len(),
            2,
            "a known quota interruption must not start afresh"
        );
        assert!(specs[1].args.iter().any(|arg| arg == "resume"));
        assert!(specs[1].args.iter().any(|arg| arg == SESSION));
    }
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    assert_eq!(runner.specs.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn manual_claim_holds_the_same_directory_until_its_owner_releases() {
    let h = Harness::new(2, true);
    h.bind("proj-1", "repo");
    h.bind("proj-2", "repo/../repo");
    h.runner.script("DEMO-1", Script::exit_code(2));
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    let first = h.latest("1").id;
    assert!(
        h.engine
            .claim_handoff(first, ExecutorKind::Codex)
            .await
            .is_err()
    );
    h.runner.gate.add_permits(1);
    h.settle().await;

    let claimed = h
        .engine
        .claim_handoff(first, ExecutorKind::Codex)
        .await
        .unwrap();
    assert_eq!(claimed.claimed_by, Some(ExecutorKind::Codex));
    assert!(!claimed.active);
    assert!(
        h.engine
            .claim_handoff(first, ExecutorKind::Codex)
            .await
            .is_err()
    );
    assert_eq!(h.runner.started(), ["DEMO-1"]);
    assert!(h.linear.set_state_calls().is_empty());

    h.linear.upsert(issue("2", "proj-2"));
    h.engine.poll_once().await.unwrap();
    let second = h.latest("2");
    assert_eq!(second.state, JobState::Queued);
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert!(
        !h.engine
            .release_handoff(first, &"0".repeat(32))
            .await
            .unwrap()
    );
    assert!(h.engine.dispatch().await.unwrap().is_empty());
    assert!(
        h.engine
            .release_handoff(first, claimed.claim_id.as_deref().unwrap())
            .await
            .unwrap()
    );
    assert_eq!(h.engine.handoff(first).unwrap().claimed_by, None);
    assert_eq!(h.engine.dispatch().await.unwrap(), [second.id]);
    h.runner.gate.add_permits(1);
    h.settle().await;
    assert_eq!(h.runner.started(), ["DEMO-1", "DEMO-2"]);
    assert!(h.linear.set_state_calls().is_empty());
}

#[tokio::test]
async fn configured_wiki_requires_a_receipt_for_an_existing_page_before_completion() {
    let wiki = tempfile::tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.settings.wiki_root = Some(wiki.path().to_path_buf());
    let h = Harness::with_config(cfg, false);
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let id = h.latest("1").id;
    let archive = h.engine.handoff(id).unwrap().archive.unwrap();
    assert!(archive.receipt.is_none());
    assert!(h.engine.mark_done(id).await.is_err());
    let page = "notes/demo.md";
    assert!(
        h.engine
            .record_archive(id, page.into(), None)
            .await
            .is_err()
    );
    assert!(h.linear.set_state_calls().is_empty());

    std::fs::create_dir(wiki.path().join("notes")).unwrap();
    std::fs::write(wiki.path().join(page), "# Demo workflow record").unwrap();
    let handoff = h
        .engine
        .record_archive(id, page.into(), None)
        .await
        .unwrap();
    let receipt = handoff.archive.unwrap().receipt.unwrap();
    assert_eq!(receipt.page, page);
    assert!(h.linear.set_state_calls().is_empty());
    std::fs::remove_file(wiki.path().join(page)).unwrap();
    assert!(
        h.engine
            .handoff(id)
            .unwrap()
            .archive
            .unwrap()
            .receipt
            .is_none()
    );
    assert!(h.engine.mark_done(id).await.is_err());
    let other_wiki = tempfile::tempdir().unwrap();
    std::fs::create_dir(other_wiki.path().join("notes")).unwrap();
    std::fs::write(other_wiki.path().join(page), "# Demo in another Wiki").unwrap();
    let mut settings = h.engine.settings();
    settings.wiki_root = Some(other_wiki.path().to_path_buf());
    h.engine.update_settings(settings).unwrap();
    assert!(h.engine.mark_done(id).await.is_err());
    assert!(h.linear.set_state_calls().is_empty());
    h.engine
        .record_archive(id, page.into(), None)
        .await
        .unwrap();
    let claim = h
        .engine
        .claim_handoff(id, ExecutorKind::Codex)
        .await
        .unwrap();
    assert!(claim.archive.unwrap().receipt.is_none());
    assert!(
        h.engine
            .record_archive(id, page.into(), None)
            .await
            .is_err()
    );
    h.engine
        .release_handoff(id, claim.claim_id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(h.engine.mark_done(id).await.is_err());
    h.engine
        .record_archive(id, page.into(), None)
        .await
        .unwrap();
    h.engine.mark_done(id).await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
    assert_eq!(
        h.linear.set_state_calls(),
        [("1".to_string(), "Done".to_string())]
    );
}

#[tokio::test]
async fn no_wiki_keeps_completion_local() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine.tick().await.unwrap();
    h.settle().await;
    let id = h.latest("1").id;
    assert!(h.engine.handoff(id).unwrap().archive.is_none());
    assert!(
        h.engine
            .record_archive(id, "notes/demo.md".into(), None)
            .await
            .is_err()
    );
    h.engine.mark_done(id).await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
}

#[tokio::test]
async fn same_linear_identifier_in_another_workspace_does_not_inherit_context() {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo");
    h.linear.upsert(issue("1", "proj-1"));
    h.engine
        .set_issue_context("DEMO-1", 0, brief("Old workspace fixture"))
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    let old = h.latest("1").id;
    let before = h.engine.workflow_run(None, Some("DEMO-1")).await.unwrap();
    assert_eq!(before["job"]["id"], old);

    let other = Arc::new(FakeLinear::new("user-1"));
    let mut same_number = issue("2", "proj-1");
    same_number.identifier = "DEMO-1".into();
    other.upsert(same_number);
    h.engine.set_linear(other);
    assert!(h.engine.issue_context("DEMO-1").await.unwrap().is_none());
    assert!(h.engine.workflow_run(None, Some("DEMO-1")).await.is_err());
    h.engine
        .set_issue_context("DEMO-1", 0, brief("New workspace fixture"))
        .await
        .unwrap();
    assert_eq!(
        h.engine.handoff(old).unwrap().context.unwrap().scope,
        "Old workspace fixture"
    );
    assert_eq!(
        h.engine.workflow_run(Some(old), None).await.unwrap()["job"]["id"],
        old
    );
    assert_eq!(
        h.engine
            .issue_context("DEMO-1")
            .await
            .unwrap()
            .unwrap()
            .scope,
        "New workspace fixture"
    );
}
