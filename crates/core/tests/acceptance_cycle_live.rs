//! Opt-in four-session Codex cycle with synthetic Linear, operator approvals and local delivery.
mod common;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use common::{fake_linear::FakeLinear, issue};
use issuerelay_core::{
    config::{Binding, Config, Effort, ExecutorKind, PermissionProfile},
    doctor::{ExecutorReport, ReadinessProbe},
    engine::{EngineEvent, JobRunner, ProcessJobRunner},
    env::ExecEnv,
    executor::{CommandSpec, outcome::is_session_id, resolve_program},
    host::{AuthMethod, Connector, Host, HostParts, LinearSetup, OpenUrl, StopMode},
    ipc::client,
    linear::LinearApi,
    model::{Job, JobState, RunMode},
    paths::AppPaths,
    runner::{RunHooks, RunLimits, RunOutput, SpawnHook},
    workflow::{ControlAction, IssueContext},
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const IDENTIFIER: &str = "DEMO-1";
const CHECKER: &str = "from pathlib import Path\nvalue = Path('delivery.txt').read_text().strip()\nprint('expected=ready actual=' + value)\nraise SystemExit(0 if value == 'ready' else 1)\n";

// This read-only stdio MCP substitutes only the task data needed by the normal developer
// prompt. It cannot reach real Linear or write a status/comment. The production prompt and
// native output are never replaced by the test.
const LINEAR_FIXTURE: &str = r#"
import json, pathlib, sys
root = pathlib.Path(__file__).parent
tools = [
  {"name":"get_issue","description":"Read the synthetic DEMO issue, including description and parent.","inputSchema":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}},
  {"name":"list_comments","description":"Read synthetic comments for the DEMO issue.","inputSchema":{"type":"object","properties":{"issueId":{"type":"string"}}}}
]
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request.get("method")
    if method == "initialize":
        result = {"protocolVersion":request["params"]["protocolVersion"],"capabilities":{"tools":{}},"serverInfo":{"name":"demo-linear-read-only","version":"1"}}
    elif method == "ping":
        result = {}
    elif method == "tools/list":
        result = {"tools":tools}
    elif method == "tools/call":
        params = request["params"]
        with (root / "linear-calls.jsonl").open("a") as log:
            log.write(json.dumps(params) + "\n")
        name = params["name"]
        snapshot = json.loads((root / "issue.json").read_text())
        if name == "get_issue":
            text = json.dumps(snapshot)
        elif name == "list_comments":
            text = json.dumps(snapshot.get("comments", []))
        else:
            print(json.dumps({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"This fixture is read-only"}}), flush=True)
            continue
        result = {"content":[{"type":"text","text":text}]}
    else:
        print(json.dumps({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"Unsupported synthetic operation"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc":"2.0","id":request["id"],"result":result}), flush=True)
"#;

struct FixtureConnector(FakeLinear);

#[async_trait::async_trait]
impl Connector for FixtureConnector {
    async fn stored(&self) -> Result<Option<(Arc<dyn LinearApi>, AuthMethod)>> {
        Ok(Some((Arc::new(self.0.clone()), AuthMethod::OAuth)))
    }

    async fn login(&self, _: OpenUrl) -> Result<Arc<dyn LinearApi>> {
        anyhow::bail!("the fixture never authenticates to Linear")
    }

    async fn api_key(&self, _: &str) -> Result<Arc<dyn LinearApi>> {
        anyhow::bail!("the fixture never stores a Linear key")
    }

    async fn logout(&self) -> Result<()> {
        Ok(())
    }
}

struct FixtureReadiness;

#[async_trait::async_trait]
impl ReadinessProbe for FixtureReadiness {
    async fn executor_reports(&self, _: bool) -> Vec<ExecutorReport> {
        vec![ExecutorReport {
            kind: ExecutorKind::Codex,
            program: resolve_program(ExecutorKind::Codex).ok(),
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

struct ObservedRunner {
    repository: PathBuf,
    fixture_mcp: PathBuf,
    python: PathBuf,
    evidence: PathBuf,
    specs: Mutex<Vec<CommandSpec>>,
    provider: ProcessJobRunner,
}

#[async_trait::async_trait]
impl JobRunner for ObservedRunner {
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
        mut spec: CommandSpec,
        kind: ExecutorKind,
        log: PathBuf,
        limits: RunLimits,
        cancel: CancellationToken,
        hooks: RunHooks,
    ) -> RunOutput {
        assert_eq!(kind, ExecutorKind::Codex);
        let original = spec.clone();
        if spec.cwd == self.repository {
            // Help-checked with codex-cli 0.159.2 on 2026-11-15. This test-only isolation
            // replaces user integrations with one read-only synthetic MCP, not permissions,
            // the engine's stdin, the provider result, session IDs or the process hooks.
            let stdin = spec.args.pop();
            assert_eq!(stdin.as_deref(), Some("-"));
            spec.args.extend(
                [
                    "--ignore-user-config",
                    "--ignore-rules",
                    "--disable",
                    "hooks",
                    "--disable",
                    "plugins",
                    "--disable",
                    "apps",
                    "--disable",
                    "memories",
                    "--disable",
                    "skill_mcp_dependency_install",
                    "-c",
                    "project_doc_max_bytes=0",
                    "-c",
                    "skills.include_instructions=false",
                ]
                .map(String::from),
            );
            let args = serde_json::to_string(&[self.fixture_mcp.to_string_lossy()]).unwrap();
            spec.args.extend([
                "-c".into(),
                format!(
                    "mcp_servers={{linear={{command={},args={args},required=true,startup_timeout_sec=30,default_tools_approval_mode=\"approve\"}}}}",
                    serde_json::to_string(&self.python.to_string_lossy()).unwrap()
                ),
            ]);
            spec.args.extend(stdin);
        }
        let index = {
            let mut specs = self.specs.lock().unwrap();
            let index = specs.len();
            specs.push(spec.clone());
            index
        };
        std::fs::write(
            self.evidence.join(format!("native-command-{index}.json")),
            serde_json::to_vec_pretty(&json!({
                "program":spec.program,"args":spec.args,"cwd":spec.cwd,"stdin":spec.stdin,
                "production_args":original.args,"production_stdin":original.stdin,
                "test_only_read_only_linear_configuration":spec.cwd == self.repository,
            }))
            .unwrap(),
        )
        .unwrap();
        // Every stage reaches the actual provider; no synthetic RunOutput or session event.
        self.provider
            .run_with_output(spec, kind, log, limits, cancel, hooks)
            .await
    }
}

fn git(directory: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new(if cfg!(windows) { "git.exe" } else { "git" })
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(directory)
        .output()?;
    ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

fn context(request: &str) -> IssueContext {
    IssueContext {
        request: request.into(),
        conversation_history: Some("This is an intentional failing-then-repaired synthetic cycle, entirely local.".into()),
        scope: "Local delivery.txt and check_delivery.py in this disposable fixture only. Do not write Linear, use real projects, contact Git remotes, push or deploy.".into(),
        acceptance: vec!["Run python3 check_delivery.py. A ready value passes; any other value fails. Report its actual exit status and output as evidence.".into()],
        source_refs: vec!["delivery.txt".into(), "check_delivery.py".into()],
        origin_executor: Some(ExecutorKind::Codex),
        ..IssueContext::default()
    }
}

fn snapshot(root: &Path, task: &IssueContext, comments: Value) -> Result<()> {
    std::fs::write(
        root.join("issue.json"),
        serde_json::to_vec_pretty(&json!({
            "id":"1","identifier":IDENTIFIER,"title":"Synthetic local acceptance cycle",
            "description":task.request,"parent":null,"comments":comments,
            "deliveryScope":task.scope,"acceptance":task.acceptance,
        }))?,
    )?;
    Ok(())
}

async fn stage_context(host: &Host, expected: u64, context: &IssueContext) -> Result<IssueContext> {
    let value = client::call(
        &host.paths().ipc_endpoint(),
        "workflow.set_context",
        json!({"issue_identifier":IDENTIFIER,"expected_revision":expected,"context":context}),
    )
    .await?;
    Ok(serde_json::from_value(value)?)
}

#[derive(serde::Deserialize)]
struct ApprovedAction {
    job_id: Option<i64>,
    result: String,
}

async fn approve(host: &Host, action: ControlAction) -> Result<ApprovedAction> {
    let endpoint = host.paths().ipc_endpoint();
    let preview = client::call(
        &endpoint,
        "workflow.prepare_action",
        json!({"action":action}),
    )
    .await?;
    let token = preview["token"].as_str().context("preview token")?;
    let result = client::call(&endpoint, "workflow.commit_action", json!({"token":token})).await?;
    ensure!(
        client::call(&endpoint, "workflow.commit_action", json!({"token":token}))
            .await
            .is_err(),
        "a token was reusable"
    );
    Ok(serde_json::from_value(result)?)
}

async fn finished(host: &Host, id: i64, evidence: &Path, phase: &str) -> Result<Job> {
    println!("Native cycle phase {phase}: waiting for run {id}");
    let mut events = host.engine().subscribe();
    let mut observed_running_session = None;
    let mut rejected_claim_while_running = false;
    let mut announced_session = false;
    let result = tokio::time::timeout(Duration::from_secs(240), async {
        loop {
            let value = client::call(
                &host.paths().ipc_endpoint(),
                "workflow.run",
                json!({"id":id}),
            )
            .await?;
            let job: Job = serde_json::from_value(value["job"].clone())?;
            while let Ok(event) = events.try_recv() {
                if let EngineEvent::Job { job } = event
                    && job.id == id
                    && job.state == JobState::Running
                    && let Some(session) = job.session_id.filter(|session| is_session_id(session))
                {
                    observed_running_session = Some(session);
                }
            }
            if job.state == JobState::Running {
                if let Some(session) = job
                    .session_id
                    .as_deref()
                    .filter(|session| is_session_id(session))
                {
                    observed_running_session = Some(session.to_owned());
                }
                rejected_claim_while_running |= client::call(
                    &host.paths().ipc_endpoint(),
                    "workflow.claim",
                    json!({"job_id":id,"executor":"codex"}),
                )
                .await
                .is_err();
            }
            if !announced_session && let Some(session) = &observed_running_session {
                println!("Native cycle phase {phase}: run {id}, native session {session}");
                announced_session = true;
            }
            if job.finished_at.is_some() && !value["handoff"]["active"].as_bool().unwrap_or(true) {
                let session = job
                    .session_id
                    .as_deref()
                    .context("missing native session")?;
                ensure!(is_session_id(session), "invalid native session");
                ensure!(
                    observed_running_session.as_deref() == Some(session),
                    "no matching native session observed while running"
                );
                ensure!(
                    rejected_claim_while_running,
                    "active writer exclusion was not observed"
                );
                std::fs::write(
                    evidence.join(format!("{phase}-run.json")),
                    serde_json::to_vec_pretty(&value)?,
                )?;
                let native = host.read_codex_session(id).await?;
                ensure!(
                    native.session_id == session && !native.messages.is_empty(),
                    "native session readback mismatch"
                );
                std::fs::write(
                    evidence.join(format!("{phase}-native.json")),
                    serde_json::to_vec_pretty(&native)?,
                )?;
                println!(
                    "Native cycle phase {phase}: run {id} terminal {:?}, report {:?}",
                    job.state, job.report
                );
                return Ok::<_, anyhow::Error>(job);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("native cycle phase timed out")??;
    Ok(result)
}

fn checked_by_native_command(
    host: &Host,
    id: i64,
    exit_code: i64,
    expected_output: &str,
) -> Result<()> {
    let log = std::fs::read_to_string(host.paths().logs_dir().join(format!("{id}.jsonl")))?;
    let observed = log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|value| {
            let event = value["text"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .unwrap_or(value);
            let item = &event["item"];
            event["type"] == "item.completed"
                && item["type"] == "command_execution"
                && item["command"]
                    .as_str()
                    .is_some_and(|command| command.contains("check_delivery.py"))
                && item["exit_code"].as_i64() == Some(exit_code)
                && item["aggregated_output"]
                    .as_str()
                    .is_some_and(|output| output.contains(expected_output))
        });
    ensure!(
        observed,
        "native run {id} did not record the actual checker command, exit {exit_code} and output {expected_output}"
    );
    Ok(())
}

fn synthetic_issue_reads(root: &Path) -> Result<usize> {
    Ok(std::fs::read_to_string(root.join("linear-calls.jsonl"))?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value["name"] == "get_issue")
        .count())
}

fn require_repair_reference(
    prompt: &str,
    source_job_id: i64,
    failed_job_id: i64,
    delivery: &str,
    evidence: &[String],
) -> Result<()> {
    // The engine appends one JSON object. Decode it before comparing strings that may
    // contain quotes, backslashes or newlines; a raw substring check loses that boundary.
    let start = prompt
        .rfind("\n{")
        .context("repair prompt has no trailing reference object")?;
    let reference: Value = serde_json::from_str(prompt[start..].trim())
        .context("repair prompt reference is not one complete JSON object")?;
    ensure!(reference.is_object(), "repair reference must be an object");
    ensure!(
        reference["previous_development_run"].as_i64() == Some(source_job_id)
            && reference["failed_verification_run"].as_i64() == Some(failed_job_id)
            && reference["previous_delivery"] == delivery,
        "repair prompt lost the exact source run, failed run or delivery reference"
    );
    let actual_evidence: Vec<String> =
        serde_json::from_value(reference["verification_finding"]["evidence"].clone())
            .context("repair reference evidence must be a complete array of strings")?;
    ensure!(
        reference["verification_finding"]["status"] == "failed"
            && actual_evidence.as_slice() == evidence,
        "repair prompt lost the exact failed-verification evidence"
    );
    Ok(())
}

#[test]
fn repair_reference_preserves_escaped_evidence_and_exact_run_links() {
    let delivery = "a".repeat(40);
    let evidence = vec![
        r"The fixture contains literal broken\n and a demo\fixture path".to_owned(),
        "The checker printed \"broken\"".to_owned(),
        "First line\nSecond line".to_owned(),
    ];
    assert!(evidence[0].contains(r"\n"));
    assert!(evidence[1].contains('"'));
    assert!(evidence[2].contains('\n'));
    let reference = json!({
        "previous_development_run":7,"failed_verification_run":8,"previous_delivery":delivery,
        "return_reason":"Repair the synthetic fixture",
        "verification_finding":{"status":"failed","summary":"Fixture mismatch","evidence":evidence},
    });
    let prompt = issuerelay_core::prompt::with_continuation_evidence(
        "Synthetic approved task\n".into(),
        &serde_json::to_string_pretty(&reference).unwrap(),
    );
    require_repair_reference(&prompt, 7, 8, &delivery, &evidence).unwrap();

    let mut escaped_newline = evidence.clone();
    escaped_newline[2] = r"First line\nSecond line".into();
    for changed in [
        json!({"previous_development_run":6}),
        json!({"previous_development_run":"7"}),
        json!({"failed_verification_run":9}),
        json!({"previous_delivery":"b".repeat(40)}),
        json!({"verification_finding":{"status":"passed","evidence":evidence}}),
        json!({"verification_finding":{"status":"failed","evidence":["Only a keyword: broken"]}}),
        json!({"verification_finding":{"status":"failed","evidence":evidence.iter().rev().collect::<Vec<_>>()}}),
        json!({"verification_finding":{"status":"failed","evidence":escaped_newline}}),
        json!({"verification_finding":{"status":"failed","evidence":"not an array"}}),
        json!({"verification_finding":{"status":"failed"}}),
    ] {
        let mut altered = reference.clone();
        altered
            .as_object_mut()
            .unwrap()
            .extend(changed.as_object().unwrap().clone());
        let prompt = issuerelay_core::prompt::with_continuation_evidence(
            format!(
                "Synthetic preface already contains all raw evidence:\n{}\n",
                evidence.join("\n")
            ),
            &serde_json::to_string_pretty(&altered).unwrap(),
        );
        assert!(require_repair_reference(&prompt, 7, 8, &delivery, &evidence).is_err());
    }
    assert!(require_repair_reference("No reference JSON", 7, 8, &delivery, &evidence).is_err());
    assert!(
        require_repair_reference(
            &(prompt + "trailing non-JSON text"),
            7,
            8,
            &delivery,
            &evidence
        )
        .is_err()
    );
}

async fn exercise(
    host: &Host,
    root: &Path,
    linear: &FakeLinear,
    runner: &ObservedRunner,
) -> Result<Value> {
    let initial = context(
        "Create the intentional first failing fixture: write exactly broken plus newline to delivery.txt, verify that exact local value, and commit the change on the fixture branch. The later independent acceptance deliberately expects ready. Finish with local_complete; this is a fixture setup task, not an acceptance pass.",
    );
    snapshot(root, &initial, json!([]))?;
    let mut saved = stage_context(host, 0, &initial).await?;
    approve(
        host,
        ControlAction::Dispatch {
            issue_identifier: IDENTIFIER.into(),
        },
    )
    .await?;
    host.engine().tick().await?;
    let source = host
        .engine()
        .list_jobs(10)?
        .into_iter()
        .find(|job| job.mode == RunMode::Implement)
        .context("initial development run")?;
    let source = finished(host, source.id, root, "initial-development").await?;
    ensure!(
        source.state == JobState::LocalComplete,
        "initial development failed: {:?}",
        source.reason
    );
    ensure!(
        std::fs::read_to_string(runner.repository.join("delivery.txt"))?.trim() == "broken",
        "initial delivery was not produced by Codex"
    );
    let first_delivery = git(&runner.repository, &["rev-parse", "HEAD"])?;
    let seed = git(&runner.repository, &["rev-list", "--max-parents=0", "HEAD"])?;
    ensure!(
        std::fs::read_to_string(runner.repository.join("check_delivery.py"))? == CHECKER,
        "initial development altered the acceptance checker"
    );
    ensure!(
        git(
            &runner.repository,
            &["diff", "--name-only", &seed, &first_delivery]
        )? == "delivery.txt",
        "initial development changed files outside its approved scope"
    );
    let initial_issue_reads = synthetic_issue_reads(root)?;
    ensure!(
        initial_issue_reads > 0,
        "initial developer did not read synthetic Linear"
    );
    ensure!(
        git(&runner.repository, &["status", "--porcelain"])?.is_empty(),
        "initial worktree is dirty"
    );

    saved = stage_context(host, saved.revision, &context("Independently run the approved fixture check; expect ready. This first delivery is intentionally broken for the local workflow test. Report the observed failure truthfully; do not repair it.")).await?;
    let queued = approve(
        host,
        ControlAction::StartVerification {
            job_id: source.id,
            executor: ExecutorKind::Codex,
            delivery: first_delivery.clone(),
        },
    )
    .await?;
    ensure!(
        queued.result == "verification_queued",
        "verification not queued"
    );
    let failed = finished(
        host,
        queued.job_id.context("first verifier")?,
        root,
        "failed-verification",
    )
    .await?;
    let failed_view = host
        .engine()
        .handoff(failed.id)?
        .acceptance
        .context("first acceptance")?;
    checked_by_native_command(host, failed.id, 1, "expected=ready actual=broken")?;
    ensure!(
        failed.report.as_deref() == Some("verification_failed")
            && !failed_view.verification_evidence.is_empty(),
        "the first real verifier did not report an evidenced failure: {:?}",
        failed.reason
    );
    ensure!(
        host.engine().dispatch_preview(IDENTIFIER).await.is_err(),
        "failed acceptance allowed development before return"
    );
    let returned = approve(
        host,
        ControlAction::ReturnForRepair {
            job_id: failed.id,
            reason: "Repair the actual ready-value mismatch reported by the synthetic verifier."
                .into(),
        },
    )
    .await?;
    ensure!(
        returned.result == "returned_for_repair",
        "return was not recorded"
    );
    ensure!(
        host.engine().dispatch_preview(IDENTIFIER).await.is_err(),
        "unchanged Linear revision allowed repair"
    );

    // The controlled Linear fixture, not the application or provider, supplies the new candidate.
    linear.update("1", |issue| {
        issue.title = "Repair synthetic delivery to ready".into()
    });
    let repair_task = context(
        "Repair only the remaining mismatch: write exactly ready plus newline to delivery.txt, run python3 check_delivery.py successfully, and commit the new delivery on the fixture branch. Read the failed verification evidence supplied with this run; do not modify check_delivery.py or previous records. Finish with local_complete.",
    );
    snapshot(
        root,
        &repair_task,
        serde_json::to_value(&failed_view.verification_evidence)?,
    )?;
    saved = stage_context(host, saved.revision, &repair_task).await?;
    approve(
        host,
        ControlAction::Dispatch {
            issue_identifier: IDENTIFIER.into(),
        },
    )
    .await?;
    host.engine().tick().await?;
    let repair = host
        .engine()
        .list_jobs(10)?
        .into_iter()
        .find(|job| job.mode == RunMode::Implement && job.id != source.id)
        .context("repair development")?;
    let repair = finished(host, repair.id, root, "repair-development").await?;
    ensure!(
        repair.state == JobState::LocalComplete && repair.resume_job_id.is_none(),
        "repair did not use a fresh completed development run"
    );
    ensure!(
        host.engine()
            .handoff(repair.id)?
            .acceptance
            .context("repair association")?
            .repair_of_job_id
            == Some(failed.id),
        "repair lost the failed-verification link"
    );
    ensure!(
        host.engine().mark_done(repair.id).await.is_err(),
        "repair closed before re-verification"
    );
    ensure!(
        host.engine()
            .prepare_control_action(ControlAction::StartVerification {
                job_id: repair.id,
                executor: ExecutorKind::Codex,
                delivery: first_delivery.clone()
            })
            .await
            .is_err(),
        "old delivery allowed re-verification"
    );
    let repaired_delivery = git(&runner.repository, &["rev-parse", "HEAD"])?;
    ensure!(
        std::fs::read_to_string(runner.repository.join("check_delivery.py"))? == CHECKER,
        "repair altered the acceptance checker"
    );
    ensure!(
        git(
            &runner.repository,
            &["diff", "--name-only", &first_delivery, &repaired_delivery]
        )? == "delivery.txt",
        "repair changed files outside its approved scope"
    );
    ensure!(
        synthetic_issue_reads(root)? > initial_issue_reads,
        "repair developer did not read synthetic Linear"
    );
    ensure!(
        first_delivery != repaired_delivery
            && git(&runner.repository, &["status", "--porcelain"])?.is_empty(),
        "repair did not produce a new clean delivery"
    );
    let queued = approve(
        host,
        ControlAction::StartVerification {
            job_id: repair.id,
            executor: ExecutorKind::Codex,
            delivery: repaired_delivery.clone(),
        },
    )
    .await?;
    let passed = finished(
        host,
        queued.job_id.context("re-verifier")?,
        root,
        "passed-verification",
    )
    .await?;
    ensure!(
        passed.report.as_deref() == Some("verification_passed"),
        "real re-verifier did not pass: {:?}",
        passed.reason
    );
    checked_by_native_command(host, passed.id, 0, "expected=ready actual=ready")?;
    let specs = runner.specs.lock().unwrap();
    ensure!(
        specs.len() == 4,
        "expected four actual Codex invocations, got {}",
        specs.len()
    );
    ensure!(
        specs[0].cwd == runner.repository && specs[2].cwd == runner.repository,
        "development left the original directory"
    );
    ensure!(
        specs[1].cwd != runner.repository
            && specs[3].cwd != runner.repository
            && specs[1].cwd != specs[3].cwd,
        "verifiers did not use distinct fresh scratch directories"
    );
    require_repair_reference(
        &specs[2].stdin,
        source.id,
        failed.id,
        &first_delivery,
        &failed_view.verification_evidence,
    )?;
    ensure!(
        !specs[2].args.iter().any(|arg| arg == "resume"),
        "repair resumed an older native session"
    );
    ensure!(
        specs
            .iter()
            .all(|spec| spec.args.iter().any(|arg| arg == "gpt-6.1-sol")
                && spec
                    .args
                    .iter()
                    .any(|arg| arg == "model_reasoning_effort=ultra")),
        "the provider selection drifted"
    );
    drop(specs);
    let runs = [&source, &failed, &repair, &passed];
    let sessions: HashSet<_> = runs
        .iter()
        .filter_map(|job| job.session_id.as_deref())
        .collect();
    ensure!(
        sessions.len() == 4,
        "the four phases did not use four actual distinct sessions"
    );
    ensure!(
        linear.set_state_calls().is_empty(),
        "the engine wrote a Linear state"
    );
    ensure!(
        std::fs::read_to_string(runner.repository.join("delivery.txt"))?.trim() == "ready",
        "the original repaired delivery changed"
    );
    ensure!(
        std::fs::read_to_string(runner.repository.join("check_delivery.py"))? == CHECKER,
        "verification altered the original checker"
    );
    ensure!(
        git(&runner.repository, &["rev-parse", "HEAD"])? == repaired_delivery
            && git(&runner.repository, &["status", "--porcelain"])?.is_empty(),
        "verification modified the delivery"
    );
    let mcp_calls = std::fs::read_to_string(root.join("linear-calls.jsonl"))?;
    ensure!(
        mcp_calls
            .lines()
            .filter(|line| line.contains("get_issue"))
            .count()
            >= 2,
        "both developers did not read synthetic Linear"
    );
    ensure!(
        mcp_calls
            .lines()
            .all(|line| line.contains("get_issue") || line.contains("list_comments")),
        "a non-read-only Linear operation reached the fixture"
    );
    ensure!(
        host.engine().list_jobs(10)?.len() == 4,
        "unexpected extra cycle jobs"
    );
    let links = host
        .engine()
        .handoff(repair.id)?
        .acceptance
        .context("final repair acceptance")?;
    ensure!(
        links.verification_job_id == Some(passed.id) && links.repair_of_job_id == Some(failed.id),
        "final cycle association mismatch"
    );
    Ok(
        json!({"real_codex_sessions":4,"runs":runs,"first_delivery":first_delivery,"repaired_delivery":repaired_delivery,"final_brief_revision":saved.revision,"test_linear":"read-only stdio fixture plus in-memory Engine LinearApi","operator_approval":"synthetic operator commits through isolated host IPC, not a real client confirmation form","production_cli_difference":"development-only user-integration isolation and a read-only synthetic Linear MCP; original stdin, permission arguments, real ProcessJobRunner and output hooks preserved","real_linear_used":false,"real_business_host_used":false}),
    )
}

async fn run_cycle() -> Result<()> {
    ensure!(
        std::env::var("ISSUERELAY_NATIVE_CYCLE").as_deref() == Ok("1"),
        "set ISSUERELAY_NATIVE_CYCLE=1 to authorize four real synthetic Codex sessions"
    );
    let evidence = PathBuf::from(
        std::env::var_os("ISSUERELAY_ACCEPTANCE_EVIDENCE")
            .context("set a private external ISSUERELAY_ACCEPTANCE_EVIDENCE directory")?,
    );
    std::fs::create_dir_all(&evidence)?;
    let evidence = evidence.canonicalize()?;
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("repository root")?
        .canonicalize()?;
    ensure!(
        !evidence.starts_with(repository_root),
        "native evidence must stay outside the public repository"
    );
    let root = tempfile::Builder::new()
        .prefix("codex-acceptance-cycle-")
        .tempdir_in(&evidence)?
        .keep();
    println!("Native cycle fixture root: {}", root.display());
    let repository = root.join("repository");
    std::fs::create_dir(&repository)?;
    std::fs::write(repository.join("delivery.txt"), "seed\n")?;
    std::fs::write(repository.join("check_delivery.py"), CHECKER)?;
    git(&repository, &["init", "-q", "-b", "fixture"])?;
    git(&repository, &["config", "user.name", "Demo Fixture"])?;
    git(&repository, &["config", "commit.gpgSign", "false"])?;
    git(
        &repository,
        &["config", "user.email", "demo@example.invalid"],
    )?;
    let hooks = root.join("empty-hooks");
    std::fs::create_dir(&hooks)?;
    git(
        &repository,
        &[
            "config",
            "core.hooksPath",
            hooks.to_str().context("fixture hooks path")?,
        ],
    )?;
    git(&repository, &["add", "delivery.txt", "check_delivery.py"])?;
    git(&repository, &["commit", "-qm", "Seed synthetic cycle"])?;
    let fixture_mcp = root.join("linear_fixture.py");
    std::fs::write(&fixture_mcp, LINEAR_FIXTURE)?;
    let python = which::which("python3").context("the local fixture requires existing Python 3")?;
    let paths = AppPaths::under(&root);
    let linear = FakeLinear::new("user-1");
    linear.upsert(issue("1", "proj-1"));
    let mut config = Config::default();
    config.settings.default_executor = ExecutorKind::Codex;
    config.settings.resume_sessions = true;
    config.settings.max_concurrent = 1;
    config.settings.run_timeout_mins = 4;
    config.settings.stall_timeout_mins = 2;
    config.bindings.push(Binding {
        id:"proj-1".into(),project_id:"proj-1".into(),project_name:"Demo Project".into(),
        directory:repository.clone(),executor:Some(ExecutorKind::Codex),permission:Some(PermissionProfile::Full),
        model:Some("gpt-6.1-sol".into()),effort:Some(Effort::Ultra),lean:false,labels:vec![],
        prompt_override:Some("This is an authorized local synthetic workflow fixture. Read the synthetic Linear MCP issue and comments, then the approved task brief. Perform only the current request in this repository. Linear writes, remote Git access, pushing and deployment are outside scope. Keep check_delivery.py unchanged. Commit only the requested delivery.txt change on the existing fixture branch. The initial task intentionally prepares a failing acceptance fixture; the later repair must make the real check pass. Record actual commands and finish with the production-required local_complete JSON.".into()),
    });
    config.save(&paths.config_file())?;
    let environment = ExecEnv::inherit().shared();
    let runner = Arc::new(ObservedRunner {
        repository,
        fixture_mcp,
        python,
        evidence: root.clone(),
        specs: Mutex::new(vec![]),
        provider: ProcessJobRunner::new(environment.clone()),
    });
    let host = Host::start(
        paths,
        Arc::new(FixtureConnector(linear.clone())),
        HostParts {
            env: Some(environment),
            readiness: Some(Arc::new(FixtureReadiness)),
            job_runner: Some(runner.clone()),
            ..HostParts::default()
        },
    )
    .await?;
    let outcome = tokio::time::timeout(Duration::from_secs(1000), async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !matches!(host.linear_setup(), LinearSetup::Connected { .. }) {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .context("synthetic host connection timed out")?;
        exercise(&host, &root, &linear, &runner).await
    })
    .await
    .context("the four-session cycle timed out")
    .and_then(|result| result);
    host.stop(StopMode::StopRuns).await;
    let report = match &outcome {
        Ok(report) => json!({"result":"passed","cycle":report,"fixture_root":root}),
        Err(error) => json!({"result":"failed","error":format!("{error:#}"),"fixture_root":root}),
    };
    std::fs::write(
        root.join("cycle-result.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("Native cycle evidence: {}", root.display());
    outcome.map(|_| ())
}

#[tokio::test]
#[ignore = "requires explicit opt-in, private external evidence and four real local Codex sessions"]
async fn codex_failed_verification_repair_and_reverification_native_cycle() {
    run_cycle().await.unwrap();
}
