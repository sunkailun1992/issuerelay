//! Previews for explicit operator actions from the local control service.
use std::time::Instant;

use super::*;
use crate::config::{PermissionProfile, binding_id_for, bindings_overlap};
use crate::model::ChildIssue;
use crate::workflow::{
    ArchiveReceipt, ControlAction, ControlActionOutcome, ControlActionPreview, ControlActionResult,
    ControlBinding, ControlConfigChange, ControlSettings,
};
use crate::workflow_store as ws;

const PREVIEW_TTL: Duration = Duration::from_secs(180);
const MAX_PREVIEWS: usize = 32;
const MAX_CONTROL_BINDINGS_LABELS: usize = 32;
const MAX_CONTROL_BINDING_TEXT: usize = 512;

#[derive(Default)]
pub(super) struct PendingActions(HashMap<String, PendingAction>);

struct PendingAction {
    action: ControlAction,
    issue_identifier: String,
    prepared_at: Instant,
    check: ActionCheck,
}

enum ActionCheck {
    Dispatch(String, Arc<dyn LinearApi>),
    Continue(String, Arc<dyn LinearApi>),
    Local(Box<Job>),
    Completion(Box<ReviewedCompletion>),
    Config(Box<ReviewedConfig>),
}

struct ReviewedConfig {
    before: Config,
    after: Config,
}

pub(super) struct ReviewedCompletion {
    pub(super) job: Job,
    linear: Arc<dyn LinearApi>,
    issue: Issue,
    wiki_root: Option<PathBuf>,
    archive: Option<ArchiveReceipt>,
    pub(super) target_state: String,
    pub(super) in_testing: String,
    children: Option<Vec<ChildIssue>>,
}

pub(super) fn check_reviewed_job(job: &Job, reviewed: Option<&Job>) -> Result<()> {
    anyhow::ensure!(
        reviewed.is_none_or(|reviewed| reviewed == job),
        "the run changed; preview the action again before confirming"
    );
    Ok(())
}

fn exact_job(store: &Store, job_id: i64) -> Result<Job> {
    anyhow::ensure!(job_id > 0, "invalid run ID");
    store
        .get(job_id)?
        .with_context(|| format!("job {job_id} not found"))
}

fn require_latest_unclaimed(store: &Store, job: &Job) -> Result<()> {
    anyhow::ensure!(
        store
            .latest_for_issue(&job.issue_uuid)?
            .is_some_and(|latest| latest.id == job.id),
        "job {} is not the latest run of {}",
        job.id,
        job.issue_identifier
    );
    anyhow::ensure!(
        ws::read_claim(store.workflow_connection(), job.id)?.is_none(),
        "release the handoff before continuing"
    );
    Ok(())
}

fn current_archive(store: &Store, id: i64, root: Option<&Path>) -> Result<Option<ArchiveReceipt>> {
    root.map(|root| {
        Engine::current_archive_receipt(store.workflow_connection(), id, root)?
            .context("archive this run in the configured Wiki before marking it done")
    })
    .transpose()
}

pub(super) fn ordered_children(mut children: Vec<ChildIssue>) -> Vec<ChildIssue> {
    children.sort_unstable_by(|a, b| a.uuid.cmp(&b.uuid));
    children
}

impl Engine {
    /// Captures the action and the exact target the operator must review. This never starts or
    /// stops an executor, writes to Linear, or changes any run state.
    pub async fn prepare_control_action(
        &self,
        action: ControlAction,
    ) -> Result<ControlActionPreview> {
        let mut preview = ControlActionPreview {
            token: String::new(),
            action: action.clone(),
            issue_identifier: String::new(),
            job_id: None,
            current_state: None,
            dispatch: None,
            continuation: None,
            linear_current_state: None,
            linear_target_state: None,
            plan_progress: None,
            config_change: None,
            expires_in_seconds: PREVIEW_TTL.as_secs(),
        };
        let check = match &action {
            ControlAction::Dispatch { issue_identifier } => {
                let linear = self.linear();
                let mut dispatch = self.dispatch_preview(issue_identifier).await?;
                anyhow::ensure!(
                    Arc::ptr_eq(&linear, &self.linear()),
                    "Linear connection changed; preview the action again"
                );
                let token = std::mem::take(&mut dispatch.token);
                preview
                    .issue_identifier
                    .clone_from(&dispatch.issue_identifier);
                preview.dispatch = Some(dispatch);
                ActionCheck::Dispatch(token, linear)
            }
            ControlAction::Continue { job_id, executor } => {
                let linear = self.linear();
                let mut continuation = self.get_continuation_preview(*job_id, *executor).await?;
                anyhow::ensure!(
                    Arc::ptr_eq(&linear, &self.linear()),
                    "Linear connection changed; preview the action again"
                );
                let token = std::mem::take(&mut continuation.token);
                preview
                    .issue_identifier
                    .clone_from(&continuation.issue_identifier);
                preview.job_id = Some(*job_id);
                preview.continuation = Some(continuation);
                ActionCheck::Continue(token, linear)
            }
            ControlAction::Retry { job_id }
            | ControlAction::Stop { job_id }
            | ControlAction::RecordLocalCompletion { job_id } => {
                let store = self.store();
                let job = exact_job(&store, *job_id)?;
                if matches!(action, ControlAction::Stop { .. }) {
                    anyhow::ensure!(job.state.is_active(), "the run is not active");
                } else {
                    require_latest_unclaimed(&store, &job)?;
                    match action {
                        ControlAction::Retry { .. } => {
                            anyhow::ensure!(
                                self.inner.require_dispatch_review,
                                "a reviewed retry requires dispatch review on this host"
                            );
                            anyhow::ensure!(
                                matches!(
                                    job.state,
                                    JobState::Failed | JobState::Canceled | JobState::NeedsInfo
                                ),
                                "only failed, stopped or needs-info runs can be retried"
                            );
                        }
                        _ => anyhow::ensure!(
                            job.needs_info_implementation() && job.finished_at.is_some(),
                            "the run is not a finished implementation waiting for information"
                        ),
                    }
                }
                preview.issue_identifier.clone_from(&job.issue_identifier);
                preview.job_id = Some(job.id);
                preview.current_state = Some(job.state);
                ActionCheck::Local(Box::new(job))
            }
            ControlAction::MarkDone { job_id } | ControlAction::CompletePlan { job_id } => {
                let completion = self
                    .completion_preview(
                        *job_id,
                        matches!(action, ControlAction::CompletePlan { .. }),
                    )
                    .await?;
                let job = &completion.job;
                preview.issue_identifier.clone_from(&job.issue_identifier);
                preview.job_id = Some(job.id);
                preview.current_state = Some(job.state);
                preview.linear_current_state = Some(clean_text(&completion.issue.state));
                preview.linear_target_state = Some(completion.target_state.clone());
                preview.plan_progress = completion
                    .children
                    .as_ref()
                    .map(|children| tally(job.id, &job.issue_uuid, children, Utc::now()));
                ActionCheck::Completion(Box::new(completion))
            }
            ControlAction::SaveBinding { .. }
            | ControlAction::RemoveBinding { .. }
            | ControlAction::SetSettings { .. } => {
                let (target, change, config) = self.config_change(&action)?;
                preview.issue_identifier = target;
                preview.config_change = Some(change);
                ActionCheck::Config(Box::new(config))
            }
        };
        let token: String = self.store().workflow_connection().query_row(
            "SELECT lower(hex(randomblob(16)))",
            [],
            |row| row.get(0),
        )?;
        let mut pending = self
            .inner
            .control_actions
            .lock()
            .expect("control action lock");
        pending
            .0
            .retain(|_, action| action.prepared_at.elapsed() < PREVIEW_TTL);
        if pending.0.len() >= MAX_PREVIEWS
            && let Some(oldest) = pending
                .0
                .iter()
                .min_by_key(|(_, action)| action.prepared_at)
                .map(|(token, _)| token.clone())
        {
            pending.0.remove(&oldest);
        }
        pending.0.insert(
            token.clone(),
            PendingAction {
                action,
                issue_identifier: preview.issue_identifier.clone(),
                prepared_at: Instant::now(),
                check,
            },
        );
        preview.token = token;
        Ok(preview)
    }

    fn config_change(
        &self,
        action: &ControlAction,
    ) -> Result<(String, ControlConfigChange, ReviewedConfig)> {
        let before = self.config();
        let mut after = before.clone();
        let (target, change) = match action {
            ControlAction::SaveBinding { binding } => {
                let next = self.review_binding(&before, binding)?;
                let current = before.bindings.iter().find(|row| row.id == next.id);
                let existing = after.bindings.iter().position(|row| row.id == next.id);
                match existing {
                    Some(index) => after.bindings[index] = next.clone(),
                    None => after.bindings.push(next.clone()),
                }
                (
                    next.id.clone(),
                    ControlConfigChange::Binding {
                        before: current.map(ControlBinding::from),
                        after: Some(ControlBinding::from(&next)),
                        effective_permission: Some(after.permission_for(&next)),
                        effective_lean: Some(after.lean_for(&next, self.home())),
                    },
                )
            }
            ControlAction::RemoveBinding { binding_id } => {
                let current = before
                    .bindings
                    .iter()
                    .find(|binding| &binding.id == binding_id)
                    .context("the binding no longer exists")?;
                after.bindings.retain(|binding| &binding.id != binding_id);
                (
                    binding_id.clone(),
                    ControlConfigChange::Binding {
                        before: Some(ControlBinding::from(current)),
                        after: None,
                        effective_permission: Some(before.permission_for(current)),
                        effective_lean: Some(before.lean_for(current, self.home())),
                    },
                )
            }
            ControlAction::SetSettings { changes } => {
                anyhow::ensure!(
                    changes
                        .poll_interval_secs
                        .is_none_or(|value| value <= 86_400)
                        && changes.max_concurrent.is_none_or(|value| value <= 64)
                        && changes.run_timeout_mins.is_none_or(|value| value <= 10_080)
                        && changes
                            .stall_timeout_mins
                            .is_none_or(|value| value <= 10_080)
                        && changes
                            .language
                            .as_ref()
                            .is_none_or(|value| value.len() <= 64),
                    "chat settings must be within 86,400 poll seconds, 64 concurrent runs, \
                     10,080 timeout minutes and 64 language bytes"
                );
                changes.apply(&mut after.settings);
                (
                    "settings".into(),
                    ControlConfigChange::Settings {
                        before: ControlSettings::snapshot(&before.settings),
                        after: ControlSettings::snapshot(&after.settings),
                    },
                )
            }
            _ => bail!("the action does not change configuration"),
        };
        after.validate()?;
        require_no_expanded_full_routing(&before, &after)?;
        anyhow::ensure!(before != after, "the configuration change has no effect");
        Ok((target, change, ReviewedConfig { before, after }))
    }

    fn review_binding(&self, config: &Config, binding: &ControlBinding) -> Result<Binding> {
        let mut next = Binding::from(binding.clone());
        anyhow::ensure!(
            next.labels.len() <= MAX_CONTROL_BINDINGS_LABELS
                && [&next.id, &next.project_id, &next.project_name]
                    .into_iter()
                    .chain(&next.labels)
                    .all(|text| text.len() <= MAX_CONTROL_BINDING_TEXT),
            "binding names or routing labels are too long for chat review"
        );
        anyhow::ensure!(
            next.id == binding_id_for(&next.project_id, &next.labels),
            "binding id must match the project and first routing label"
        );
        anyhow::ensure!(
            next.directory.is_absolute() && next.directory.is_dir(),
            "the binding directory must be an existing absolute folder"
        );
        // Commit the reviewed directory, never a symlink whose target can change during approval.
        next.directory = next
            .directory
            .canonicalize()
            .context("cannot resolve binding directory")?;
        let current = config.bindings.iter().find(|row| row.id == next.id);
        anyhow::ensure!(
            current.is_none_or(|row| row.project_id == next.project_id),
            "the binding id already belongs to another project"
        );
        anyhow::ensure!(
            !config
                .bindings
                .iter()
                .any(|row| row.id != next.id && bindings_overlap(row, &next)),
            "the change would replace another binding; review that in the IssueRelay panel"
        );
        if let Some(current) = current {
            next.prompt_override.clone_from(&current.prompt_override);
        }
        if config.permission_for(&next) == PermissionProfile::Full {
            anyhow::ensure!(
                config.settings.full_access_acknowledged
                    && current.is_some_and(|row| config.permission_for(row)
                        == PermissionProfile::Full
                        && canonical_dir(&row.directory) == next.directory
                        && row.labels == next.labels
                        && row.executor == next.executor
                        && row.model == next.model
                        && row.lean == next.lean),
                "new or expanded Full Access must be configured in the IssueRelay panel"
            );
        }
        Ok(next)
    }

    fn commit_control_config(
        &self,
        action: &ControlAction,
        reviewed: ReviewedConfig,
    ) -> Result<()> {
        self.update_config_checked(|current| {
            anyhow::ensure!(
                *current == reviewed.before,
                "the configuration changed; preview the action again"
            );
            if let ControlAction::SaveBinding { binding } = action {
                let current_directory = self.review_binding(current, binding)?.directory;
                anyhow::ensure!(
                    reviewed
                        .after
                        .bindings
                        .iter()
                        .any(|row| row.id == binding.id && row.directory == current_directory),
                    "the binding directory changed; preview the action again"
                );
            }
            *current = reviewed.after;
            Ok(())
        })?;
        self.poll_now();
        Ok(())
    }

    async fn completion_preview(&self, job_id: i64, plan: bool) -> Result<ReviewedCompletion> {
        let settings = self.settings();
        let (job, archive) = {
            let store = self.store();
            let job = exact_job(&store, job_id)?;
            require_latest_unclaimed(&store, &job)?;
            if plan {
                anyhow::ensure!(
                    job.mode == RunMode::Plan
                        && job.state == JobState::NeedsInfo
                        && job.report.as_deref() == Some("planned")
                        && job.finished_at.is_some(),
                    "the run is not an open plan"
                );
            } else {
                anyhow::ensure!(
                    matches!(
                        job.state,
                        JobState::AwaitingAcceptance | JobState::LocalComplete
                    ),
                    "the run is not awaiting acceptance"
                );
            }
            (
                job,
                current_archive(&store, job_id, settings.wiki_root.as_deref())?,
            )
        };
        let linear = self.linear();
        let issue = linear
            .issue(&job.issue_uuid)
            .await?
            .filter(|issue| issue.uuid == job.issue_uuid)
            .context("the issue no longer exists in the connected Linear workspace")?;
        let children = if plan {
            anyhow::ensure!(
                issue.state_type.as_deref() != Some("canceled"),
                "the plan is canceled in Linear; settle it there"
            );
            let children = ordered_children(linear.children(&job.issue_uuid).await?);
            let progress = tally(job.id, &job.issue_uuid, &children, Utc::now());
            anyhow::ensure!(
                progress.closable,
                "the plan's child issues are not ready for closure"
            );
            Some(children)
        } else {
            anyhow::ensure!(
                same_name(&issue.state, &settings.trigger.in_testing),
                "the issue must be in {:?} in Linear before marking done",
                settings.trigger.in_testing
            );
            None
        };
        let reviewed = ReviewedCompletion {
            job,
            linear: linear.clone(),
            issue: issue.clone(),
            wiki_root: settings.wiki_root,
            archive,
            target_state: settings.trigger.done,
            in_testing: settings.trigger.in_testing,
            children,
        };
        // Both read-only prepare and write commit fail closed on a connection or local change.
        self.check_reviewed_completion(&reviewed, &linear, &issue, reviewed.children.as_deref())?;
        Ok(reviewed)
    }

    pub(super) fn check_reviewed_completion(
        &self,
        reviewed: &ReviewedCompletion,
        linear: &Arc<dyn LinearApi>,
        issue: &Issue,
        children: Option<&[ChildIssue]>,
    ) -> Result<()> {
        let settings = self.settings();
        anyhow::ensure!(
            Arc::ptr_eq(linear, &reviewed.linear)
                && Arc::ptr_eq(linear, &self.linear())
                && reviewed.issue == *issue
                && reviewed.wiki_root == settings.wiki_root
                && reviewed.target_state == settings.trigger.done
                && reviewed.in_testing == settings.trigger.in_testing
                && reviewed.children.as_deref() == children,
            "the Linear issue, child issues, or settings changed; preview the action again"
        );
        let store = self.store();
        let current = exact_job(&store, reviewed.job.id)?;
        check_reviewed_job(&current, Some(&reviewed.job))?;
        require_latest_unclaimed(&store, &current)?;
        anyhow::ensure!(
            current_archive(&store, current.id, settings.wiki_root.as_deref())? == reviewed.archive,
            "the Wiki receipt changed; preview the action again"
        );
        Ok(())
    }

    /// Consumes the selected preview after operator approval. A stale or failed attempt also
    /// consumes it, so callers must read back the current state before preparing a new action.
    pub async fn commit_control_action(&self, token: &str) -> Result<ControlActionResult> {
        anyhow::ensure!(
            token.len() == 32 && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid action preview token"
        );
        let pending = self
            .inner
            .control_actions
            .lock()
            .expect("control action lock")
            .0
            .remove(token)
            .context("action preview expired or was already used")?;
        anyhow::ensure!(
            pending.prepared_at.elapsed() < PREVIEW_TTL,
            "action preview expired"
        );
        let (job_id, result) = match (&pending.action, pending.check) {
            (
                ControlAction::Dispatch { issue_identifier },
                ActionCheck::Dispatch(token, linear),
            ) => {
                self.approve_dispatch_inner(issue_identifier, &token, Some(&linear))
                    .await?;
                (None, ControlActionOutcome::DispatchApproved)
            }
            (ControlAction::Continue { job_id, .. }, ActionCheck::Continue(token, linear)) => {
                let queued = self
                    .approve_continuation_inner(*job_id, &token, Some(&linear))
                    .await?;
                (Some(queued), ControlActionOutcome::ContinuationQueued)
            }
            (ControlAction::Retry { job_id }, ActionCheck::Local(job)) => {
                let queued = self.retry_job_inner(*job_id, Some(&job))?;
                (Some(queued), ControlActionOutcome::RetryQueuedForReview)
            }
            (ControlAction::Stop { job_id }, ActionCheck::Local(job)) => {
                self.stop_reviewed_run(&job).await?;
                (Some(*job_id), ControlActionOutcome::StopRequested)
            }
            (ControlAction::RecordLocalCompletion { job_id }, ActionCheck::Local(job)) => {
                self.record_local_completion_inner(*job_id, Some(&job))
                    .await?;
                (Some(*job_id), ControlActionOutcome::LocalCompletionRecorded)
            }
            (ControlAction::MarkDone { job_id }, ActionCheck::Completion(reviewed)) => {
                self.mark_done_inner(*job_id, Some(&reviewed)).await?;
                (Some(*job_id), ControlActionOutcome::Done)
            }
            (ControlAction::CompletePlan { job_id }, ActionCheck::Completion(reviewed)) => {
                self.complete_plan_inner(*job_id, Some(&reviewed)).await?;
                (Some(*job_id), ControlActionOutcome::PlanDone)
            }
            (ControlAction::SaveBinding { .. }, ActionCheck::Config(config)) => {
                self.commit_control_config(&pending.action, *config)?;
                (None, ControlActionOutcome::BindingSaved)
            }
            (ControlAction::RemoveBinding { .. }, ActionCheck::Config(config)) => {
                self.commit_control_config(&pending.action, *config)?;
                (None, ControlActionOutcome::BindingRemoved)
            }
            (ControlAction::SetSettings { .. }, ActionCheck::Config(config)) => {
                self.commit_control_config(&pending.action, *config)?;
                (None, ControlActionOutcome::SettingsUpdated)
            }
            _ => bail!("action preview did not match its saved target"),
        };
        Ok(ControlActionResult {
            action: pending.action,
            issue_identifier: pending.issue_identifier,
            job_id,
            result,
        })
    }

    async fn stop_reviewed_run(&self, reviewed: &Job) -> Result<()> {
        let _cycle = self.inner.cycle.lock().await;
        let id = reviewed.id;
        {
            let store = self.store();
            let job = exact_job(&store, id)?;
            check_reviewed_job(&job, Some(reviewed))?;
            if job.state != JobState::Running {
                anyhow::ensure!(
                    store.set_state_if(
                        id,
                        &[job.state],
                        JobState::Canceled,
                        Some("stopped by the user")
                    )?,
                    "the run changed; preview the action again before confirming"
                );
            }
        }
        if reviewed.state == JobState::Running {
            let running = self.inner.running.lock().expect("running lock");
            let run = running
                .get(&id)
                .filter(|run| !run.finishing && !run.cancel.is_cancelled())
                .context("the run is already finishing or stopped; read its latest status")?;
            run.cancel.cancel();
        } else {
            self.emit_job(id);
        }
        Ok(())
    }
}

fn require_no_expanded_full_routing(before: &Config, after: &Config) -> Result<()> {
    // Removing one labeled route can send its issues to a Full default, or unblock a
    // remaining Full route when an issue has multiple labels. Handle both conservatively.
    let routes_disappear = before.bindings.iter().any(|binding| {
        let current = after.bindings.iter().find(|row| row.id == binding.id);
        let label_removed = binding.labels.iter().any(|label| {
            current.is_none_or(|row| !row.labels.iter().any(|kept| same_name(label, kept)))
        });
        label_removed
            && after.bindings.iter().any(|fallback| {
                fallback.id != binding.id
                    && fallback.project_id == binding.project_id
                    && after.permission_for(fallback) == PermissionProfile::Full
            })
    });
    anyhow::ensure!(
        !routes_disappear,
        "removing this route can send more issues to a Full Access binding; review it in the IssueRelay panel"
    );
    Ok(())
}
