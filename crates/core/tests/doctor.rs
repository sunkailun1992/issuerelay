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
    env::ExecEnv,
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

// `claude mcp list` starts MCP servers of its own: a check that outlives its limit must take
// everything it started with it, not only the process it launched. The limit is generous because
// the check only proves something once the heartbeat child has really started writing.
#[cfg(unix)]
#[tokio::test]
async fn a_timed_out_check_ends_its_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let script = script(
        dir.path(),
        serde_json::json!([{"heartbeat": beat, "every_ms": 50}, {"sleep_ms": 30_000}]),
    );
    let runner =
        SystemRunner::with_limits(Duration::from_millis(1500), Duration::from_millis(1500));
    let error = runner.run(agent(), &[script.as_str()]).await.unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error:#}");
    // The heartbeat grandchild was in the check's process group: it must stop growing the file.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let size = std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0);
    assert!(size > 0, "the heartbeat child never ran");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0), size);
}

/// Runs `script`, trying again while the OS reports it as busy: executing a script right after
/// writing it can fail with `ETXTBSY` when another test thread forks in between.
async fn run_script(runner: &SystemRunner, script: &Path, args: &[&str]) -> Result<CmdOutput> {
    let mut retries = 0;
    loop {
        match runner.run(script, args).await {
            Err(err) if retries < 5 && is_executable_file_busy(&err) => {
                retries += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => return other,
        }
    }
}

fn is_executable_file_busy(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::ExecutableFileBusy)
}

/// A variable every process environment has. The scripts print it, so the test can see that the
/// environment reached them besides `NO_COLOR`.
#[cfg(unix)]
const INHERITED_VAR: &str = "PATH";
#[cfg(windows)]
const INHERITED_VAR: &str = "SystemRoot";

/// `script` ignores its arguments, takes about a second and then prints `ok`, the value of
/// `NO_COLOR` and the value of `INHERITED_VAR`. Under a 300 ms check limit and a 5 s `mcp list`
/// limit, `run` has to give `mcp list` the longer limit and every other command the shorter
/// one, whichever way it picks.
async fn assert_each_command_gets_its_own_limit(script: &Path) {
    let runner = SystemRunner::with_limits(Duration::from_millis(300), Duration::from_secs(5));
    let inherited = std::env::var_os(INHERITED_VAR).expect("the test process has this variable");

    let output = run_script(&runner, script, &["mcp", "list"]).await.unwrap();
    // `NO_COLOR=1` reached the process, and so did the rest of the environment.
    assert_eq!(
        output.stdout.trim_end(),
        format!("ok 1 {}", inherited.to_string_lossy()),
        "{output:?}"
    );

    let err = run_script(&runner, script, &["--version"])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out after 300ms"), "{err}");
}

#[cfg(unix)]
#[tokio::test]
async fn run_gives_mcp_list_its_own_limit_and_sets_no_color_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("check.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nsleep 1\nprintf 'ok %s %s\\n' \"$NO_COLOR\" \"$PATH\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_each_command_gets_its_own_limit(&script).await;
}

#[cfg(windows)]
#[tokio::test]
async fn run_gives_mcp_list_its_own_limit_and_sets_no_color_windows() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("check.cmd");
    // `ping -n 2` waits about a second between its two requests.
    std::fs::write(
        &script,
        "@ping -n 2 127.0.0.1 >NUL\r\n@echo ok %NO_COLOR% %SystemRoot%\r\n",
    )
    .unwrap();
    assert_each_command_gets_its_own_limit(&script).await;
}

/// `script` ignores its arguments and prints the value of `ISSUERELAY_TEST_PROXY` and its working
/// directory. A check command has to get the proxy of the shared environment, and has to run in
/// the user's home directory: `claude mcp list` answers differently per directory, and a host
/// started from Finder starts in `/`.
async fn assert_check_runs_in_the_home_directory_with_the_shared_environment(
    script: &Path,
    home: &Path,
) {
    let env = ExecEnv {
        proxy: vec![("ISSUERELAY_TEST_PROXY".into(), "http://127.0.0.1:9".into())],
        ..ExecEnv::inherit()
    }
    .shared();
    let runner = SystemRunner::with_env(env);
    let output = run_script(&runner, script, &[]).await.unwrap();
    assert_eq!(
        output.stdout.trim_end(),
        format!("http://127.0.0.1:9 {}", home.display()),
        "{output:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_check_runs_in_the_home_directory_with_the_shared_environment_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("check.sh");
    // `pwd -P` prints the physical directory, so the expected path is resolved the same way.
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s %s\\n' \"$ISSUERELAY_TEST_PROXY\" \"$(pwd -P)\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = dirs::home_dir().expect("the test user has a home directory");
    assert_check_runs_in_the_home_directory_with_the_shared_environment(
        &script,
        &home.canonicalize().unwrap(),
    )
    .await;
}

#[cfg(windows)]
#[tokio::test]
async fn a_check_runs_in_the_home_directory_with_the_shared_environment_windows() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("check.cmd");
    std::fs::write(&script, "@echo %ISSUERELAY_TEST_PROXY% %CD%\r\n").unwrap();
    let home = dirs::home_dir().expect("the test user has a home directory");
    assert_check_runs_in_the_home_directory_with_the_shared_environment(&script, &home).await;
}
