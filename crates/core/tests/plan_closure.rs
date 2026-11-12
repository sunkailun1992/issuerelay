//! Closing a split plan (plan-closure design): the progress of a plan issue's direct children
//! and the panel-only closure.
mod common;

use chrono::Utc;
use common::{
    engine_kit::{Harness, Script},
    issue,
};
use issuerelay_core::{
    engine::{PLAN_PROGRESS_LIMIT, PlanProgress, tally},
    model::{ChildIssue, Issue, Job, JobState},
};

fn harness() -> Harness {
    let h = Harness::new(1, false);
    h.bind("proj-1", "repo");
    h
}

/// The plan issue `DEMO-<uuid>` after its planning run posted the plan.
async fn planned(h: &Harness, uuid: &str) -> Job {
    let mut plan = issue(uuid, "proj-1");
    plan.labels = vec!["ai-plan".into()];
    h.linear.upsert(plan);
    h.runner.script(
        &format!("DEMO-{uuid}"),
        Script {
            set_state: None,
            final_text: Some(r#"{"result":"planned","summary":"The plan is ready"}"#.into()),
            ..Script::success()
        },
    );
    h.engine.tick().await.unwrap();
    h.settle().await;
    let job = h.latest(uuid);
    assert_eq!(
        (job.state, job.report.as_deref()),
        (JobState::NeedsInfo, Some("planned"))
    );
    job
}

/// A child of `DEMO-<parent>` whose status is of `kind` (`completed`, `canceled` or
/// `unstarted`). It has no trigger label, so it is never run.
fn child(uuid: &str, parent: &str, kind: &str) -> Issue {
    let mut child = issue(uuid, "proj-1");
    child.labels.clear();
    child.parent_identifier = Some(format!("DEMO-{parent}"));
    child.state = match kind {
        "completed" => "Done",
        "canceled" => "Canceled",
        _ => "Todo",
    }
    .into();
    child.state_type = Some(kind.into());
    child
}

fn kid(uuid: &str, kind: &str, archived: bool) -> ChildIssue {
    ChildIssue {
        uuid: uuid.into(),
        identifier: format!("DEMO-{uuid}"),
        state_type: Some(kind.into()),
        archived,
    }
}

fn counts(p: &PlanProgress) -> (u32, u32, u32, u32, bool) {
    (p.total, p.completed, p.canceled, p.open, p.closable)
}

fn progress_of(h: &Harness, job: &Job) -> PlanProgress {
    h.engine
        .plan_progress()
        .into_iter()
        .find(|p| p.job_id == job.id)
        .expect("no progress for the plan")
}

#[test]
fn tally_counts_completed_canceled_and_open_and_ignores_deleted_children() {
    let children = [
        kid("2", "completed", false),
        kid("3", "canceled", false),
        kid("4", "unstarted", false),
        // Archived and not ended: deleted in Linear, not counted.
        kid("5", "unstarted", true),
        // Archived after it ended: still counted.
        kid("6", "completed", true),
    ];
    let progress = tally(7, "uuid-1", &children, Utc::now());
    assert_eq!(
        (progress.job_id, progress.issue_uuid.as_str()),
        (7, "uuid-1")
    );
    assert_eq!(counts(&progress), (4, 2, 1, 1, false));
    assert_eq!(progress.error, None);
}

#[test]
fn progress_is_closable_only_with_one_completed_and_none_open() {
    let now = Utc::now();
    let done = [kid("2", "completed", false), kid("3", "completed", false)];
    assert!(tally(1, "u", &done, now).closable);
    let mixed = [kid("2", "completed", false), kid("3", "canceled", false)];
    assert!(tally(1, "u", &mixed, now).closable);
    let canceled = [kid("2", "canceled", false), kid("3", "canceled", false)];
    assert!(!tally(1, "u", &canceled, now).closable);
    assert_eq!(counts(&tally(1, "u", &[], now)), (0, 0, 0, 0, false));
}

#[tokio::test]
async fn a_poll_records_the_progress_of_an_open_plan() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.upsert(child("3", "1", "unstarted"));
    h.engine.tick().await.unwrap();
    assert_eq!(counts(&progress_of(&h, &plan)), (2, 1, 0, 1, false));

    h.linear.update("3", |issue| {
        issue.state = "Done".into();
        issue.state_type = Some("completed".into());
    });
    // What was read stands for a while; the next check after that reads the change.
    h.engine.forget_plan_progress();
    h.engine.tick().await.unwrap();
    assert_eq!(counts(&progress_of(&h, &plan)), (2, 2, 0, 0, true));
}

#[tokio::test]
async fn a_poll_reads_a_plan_again_only_once_its_progress_has_aged() {
    let h = harness();
    planned(&h, "1").await;
    h.engine.tick().await.unwrap();
    let first = h.linear.children_reads("1");
    assert!(first >= 1);
    h.engine.tick().await.unwrap();
    assert_eq!(
        h.linear.children_reads("1"),
        first,
        "read again before it aged"
    );
    h.engine.forget_plan_progress();
    h.engine.tick().await.unwrap();
    assert_eq!(h.linear.children_reads("1"), first + 1);
}

#[tokio::test]
async fn a_poll_survives_a_failed_children_read() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.fail_children("1");
    h.engine.tick().await.unwrap();
    let progress = progress_of(&h, &plan);
    assert!(progress.error.is_some(), "{progress:?}");
    assert!(!progress.closable);
}

#[tokio::test]
async fn a_poll_refreshes_at_most_the_limit_of_open_plans() {
    let h = harness();
    let plans: Vec<Job> = {
        let mut plans = Vec::new();
        for n in 1..=PLAN_PROGRESS_LIMIT + 1 {
            plans.push(planned(&h, &format!("p{n}")).await);
        }
        plans
    };
    // Every plan has aged: the check picks the most recently finished ones.
    h.engine.forget_plan_progress();
    let before: Vec<usize> = plans
        .iter()
        .map(|p| h.linear.children_reads(&p.issue_uuid))
        .collect();
    h.engine.tick().await.unwrap();
    let read: Vec<usize> = plans
        .iter()
        .zip(&before)
        .map(|(p, b)| h.linear.children_reads(&p.issue_uuid) - b)
        .collect();
    // The plan that finished first is left for its details page; the others are read once.
    assert_eq!(read[0], 0, "{read:?}");
    assert!(read[1..].iter().all(|&n| n == 1), "{read:?}");
}

#[tokio::test]
async fn refresh_plan_progress_reads_one_plan_on_demand() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    let progress = h.engine.refresh_plan_progress(plan.id).await.unwrap();
    assert_eq!(counts(&progress), (1, 1, 0, 0, true));
    assert_eq!(progress_of(&h, &plan), progress);
}

#[tokio::test]
async fn refresh_plan_progress_rejects_a_run_that_is_not_a_plan() {
    let h = harness();
    h.linear.upsert(issue("1", "proj-1"));
    h.runner.script("DEMO-1", Script::success());
    h.engine.tick().await.unwrap();
    h.settle().await;
    let run = h.latest("1");
    assert!(h.engine.refresh_plan_progress(run.id).await.is_err());
}

/// The plan run's state and Linear's writes after a refused closure: neither changed.
fn assert_untouched(h: &Harness, plan: &Job) {
    assert!(
        h.linear.set_state_calls().is_empty(),
        "{:?}",
        h.linear.set_state_calls()
    );
    assert_eq!(h.latest(&plan.issue_uuid).state, JobState::NeedsInfo);
}

#[tokio::test]
async fn completes_a_plan_when_every_child_has_ended() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.upsert(child("3", "1", "canceled"));
    h.engine.complete_plan(plan.id).await.unwrap();
    assert_eq!(
        h.linear.set_state_calls(),
        [("1".to_string(), "Done".to_string())]
    );
    assert_eq!(h.latest("1").state, JobState::Done);
}

#[tokio::test]
async fn refuses_while_a_child_is_open() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.upsert(child("3", "1", "unstarted"));
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);
}

#[tokio::test]
async fn refuses_when_every_child_was_canceled() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "canceled"));
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);
}

#[tokio::test]
async fn refuses_without_children() {
    let h = harness();
    let plan = planned(&h, "1").await;
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);
}

#[tokio::test]
async fn records_locally_when_linear_already_shows_the_plan_done() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.update("1", |issue| {
        issue.state = "Done".into();
        issue.state_type = Some("completed".into());
    });
    h.engine.complete_plan(plan.id).await.unwrap();
    assert!(h.linear.set_state_calls().is_empty());
    assert_eq!(h.latest("1").state, JobState::Done);
}

#[tokio::test]
async fn refuses_a_plan_canceled_in_linear() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.update("1", |issue| {
        issue.state = "Canceled".into();
        issue.state_type = Some("canceled".into());
    });
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);
}

#[tokio::test]
async fn refuses_when_children_cannot_be_read() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.linear.fail_children("1");
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);
}

#[tokio::test]
async fn refuses_a_run_that_is_no_longer_the_newest_for_its_issue() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    // The plan issue goes on to development: a newer run of the same issue.
    h.linear
        .update("1", |issue| issue.labels = vec!["ai-ready".into()]);
    h.runner.script("DEMO-1", Script::success());
    h.engine.tick().await.unwrap();
    h.settle().await;
    assert_ne!(h.latest("1").id, plan.id);
    let calls_before = h.linear.set_state_calls();
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_eq!(h.linear.set_state_calls(), calls_before);
}

#[tokio::test]
async fn refuses_a_claimed_run() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    h.engine
        .claim_handoff(plan.id, issuerelay_core::config::ExecutorKind::Codex)
        .await
        .unwrap();
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);
}

#[tokio::test]
async fn requires_a_wiki_receipt_when_a_wiki_is_configured() {
    let h = harness();
    let plan = planned(&h, "1").await;
    h.linear.upsert(child("2", "1", "completed"));
    let wiki = h.root.path().join("wiki");
    std::fs::create_dir(&wiki).unwrap();
    std::fs::write(wiki.join("plan.md"), "# Demo plan\n").unwrap();
    let mut settings = h.engine.settings();
    settings.wiki_root = Some(wiki);
    h.engine.update_settings(settings).unwrap();
    assert!(h.engine.complete_plan(plan.id).await.is_err());
    assert_untouched(&h, &plan);

    h.engine
        .record_archive(plan.id, "plan.md".into(), None)
        .await
        .unwrap();
    h.engine.complete_plan(plan.id).await.unwrap();
    assert_eq!(h.latest("1").state, JobState::Done);
}
