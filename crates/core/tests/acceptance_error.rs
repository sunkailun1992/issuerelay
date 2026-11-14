mod common;

use std::{path::Path, process::Command, time::Duration};

use common::{
    engine_kit::{Harness, Script},
    issue,
};
use issuerelay_core::{
    config::ExecutorKind,
    model::{JobState, RunMode},
    store::Store,
    workflow::{ControlAction, IssueContext},
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

#[tokio::test]
async fn verification_finish_read_error_does_not_leave_a_phantom_running_job() {
    let database = tempfile::tempdir().unwrap();
    let database_path = database.path().join("state.db");
    let h = Harness::with_dispatch_review_store(Store::open(&database_path).unwrap());
    let repo = h.bind("proj-1", "repo");
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Demo"]);
    git(&repo, &["config", "user.email", "demo@example.invalid"]);
    let fixture = repo.join("delivery.txt");
    std::fs::write(&fixture, "approved synthetic delivery").unwrap();
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
    h.engine
        .set_issue_context(
            "DEMO-1",
            0,
            IssueContext {
                request: "Verify the synthetic delivery".into(),
                conversation_history: Some("The fixture output must stay stable.".into()),
                scope: "Only delivery.txt".into(),
                acceptance: vec!["The synthetic fixture is readable".into()],
                ..IssueContext::default()
            },
        )
        .await
        .unwrap();
    let dispatch = h.engine.dispatch_preview("DEMO-1").await.unwrap();
    h.engine
        .approve_dispatch("DEMO-1", &dispatch.token)
        .await
        .unwrap();
    h.engine.tick().await.unwrap();
    h.settle().await;
    let source = h.latest("1");
    assert_eq!(source.state, JobState::LocalComplete);

    // Hold the verifier after it has started, then make the persisted brief unreadable.
    h.runner
        .gate
        .try_acquire_many(h.runner.gate.available_permits() as u32)
        .unwrap()
        .forget();
    h.runner.script("", Script::exit_code(0));
    let preview = h
        .engine
        .prepare_control_action(ControlAction::StartVerification {
            job_id: source.id,
            executor: ExecutorKind::Codex,
            delivery: delivery.clone(),
        })
        .await
        .unwrap();
    let verification = h
        .engine
        .commit_control_action(&preview.token)
        .await
        .unwrap()
        .job_id
        .unwrap();
    assert_eq!(h.engine.dispatch().await.unwrap(), [verification]);
    tokio::time::timeout(Duration::from_secs(5), async {
        while h.runner.started().len() != 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the synthetic verifier did not reach its gate");
    assert_eq!(
        h.engine.job(verification).unwrap().unwrap().state,
        JobState::Running
    );
    let connection = rusqlite::Connection::open(&database_path).unwrap();
    assert_eq!(
        connection
            .execute("UPDATE workflow_context SET payload = ?1", ["invalid JSON"])
            .unwrap(),
        1
    );
    h.runner.gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), h.engine.wait_idle())
        .await
        .expect("the verification kept its running slot");

    let failed = h.engine.job(verification).unwrap().unwrap();
    assert_eq!(failed.mode, RunMode::Verify);
    assert_eq!(failed.state, JobState::Failed);
    assert!(failed.started_at.is_some());
    assert!(failed.finished_at.is_some());
    assert_eq!(
        failed.reason.as_deref(),
        Some("the run ended before its result was recorded")
    );
    assert!(!h.engine.handoff(verification).unwrap().active);
    assert_eq!(h.engine.job(source.id).unwrap(), Some(source));
    assert!(h.linear.set_state_calls().is_empty());
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), delivery);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(&fixture).unwrap(),
        "approved synthetic delivery"
    );
}
