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
    hold: std::sync::atomic::AtomicBool,
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
            if self.hold.load(std::sync::atomic::Ordering::SeqCst) {
                if let Some(hook) = hooks.on_stdout.as_ref() {
                    hook(
                        "{\"type\":\"thread.started\",\"thread_id\":\"00000000-0000-4000-8000-000000000002\"}",
                    );
                }
                return std::future::pending().await;
            }
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
            hold: std::sync::atomic::AtomicBool::new(false),
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

    async fn start_verification(&self) -> i64 {
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
        id
    }

    async fn verify(&self) -> i64 {
        let id = self.start_verification().await;
        tokio::time::timeout(Duration::from_secs(10), self.engine.wait_idle())
            .await
            .unwrap();
        assert_eq!(
            self.runner
                .scratch
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .exists(),
            self.runner.remove_evidence,
            "only successful retention plus result persistence may clean the workspace"
        );
        id
    }
}

struct InterruptedFixture {
    root: tempfile::TempDir,
    linear: FakeLinear,
    runner: Arc<EvidenceRunner>,
    id: i64,
    scratch: PathBuf,
}

impl InterruptedFixture {
    fn new() -> Self {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (fixture, id, scratch) = runtime.block_on(async {
            let f = Fixture::new("verification_passed", false);
            f.runner
                .hold
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let id = f.start_verification().await;
            let scratch = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(path) = f.runner.scratch.lock().unwrap().clone()
                        && path.join(".issuerelay-evidence/screens/page.txt").is_file()
                    {
                        break path;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            (f, id, scratch)
        });
        // Private async tasks only; no real application or OS process is killed.
        drop(runtime);
        assert!(
            scratch.join(".issuerelay-evidence/report.md").is_file(),
            "an interrupted host must not discard the only raw evidence before durable publication"
        );
        let Fixture {
            root,
            linear,
            runner,
            engine,
            ..
        } = fixture;
        drop(engine);
        // Runtime Drop runs Rust destructors; a machine crash does not. Restore the private
        // fixture's persisted pre-crash Running row before real Engine initialization.
        Store::open(&root.path().join("state.db"))
            .unwrap()
            .set_state(id, JobState::Running, None)
            .unwrap();
        Self {
            root,
            linear,
            runner,
            id,
            scratch,
        }
    }

    fn index(&self) -> PathBuf {
        self.root
            .path()
            .join("verification-evidence")
            .join(self.id.to_string())
            .join("index.json")
    }

    fn marker(&self) -> PathBuf {
        self.scratch.parent().unwrap().join("owner.json")
    }

    fn rewrite_marker(&self, edit: impl FnOnce(&mut Value)) {
        let mut value: Value =
            serde_json::from_slice(&std::fs::read(self.marker()).unwrap()).unwrap();
        edit(&mut value);
        std::fs::write(self.marker(), serde_json::to_vec(&value).unwrap()).unwrap();
    }

    fn recover(&self) -> Engine {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let loaded = Fixture::engine(self.root.path(), &self.linear, self.runner.clone());
            let stop = CancellationToken::new();
            stop.cancel(); // Recovery runs before the scheduler; never launch another fake task.
            loaded.run_forever(stop).await;
            loaded
        })
    }

    fn interrupt(&self) -> chrono::DateTime<chrono::Utc> {
        let store = Store::open(&self.root.path().join("state.db")).unwrap();
        assert_eq!(store.recover_interrupted().unwrap().len(), 1);
        // A fixed historical run end makes a later recovery timestamp overwrite observable
        // without depending on clock resolution or delaying a private test runtime.
        rusqlite::Connection::open(self.root.path().join("state.db"))
            .unwrap()
            .execute(
                "UPDATE jobs SET finished_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
                [self.id],
            )
            .unwrap();
        store.get(self.id).unwrap().unwrap().finished_at.unwrap()
    }

    fn watch_recovery_updates(&self) {
        rusqlite::Connection::open(self.root.path().join("state.db"))
            .unwrap()
            .execute_batch(&format!(
                "CREATE TABLE recovery_updates (id INTEGER); \
                 CREATE TRIGGER observe_recovery_job AFTER UPDATE ON jobs WHEN NEW.id = {0} \
                 BEGIN INSERT INTO recovery_updates VALUES (NEW.id); END; \
                 CREATE TRIGGER observe_recovery_acceptance AFTER UPDATE ON acceptance_runs \
                 WHEN NEW.verification_job_id = {0} \
                 BEGIN INSERT INTO recovery_updates VALUES (NEW.verification_job_id); END;",
                self.id
            ))
            .unwrap();
    }

    fn assert_no_recovery_updates(&self) {
        let count: i64 = rusqlite::Connection::open(self.root.path().join("state.db"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM recovery_updates", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 0,
            "identical recovery must not rewrite historical rows"
        );
    }

    fn assert_blocked(&self, engine: &Engine) {
        assert_eq!(
            engine
                .handoff(self.id)
                .unwrap()
                .acceptance
                .unwrap()
                .verification_status,
            Some(VerificationStatus::Blocked)
        );
        assert!(
            self.scratch
                .join(".issuerelay-evidence/report.md")
                .is_file()
        );
        assert!(self.linear.set_state_calls().is_empty());
    }
}

#[test]
fn interrupted_verification_recovers_partial_raw_without_a_final_report() {
    let f = InterruptedFixture::new();
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    let view = loaded.handoff(f.id).unwrap().acceptance.unwrap();
    assert!(
        view.verification_evidence
            .iter()
            .any(|item| item.contains("index.json"))
    );
    let report: Value = serde_json::from_slice(
        &std::fs::read(f.index().parent().unwrap().join("result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["interrupted"], true);
    assert!(report["final_text"].is_null());
    assert_eq!(report["worker_final_complete"], false);
    chrono::DateTime::parse_from_rfc3339(report["recovered_at"].as_str().unwrap()).unwrap();
    assert!(
        f.index()
            .parent()
            .unwrap()
            .join("files/screens/page.txt")
            .is_file()
    );
}

#[test]
fn interrupted_recovery_is_idempotent_and_does_not_overwrite_published_payloads() {
    let f = InterruptedFixture::new();
    let ended_at = f.interrupt();
    let first = f.recover();
    f.assert_blocked(&first);
    let before_run = first.job(f.id).unwrap().unwrap();
    assert_eq!(before_run.finished_at, Some(ended_at));
    drop(first);
    let before = std::fs::read(f.index()).unwrap();
    f.watch_recovery_updates();
    let second = f.recover();
    f.assert_blocked(&second);
    assert_eq!(std::fs::read(f.index()).unwrap(), before);
    assert_eq!(second.job(f.id).unwrap().unwrap(), before_run);
    f.assert_no_recovery_updates();
    assert!(
        second
            .job(f.id)
            .unwrap()
            .unwrap()
            .reason
            .unwrap()
            .starts_with("interrupted: verification evidence recovered")
    );
}

#[test]
fn unavailable_recovery_is_idempotent_and_preserves_the_interrupted_end_time() {
    let f = InterruptedFixture::new();
    let ended_at = f.interrupt();
    std::fs::create_dir_all(f.index().parent().unwrap()).unwrap();
    std::fs::write(f.index(), "synthetic conflicting index").unwrap();
    let first = f.recover();
    f.assert_blocked(&first);
    let before = first.job(f.id).unwrap().unwrap();
    assert_eq!(before.finished_at, Some(ended_at));
    assert_eq!(
        before.reason.as_deref(),
        Some("interrupted: verification evidence unavailable")
    );
    drop(first);
    f.watch_recovery_updates();
    let second = f.recover();
    f.assert_blocked(&second);
    assert_eq!(second.job(f.id).unwrap().unwrap(), before);
    f.assert_no_recovery_updates();
    assert_eq!(
        std::fs::read_to_string(f.index()).unwrap(),
        "synthetic conflicting index"
    );
}

#[test]
fn interrupted_recovery_rejects_a_stale_attempt_and_keeps_raw() {
    let f = InterruptedFixture::new();
    f.rewrite_marker(|value| value["started_at"] = json!("2000-01-01T00:00:00Z"));
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert!(!f.index().exists());
}

#[test]
fn interrupted_recovery_rejects_a_source_fingerprint_mismatch_and_keeps_raw() {
    let f = InterruptedFixture::new();
    f.rewrite_marker(|value| value["fingerprint"] = json!("synthetic-wrong-source"));
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert!(!f.index().exists());
}

#[test]
fn interrupted_recovery_refuses_multiple_matching_attempts() {
    let f = InterruptedFixture::new();
    let other = f
        .scratch
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("{}-duplicate", f.id));
    std::fs::create_dir(&other).unwrap();
    std::fs::create_dir(other.join("work")).unwrap();
    let mut marker: Value = serde_json::from_slice(&std::fs::read(f.marker()).unwrap()).unwrap();
    marker["attempt"] = json!(other.file_name().unwrap().to_str().unwrap());
    std::fs::write(
        other.join("owner.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert!(!f.index().exists());
    assert!(other.join("owner.json").is_file());
}

#[test]
fn interrupted_recovery_preserves_a_conflicting_published_directory() {
    let f = InterruptedFixture::new();
    std::fs::create_dir_all(f.index().parent().unwrap()).unwrap();
    std::fs::write(f.index(), "synthetic conflicting index").unwrap();
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert_eq!(
        std::fs::read_to_string(f.index()).unwrap(),
        "synthetic conflicting index"
    );
    assert!(
        loaded
            .job(f.id)
            .unwrap()
            .unwrap()
            .reason
            .unwrap()
            .starts_with("interrupted: verification evidence unavailable")
    );
    assert!(
        loaded
            .handoff(f.id)
            .unwrap()
            .acceptance
            .unwrap()
            .verification_evidence
            .is_empty()
    );
}

#[test]
fn interrupted_recovery_keeps_over_budget_raw_and_never_claims_retention() {
    let f = InterruptedFixture::new();
    std::fs::write(
        f.scratch.join(".issuerelay-evidence/oversized.bin"),
        vec![0; 16 * 1024 * 1024 + 1],
    )
    .unwrap();
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert!(!f.index().exists());
    assert!(
        f.scratch
            .join(".issuerelay-evidence/oversized.bin")
            .is_file()
    );
    assert!(
        loaded
            .job(f.id)
            .unwrap()
            .unwrap()
            .reason
            .unwrap()
            .starts_with("interrupted: verification evidence unavailable")
    );
}

#[cfg(unix)]
#[test]
fn interrupted_recovery_does_not_follow_a_worker_attachment_symlink() {
    let f = InterruptedFixture::new();
    let outside = f.root.path().join("synthetic-outside.txt");
    std::fs::write(&outside, "synthetic outside must stay untouched").unwrap();
    std::os::unix::fs::symlink(&outside, f.scratch.join(".issuerelay-evidence/link")).unwrap();
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert!(!f.index().exists());
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "synthetic outside must stay untouched"
    );
}

#[test]
fn interrupted_recovery_rejects_tampered_published_payload_without_overwrite() {
    let f = InterruptedFixture::new();
    drop(f.recover());
    let before = std::fs::read(f.index()).unwrap();
    std::fs::write(
        f.index().parent().unwrap().join("files/report.md"),
        "synthetic tamper",
    )
    .unwrap();
    let loaded = f.recover();
    f.assert_blocked(&loaded);
    assert_eq!(std::fs::read(f.index()).unwrap(), before);
    assert!(
        loaded
            .handoff(f.id)
            .unwrap()
            .acceptance
            .unwrap()
            .verification_evidence
            .is_empty()
    );
}

#[test]
fn published_final_before_result_commit_recovers_as_blocked_and_keeps_original_index() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (f, id) = runtime.block_on(async {
        let f = Fixture::new("verification_passed", false);
        rusqlite::Connection::open(f.root.path().join("state.db")).unwrap()
            .execute_batch("CREATE TRIGGER synthetic_result_failure BEFORE UPDATE OF recommendation ON acceptance_runs BEGIN SELECT RAISE(FAIL, 'synthetic result failure'); END;").unwrap();
        let id = f.start_verification().await;
        tokio::time::timeout(Duration::from_secs(5), f.engine.wait_idle()).await.unwrap();
        (f, id)
    });
    let scratch = f.runner.scratch.lock().unwrap().clone().unwrap();
    assert!(
        scratch.join(".issuerelay-evidence/report.md").is_file(),
        "DB failure must preserve raw even after publication"
    );
    let index = f
        .root
        .path()
        .join("verification-evidence")
        .join(id.to_string())
        .join("index.json");
    let original = std::fs::read(&index).unwrap();
    let published: Value = serde_json::from_slice(
        &std::fs::read(index.parent().unwrap().join("result.json")).unwrap(),
    )
    .unwrap();
    assert!(
        published["final_text"]
            .as_str()
            .unwrap()
            .contains("verification_passed")
    );
    assert!(
        f.engine
            .job(id)
            .unwrap()
            .unwrap()
            .reason
            .unwrap()
            .contains("persist")
    );
    {
        let store = Store::open(&f.root.path().join("state.db")).unwrap();
        rusqlite::Connection::open(f.root.path().join("state.db"))
            .unwrap()
            .execute_batch("DROP TRIGGER synthetic_result_failure")
            .unwrap();
        // Model a crash between immutable publication and the result transaction, not a new
        // worker conclusion or a real application restart.
        store.set_state(id, JobState::Running, None).unwrap();
    }
    let Fixture {
        root,
        linear,
        runner,
        engine,
        ..
    } = f;
    drop(engine);
    drop(runtime);
    let interrupted = InterruptedFixture {
        root,
        linear,
        runner,
        id,
        scratch,
    };
    let loaded = interrupted.recover();
    interrupted.assert_blocked(&loaded);
    assert_eq!(std::fs::read(&index).unwrap(), original);
    assert!(
        loaded
            .job(id)
            .unwrap()
            .unwrap()
            .reason
            .unwrap()
            .starts_with("interrupted: verification evidence recovered")
    );
    assert_eq!(
        loaded.job(id).unwrap().unwrap().report.as_deref(),
        Some("verification_blocked")
    );
}

#[tokio::test]
async fn verification_uses_a_host_owned_durable_workspace() {
    let f = Fixture::new("verification_passed", false);
    f.verify().await;
    let scratch = f.runner.scratch.lock().unwrap().clone().unwrap();
    let durable = f
        .root
        .path()
        .canonicalize()
        .unwrap()
        .join("verification-work");
    assert!(
        scratch.starts_with(&durable),
        "active verification raw must live in host-owned durable staging, not the system temp root: {}",
        scratch.display()
    );
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
        // Temporary roots may use a different spelling than the canonical host path on Windows.
        let index_alias = base.join("..").join(id.to_string()).join("index.json");
        let canonical_index = index_alias.canonicalize().unwrap();
        assert_eq!(canonical_index, index.canonicalize().unwrap());
        assert!(view.verification_evidence.contains(&format!(
            "Retained verification evidence index: {}",
            canonical_index.display()
        )));
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
