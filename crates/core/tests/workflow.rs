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
    engine::{Engine, EngineDeps, JobRunner},
    env::ExecEnv,
    executor::CommandSpec,
    model::{Job, JobState},
    runner::{self, RunHooks, RunLimits, RunOutput, SpawnHook},
    store::Store,
    workflow::{ExecutorSource, IssueContext},
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
    let engine = Engine::new(EngineDeps {
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
