//! Durable evidence is checked through a real Engine and the native process fixture.
mod common;

use common::{engine_kit::FakeReadiness, fake_linear::FakeLinear, issue};
use issuerelay_core::{
    config::{Binding, Config, ExecutorKind},
    engine::{Engine, EngineDeps, JobRunner},
    env::ExecEnv,
    executor::CommandSpec,
    model::JobState,
    runner::{self, RunHooks, RunLimits, RunOutput, SpawnHook},
    store::Store,
    workflow::{ControlAction, IssueContext, VerificationStatus},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct EvidenceRunner {
    result: &'static str,
    remove_evidence: bool,
    scratch: Mutex<Option<PathBuf>>,
}

#[async_trait::async_trait]
impl JobRunner for EvidenceRunner {
    async fn run(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        on_spawn: SpawnHook,
    ) -> RunOutput {
        self.run_with_output(
            spec,
            kind,
            log,
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
        log: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        hooks: RunHooks,
    ) -> RunOutput {
        let verify = spec.stdin.contains("\"development_run\"");
        let report = if verify {
            *self.scratch.lock().unwrap() = Some(spec.cwd.clone());
            let dir = spec.cwd.join(".issuerelay-evidence");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("report.md"), "Synthetic independent report.\n").unwrap();
            std::fs::create_dir(dir.join("screens")).unwrap();
            std::fs::write(
                dir.join("screens/page.txt"),
                "Synthetic rendered page evidence.\n",
            )
            .unwrap();
            if self.remove_evidence {
                std::fs::remove_dir_all(&dir).unwrap();
            }
            let data: Value =
                serde_json::from_str(spec.stdin.rsplit_once("\n\n").unwrap().1).unwrap();
            json!({"result":self.result,"summary":"Synthetic independent conclusion",
                "delivery":data["delivery"],"evidence":["Report: .issuerelay-evidence/report.md"]})
        } else {
            json!({"result":"local_complete","summary":"Synthetic delivery"})
        };
        let script = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(script.path(), json!([
            {"out":json!({"type":"thread.started","thread_id":"00000000-0000-4000-8000-000000000001"}).to_string()},
            {"out":json!({"type":"item.completed","item":{"type":"agent_message","text":report.to_string()}}).to_string()},
            {"exit":0}
        ]).to_string()).unwrap();
        spec.program = PathBuf::from(env!("CARGO_BIN_EXE_fake-agent"));
        spec.args = vec![script.path().to_str().unwrap().into()];
        spec.env.clear();
        runner::run_with_output(spec, kind, &log, limits, cancel, &ExecEnv::inherit(), hooks).await
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

struct Fixture {
    root: tempfile::TempDir,
    linear: FakeLinear,
    runner: Arc<EvidenceRunner>,
    engine: Engine,
    delivery: String,
}

impl Fixture {
    fn new(result: &'static str, remove_evidence: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "Demo"]);
        git(&repo, &["config", "user.email", "demo@example.invalid"]);
        std::fs::write(repo.join("delivery.txt"), "Synthetic delivery.\n").unwrap();
        git(&repo, &["add", "delivery.txt"]);
        git(&repo, &["commit", "-qm", "Synthetic delivery"]);
        let delivery = git(&repo, &["rev-parse", "HEAD"]);
        let mut cfg = Config::default();
        cfg.settings.default_executor = ExecutorKind::Codex;
        cfg.bindings.push(Binding {
            id: "proj-1".into(),
            project_id: "proj-1".into(),
            project_name: "Demo".into(),
            directory: repo,
            executor: None,
            permission: None,
            model: None,
            prompt_override: None,
            lean: false,
            effort: None,
            labels: vec![],
        });
        cfg.save(&root.path().join("config.toml")).unwrap();
        let linear = FakeLinear::new("user-1");
        linear.upsert(issue("1", "proj-1"));
        let runner = Arc::new(EvidenceRunner {
            result,
            remove_evidence,
            scratch: Mutex::new(None),
        });
        let engine = Self::engine(root.path(), &linear, runner.clone());
        Self {
            root,
            linear,
            runner,
            engine,
            delivery,
        }
    }

    fn engine(root: &Path, linear: &FakeLinear, runner: Arc<EvidenceRunner>) -> Engine {
        Engine::new(EngineDeps {
            config_path: root.join("config.toml"),
            store: Store::open(&root.join("state.db")).unwrap(),
            linear: Arc::new(linear.clone()),
            readiness: Arc::new(FakeReadiness),
            job_runner: runner,
            logs_dir: root.join("logs"),
            user_home: None,
            claude_config_dir: None,
        })
        .unwrap()
    }

    async fn verify(&self) -> i64 {
        let preview = self.engine.dispatch_preview("DEMO-1").await.unwrap();
        self.engine
            .approve_dispatch("DEMO-1", &preview.token)
            .await
            .unwrap();
        self.engine.tick().await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.engine.wait_idle())
            .await
            .unwrap();
        let source = self.engine.list_jobs(10).unwrap()[0].clone();
        assert_eq!(source.state, JobState::LocalComplete);
        self.engine
            .set_issue_context(
                "DEMO-1",
                0,
                IssueContext {
                    request: "Verify the synthetic delivery".into(),
                    conversation_history: Some(
                        "Preserve independent reports after cleanup.".into(),
                    ),
                    scope: "Only the synthetic fixture".into(),
                    acceptance: vec!["Report remains readable".into()],
                    ..IssueContext::default()
                },
            )
            .await
            .unwrap();
        let preview = self
            .engine
            .prepare_control_action(ControlAction::StartVerification {
                job_id: source.id,
                executor: ExecutorKind::Codex,
                delivery: self.delivery.clone(),
            })
            .await
            .unwrap();
        let id = self
            .engine
            .commit_control_action(&preview.token)
            .await
            .unwrap()
            .job_id
            .unwrap();
        assert_eq!(self.engine.dispatch().await.unwrap(), [id]);
        tokio::time::timeout(Duration::from_secs(10), self.engine.wait_idle())
            .await
            .unwrap();
        assert!(
            !self
                .runner
                .scratch
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .exists()
        );
        id
    }
}

#[tokio::test]
async fn verification_preserves_artifacts_after_scratch_cleanup_and_engine_reload() {
    for result in [
        "verification_passed",
        "verification_failed",
        "verification_blocked",
    ] {
        let f = Fixture::new(result, false);
        let id = f.verify().await;
        let base = f
            .root
            .path()
            .join("verification-evidence")
            .join(id.to_string());
        let index = base.join("index.json");
        assert!(
            index.is_file(),
            "finished verification lost its artifact index"
        );
        assert_eq!(
            std::fs::read_to_string(base.join("files/report.md")).unwrap(),
            "Synthetic independent report.\n"
        );
        assert!(base.join("files/screens/page.txt").is_file());
        assert!(!base.join("files/delivery.txt").exists());
        let capture: Value =
            serde_json::from_slice(&std::fs::read(base.join("result.json")).unwrap()).unwrap();
        assert_eq!(capture["job_id"], id);
        assert_eq!(
            capture["native_session_id"],
            f.engine.job(id).unwrap().unwrap().session_id.unwrap()
        );
        assert!(capture["final_text"].as_str().unwrap().contains(result));
        let manifest: Value = serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        for file in manifest["files"].as_array().unwrap() {
            let bytes = std::fs::read(base.join(file["path"].as_str().unwrap())).unwrap();
            assert_eq!(bytes.len() as u64, file["bytes"].as_u64().unwrap());
            let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &bytes);
            let hex: String = digest.as_ref().iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, file["sha256"].as_str().unwrap());
        }
        let loaded = Fixture::engine(f.root.path(), &f.linear, f.runner.clone());
        let view = loaded.handoff(id).unwrap().acceptance.unwrap();
        assert!(
            view.verification_evidence
                .iter()
                .any(|e| e.contains(index.to_str().unwrap()))
        );
        assert_eq!(
            loaded.job(id).unwrap().unwrap().report.as_deref(),
            Some(result)
        );
        assert!(f.linear.set_state_calls().is_empty());
    }
}

#[tokio::test]
async fn lost_attachment_directory_blocks_an_otherwise_passing_verification() {
    let f = Fixture::new("verification_passed", true);
    let id = f.verify().await;
    let job = f.engine.job(id).unwrap().unwrap();
    assert_eq!(job.state, JobState::Failed);
    assert_eq!(job.report.as_deref(), Some("verification_blocked"));
    assert_eq!(
        f.engine
            .handoff(id)
            .unwrap()
            .acceptance
            .unwrap()
            .verification_status,
        Some(VerificationStatus::Blocked)
    );
    assert!(job.reason.as_deref().unwrap().contains("evidence"));
    assert!(f.linear.set_state_calls().is_empty());
}
