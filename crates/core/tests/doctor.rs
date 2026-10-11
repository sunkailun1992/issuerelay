mod common;

use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use common::fake_linear::FakeLinear;
use issuerelay_core::{
    config::TriggerNames,
    doctor::{CmdOutput, CommandRunner, SystemRunner, check_workspace},
    linear::{LinearApi, Project, Team, Viewer},
    model::Issue,
};

#[tokio::test]
async fn workspace_with_all_names_is_ok() {
    let report = check_workspace(&FakeLinear::new("user-1"), &TriggerNames::default())
        .await
        .unwrap();
    assert!(report.ok(), "{report:?}");
}

#[tokio::test]
async fn missing_status_and_label_are_reported() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["Todo", "In Progress", "Done"]);
    let names = TriggerNames {
        label: "robot".into(),
        ..TriggerNames::default()
    };
    let report = check_workspace(&linear, &names).await.unwrap();
    assert!(!report.label_ok);
    assert_eq!(report.missing_states, ["Demo: In Testing"]);
}

#[tokio::test]
async fn names_match_ignoring_case_and_surrounding_whitespace() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["todo ", "In Progress", "In Testing"]);
    let report = check_workspace(&linear, &TriggerNames::default())
        .await
        .unwrap();
    assert!(report.label_ok);
    assert_eq!(report.missing_states, ["Demo: Done"]);

    let names = TriggerNames {
        label: "  AI-READY ".into(),
        todo: "TODO".into(),
        ..TriggerNames::default()
    };
    let report = check_workspace(&linear, &names).await.unwrap();
    assert!(report.label_ok);
    assert_eq!(report.missing_states, ["Demo: Done"]);
}

#[tokio::test]
async fn a_missing_todo_status_is_reported() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["In Progress", "In Testing", "Done"]);
    let report = check_workspace(&linear, &TriggerNames::default())
        .await
        .unwrap();
    assert!(!report.ok());
    assert_eq!(report.missing_states, ["Demo: Todo"]);
}

#[tokio::test]
async fn missing_statuses_are_listed_in_workflow_order() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&[]);
    let report = check_workspace(&linear, &TriggerNames::default())
        .await
        .unwrap();
    assert_eq!(
        report.missing_states,
        [
            "Demo: Todo",
            "Demo: In Progress",
            "Demo: In Testing",
            "Demo: Done"
        ]
    );
}

#[tokio::test]
async fn a_failing_linear_call_is_an_error() {
    // `fail_next` makes the first call, `labels`, fail.
    let linear = FakeLinear::new("user-1");
    linear.fail_next("boom");
    let err = check_workspace(&linear, &TriggerNames::default())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
}

/// A `LinearApi` with several teams whose label, team or status call can be made to fail. Only
/// what `check_workspace` calls is implemented.
struct Workspace {
    labels: Vec<&'static str>,
    /// Team id, team name and the names of its statuses.
    teams: Vec<(&'static str, &'static str, Vec<&'static str>)>,
    /// The call that fails: `labels`, `teams` or `statuses`.
    fail: Option<&'static str>,
}

impl Workspace {
    fn fail_if(&self, call: &str) -> Result<()> {
        match self.fail {
            Some(failing) if failing == call => Err(anyhow!("{call} failed")),
            _ => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl LinearApi for Workspace {
    async fn labels(&self) -> Result<Vec<String>> {
        self.fail_if("labels")?;
        Ok(self.labels.iter().map(|l| l.to_string()).collect())
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        self.fail_if("teams")?;
        Ok(self
            .teams
            .iter()
            .map(|(id, name, _)| Team {
                id: id.to_string(),
                name: name.to_string(),
            })
            .collect())
    }

    async fn statuses(&self, team_id: &str) -> Result<Vec<String>> {
        self.fail_if("statuses")?;
        let (_, _, statuses) = self
            .teams
            .iter()
            .find(|(id, _, _)| *id == team_id)
            .expect("a team this workspace listed");
        Ok(statuses.iter().map(|s| s.to_string()).collect())
    }

    async fn viewer(&self) -> Result<Viewer> {
        unimplemented!("not used by check_workspace")
    }

    async fn candidate_issues(&self, _label: &str, _state: &str) -> Result<Vec<Issue>> {
        unimplemented!("not used by check_workspace")
    }

    async fn issue(&self, _id: &str) -> Result<Option<Issue>> {
        unimplemented!("not used by check_workspace")
    }

    async fn set_state(&self, _id: &str, _state: &str) -> Result<()> {
        unimplemented!("not used by check_workspace")
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        unimplemented!("not used by check_workspace")
    }
}

fn two_teams(fail: Option<&'static str>) -> Workspace {
    Workspace {
        labels: vec!["ai-ready"],
        teams: vec![
            (
                "team-1",
                "Demo",
                vec!["Todo", "In Progress", "In Testing", "Done"],
            ),
            ("team-2", "Beta", vec!["Todo", "Done"]),
        ],
        fail,
    }
}

#[tokio::test]
async fn every_team_is_checked() {
    let report = check_workspace(&two_teams(None), &TriggerNames::default())
        .await
        .unwrap();
    assert!(report.label_ok);
    assert_eq!(
        report.missing_states,
        ["Beta: In Progress", "Beta: In Testing"]
    );
}

#[tokio::test]
async fn each_failing_linear_call_is_returned_as_the_error() {
    for call in ["labels", "teams", "statuses"] {
        let err = check_workspace(&two_teams(Some(call)), &TriggerNames::default())
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), format!("{call} failed"));
    }
}

fn agent() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_fake-agent"))
}

/// Writes a fake-agent script and returns its path as the argument the agent takes.
fn script(dir: &Path, steps: serde_json::Value) -> String {
    let path = dir.join("script.json");
    std::fs::write(&path, steps.to_string()).unwrap();
    path.to_str().unwrap().to_string()
}

#[tokio::test]
async fn system_runner_returns_the_exit_code_and_both_streams() {
    let dir = tempfile::tempdir().unwrap();
    let script = script(
        dir.path(),
        serde_json::json!([{"out": "hello"}, {"err": "oops"}, {"exit": 3}]),
    );
    let output = SystemRunner::default()
        .run(agent(), &[script.as_str()])
        .await
        .unwrap();
    // Windows pipes may carry `\r\n`.
    let output = CmdOutput {
        stdout: output.stdout.replace("\r\n", "\n"),
        stderr: output.stderr.replace("\r\n", "\n"),
        ..output
    };
    assert_eq!(
        output,
        CmdOutput {
            code: Some(3),
            stdout: "hello\n".into(),
            stderr: "oops\n".into(),
        }
    );
}

#[tokio::test]
async fn system_runner_gives_up_on_a_command_that_outlives_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let script = script(dir.path(), serde_json::json!([{"sleep_ms": 5000}]));
    let runner = SystemRunner::with_limits(Duration::from_millis(300), Duration::from_millis(300));
    let started = Instant::now();
    let err = runner.run(agent(), &[script.as_str()]).await.unwrap_err();
    assert!(err.to_string().contains("timed out after 300ms"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the call waited for the command: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_command_that_timed_out_is_killed() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("finished");
    // After 600 ms the agent would copy its (empty) stdin to `marker`, if it were still alive.
    let script = script(
        dir.path(),
        serde_json::json!([{"sleep_ms": 600}, {"stdin_to": marker}]),
    );
    let runner = SystemRunner::with_limits(Duration::from_millis(150), Duration::from_millis(150));
    runner.run(agent(), &[script.as_str()]).await.unwrap_err();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(
        !marker.exists(),
        "the timed-out command kept running and finished"
    );
}
