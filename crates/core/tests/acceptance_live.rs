//! Opt-in verification through the local host with real provider CLIs and synthetic Linear data.
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use common::{fake_linear::FakeLinear, issue};
use issuerelay_core::{
    config::{Binding, Config, Effort, ExecutorKind},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{EngineEvent, JobRunner, ProcessJobRunner},
    env::ExecEnv,
    executor::{CommandSpec, outcome::is_session_id, resolve_program},
    host::{AuthMethod, Connector, Host, HostParts, LinearSetup, OpenUrl, StopMode},
    linear::LinearApi,
    model::{JobState, RunMode},
    paths::AppPaths,
    runner::{ExitKind, RunHooks, RunLimits, RunOutput, SpawnHook},
    workflow::{ControlAction, ControlActionOutcome, IssueContext, VerificationStatus},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const DELIVERED: &str = "issue-relay-delivery-2407";

struct SandboxConnector(FakeLinear);

#[async_trait::async_trait]
impl Connector for SandboxConnector {
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>> {
        Ok(Some((Arc::new(self.0.clone()), AuthMethod::OAuth)))
    }

    async fn login(&self, _: OpenUrl) -> Result<Arc<dyn LinearApi>> {
        anyhow::bail!("the sandbox connector never uses OAuth");
    }

    async fn api_key(&self, _: &str) -> Result<Arc<dyn LinearApi>> {
        anyhow::bail!("the sandbox connector never uses an API key");
    }

    async fn logout(&self) -> Result<()> {
        Ok(())
    }
}

struct SandboxReadiness(ExecutorKind);

#[async_trait::async_trait]
impl ReadinessProbe for SandboxReadiness {
    async fn executor_reports(&self, _: bool) -> Vec<ExecutorReport> {
        // Substitute for the Linear-dependent readiness check. The actual executable and its
        // native account must still work for the opt-in run to produce the required evidence.
        vec![ExecutorReport {
            kind: self.0,
            program: resolve_program(self.0).ok(),
            version: None,
            logged_in: true,
            linear_mcp: true,
            problems: vec![],
            lean_linear_mcp: false,
            lean_logged_in: false,
            lean_problems: vec![],
        }]
    }
}

struct SandboxRunner {
    repository: PathBuf,
    linear: FakeLinear,
    provider: ProcessJobRunner,
}

#[async_trait::async_trait]
impl JobRunner for SandboxRunner {
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
                ..RunHooks::default()
            },
        )
        .await
    }

    async fn run_with_output(
        &self,
        spec: CommandSpec,
        kind: ExecutorKind,
        log: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        hooks: RunHooks,
    ) -> RunOutput {
        if spec.cwd == self.repository {
            // Generate the source development record through the ordinary approved dispatch.
            // The real provider is reserved for the single independent verification in scratch.
            self.linear
                .update("2407", |issue| issue.state = "In Testing".into());
            return RunOutput::ended(
                ExitKind::Exited(Some(0)),
                Some(r#"{"result":"in_testing","summary":"Synthetic delivery prepared"}"#.into()),
            );
        }
        assert_eq!(
            std::fs::read_to_string(spec.cwd.join("delivery.txt")).unwrap(),
            DELIVERED
        );
        let checked = spec.cwd.join("checked.txt");
        let output = self
            .provider
            .run_with_output(spec, kind, log, limits, cancel, hooks)
            .await;
        if cfg!(unix)
            && output.exit == ExitKind::Exited(Some(0))
            && output
                .final_text
                .as_deref()
                .is_some_and(|text| text.contains("verification_passed"))
        {
            assert_eq!(std::fs::read_to_string(checked).unwrap().trim(), DELIVERED);
        }
        output
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

async fn run_provider(kind: ExecutorKind) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let repository = root.join("repository");
    let wiki = root.join("wiki");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&wiki).unwrap();
    std::fs::write(repository.join("delivery.txt"), DELIVERED).unwrap();
    git(&repository, &["init", "-q"]);
    git(&repository, &["add", "delivery.txt"]);
    git(
        &repository,
        &[
            "-c",
            "user.name=IssueRelay Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "Synthetic delivery",
        ],
    );
    let delivery = git(&repository, &["rev-parse", "HEAD"]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/fixture", listener.local_addr().unwrap());
    let marker = format!("local-api-{}", &delivery[..12]);
    let server_marker = marker.clone();
    let served = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_served = served.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let count = connection.read(&mut request).await.unwrap();
            if !request[..count].starts_with(b"GET /fixture HTTP/") {
                continue;
            }
            server_served.store(true, std::sync::atomic::Ordering::SeqCst);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                server_marker.len(),
                server_marker,
            );
            connection.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let synthetic = issue("2407", "project-2407");
    let linear = FakeLinear::new("user-1");
    linear.upsert(synthetic.clone());
    let paths = AppPaths::under(&root);
    let mut config = Config::default();
    config.settings.wiki_root = Some(wiki.clone());
    let binding = Binding {
        id: "project-2407".into(),
        project_id: synthetic.project_id.clone().unwrap(),
        project_name: "Synthetic verification".into(),
        directory: repository.clone(),
        executor: Some(kind),
        permission: None,
        model: (kind == ExecutorKind::Codex).then(|| "gpt-6.1-sol".into()),
        prompt_override: None,
        lean: false,
        effort: Some(if kind == ExecutorKind::Codex {
            Effort::Ultra
        } else {
            Effort::Max
        }),
        labels: vec![],
    };
    config.bindings.push(binding);
    config.save(&paths.config_file()).unwrap();
    let env = ExecEnv::inherit().shared();
    let host = Host::start(
        paths,
        Arc::new(SandboxConnector(linear.clone())),
        HostParts {
            env: Some(env.clone()),
            readiness: Some(Arc::new(SandboxReadiness(kind))),
            job_runner: Some(Arc::new(SandboxRunner {
                repository: repository.clone(),
                linear: linear.clone(),
                provider: ProcessJobRunner::new(env),
            })),
            ..HostParts::default()
        },
    )
    .await
    .unwrap();
    let engine = host.engine();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !matches!(host.linear_setup(), LinearSetup::Connected { .. }) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let outside = root.join("outside-probe.txt");
    let exercise = if cfg!(unix) {
        // This path belongs to the temporary test only, and is safe to probe. Quoting for the
        // model's displayed POSIX test command isn't used to construct a provider argument.
        let quoted = outside.to_str().unwrap().replace('\'', "'\"'\"'");
        format!(
            "Execute this local test in the current directory: `if printf probe > '{quoted}'; \
             then echo outside-allowed; else echo outside-blocked; fi; \
             cat delivery.txt | tee checked.txt`. Put the value read from delivery.txt and the \
             outside-write result on separate evidence entries. Report failure if it can write \
             outside or the in-scratch copy is missing."
        )
    } else {
        "Read delivery.txt in the current directory and put its value in the evidence array.".into()
    };
    engine
        .set_issue_context(
            &synthetic.identifier,
            0,
            IssueContext {
                request: "Independently verify the synthetic fixture in the staged workspace."
                    .into(),
                conversation_history: Some(
                    "The maintainer prepared a synthetic fixture for this local check.".into(),
                ),
                scope: "The fixture and these test paths only. Do not inspect real projects."
                    .into(),
                acceptance: vec![exercise, format!(
                    "Fetch the authorized synthetic local API using curl --fail --silent {endpoint}. Include its exact response as evidence. Report blocked if it cannot be reached."
                )],
                source_refs: vec!["delivery.txt".into()],
                ..IssueContext::default()
            },
        )
        .await
        .unwrap();
    let initial = engine
        .dispatch_preview(&synthetic.identifier)
        .await
        .unwrap();
    engine
        .approve_dispatch(&synthetic.identifier, &initial.token)
        .await
        .unwrap();
    engine.tick().await.unwrap();
    engine.wait_idle().await;
    let source_id = engine
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|job| job.mode == RunMode::Implement && job.state == JobState::AwaitingAcceptance)
        .expect("approved synthetic development finished")
        .id;
    let reviewed_issue = linear.get(&synthetic.uuid);
    assert_eq!(reviewed_issue.state, "In Testing");
    let preview = engine
        .prepare_control_action(ControlAction::StartVerification {
            job_id: source_id,
            executor: kind,
            delivery: delivery.clone(),
        })
        .await
        .unwrap();
    let mut events = engine.subscribe();
    let queued = engine.commit_control_action(&preview.token).await.unwrap();
    assert_eq!(queued.result, ControlActionOutcome::VerificationQueued);
    let verification_id = queued.job_id.expect("verification run ID");
    assert_ne!(source_id, verification_id);

    let outcome = tokio::time::timeout(Duration::from_secs(150), async {
        let mut captured = None;
        loop {
            let job = engine.job(verification_id).unwrap().unwrap();
            while let Ok(event) = events.try_recv() {
                if let EngineEvent::Job { job } = event
                    && job.id == verification_id
                    && job.state == JobState::Running
                    && let Some(id) = job.session_id.as_deref().filter(|id| is_session_id(id))
                {
                    captured.get_or_insert_with(|| id.to_owned());
                }
            }
            if job.finished_at.is_some() && !engine.handoff(verification_id).unwrap().active {
                break (job, captured);
            }
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
    })
    .await;
    let (finished, started) = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            host.stop(StopMode::StopRuns).await;
            panic!("the synthetic provider timed out: {error}");
        }
    };
    let session = started.expect("native verification session");
    println!("{kind:?} verification native session: {session}");
    assert_eq!(finished.session_id.as_deref(), Some(session.as_str()));
    let handoff = engine.handoff(verification_id).unwrap();
    let acceptance = handoff.acceptance.unwrap();
    assert_eq!(acceptance.verification_job_id, Some(verification_id));
    assert_eq!(acceptance.delivery.as_deref(), Some(delivery.as_str()));
    assert!(!outside.exists());
    assert_eq!(
        std::fs::read_to_string(repository.join("delivery.txt")).unwrap(),
        DELIVERED
    );
    assert_eq!(
        linear.get(&synthetic.uuid).updated_at,
        reviewed_issue.updated_at
    );
    assert_eq!(linear.get(&synthetic.uuid).state, reviewed_issue.state);
    std::fs::write(wiki.join("fixture.md"), "Synthetic verification evidence.").unwrap();
    engine
        .record_archive(verification_id, "fixture.md".into(), None)
        .await
        .unwrap();
    let readback = engine
        .workflow_run(Some(verification_id), None)
        .await
        .unwrap();
    if let Some(directory) = std::env::var_os("ISSUERELAY_ACCEPTANCE_EVIDENCE") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let filename = format!("{}-{session}.json", kind.as_str());
        std::fs::write(
            directory.join(filename),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    }
    let archive = engine.handoff(verification_id).unwrap().archive.unwrap();
    assert_eq!(archive.receipt.unwrap().page, "fixture.md");
    let status = acceptance.verification_status;
    server.abort();
    if status != Some(VerificationStatus::Passed) {
        let termination = handoff.memory.and_then(|memory| memory.interruption);
        host.stop(StopMode::StopRuns).await;
        panic!("real {kind:?} did not pass: {status:?}, {termination:?}");
    }
    assert!(
        acceptance
            .verification_evidence
            .iter()
            .any(|item| item.contains(DELIVERED))
    );
    assert!(
        served.load(std::sync::atomic::Ordering::SeqCst),
        "the verifier did not contact the local fixture API"
    );
    assert!(
        acceptance
            .verification_evidence
            .iter()
            .any(|item| item.contains(&marker))
    );
    if cfg!(unix) {
        assert!(
            acceptance
                .verification_evidence
                .iter()
                .any(|item| item.contains("outside-blocked"))
        );
    }
    if kind == ExecutorKind::Codex {
        let native = host.read_codex_session(verification_id).await.unwrap();
        assert_eq!(native.session_id, session);
        assert!(native.messages.iter().any(|m| m.text.contains(DELIVERED)));
    }
    host.stop(StopMode::StopRuns).await;
}

#[tokio::test]
#[ignore = "requires a local Codex sign-in and runs one synthetic inference"]
async fn codex_verification_native_run() {
    run_provider(ExecutorKind::Codex).await;
}

#[tokio::test]
#[ignore = "requires a local Claude Code sign-in and runs one synthetic inference"]
async fn claude_verification_native_run() {
    run_provider(ExecutorKind::Claude).await;
}
