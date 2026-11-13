//! Local workflow metadata, executor choice, and parsing of bounded executor evidence.

use std::path::PathBuf;

use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Binding, Effort, ExecutorKind, LeanStatus, PermissionProfile, Settings};
use crate::engine::PlanProgress;
use crate::executor::outcome::is_session_id;
use crate::model::{JobState, RunMode};
use crate::text::is_invisible;

const MAX_ISSUE_KEY_BYTES: usize = 64;
const MAX_PROVIDER_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHECKPOINT_BYTES: usize = 64 * 1024;
pub const MAX_GIT_CHANGES: usize = 128;
pub const MAX_GIT_CHANGE_BYTES: usize = 512;

/// The context confirmed for a Linear issue. References point to source material without copying it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IssueContext {
    pub revision: u64,
    pub origin_executor: Option<ExecutorKind>,
    pub preferred_executor: Option<ExecutorKind>,
    pub request: String,
    // Older briefs and run snapshots may lack history and must remain readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_history: Option<String>,
    pub scope: String,
    pub acceptance: Vec<String>,
    pub source_refs: Vec<String>,
}

impl IssueContext {
    pub(crate) fn has_conversation_history(&self) -> bool {
        self.conversation_history
            .as_deref()
            .is_some_and(|history| !history.trim().is_empty())
    }

    pub(crate) fn require_conversation_history(&self) -> Result<()> {
        ensure!(
            self.has_conversation_history(),
            "conversation_history is required for a task brief. Provide the task-relevant current conversation text before desktop confirmation."
        );
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        validate_text("request", &self.request, 16 * 1024)?;
        if let Some(history) = &self.conversation_history {
            validate_text("conversation history", history, 96 * 1024)?;
        }
        validate_text("scope", &self.scope, 16 * 1024)?;
        validate_list("acceptance", &self.acceptance, 32, 2048)?;
        validate_list("source_refs", &self.source_refs, 32, 1024)
    }
}

/// The exact brief and destination shown before desktop approval.
#[derive(Debug, Clone, Serialize)]
pub struct DispatchPreview {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    pub issue_uuid: String,
    pub issue_identifier: String,
    pub title: String,
    pub mode: crate::model::RunMode,
    pub context: Option<IssueContext>,
    pub executor: ExecutorKind,
    pub directory: PathBuf,
    pub model: Option<String>,
    pub effort: Option<crate::config::Effort>,
    pub permission: PermissionProfile,
    pub lean: LeanStatus,
    pub wiki_root: Option<PathBuf>,
}

/// The execution settings bound to a desktop approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchDestination {
    pub binding_id: String,
    pub directory: PathBuf,
    pub canonical_directory: PathBuf,
    pub executor: ExecutorKind,
    pub mode: RunMode,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub permission: PermissionProfile,
    pub lean: LeanStatus,
    pub wiki_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub claude_ultracode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationStrategy {
    ResumeSession,
    Handoff,
}

/// The exact context and evidence shown before approving a continued run.
#[derive(Debug, Clone, Serialize)]
pub struct ContinuationPreview {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    pub source_job_id: i64,
    pub issue_uuid: String,
    pub issue_identifier: String,
    pub title: String,
    pub executor: ExecutorKind,
    pub source_executor: ExecutorKind,
    pub strategy: ContinuationStrategy,
    pub mode: RunMode,
    pub directory: PathBuf,
    pub model: Option<String>,
    pub effort: Option<Effort>,
    pub permission: PermissionProfile,
    pub lean: LeanStatus,
    pub claude_ultracode: bool,
    pub context: Option<IssueContext>,
    pub wiki_root: Option<PathBuf>,
    pub evidence: String,
}

/// An operator action in the control service. Each variant names exactly one existing target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlAction {
    Dispatch { issue_identifier: String },
    Continue { job_id: i64, executor: ExecutorKind },
    Retry { job_id: i64 },
    Stop { job_id: i64 },
    RecordLocalCompletion { job_id: i64 },
    MarkDone { job_id: i64 },
    CompletePlan { job_id: i64 },
    SaveBinding { binding: ControlBinding },
    RemoveBinding { binding_id: String },
    SetSettings { changes: ControlSettings },
}

/// Binding fields an operator can review in a chat. Prompt overrides stay with the desktop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlBinding {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    pub directory: PathBuf,
    pub executor: Option<ExecutorKind>,
    pub permission: Option<PermissionProfile>,
    pub model: Option<String>,
    #[serde(default = "control_default_lean")]
    pub lean: bool,
    pub effort: Option<Effort>,
    #[serde(default)]
    pub labels: Vec<String>,
}

fn control_default_lean() -> bool {
    true
}

impl From<ControlBinding> for Binding {
    fn from(binding: ControlBinding) -> Self {
        Self {
            id: binding.id,
            project_id: binding.project_id,
            project_name: binding.project_name,
            directory: binding.directory,
            executor: binding.executor,
            permission: binding.permission,
            model: binding.model,
            prompt_override: None,
            lean: binding.lean,
            effort: binding.effort,
            labels: binding.labels,
        }
    }
}

impl From<&Binding> for ControlBinding {
    fn from(binding: &Binding) -> Self {
        Self {
            id: binding.id.clone(),
            project_id: binding.project_id.clone(),
            project_name: binding.project_name.clone(),
            directory: binding.directory.clone(),
            executor: binding.executor,
            permission: binding.permission,
            model: binding.model.clone(),
            lean: binding.lean,
            effort: binding.effort,
            labels: binding.labels.clone(),
        }
    }
}

/// The settings that can be edited in chat. Security and machine settings are separate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSettings {
    pub poll_interval_secs: Option<u64>,
    pub max_concurrent: Option<usize>,
    pub run_timeout_mins: Option<u64>,
    pub stall_timeout_mins: Option<u64>,
    pub language: Option<String>,
}

impl ControlSettings {
    pub(crate) fn snapshot(settings: &Settings) -> Self {
        Self {
            poll_interval_secs: Some(settings.poll_interval_secs),
            max_concurrent: Some(settings.max_concurrent),
            run_timeout_mins: Some(settings.run_timeout_mins),
            stall_timeout_mins: Some(settings.stall_timeout_mins),
            language: Some(settings.language.clone()),
        }
    }

    pub(crate) fn apply(&self, settings: &mut Settings) {
        if let Some(value) = self.poll_interval_secs {
            settings.poll_interval_secs = value;
        }
        if let Some(value) = self.max_concurrent {
            settings.max_concurrent = value;
        }
        if let Some(value) = self.run_timeout_mins {
            settings.run_timeout_mins = value;
        }
        if let Some(value) = self.stall_timeout_mins {
            settings.stall_timeout_mins = value;
        }
        if let Some(value) = &self.language {
            settings.language.clone_from(value);
        }
    }
}

/// Only the configuration fields relevant to this operation are shown in the preview.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlConfigChange {
    Binding {
        before: Option<ControlBinding>,
        after: Option<ControlBinding>,
        effective_permission: Option<PermissionProfile>,
        effective_lean: Option<LeanStatus>,
    },
    Settings {
        before: ControlSettings,
        after: ControlSettings,
    },
}

/// An action awaiting approval. The control adapter keeps the token out of the model response.
#[derive(Debug, Serialize)]
pub struct ControlActionPreview {
    pub token: String,
    pub action: ControlAction,
    pub issue_identifier: String,
    pub job_id: Option<i64>,
    pub current_state: Option<JobState>,
    pub dispatch: Option<DispatchPreview>,
    pub continuation: Option<ContinuationPreview>,
    pub linear_current_state: Option<String>,
    pub linear_target_state: Option<String>,
    pub plan_progress: Option<PlanProgress>,
    pub config_change: Option<ControlConfigChange>,
    pub expires_in_seconds: u64,
}

/// What the host actually accepted. Starting and stopping executors remain asynchronous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlActionOutcome {
    DispatchApproved,
    ContinuationQueued,
    RetryQueuedForReview,
    StopRequested,
    LocalCompletionRecorded,
    Done,
    PlanDone,
    BindingSaved,
    BindingRemoved,
    SettingsUpdated,
}

#[derive(Debug, Serialize)]
pub struct ControlActionResult {
    pub action: ControlAction,
    pub issue_identifier: String,
    pub job_id: Option<i64>,
    pub result: ControlActionOutcome,
}

/// The reviewed evidence and destination saved for a single continuation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationIntent {
    pub source_job_id: i64,
    pub executor: ExecutorKind,
    pub strategy: ContinuationStrategy,
    pub evidence: String,
    pub destination: DispatchDestination,
}

impl ContinuationIntent {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.source_job_id > 0, "invalid source run");
        ensure!(
            self.executor == self.destination.executor,
            "continuation executor differs from its reviewed destination"
        );
        validate_text("continuation evidence", &self.evidence, 512 * 1024)
    }
}

/// An executor's progress report. Verification in this record is the executor's own account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub summary: String,
    pub done: Vec<String>,
    pub next: Vec<String>,
    pub verification: Vec<String>,
    pub blockers: Vec<String>,
}

impl Checkpoint {
    pub fn validate(&self) -> Result<()> {
        validate_text("summary", &self.summary, 1024)?;
        validate_list("done", &self.done, 16, 512)?;
        validate_list("next", &self.next, 16, 512)?;
        validate_list("verification", &self.verification, 16, 512)?;
        validate_list("blockers", &self.blockers, 16, 512)
    }
}

/// A confirmed provider quota event. The reset time is optional provider data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interruption {
    pub code: String,
    pub reset_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorSource {
    Explicit,
    Binding,
    Origin,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionDecision {
    pub executor: ExecutorKind,
    pub source: ExecutorSource,
}

/// The observed worktree state. Entries name changed paths and their status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitFacts {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub changed: Vec<String>,
}

impl GitFacts {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(head) = &self.head {
            ensure!(is_git_object_id(head), "invalid Git object ID");
        }
        if let Some(branch) = &self.branch {
            validate_text("Git branch", branch, 1024)?;
        }
        validate_list(
            "Git changes",
            &self.changed,
            MAX_GIT_CHANGES,
            MAX_GIT_CHANGE_BYTES,
        )
    }
}

/// The most recent local facts about one run, independent of its provider session and log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMemory {
    #[serde(default)]
    pub context: Option<IssueContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiki_root: Option<PathBuf>,
    #[serde(default)]
    pub milestones: Vec<CheckpointMilestone>,
    pub decision: Option<ExecutionDecision>,
    pub checkpoint: Option<Checkpoint>,
    pub interruption: Option<Interruption>,
    pub git: Option<GitFacts>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMilestone {
    pub at: DateTime<Utc>,
    pub summary: String,
}

impl RunMemory {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.milestones.len() <= 16 && self.milestones.iter().all(|m| m.summary.len() <= 512),
            "invalid checkpoint history"
        );
        if let Some(context) = &self.context {
            context.validate()?;
        }
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint.validate()?;
        }
        if let Some(interruption) = &self.interruption {
            ensure!(
                quota_code(&interruption.code).is_some()
                    || interruption.code == "claude_quota_rejected",
                "invalid quota evidence code"
            );
        }
        if let Some(git) = &self.git {
            git.validate()?;
        }
        Ok(())
    }
}

/// The recorded page and commit of a task's Wiki archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveReceipt {
    pub wiki_root: PathBuf,
    pub page: String,
    pub commit: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

impl ArchiveReceipt {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.wiki_root.is_absolute(), "invalid archive Wiki root");
        validate_text("archive page", &self.page, 1024)?;
        ensure!(
            self.page.trim() == self.page
                && !self.page.contains([':', '\\'])
                && !self.page.chars().any(char::is_control)
                && self
                    .page
                    .split('/')
                    .all(|part| { !part.is_empty() && !part.ends_with(['.', ' ']) }),
            "archive page must be a relative Wiki path with forward slashes"
        );
        if let Some(commit) = &self.commit {
            ensure!(is_git_object_id(commit), "invalid archive commit ID");
        }
        Ok(())
    }
}

fn is_git_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The configured Wiki destination, instructions, and an optional archive receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WikiArchive {
    pub root: String,
    pub prompt: String,
    pub receipt: Option<ArchiveReceipt>,
}

/// The data and prompts a new session needs to inspect or continue a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    pub job_id: i64,
    pub issue_identifier: String,
    pub executor: Option<ExecutorKind>,
    pub active: bool,
    pub context: Option<IssueContext>,
    pub memory: Option<RunMemory>,
    pub prompt: String,
    pub review_prompt: String,
    pub claimed_by: Option<ExecutorKind>,
    pub claim_id: Option<String>,
    #[serde(default)]
    pub archive: Option<WikiArchive>,
}

/// Canonical local key for a Linear issue identifier or UUID.
pub fn issue_key(raw: &str) -> Result<String> {
    let key = raw.trim();
    ensure!(
        raw.len() <= MAX_ISSUE_KEY_BYTES,
        "Linear issue identifier or UUID is too long"
    );
    if is_session_id(key) {
        return Ok(key.to_ascii_lowercase());
    }
    let Some((team, number)) = key.split_once('-') else {
        anyhow::bail!("expected a Linear issue identifier or UUID");
    };
    ensure!(
        !team.is_empty()
            && team.bytes().all(|byte| byte.is_ascii_alphanumeric())
            && !number.is_empty()
            && number.bytes().all(|byte| byte.is_ascii_digit()),
        "expected a Linear issue identifier or UUID"
    );
    Ok(key.to_ascii_uppercase())
}

/// Workflow MCP routes are keyed by the same human-readable identifier the scheduler uses.
pub fn issue_identifier(raw: &str) -> Result<String> {
    let key = issue_key(raw)?;
    ensure!(
        !is_session_id(&key),
        "use the Linear issue identifier, such as DEMO-42"
    );
    Ok(key)
}

/// The one executor selection used for readiness checks, queue entries and execution.
pub fn resolve_executor(
    context: Option<&IssueContext>,
    binding: Option<ExecutorKind>,
    global: ExecutorKind,
) -> ExecutionDecision {
    let (executor, source) = if let Some(executor) = context.and_then(|ctx| ctx.preferred_executor)
    {
        (executor, ExecutorSource::Explicit)
    } else if let Some(executor) = binding {
        (executor, ExecutorSource::Binding)
    } else if let Some(executor) = context.and_then(|ctx| ctx.origin_executor) {
        (executor, ExecutorSource::Origin)
    } else {
        (global, ExecutorSource::Global)
    };
    ExecutionDecision { executor, source }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointEnvelope {
    issuerelay_checkpoint: Checkpoint,
}

fn checkpoint_text(text: &str) -> Option<Checkpoint> {
    text.lines().rev().find_map(|line| {
        let line = line.trim();
        if line.len() > MAX_CHECKPOINT_BYTES || !line.starts_with('{') {
            return None;
        }
        let envelope: CheckpointEnvelope = serde_json::from_str(line).ok()?;
        envelope.issuerelay_checkpoint.validate().ok()?;
        Some(envelope.issuerelay_checkpoint)
    })
}

/// Extracts a checkpoint from completed assistant text in the executor's JSON event stream.
pub fn checkpoint_event(kind: ExecutorKind, line: &str) -> Option<Checkpoint> {
    if line.len() > MAX_PROVIDER_EVENT_BYTES || !line.contains("issuerelay_checkpoint") {
        return None;
    }
    let event: Value = serde_json::from_str(line).ok()?;
    match (kind, event["type"].as_str()?) {
        (ExecutorKind::Codex, "item.completed") if event["item"]["type"] == "agent_message" => {
            checkpoint_text(event["item"]["text"].as_str()?)
        }
        (ExecutorKind::Claude, "assistant")
            if event["parent_tool_use_id"].is_null() && event["message"]["role"] == "assistant" =>
        {
            event["message"]["content"]
                .as_array()?
                .iter()
                .rev()
                .find_map(|block| {
                    if block["type"] == "text" {
                        checkpoint_text(block["text"].as_str()?)
                    } else {
                        None
                    }
                })
        }
        _ => None,
    }
}

fn quota_code(raw: &str) -> Option<&'static str> {
    match raw {
        "usage_limit_reached" => Some("usage_limit_reached"),
        "usage_limit_exceeded" | "usageLimitExceeded" => Some("usage_limit_exceeded"),
        "insufficient_quota" => Some("insufficient_quota"),
        "quota_exceeded" => Some("quota_exceeded"),
        _ => None,
    }
}

fn provider_time(value: &Value) -> Option<DateTime<Utc>> {
    if let Some(seconds) = value.as_i64() {
        return (0..=253_402_300_799)
            .contains(&seconds)
            .then(|| DateTime::from_timestamp(seconds, 0))
            .flatten();
    }
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|at| at.to_utc())
}

/// Classifies an explicit provider quota signal. Prose, generic rate limits and tool output carry
/// no such evidence. The runner decides whether this signal accompanied an interrupted run.
pub fn quota_event(kind: ExecutorKind, line: &str) -> Option<Interruption> {
    if line.len() > MAX_PROVIDER_EVENT_BYTES {
        return None;
    }
    let event: Value = serde_json::from_str(line).ok()?;
    let event_type = event["type"].as_str()?;
    if kind == ExecutorKind::Claude && event_type == "rate_limit_event" {
        let info = &event["rate_limit_info"];
        if info["status"] != "rejected"
            || info["isUsingOverage"] == true
            || info["overageStatus"] == "allowed"
            || !matches!(
                info["rateLimitType"].as_str(),
                Some("five_hour" | "seven_day" | "seven_day_opus" | "seven_day_sonnet" | "overage")
            )
        {
            return None;
        }
        return Some(Interruption {
            code: "claude_quota_rejected".into(),
            reset_at: provider_time(&info["resetsAt"]),
        });
    }
    let error = match (kind, event_type) {
        (_, "error") if event["error"].is_object() => &event["error"],
        (_, "error") => &event,
        (ExecutorKind::Codex, "turn.failed") => &event["error"],
        _ => return None,
    };
    let code = ["code", "type", "codexErrorInfo"]
        .iter()
        .find_map(|name| error[*name].as_str().and_then(quota_code))?;
    Some(Interruption {
        code: code.into(),
        reset_at: provider_time(&error["reset_at"]),
    })
}

fn validate_text(field: &str, value: &str, max_bytes: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= max_bytes,
        "{field} is empty or too long"
    );
    ensure!(
        !value
            .chars()
            .any(|c| is_invisible(c) || (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))),
        "{field} contains unsupported characters"
    );
    Ok(())
}

fn validate_list(field: &str, list: &[String], max_items: usize, max_bytes: usize) -> Result<()> {
    ensure!(list.len() <= max_items, "{field} has too many items");
    list.iter()
        .try_for_each(|value| validate_text(field, value, max_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn issue_keys_are_canonical_and_reject_non_identifiers() {
        assert_eq!(issue_key(" demo-42 ").unwrap(), "DEMO-42");
        assert_eq!(
            issue_key("00000000-0000-4000-8000-00000000000A").unwrap(),
            "00000000-0000-4000-8000-00000000000a"
        );
        for key in ["", "DEMO", "-1", "DEMO-1; DELETE", "DEMO-1/2", "DEMO-1\n2"] {
            assert!(issue_key(key).is_err(), "{key:?}");
        }
    }

    #[test]
    fn issue_choice_and_binding_take_precedence_over_entry_hint() {
        let mut context = IssueContext {
            origin_executor: Some(ExecutorKind::Codex),
            preferred_executor: Some(ExecutorKind::Claude),
            ..IssueContext::default()
        };
        assert_eq!(
            resolve_executor(
                Some(&context),
                Some(ExecutorKind::Codex),
                ExecutorKind::Codex
            ),
            ExecutionDecision {
                executor: ExecutorKind::Claude,
                source: ExecutorSource::Explicit,
            }
        );
        context.preferred_executor = None;
        assert_eq!(
            resolve_executor(
                Some(&context),
                Some(ExecutorKind::Claude),
                ExecutorKind::Codex
            )
            .source,
            ExecutorSource::Binding
        );
        assert_eq!(
            resolve_executor(Some(&context), None, ExecutorKind::Claude),
            ExecutionDecision {
                executor: ExecutorKind::Codex,
                source: ExecutorSource::Origin,
            }
        );
        assert_eq!(
            resolve_executor(None, None, ExecutorKind::Claude),
            ExecutionDecision {
                executor: ExecutorKind::Claude,
                source: ExecutorSource::Global,
            }
        );
    }

    #[test]
    fn archive_pages_are_relative_and_commits_are_full_git_ids() {
        let root = tempfile::tempdir().unwrap();
        let mut receipt = ArchiveReceipt {
            wiki_root: root.path().canonicalize().unwrap(),
            page: "projects/demo/示例.md".into(),
            commit: Some("1".repeat(40)),
            recorded_at: Utc::now(),
        };
        assert!(receipt.validate().is_ok());
        for page in [
            "",
            "/demo.md",
            "../demo.md",
            "projects/../demo.md",
            "projects/.. /demo.md",
            "projects/./demo.md",
            "projects//demo.md",
            "C:demo.md",
            r"projects\demo.md",
            "projects/demo.md
new.md",
        ] {
            receipt.page = page.into();
            assert!(receipt.validate().is_err(), "{page:?}");
        }
        receipt.page = "projects/demo.md".into();
        for commit in [Some("123".into()), Some("g".repeat(40))] {
            receipt.commit = commit;
            assert!(receipt.validate().is_err());
        }
        receipt.commit = Some("a".repeat(64));
        assert!(receipt.validate().is_ok());
        receipt.commit = None;
        assert!(receipt.validate().is_ok());
    }

    fn checkpoint_text() -> String {
        json!({"issuerelay_checkpoint":{
            "summary":"Read the spec",
            "done":["Checked the fixtures"],
            "next":["Run a focused test"],
            "verification":[],
            "blockers":[]
        }})
        .to_string()
    }

    #[test]
    fn checkpoints_are_taken_only_from_completed_assistant_text() {
        let text = checkpoint_text();
        let codex = |item_type| {
            json!({"type":"item.completed","item":{"type":item_type,"text":text}}).to_string()
        };
        assert_eq!(
            checkpoint_event(ExecutorKind::Codex, &codex("agent_message"))
                .unwrap()
                .next,
            ["Run a focused test"]
        );
        assert!(checkpoint_event(ExecutorKind::Codex, &codex("command_execution")).is_none());
        let claude = |role, parent| {
            json!({
                "type":"assistant","parent_tool_use_id":parent,
                "message":{"role":role,"content":[{"type":"text","text":text}]}
            })
            .to_string()
        };
        assert_eq!(
            checkpoint_event(ExecutorKind::Claude, &claude("assistant", None))
                .unwrap()
                .summary,
            "Read the spec"
        );
        assert!(checkpoint_event(ExecutorKind::Claude, &claude("user", None)).is_none());
        assert!(
            checkpoint_event(ExecutorKind::Claude, &claude("assistant", Some("tool"))).is_none()
        );
    }

    #[test]
    fn checkpoint_protocol_rejects_unknown_fields_and_oversized_values() {
        let mut envelope: serde_json::Value = serde_json::from_str(&checkpoint_text()).unwrap();
        let codex = |text: String| {
            json!({"type":"item.completed","item":{"type":"agent_message","text":text}}).to_string()
        };
        assert!(checkpoint_event(ExecutorKind::Codex, &codex(envelope.to_string())).is_some());
        envelope["issuerelay_checkpoint"]["unexpected"] = json!(true);
        assert!(checkpoint_event(ExecutorKind::Codex, &codex(envelope.to_string())).is_none());
        envelope["issuerelay_checkpoint"]
            .as_object_mut()
            .unwrap()
            .remove("unexpected");
        envelope["issuerelay_checkpoint"]["summary"] = json!("a".repeat(100_000));
        assert!(checkpoint_event(ExecutorKind::Codex, &codex(envelope.to_string())).is_none());
    }

    #[test]
    fn quota_requires_a_structured_provider_signal() {
        let confirmed = r#"{"type":"turn.failed","error":{"code":"usage_limit_reached"}}"#;
        let quota = quota_event(ExecutorKind::Codex, confirmed).unwrap();
        assert_eq!(quota.code, "usage_limit_reached");
        assert_eq!(quota.reset_at, None);
        for event in [
            r#"{"type":"turn.failed","error":{"code":"rate_limit_exceeded","status":429}}"#,
            r#"{"type":"error","message":"usage_limit_reached"}"#,
            r#"{"type":"error","code":"authentication_error"}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","code":"quota_exceeded"}}"#,
        ] {
            assert!(quota_event(ExecutorKind::Codex, event).is_none(), "{event}");
        }
    }

    #[test]
    fn claude_rejected_quota_preserves_provider_reset_without_using_warning() {
        let event = |status| {
            json!({"type":"rate_limit_event","rate_limit_info":{
                "status":status,"rateLimitType":"five_hour","resetsAt":1771606800,
                "overageStatus":"rejected"
            }})
            .to_string()
        };
        assert!(quota_event(ExecutorKind::Claude, &event("allowed")).is_none());
        assert!(quota_event(ExecutorKind::Claude, &event("allowed_warning")).is_none());
        assert_eq!(
            quota_event(ExecutorKind::Claude, &event("rejected"))
                .unwrap()
                .reset_at,
            DateTime::from_timestamp(1771606800, 0)
        );
        let missing_reset = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"seven_day"}}"#;
        assert_eq!(
            quota_event(ExecutorKind::Claude, missing_reset)
                .unwrap()
                .reset_at,
            None
        );
    }
}
