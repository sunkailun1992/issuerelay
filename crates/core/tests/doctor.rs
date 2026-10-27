mod common;

use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use common::fake_linear::FakeLinear;
use issuerelay_core::{
    config::{Binding, TriggerNames},
    doctor::{CmdOutput, CommandRunner, SystemRunner, check_workspace},
    env::ExecEnv,
    linear::{LinearApi, Project, Team, Viewer},
    model::Issue,
};

#[tokio::test]
async fn workspace_with_all_names_is_ok() {
    let report = check_workspace(&FakeLinear::new("user-1"), &TriggerNames::default(), &[])
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
    let report = check_workspace(&linear, &names, &[]).await.unwrap();
    assert!(!report.label_ok);
    assert_eq!(report.missing_states, ["Demo: In Testing"]);
}

#[tokio::test]
async fn names_match_ignoring_case_and_surrounding_whitespace() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["todo ", "In Progress", "In Testing"]);
    let report = check_workspace(&linear, &TriggerNames::default(), &[])
        .await
        .unwrap();
    assert!(report.label_ok);
    assert_eq!(report.missing_states, ["Demo: Done"]);

    let names = TriggerNames {
        label: "  AI-READY ".into(),
        todo: "TODO".into(),
        ..TriggerNames::default()
    };
    let report = check_workspace(&linear, &names, &[]).await.unwrap();
    assert!(report.label_ok);
    assert_eq!(report.missing_states, ["Demo: Done"]);
}

#[tokio::test]
async fn a_missing_todo_status_is_reported() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["In Progress", "In Testing", "Done"]);
    let report = check_workspace(&linear, &TriggerNames::default(), &[])
        .await
        .unwrap();
    assert!(!report.ok());
    assert_eq!(report.missing_states, ["Demo: Todo"]);
}

#[tokio::test]
async fn missing_statuses_are_listed_in_workflow_order() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&[]);
    let report = check_workspace(&linear, &TriggerNames::default(), &[])
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
    let err = check_workspace(&linear, &TriggerNames::default(), &[])
        .await
        .unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
}

/// A `LinearApi` with several teams whose label, team, project or status call can be made to
/// fail. Only what `check_workspace` calls is implemented.
struct Workspace {
    labels: Vec<&'static str>,
    /// Team id, team name and the names of its statuses.
    teams: Vec<(&'static str, &'static str, Vec<&'static str>)>,
    /// Project id and the ids of the teams it belongs to.
    projects: Vec<(&'static str, Vec<&'static str>)>,
    /// The call that fails: `labels`, `teams`, `projects` or `statuses`.
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

    async fn labeled_issues(&self, _label: &str) -> Result<Vec<Issue>> {
        unimplemented!("not used by check_workspace")
    }

    async fn issue(&self, _id: &str) -> Result<Option<Issue>> {
        unimplemented!("not used by check_workspace")
    }

    async fn set_state(&self, _id: &str, _state: &str) -> Result<()> {
        unimplemented!("not used by check_workspace")
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        self.fail_if("projects")?;
        Ok(self
            .projects
            .iter()
            .map(|(id, teams)| Project {
                id: id.to_string(),
                name: format!("Project {id}"),
                team_ids: teams.iter().map(|team| team.to_string()).collect(),
            })
            .collect())
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
        projects: vec![("proj-1", vec!["team-1"]), ("proj-2", vec!["team-2"])],
        fail,
    }
}

/// A binding of the Linear project `project_id`; the workspace check looks at nothing else of it.
fn binding_to(project_id: &str) -> Binding {
    Binding {
        id: project_id.into(),
        project_id: project_id.into(),
        project_name: "Demo Project".into(),
        directory: std::env::temp_dir(),
        executor: None,
        permission: None,
        model: None,
        prompt_override: None,
        lean: true,
        effort: None,
        labels: Vec::new(),
    }
}

#[tokio::test]
async fn every_team_is_checked() {
    let report = check_workspace(&two_teams(None), &TriggerNames::default(), &[])
        .await
        .unwrap();
    assert!(report.label_ok);
    assert_eq!(
        report.missing_states,
        ["Beta: In Progress", "Beta: In Testing"]
    );
}

#[tokio::test]
async fn only_the_teams_of_bound_projects_are_checked() {
    let linear = FakeLinear::new("user-1");
    linear.set_teams(&[("team-1", "Demo"), ("team-2", "Unrelated")]);
    linear.set_projects(&[("proj-1", "Demo Project", &["team-1"])]);
    linear.set_team_statuses("team-1", &["Todo", "In Progress", "In Testing", "Done"]);
    linear.set_team_statuses("team-2", &["Backlog"]);
    let binding = binding_to("proj-1");
    let report = check_workspace(&linear, &TriggerNames::default(), &[binding])
        .await
        .unwrap();
    assert!(
        report.missing_states.is_empty(),
        "{:?}",
        report.missing_states
    );
    assert_eq!(report.teams_checked, ["Demo"]);
    let all = check_workspace(&linear, &TriggerNames::default(), &[])
        .await
        .unwrap();
    assert_eq!(
        all.teams_checked,
        ["Demo", "Unrelated"],
        "before any binding every team is checked"
    );
    assert!(
        all.missing_states
            .iter()
            .any(|m| m.starts_with("Unrelated: "))
    );
}

#[tokio::test]
async fn a_bound_project_decides_which_team_is_looked_at() {
    let names = TriggerNames::default();
    // Demo has every status, Beta lacks two: binding one project or the other tells them apart.
    let demo = check_workspace(&two_teams(None), &names, &[binding_to("proj-1")])
        .await
        .unwrap();
    assert_eq!(demo.teams_checked, ["Demo"]);
    assert!(demo.ok(), "{demo:?}");
    let beta = check_workspace(&two_teams(None), &names, &[binding_to("proj-2")])
        .await
        .unwrap();
    assert_eq!(beta.teams_checked, ["Beta"]);
    assert_eq!(
        beta.missing_states,
        ["Beta: In Progress", "Beta: In Testing"]
    );
    // Both bound: both teams, in the order Linear lists them rather than the order of binding.
    let both = check_workspace(
        &two_teams(None),
        &names,
        &[binding_to("proj-2"), binding_to("proj-1")],
    )
    .await
    .unwrap();
    assert_eq!(both.teams_checked, ["Demo", "Beta"]);
    assert_eq!(both.missing_states, beta.missing_states);
    // The label does not depend on the teams.
    assert!(both.label_ok && beta.label_ok);
}

#[tokio::test]
async fn a_team_shared_by_projects_is_checked_once_and_a_project_of_two_teams_checks_both() {
    let mut workspace = two_teams(None);
    workspace.projects = vec![
        ("proj-1", vec!["team-1"]),
        ("proj-2", vec!["team-1"]),
        ("proj-3", vec!["team-1", "team-2"]),
        ("proj-4", vec![]),
    ];
    let names = TriggerNames::default();
    let one_team = [binding_to("proj-1"), binding_to("proj-2")];
    let report = check_workspace(&workspace, &names, &one_team)
        .await
        .unwrap();
    assert_eq!(report.teams_checked, ["Demo"]);
    let both = check_workspace(&workspace, &names, &[binding_to("proj-3")])
        .await
        .unwrap();
    assert_eq!(both.teams_checked, ["Demo", "Beta"]);
    assert_eq!(
        both.missing_states,
        ["Beta: In Progress", "Beta: In Testing"]
    );
    // A project that names no team, or one Linear no longer lists, adds nothing to the teams the
    // others name.
    let mixed = [
        binding_to("proj-1"),
        binding_to("proj-4"),
        binding_to("proj-gone"),
    ];
    let report = check_workspace(&workspace, &names, &mixed).await.unwrap();
    assert_eq!(report.teams_checked, ["Demo"]);
    assert!(report.ok(), "{report:?}");
}

#[tokio::test]
async fn bound_projects_that_name_no_team_leave_every_team_checked() {
    // Linear sent `teams` as null, left it out or sent []: nothing narrows the check down, and
    // checking no team would report a workspace with missing statuses as fine.
    let mut workspace = two_teams(None);
    workspace.projects = vec![("proj-1", vec![]), ("proj-2", vec![])];
    let names = TriggerNames::default();
    for bound in [
        vec![binding_to("proj-1")],
        vec![binding_to("proj-1"), binding_to("proj-2")],
    ] {
        let report = check_workspace(&workspace, &names, &bound).await.unwrap();
        assert_eq!(report.teams_checked, ["Demo", "Beta"]);
        assert_eq!(
            report.missing_states,
            ["Beta: In Progress", "Beta: In Testing"]
        );
        assert!(!report.ok(), "{report:?}");
    }
    // A single team that is named brings the narrowing back, even next to a project without one.
    workspace.projects = vec![("proj-1", vec![]), ("proj-2", vec!["team-2"])];
    let both = [binding_to("proj-1"), binding_to("proj-2")];
    let report = check_workspace(&workspace, &names, &both).await.unwrap();
    assert_eq!(report.teams_checked, ["Beta"]);
}

#[tokio::test]
async fn a_bound_project_whose_teams_linear_no_longer_lists_leaves_every_team_checked() {
    // Linear's team list leaves archived teams out, while a project can still name one. Narrowing
    // the check to teams that are not listed would check no team at all and pass a workspace with
    // missing statuses.
    let mut workspace = two_teams(None);
    workspace.projects = vec![
        ("proj-1", vec!["team-archived"]),
        ("proj-2", vec!["team-2"]),
    ];
    let names = TriggerNames::default();
    let report = check_workspace(&workspace, &names, &[binding_to("proj-1")])
        .await
        .unwrap();
    assert_eq!(report.teams_checked, ["Demo", "Beta"]);
    assert_eq!(
        report.missing_states,
        ["Beta: In Progress", "Beta: In Testing"]
    );
    assert!(!report.ok(), "{report:?}");
    // Next to a team that is listed, that team is checked alone.
    let both = [binding_to("proj-1"), binding_to("proj-2")];
    let report = check_workspace(&workspace, &names, &both).await.unwrap();
    assert_eq!(report.teams_checked, ["Beta"]);
}

#[tokio::test]
async fn bound_projects_that_linear_no_longer_lists_or_lists_without_a_team_leave_every_team_checked()
 {
    let names = TriggerNames::default();
    let gone = [binding_to("proj-gone"), binding_to("proj-also-gone")];
    let report = check_workspace(&two_teams(None), &names, &gone)
        .await
        .unwrap();
    assert_eq!(report.teams_checked, ["Demo", "Beta"]);
    assert_eq!(
        report.missing_states,
        ["Beta: In Progress", "Beta: In Testing"]
    );
    // Through the in-memory Linear as well, where the project is listed with no team.
    let linear = FakeLinear::new("user-1");
    linear.set_teams(&[("team-1", "Demo"), ("team-2", "Unrelated")]);
    linear.set_projects(&[("proj-1", "Demo Project", &[])]);
    linear.set_team_statuses("team-2", &["Backlog"]);
    let report = check_workspace(&linear, &names, &[binding_to("proj-1")])
        .await
        .unwrap();
    assert_eq!(report.teams_checked, ["Demo", "Unrelated"]);
    assert!(
        report
            .missing_states
            .iter()
            .any(|m| m.starts_with("Unrelated: "))
    );
}

#[tokio::test]
async fn a_failing_project_list_is_returned_as_the_error_once_a_project_is_bound() {
    let names = TriggerNames::default();
    let err = check_workspace(
        &two_teams(Some("projects")),
        &names,
        &[binding_to("proj-1")],
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "projects failed");
    // Without a binding the project list is not asked for.
    let report = check_workspace(&two_teams(Some("projects")), &names, &[])
        .await
        .unwrap();
    assert_eq!(report.teams_checked, ["Demo", "Beta"]);
}

#[tokio::test]
async fn each_failing_linear_call_is_returned_as_the_error() {
    for call in ["labels", "teams", "statuses"] {
        let err = check_workspace(&two_teams(Some(call)), &TriggerNames::default(), &[])
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
async fn run_limited_gives_up_at_the_callers_limit_not_the_runners() {
    // The runner's own limit is 10 s, so only the 300 ms the caller passes ends this 5 s command.
    let dir = tempfile::tempdir().unwrap();
    let script = script(dir.path(), serde_json::json!([{"sleep_ms": 5000}]));
    let runner = SystemRunner::with_limits(Duration::from_secs(10), Duration::from_secs(10));
    let started = Instant::now();
    let err = runner
        .run_limited(agent(), &[script.as_str()], Duration::from_millis(300))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out after 300ms"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the call waited for the command: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn run_limited_waits_for_the_callers_limit_not_the_runners() {
    // The runner's own limit is 300 ms, so only the 5 s the caller passes lets this 1 s command run.
    let dir = tempfile::tempdir().unwrap();
    let script = script(
        dir.path(),
        serde_json::json!([{"sleep_ms": 1000}, {"out": "ok"}]),
    );
    let runner = SystemRunner::with_limits(Duration::from_millis(300), Duration::from_millis(300));
    let output = runner
        .run_limited(agent(), &[script.as_str()], Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(output.code, Some(0), "{output:?}");
    assert_eq!(output.stdout.trim_end(), "ok", "{output:?}");
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

fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Waits until `path` has content. Returns false if it is still empty after 10 s.
async fn wait_for_content(path: &Path) -> bool {
    let give_up = Instant::now() + Duration::from_secs(10);
    while file_len(path) == 0 {
        if Instant::now() >= give_up {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

/// Fails with `message` unless the file at `path` has stopped growing: it is measured 500 ms from
/// now and again 500 ms later.
async fn assert_stops_growing(path: &Path, message: &str) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = file_len(path);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(file_len(path), settled, "{message}");
}

// `claude mcp list` starts MCP servers of its own: a check that outlives its limit must take
// everything it started with it, not only the process it launched (on Windows the Job Object holds
// the heartbeat child). The limit is long enough for the child to start on a slow runner, and the
// check is driven until the child has written something, before the limit can end it, so the test
// can neither pass without having tested anything nor be cut short by a slow start.
#[tokio::test]
async fn a_timed_out_check_ends_its_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let script = script(
        dir.path(),
        serde_json::json!([{"heartbeat": beat, "every_ms": 50}, {"sleep_ms": 30_000}]),
    );
    let runner = SystemRunner::with_limits(Duration::from_secs(5), Duration::from_secs(5));
    let args = [script.as_str()];
    let mut check = runner.run(agent(), &args);
    tokio::select! {
        result = &mut check => panic!("the check ended before the heartbeat appeared: {result:?}"),
        started = wait_for_content(&beat) => assert!(started, "the heartbeat child never ran"),
    }
    let error = check.await.unwrap_err();
    assert!(error.to_string().contains("timed out"), "{error:#}");
    // The heartbeat grandchild was in the check's process group (Windows: its Job Object): it
    // must stop growing the file.
    assert_stops_growing(&beat, "the heartbeat child outlived the timed-out check").await;
}

// A check command can leave a process behind that inherited its output pipe (`claude mcp list`
// leaves stdio MCP servers). The check is done when the command itself exits: the leftover holding
// stdout open must neither delay the result nor turn it into a timeout.
#[cfg(unix)]
#[tokio::test]
async fn a_background_process_holding_the_output_does_not_turn_a_finished_check_into_a_timeout() {
    let runner = SystemRunner::with_limits(Duration::from_secs(10), Duration::from_secs(10));
    let started = Instant::now();
    let output = runner
        .run(Path::new("/bin/sh"), &["-c", "sleep 30 & echo done"])
        .await
        .unwrap();
    let took = started.elapsed();
    assert_eq!(output.stdout, "done\n", "{output:?}");
    assert_eq!(output.code, Some(0), "{output:?}");
    assert!(
        took < Duration::from_secs(5),
        "the check waited for the background process: {took:?}"
    );
}

// What a check command left running in its process group (Windows: its Job Object) is ended once
// the command exits. On Windows the heartbeat child also inherits the command's pipes, so this is
// the Windows case of the test above too.
#[tokio::test]
async fn a_finished_check_ends_what_it_left_running() {
    let dir = tempfile::tempdir().unwrap();
    let beat = dir.path().join("beat");
    let script = script(
        dir.path(),
        serde_json::json!([{"heartbeat": beat, "every_ms": 50}, {"sleep_ms": 300}, {"out": "done"}]),
    );
    let runner = SystemRunner::with_limits(Duration::from_secs(10), Duration::from_secs(10));
    let started = Instant::now();
    let output = runner.run(agent(), &[script.as_str()]).await.unwrap();
    let took = started.elapsed();
    assert_eq!(output.code, Some(0), "{output:?}");
    // Windows pipes may carry `\r\n`.
    assert_eq!(output.stdout.replace("\r\n", "\n"), "done\n", "{output:?}");
    assert!(
        took < Duration::from_secs(5),
        "the check waited for the heartbeat child: {took:?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let size = std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0);
    assert!(size > 0, "the heartbeat child never ran");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        std::fs::metadata(&beat).map(|m| m.len()).unwrap_or(0),
        size,
        "the heartbeat child outlived the check"
    );
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

/// `run_script` through `run_in`, in `cwd`.
async fn run_script_in(runner: &SystemRunner, script: &Path, cwd: &Path) -> Result<CmdOutput> {
    let mut retries = 0;
    loop {
        match runner.run_in(script, &[], cwd).await {
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

/// `script` ignores its arguments and prints its working directory. `run_in` has to run the
/// check there, not in the home directory: Claude's lean sign-in check runs in an empty folder.
async fn assert_run_in_runs_in_the_folder(script: &Path) {
    let folder = tempfile::tempdir().unwrap();
    let output = run_script_in(&SystemRunner::default(), script, folder.path())
        .await
        .unwrap();
    // Compared once resolved: the temporary folder may be reached through a link (macOS) or by
    // its short name (Windows).
    assert_eq!(
        Path::new(output.stdout.trim_end()).canonicalize().unwrap(),
        folder.path().canonicalize().unwrap(),
        "{output:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn run_in_runs_the_check_in_the_folder_it_is_given_unix() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("check.sh");
    std::fs::write(&script, "#!/bin/sh\npwd -P\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_run_in_runs_in_the_folder(&script).await;
}

#[cfg(windows)]
#[tokio::test]
async fn run_in_runs_the_check_in_the_folder_it_is_given_windows() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("check.cmd");
    std::fs::write(&script, "@echo %CD%\r\n").unwrap();
    assert_run_in_runs_in_the_folder(&script).await;
}
