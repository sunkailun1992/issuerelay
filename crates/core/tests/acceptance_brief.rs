mod common;

use std::{path::Path, process::Command, time::Duration};

use common::{
    engine_kit::{Harness, Script},
    issue,
};
use issuerelay_core::{
    config::ExecutorKind,
    model::{Job, JobState, RunMode},
    store::Store,
    workflow::{ControlAction, ControlActionOutcome, ControlActionPreview, IssueContext},
};

fn git(directory: &Path, args: &[&str]) -> String {
    let output = Command::new(if cfg!(windows) { "git.exe" } else { "git" })
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(directory)
        .output()
        .expect("the fixture requires Git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn brief() -> IssueContext {
    IssueContext {
        request: "Verify the synthetic delivery".into(),
        conversation_history: Some("The fixture output must stay stable.".into()),
        scope: "Only delivery.txt".into(),
        acceptance: vec!["The synthetic fixture is readable".into()],
        ..IssueContext::default()
    }
}

struct Fixture {
    h: Harness,
    database: tempfile::TempDir,
    source: Job,
    delivery: String,
}

impl Fixture {
    async fn new() -> Self {
        let database = tempfile::tempdir().unwrap();
        let h = Harness::with_dispatch_review_store(
            Store::open(&database.path().join("state.db")).unwrap(),
        );
        let repo = h.bind("proj-1", "repo");
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Demo"]);
        git(&repo, &["config", "user.email", "demo@example.invalid"]);
        std::fs::write(repo.join("delivery.txt"), "approved synthetic delivery").unwrap();
        git(&repo, &["add", "delivery.txt"]);
        git(&repo, &["commit", "-qm", "Initial synthetic delivery"]);
        let delivery = git(&repo, &["rev-parse", "HEAD"]);
        h.linear.upsert(issue("1", "proj-1"));
        h.runner.script(
            "DEMO-1",
            Script {
                final_text: Some(
                    r#"{"result":"local_complete","summary":"Synthetic delivery ready"}"#.into(),
                ),
                ..Script::exit_code(0)
            },
        );
        for revision in 0..7 {
            let staged = h
                .engine
                .set_issue_context("DEMO-1", revision, brief())
                .await
                .unwrap();
            assert_eq!(staged.revision, revision + 1);
        }
        let dispatch = h.engine.dispatch_preview("DEMO-1").await.unwrap();
        h.engine
            .approve_dispatch("DEMO-1", &dispatch.token)
            .await
            .unwrap();
        h.engine.tick().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), h.settle())
            .await
            .expect("the synthetic development run did not finish");
        let source = h.latest("1");
        assert_eq!(source.state, JobState::LocalComplete);
        let fixture = Self {
            h,
            database,
            source,
            delivery,
        };
        assert_eq!(fixture.saved_context().revision, 7);
        fixture
    }

    fn connection(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.database.path().join("state.db")).unwrap()
    }

    fn saved_context(&self) -> IssueContext {
        let payload: String = self
            .connection()
            .query_row("SELECT payload FROM workflow_context", [], |row| row.get(0))
            .unwrap();
        serde_json::from_str(&payload).unwrap()
    }

    async fn stage_to_ten(&self) -> IssueContext {
        let mut staged = brief();
        for revision in 7..10 {
            staged.scope = format!("Only delivery.txt, reviewed edit {}", revision + 1);
            staged = self
                .h
                .engine
                .set_issue_context("DEMO-1", revision, staged)
                .await
                .unwrap();
        }
        assert_eq!(staged.revision, 10);
        assert_eq!(self.saved_context().revision, 7);
        staged
    }

    async fn preview(&self) -> ControlActionPreview {
        self.h
            .engine
            .prepare_control_action(ControlAction::StartVerification {
                job_id: self.source.id,
                executor: ExecutorKind::Codex,
                delivery: self.delivery.clone(),
            })
            .await
            .unwrap()
    }

    fn assert_no_dispatch_approval(&self) {
        assert_eq!(
            self.connection()
                .query_row("SELECT count(*) FROM workflow_dispatch_review", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }

    async fn assert_queued(&self, preview: &ControlActionPreview, reviewed: &IssueContext) {
        let result = self
            .h
            .engine
            .commit_control_action(&preview.token)
            .await
            .unwrap();
        assert_eq!(result.result, ControlActionOutcome::VerificationQueued);
        let verification = self.h.engine.job(result.job_id.unwrap()).unwrap().unwrap();
        assert_eq!(verification.mode, RunMode::Verify);
        assert_eq!(verification.state, JobState::Queued);
        assert_eq!(self.saved_context(), *reviewed);
        let approval: String = self
            .connection()
            .query_row("SELECT approval FROM acceptance_runs", [], |row| row.get(0))
            .unwrap();
        let approval: serde_json::Value = serde_json::from_str(&approval).unwrap();
        assert_eq!(approval["context"], serde_json::to_value(reviewed).unwrap());
        assert_eq!(
            self.h.engine.issue_context("DEMO-1").await.unwrap(),
            Some(reviewed.clone())
        );
        assert_eq!(
            self.h.engine.job(self.source.id).unwrap(),
            Some(self.source.clone())
        );
        assert!(
            self.h
                .engine
                .commit_control_action(&preview.token)
                .await
                .is_err()
        );
        assert_eq!(self.h.engine.list_jobs(10).unwrap().len(), 2);
        assert_eq!(self.h.runner.started().len(), 1);
        assert!(self.h.linear.set_state_calls().is_empty());
        self.assert_no_dispatch_approval();
    }
}

#[tokio::test]
async fn verification_preserves_the_reviewed_revision_after_multiple_staged_edits() {
    let fixture = Fixture::new().await;
    let reviewed = fixture.stage_to_ten().await;
    let preview = fixture.preview().await;
    let acceptance = preview.acceptance.as_ref().unwrap();
    assert_eq!(acceptance.brief_revision, Some(10));
    assert_eq!(acceptance.context.as_ref(), Some(&reviewed));
    assert_eq!(fixture.h.engine.list_jobs(10).unwrap().len(), 1);
    let other_preview = fixture.preview().await;
    fixture.assert_queued(&preview, &reviewed).await;
    assert!(
        fixture
            .h
            .engine
            .commit_control_action(&other_preview.token)
            .await
            .is_err()
    );
    assert_eq!(fixture.h.engine.list_jobs(10).unwrap().len(), 2);
}

#[tokio::test]
async fn verification_rolls_back_the_saved_brief_and_job_but_keeps_the_draft_on_failure() {
    let fixture = Fixture::new().await;
    let reviewed = fixture.stage_to_ten().await;
    let preview = fixture.preview().await;
    fixture
        .connection()
        .execute_batch(
            "CREATE TRIGGER fail_verification BEFORE INSERT ON acceptance_runs
             BEGIN SELECT RAISE(ABORT, 'simulated verification failure'); END;",
        )
        .unwrap();
    let error = fixture
        .h
        .engine
        .commit_control_action(&preview.token)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("simulated verification failure"));
    assert_eq!(fixture.saved_context().revision, 7);
    assert_eq!(fixture.h.engine.list_jobs(10).unwrap().len(), 1);
    assert_eq!(
        fixture.h.engine.issue_context("DEMO-1").await.unwrap(),
        Some(reviewed.clone())
    );
    assert!(
        fixture
            .h
            .engine
            .commit_control_action(&preview.token)
            .await
            .is_err()
    );
    fixture.assert_no_dispatch_approval();
    fixture
        .connection()
        .execute_batch("DROP TRIGGER fail_verification")
        .unwrap();
    fixture
        .assert_queued(&fixture.preview().await, &reviewed)
        .await;
}

#[tokio::test]
async fn verification_rejects_a_changed_draft_and_keeps_the_new_revision_for_review() {
    let fixture = Fixture::new().await;
    let mut reviewed = fixture.stage_to_ten().await;
    let stale = fixture.preview().await;
    reviewed
        .acceptance
        .push("Check the clarified output".into());
    let reviewed = fixture
        .h
        .engine
        .set_issue_context("DEMO-1", 10, reviewed)
        .await
        .unwrap();
    assert!(
        fixture
            .h
            .engine
            .commit_control_action(&stale.token)
            .await
            .is_err()
    );
    assert_eq!(fixture.saved_context().revision, 7);
    assert_eq!(fixture.h.engine.list_jobs(10).unwrap().len(), 1);
    assert_eq!(
        fixture.h.engine.issue_context("DEMO-1").await.unwrap(),
        Some(reviewed.clone())
    );
    fixture
        .assert_queued(&fixture.preview().await, &reviewed)
        .await;
}

#[tokio::test]
async fn verification_rejects_changed_linear_input_and_delivery_without_saving_the_draft() {
    let fixture = Fixture::new().await;
    let reviewed = fixture.stage_to_ten().await;
    let stale = fixture.preview().await;
    fixture.h.linear.update("1", |issue| {
        issue.title = "Clarified synthetic delivery".into()
    });
    assert!(
        fixture
            .h
            .engine
            .commit_control_action(&stale.token)
            .await
            .is_err()
    );
    let stale = fixture.preview().await;
    let repo = fixture.source.directory.as_ref().unwrap();
    std::fs::write(repo.join("delivery.txt"), "changed synthetic delivery").unwrap();
    git(repo, &["commit", "-qam", "Changed synthetic delivery"]);
    assert!(
        fixture
            .h
            .engine
            .commit_control_action(&stale.token)
            .await
            .is_err()
    );
    assert_eq!(fixture.saved_context().revision, 7);
    assert_eq!(fixture.h.engine.list_jobs(10).unwrap().len(), 1);
    assert_eq!(
        fixture.h.engine.issue_context("DEMO-1").await.unwrap(),
        Some(reviewed.clone())
    );
    fixture.assert_no_dispatch_approval();
    git(repo, &["checkout", "--detach", &fixture.delivery]);
    fixture
        .assert_queued(&fixture.preview().await, &reviewed)
        .await;
}
