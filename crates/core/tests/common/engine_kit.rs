//! An Engine wired to fakes: in-memory Linear, scripted runs, always-ready executors.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use issuerelay_core::{
    config::{Binding, Config, ExecutorKind},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{Engine, EngineDeps, JobRunner},
    executor::CommandSpec,
    model::ProcessTag,
    runner::{ExitKind, RunLimits, RunOutput, SpawnHook},
    store::Store,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::fake_linear::FakeLinear;

pub struct FakeReadiness;

#[async_trait::async_trait]
impl ReadinessProbe for FakeReadiness {
    async fn executor_reports(&self, _fresh: bool) -> Vec<ExecutorReport> {
        [ExecutorKind::Claude, ExecutorKind::Codex]
            .into_iter()
            .map(|kind| ExecutorReport {
                kind,
                program: Some(PathBuf::from(format!("/fake/{}", kind.as_str()))),
                version: Some("test".into()),
                logged_in: true,
                linear_mcp: true,
                problems: Vec::new(),
            })
            .collect()
    }
}

/// What a scripted run does: optionally move the issue's status (as the AI would), then exit.
#[derive(Clone)]
pub struct Script {
    pub set_state: Option<&'static str>,
    pub exit: ExitKind,
    pub final_text: Option<String>,
    /// Panics right after the status change, as a bug inside the run task would.
    pub panic: bool,
}

impl Script {
    pub fn success() -> Self {
        Self {
            set_state: Some("In Testing"),
            exit: ExitKind::Exited(Some(0)),
            final_text: Some(r#"{"result":"in_testing","summary":"pushed to test"}"#.into()),
            panic: false,
        }
    }

    pub fn exit_code(code: i32) -> Self {
        Self {
            set_state: None,
            exit: ExitKind::Exited(Some(code)),
            final_text: None,
            panic: false,
        }
    }

    pub fn needs_info() -> Self {
        Self {
            set_state: Some("Todo"),
            exit: ExitKind::Exited(Some(0)),
            final_text: Some(r#"{"result":"needs_info","summary":"which page?"}"#.into()),
            panic: false,
        }
    }
}

pub struct ScriptedRunner {
    pub linear: FakeLinear,
    pub scripts: Mutex<HashMap<String, Script>>,
    pub started: Mutex<Vec<String>>,
    pub specs: Mutex<Vec<CommandSpec>>,
    /// The executor kind and the time limits of each run, in start order (like `specs`).
    pub kinds: Mutex<Vec<ExecutorKind>>,
    pub limits: Mutex<Vec<RunLimits>>,
    /// Runs wait for a permit before finishing; tests add permits to let them end.
    pub gate: Arc<Semaphore>,
}

impl ScriptedRunner {
    pub fn new(linear: FakeLinear, gated: bool) -> Self {
        Self {
            linear,
            scripts: Mutex::new(HashMap::new()),
            started: Mutex::new(Vec::new()),
            specs: Mutex::new(Vec::new()),
            kinds: Mutex::new(Vec::new()),
            limits: Mutex::new(Vec::new()),
            gate: Arc::new(Semaphore::new(if gated { 0 } else { 1_000 })),
        }
    }

    pub fn script(&self, identifier: &str, script: Script) {
        self.scripts
            .lock()
            .unwrap()
            .insert(identifier.into(), script);
    }

    pub fn started(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }
}

fn identifier_in(prompt: &str) -> String {
    prompt
        .split("Linear issue: ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_default()
        .to_string()
}

#[async_trait::async_trait]
impl JobRunner for ScriptedRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log_path: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        on_spawn: SpawnHook,
    ) -> RunOutput {
        // A made-up process, reported before the run counts as started. It only goes into the
        // store: no test may let a host end it (`runner::end_leftover`), as the PID may belong to
        // anything on the machine running the tests.
        on_spawn(ProcessTag {
            pid: 4242,
            started: "1700000000.000001".into(),
        });
        let id = identifier_in(&spec.stdin);
        self.started.lock().unwrap().push(id.clone());
        self.specs.lock().unwrap().push(spec);
        self.kinds.lock().unwrap().push(kind);
        self.limits.lock().unwrap().push(limits);
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        std::fs::write(&log_path, "{\"type\":\"result\"}\n").unwrap();
        let script = self
            .scripts
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_else(Script::success);
        if let Some(state) = script.set_state {
            self.linear.update(&id, |issue| issue.state = state.into());
        }
        if script.panic {
            panic!("scripted panic");
        }
        tokio::select! {
            permit = self.gate.acquire() => permit.expect("gate closed").forget(),
            _ = cancel.cancelled() => return RunOutput { exit: ExitKind::Canceled, final_text: None },
        }
        RunOutput {
            exit: script.exit,
            final_text: script.final_text,
        }
    }
}

pub struct Harness {
    pub engine: Engine,
    pub linear: FakeLinear,
    pub runner: Arc<ScriptedRunner>,
    pub root: tempfile::TempDir,
}

impl Harness {
    pub fn new(max_concurrent: usize, gated: bool) -> Self {
        let mut cfg = Config::default();
        cfg.settings.max_concurrent = max_concurrent;
        Self::with_config(cfg, gated)
    }

    /// Starts from `cfg`, which is saved as the engine's configuration file.
    pub fn with_config(cfg: Config, gated: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let config_path = root.path().join("config.toml");
        cfg.save(&config_path).unwrap();
        let linear = FakeLinear::new("user-1");
        let runner = Arc::new(ScriptedRunner::new(linear.clone(), gated));
        let engine = Engine::new(EngineDeps {
            config_path,
            store: Store::open_in_memory().unwrap(),
            linear: Arc::new(linear.clone()),
            readiness: Arc::new(FakeReadiness),
            job_runner: runner.clone(),
            logs_dir: root.path().join("logs"),
        })
        .unwrap();
        Self {
            engine,
            linear,
            runner,
            root,
        }
    }

    /// Creates `<root>/<dir_name>` and binds `project` to it with the default executor.
    pub fn bind(&self, project: &str, dir_name: &str) -> PathBuf {
        let dir = self.root.path().join(dir_name);
        std::fs::create_dir_all(&dir).unwrap();
        self.engine
            .add_binding(Binding {
                id: project.into(),
                project_id: project.into(),
                project_name: "Demo Project".into(),
                directory: dir.clone(),
                executor: None,
                permission: None,
                model: None,
                prompt_override: None,
            })
            .unwrap();
        dir
    }

    pub async fn settle(&self) {
        self.engine.wait_idle().await;
    }

    pub fn latest(&self, uuid: &str) -> issuerelay_core::model::Job {
        self.engine
            .list_jobs(100)
            .unwrap()
            .into_iter()
            .find(|job| job.issue_uuid == uuid)
            .expect("no job for issue")
    }
}
