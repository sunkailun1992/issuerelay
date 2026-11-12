//! Desktop review of the exact task brief before it can enter an executor prompt.
use super::*;
use crate::config::Route;
use crate::workflow::{
    ContinuationIntent, ContinuationPreview, ContinuationStrategy,
    DispatchDestination as Destination, DispatchPreview, IssueContext,
};
use crate::workflow_store as ws;

const MAX_DRAFTS: usize = 128;

#[derive(Default)]
pub(super) struct ReviewState {
    drafts: HashMap<String, IssueContext>,
    // Only the latest open preview can be confirmed. Its token also binds the current brief.
    preview: Option<OpenPreview>,
    continuation: Option<OpenContinuation>,
}

#[derive(PartialEq, Eq)]
struct ReviewInput {
    updated_at: DateTime<Utc>,
    committed_revision: u64,
    context: Option<IssueContext>,
    staged: bool,
    destination: Destination,
}

struct OpenPreview {
    issue_uuid: String,
    token: String,
    input: ReviewInput,
}

#[derive(PartialEq, Eq)]
struct ContinuationInput {
    review: ReviewInput,
    intent: ContinuationIntent,
    source_executor: ExecutorKind,
    latest_job_id: i64,
}

struct OpenContinuation {
    issue_uuid: String,
    token: String,
    input: ContinuationInput,
}

impl Destination {
    fn for_issue(cfg: &Config, issue: &Issue, mode: RunMode, home: Option<&Path>) -> Option<Self> {
        let Route::Bound(binding) = cfg.route(issue) else {
            return None;
        };
        let executor = cfg.executor_for(binding);
        let permission = cfg.permission_for(binding);
        Some(Self {
            binding_id: binding.id.clone(),
            directory: binding.directory.clone(),
            canonical_directory: canonical_dir(&binding.directory),
            executor,
            mode,
            model: cfg.model_for(binding, executor).map(str::to_owned),
            effort: cfg.effort_for_executor(binding, executor),
            permission,
            lean: cfg.lean_for(binding, home),
            wiki_root: cfg.settings.wiki_root.clone(),
            claude_ultracode: executor == ExecutorKind::Claude
                && mode == RunMode::Implement
                && cfg.settings.claude_ultracode,
        })
    }
}

impl Engine {
    pub(super) fn latest_context(&self, store: &Store, key: &str) -> Result<Option<IssueContext>> {
        let review = self
            .inner
            .dispatch_review
            .lock()
            .expect("dispatch review lock");
        if let Some(draft) = review.drafts.get(key) {
            return Ok(Some(draft.clone()));
        }
        ws::read_context(store.workflow_connection(), key)
    }

    pub(super) fn stage_dispatch_context(
        &self,
        store: &Store,
        key: &str,
        draft: &IssueContext,
    ) -> Result<()> {
        let mut review = self
            .inner
            .dispatch_review
            .lock()
            .expect("dispatch review lock");
        anyhow::ensure!(
            review.drafts.contains_key(key) || review.drafts.len() < MAX_DRAFTS,
            "too many pending dispatch briefs; review an existing brief first"
        );
        // Keep the previous content for history, but do not let it start while an edit awaits
        // approval, including if the host exits before the draft has been reviewed.
        ws::clear_dispatch_approval(store.workflow_connection(), key)?;
        review.drafts.insert(key.to_owned(), draft.clone());
        if review
            .preview
            .as_ref()
            .is_some_and(|preview| preview.issue_uuid == key)
        {
            review.preview = None;
        }
        Ok(())
    }

    pub(super) fn invalidate_dispatch_preview(&self, key: &str) {
        let mut review = self
            .inner
            .dispatch_review
            .lock()
            .expect("dispatch review lock");
        if review
            .preview
            .as_ref()
            .is_some_and(|preview| preview.issue_uuid == key)
        {
            review.preview = None;
        }
        if review
            .continuation
            .as_ref()
            .is_some_and(|preview| preview.issue_uuid == key)
        {
            review.continuation = None;
        }
    }

    pub(super) fn needs_dispatch_review(
        &self,
        store: &Store,
        routed: &Config,
        issue: &Issue,
        mode: RunMode,
        context: Option<&IssueContext>,
    ) -> Result<bool> {
        if !self.inner.require_dispatch_review {
            return Ok(false);
        }
        if context.is_some_and(|context| !context.has_conversation_history()) {
            return Ok(true);
        }
        let Some(destination) = Destination::for_issue(routed, issue, mode, self.home()) else {
            return Ok(false);
        };
        Ok(!ws::dispatch_approved(
            store.workflow_connection(),
            &issue.uuid,
            issue.updated_at,
            context.map_or(0, |context| context.revision),
            &serde_json::to_string(&destination)?,
        )?)
    }

    pub(super) fn continuation_needs_review(
        &self,
        store: &Store,
        job: &Job,
        intent: &ContinuationIntent,
        routed: &Config,
        issue: &Issue,
        context: Option<&IssueContext>,
    ) -> Result<bool> {
        if context.is_some_and(|context| !context.has_conversation_history()) {
            return Ok(true);
        }
        let mode = job.mode;
        let destination = Destination::for_issue(routed, issue, mode, self.home());
        Ok(destination.as_ref() != Some(&intent.destination)
            || !store.continuation_approved(
                job.id,
                issue.updated_at,
                context.map_or(0, |context| context.revision),
            )?
            || !ws::dispatch_approved(
                store.workflow_connection(),
                &issue.uuid,
                issue.updated_at,
                context.map_or(0, |context| context.revision),
                &serde_json::to_string(&intent.destination)?,
            )?)
    }

    async fn review_candidate(&self, identifier: &str) -> Result<(Issue, RunMode, Config)> {
        let linear = self.linear();
        let issue = Self::resolve_issue_on(linear.as_ref(), identifier).await?;
        let viewer = self.viewer(&linear).await?;
        anyhow::ensure!(
            Arc::ptr_eq(&linear, &self.linear()),
            "Linear connection changed; reopen the dispatch preview"
        );
        let cfg = self.config();
        let mode = trigger::candidate_mode(&issue, &cfg.settings.trigger, &viewer.id)
            .context("issue no longer matches the dispatch conditions")?;
        Ok((issue, mode, cfg))
    }

    fn review_input(
        &self,
        store: &Store,
        review: &ReviewState,
        issue: &Issue,
        mode: RunMode,
        cfg: &Config,
    ) -> Result<ReviewInput> {
        let job = store.latest_for_issue(&issue.uuid)?;
        anyhow::ensure!(
            job.as_ref()
                .is_none_or(|job| job.state != JobState::Running),
            "the issue already has a running executor"
        );
        if let Some(job) = job {
            anyhow::ensure!(
                !job.state.is_active() || store.read_continuation(job.id)?.is_none(),
                "use the continuation preview for this run"
            );
            anyhow::ensure!(
                ws::read_claim(store.workflow_connection(), job.id)?.is_none(),
                "release the handoff before approving dispatch"
            );
            anyhow::ensure!(
                matches!(job.state, JobState::Queued | JobState::WaitingBinding)
                    || trigger::should_enqueue(Some(&job), issue),
                "this Linear revision has already been handled; prepare a new run before confirming"
            );
        }
        let confirmed = ws::read_context(store.workflow_connection(), &issue.uuid)?;
        let committed_revision = confirmed.as_ref().map_or(0, |context| context.revision);
        let draft = review.drafts.get(&issue.uuid);
        let context = draft.cloned().or(confirmed);
        if let Some(context) = &context {
            context.require_conversation_history()?;
        }
        let (routed, _) = Self::selection_for(cfg, issue, context.as_ref());
        let destination = Destination::for_issue(&routed, issue, mode, self.home())
            .context("the issue no longer has a matching project binding")?;
        Ok(ReviewInput {
            updated_at: issue.updated_at,
            committed_revision,
            context,
            staged: draft.is_some(),
            destination,
        })
    }

    /// Builds an ephemeral preview. Only the desktop panel can approve this exact payload.
    pub async fn dispatch_preview(&self, identifier: &str) -> Result<DispatchPreview> {
        let _cycle = self.inner.cycle.lock().await;
        let (issue, mode, cfg) = self.review_candidate(identifier).await?;
        let store = self.store();
        let mut review = self
            .inner
            .dispatch_review
            .lock()
            .expect("dispatch review lock");
        let input = self.review_input(&store, &review, &issue, mode, &cfg)?;
        let token: String = store.workflow_connection().query_row(
            "SELECT lower(hex(randomblob(16)))",
            [],
            |row| row.get(0),
        )?;
        let preview = DispatchPreview {
            token: token.clone(),
            issue_uuid: issue.uuid.clone(),
            issue_identifier: issue.identifier,
            title: clean_text(&issue.title),
            mode,
            context: input.context.clone(),
            executor: input.destination.executor,
            directory: input.destination.canonical_directory.clone(),
            model: input.destination.model.clone(),
            effort: input.destination.effort,
            wiki_root: input.destination.wiki_root.clone(),
        };
        review.preview = Some(OpenPreview {
            issue_uuid: issue.uuid,
            token,
            input,
        });
        review.continuation = None;
        Ok(preview)
    }

    /// Resolves the current Linear URL by the planned run's immutable issue identity.
    pub async fn development_issue_url(&self, id: i64) -> Result<String> {
        let job = self.store().get(id)?.context("planning run not found")?;
        anyhow::ensure!(
            job.mode == RunMode::Plan
                && job.state == JobState::NeedsInfo
                && job.report.as_deref() == Some("planned"),
            "the run has not produced a plan"
        );
        let linear = self.linear();
        let issue = linear
            .issue(&job.issue_uuid)
            .await?
            .filter(|issue| issue.uuid == job.issue_uuid)
            .context("the planned issue is not in the connected Linear workspace")?;
        anyhow::ensure!(
            Arc::ptr_eq(&linear, &self.linear()),
            "Linear connection changed; try again"
        );
        Ok(issue.url)
    }

    /// Saves the reviewed context and approval together, then requests normal scheduling.
    pub async fn approve_dispatch(&self, identifier: &str, token: &str) -> Result<()> {
        let _cycle = self.inner.cycle.lock().await;
        let (issue, mode, cfg) = self.review_candidate(identifier).await?;
        {
            let store = self.store();
            let mut review = self
                .inner
                .dispatch_review
                .lock()
                .expect("dispatch review lock");
            let current = self.review_input(&store, &review, &issue, mode, &cfg)?;
            anyhow::ensure!(
                review
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.issue_uuid == issue.uuid
                        && preview.token == token
                        && preview.input == current),
                "dispatch preview changed; reopen it before confirming"
            );
            ws::approve_dispatch(
                store.workflow_connection(),
                &issue.uuid,
                current.updated_at,
                current.committed_revision,
                current.context.as_ref().filter(|_| current.staged),
                &serde_json::to_string(&current.destination)?,
            )?;
            review.drafts.remove(&issue.uuid);
            review.preview = None;
        }
        self.poll_now();
        Ok(())
    }

    async fn continuation_candidate(&self, id: i64) -> Result<(Issue, Config)> {
        let source = self.store().get(id)?.context("interrupted run not found")?;
        let linear = self.linear();
        let viewer = self.viewer(&linear).await?;
        let issue = linear
            .issue(&source.issue_uuid)
            .await?
            .filter(|issue| issue.uuid == source.issue_uuid)
            .context("the issue is not in the connected Linear workspace")?;
        anyhow::ensure!(
            Arc::ptr_eq(&linear, &self.linear()),
            "Linear connection changed; reopen the continuation preview"
        );
        let cfg = self.config();
        let needs_info_source = source.needs_info_implementation();
        let issue = super::continuation_candidate(
            issue,
            &cfg.settings.trigger,
            &viewer.id,
            source.mode,
            needs_info_source,
        )
        .with_context(|| {
            if needs_info_source {
                "the issue must still be assigned to you, in Todo or In Progress, without switching to planning"
            } else {
                "the issue must still be assigned to you, in Todo or In Progress, and carry the same execution label"
            }
        })?;
        Ok((issue, cfg))
    }

    fn continuation_input(
        &self,
        store: &Store,
        review: &ReviewState,
        issue: &Issue,
        cfg: &Config,
        id: i64,
        executor: ExecutorKind,
    ) -> Result<ContinuationInput> {
        let source = store.get(id)?.context("interrupted run not found")?;
        let mode = source.mode;
        let latest = store
            .latest_for_issue(&issue.uuid)?
            .context("no runs for this issue")?;
        let conn = store.workflow_connection();
        anyhow::ensure!(
            source.issue_uuid == issue.uuid
                && source.can_source_continuation()
                && (latest.id == id
                    || (latest.resume_job_id == Some(id)
                        && latest.started_at.is_none()
                        && matches!(
                            latest.state,
                            JobState::Queued | JobState::WaitingBinding | JobState::Canceled
                        ))),
            "select the latest interrupted run of this issue"
        );
        anyhow::ensure!(
            ws::read_claim(conn, id)?.is_none() && ws::read_claim(conn, latest.id)?.is_none(),
            "release the handoff before continuing the run"
        );
        let source_executor = source
            .executor
            .context("the interrupted run has no recorded executor")?;
        let confirmed = ws::read_context(conn, &issue.uuid)?;
        let committed_revision = confirmed.as_ref().map_or(0, |context| context.revision);
        let draft = review.drafts.get(&issue.uuid);
        let context = draft.cloned().or(confirmed);
        if source.needs_info_implementation() {
            let previous = ws::read_memory(conn, source.id)?.and_then(|memory| memory.context);
            anyhow::ensure!(
                context.as_ref().is_some_and(|context| {
                    context.has_conversation_history()
                        && previous.as_ref().is_none_or(|previous| {
                            context.revision > previous.revision
                                && context.conversation_history != previous.conversation_history
                        })
                }),
                "supplement conversation_history with the information requested by the last run, then reopen the continuation preview"
            );
        } else if let Some(context) = &context {
            context.require_conversation_history()?;
        }
        let (routed, _) =
            Self::selection_with_executor(cfg, issue, context.as_ref(), Some(executor));
        let Route::Bound(binding) = routed.route(issue) else {
            bail!("the issue no longer has a matching project binding");
        };
        let destination = Destination::for_issue(&routed, issue, mode, self.home())
            .context("the issue no longer has a matching project binding")?;
        anyhow::ensure!(
            destination.canonical_directory.is_dir()
                && source.directory.as_deref().map(canonical_dir).as_ref()
                    == Some(&destination.canonical_directory),
            "restore the original project binding before continuing; its worktree contains the earlier changes"
        );
        let resume_unavailable =
            Self::resume_for_source(cfg, &source, executor, binding, mode).err();
        let strategy = match resume_unavailable {
            None => ContinuationStrategy::ResumeSession,
            Some(_) => ContinuationStrategy::Handoff,
        };
        let evidence =
            continuation_evidence(store, &source, resume_unavailable, context.is_none())?;
        Ok(ContinuationInput {
            review: ReviewInput {
                updated_at: issue.updated_at,
                committed_revision,
                context,
                staged: draft.is_some(),
                destination: destination.clone(),
            },
            intent: ContinuationIntent {
                source_job_id: id,
                executor,
                strategy,
                evidence,
                destination,
            },
            source_executor,
            latest_job_id: latest.id,
        })
    }

    /// Reads the latest brief used by the desktop's continuation supplement form.
    pub async fn continuation_context(&self, id: i64) -> Result<Option<IssueContext>> {
        let (issue, _) = self.continuation_candidate(id).await?;
        let store = self.store();
        self.latest_context(&store, &issue.uuid)
    }

    /// Stages added information without replacing the original brief or approving a run.
    pub async fn supplement_continuation_context(
        &self,
        id: i64,
        expected_revision: u64,
        supplement: &str,
    ) -> Result<IssueContext> {
        anyhow::ensure!(
            !supplement.trim().is_empty() && supplement.len() <= 32 * 1024,
            "provide up to 32 KiB of additional task information"
        );
        let (issue, _) = self.continuation_candidate(id).await?;
        let mut context = {
            let store = self.store();
            let source = store.get(id)?.context("interrupted run not found")?;
            let latest = store
                .latest_for_issue(&issue.uuid)?
                .context("no runs for this issue")?;
            anyhow::ensure!(
                source.can_source_continuation()
                    && (latest.id == id
                        || latest.resume_job_id == Some(id)
                            && latest.started_at.is_none()
                            && matches!(
                                latest.state,
                                JobState::Queued | JobState::WaitingBinding | JobState::Canceled
                            )),
                "select the latest interrupted run of this issue"
            );
            let current = self.latest_context(&store, &issue.uuid)?;
            anyhow::ensure!(
                current.as_ref().map_or(0, |context| context.revision) == expected_revision,
                "issue context revision changed; read it and try again"
            );
            current
                .or(ws::read_memory(store.workflow_connection(), id)?
                    .and_then(|memory| memory.context))
                .unwrap_or_else(|| IssueContext {
                    request: source.title,
                    ..IssueContext::default()
                })
        };
        let history = context.conversation_history.get_or_insert_with(String::new);
        if !history.is_empty() {
            history.push_str("\n\n");
        }
        history.push_str(supplement);
        self.set_issue_context(&issue.identifier, expected_revision, context)
            .await
    }

    /// Previews one interrupted run and the data that would reach the selected executor.
    pub async fn get_continuation_preview(
        &self,
        id: i64,
        executor: ExecutorKind,
    ) -> Result<ContinuationPreview> {
        let _cycle = self.inner.cycle.lock().await;
        let (issue, cfg) = self.continuation_candidate(id).await?;
        let store = self.store();
        let mut review = self
            .inner
            .dispatch_review
            .lock()
            .expect("dispatch review lock");
        let input = self.continuation_input(&store, &review, &issue, &cfg, id, executor)?;
        let token: String = store.workflow_connection().query_row(
            "SELECT lower(hex(randomblob(16)))",
            [],
            |row| row.get(0),
        )?;
        let preview = ContinuationPreview {
            token: token.clone(),
            source_job_id: id,
            issue_uuid: issue.uuid.clone(),
            issue_identifier: issue.identifier,
            title: clean_text(&issue.title),
            executor,
            source_executor: input.source_executor,
            strategy: input.intent.strategy,
            mode: input.intent.destination.mode,
            directory: input.intent.destination.canonical_directory.clone(),
            model: input.intent.destination.model.clone(),
            effort: input.intent.destination.effort,
            permission: input.intent.destination.permission,
            claude_ultracode: input.intent.destination.claude_ultracode,
            context: input.review.context.clone(),
            wiki_root: input.intent.destination.wiki_root.clone(),
            evidence: input.intent.evidence.clone(),
        };
        review.preview = None;
        review.continuation = Some(OpenContinuation {
            issue_uuid: issue.uuid,
            token,
            input,
        });
        Ok(preview)
    }

    /// Creates or adopts an unstarted attempt without modifying the source run or Linear.
    pub async fn approve_continuation(&self, id: i64, token: &str) -> Result<i64> {
        let _cycle = self.inner.cycle.lock().await;
        let (issue, cfg) = self.continuation_candidate(id).await?;
        let job_id = {
            let store = self.store();
            let mut review = self
                .inner
                .dispatch_review
                .lock()
                .expect("dispatch review lock");
            let pending = review
                .continuation
                .as_ref()
                .filter(|preview| {
                    preview.token == token
                        && preview.issue_uuid == issue.uuid
                        && preview.input.intent.source_job_id == id
                })
                .context("continuation preview expired; reopen it before confirming")?;
            let executor = pending.input.intent.executor;
            let input = self.continuation_input(&store, &review, &issue, &cfg, id, executor)?;
            anyhow::ensure!(
                pending.input == input,
                "continuation preview changed; reopen it before confirming"
            );
            let (routed, _) = Self::selection_with_executor(
                &cfg,
                &issue,
                input.review.context.as_ref(),
                Some(executor),
            );
            let Route::Bound(binding) = routed.route(&issue) else {
                bail!("the issue no longer has a matching project binding");
            };
            let issue = Issue {
                title: clean_text(&issue.title),
                ..issue
            };
            let job = store.approve_continuation(
                NewJob {
                    issue: &issue,
                    binding: Some(binding),
                    executor: Some(executor),
                    state: JobState::WaitingBinding,
                    reason: None,
                    mode: input.intent.destination.mode,
                    resume_job_id: Some(id),
                },
                &input.intent,
                input.latest_job_id,
                input.review.committed_revision,
                input
                    .review
                    .context
                    .as_ref()
                    .filter(|_| input.review.staged),
            )?;
            review.drafts.remove(&issue.uuid);
            review.continuation = None;
            job.id
        };
        self.emit_job(job_id);
        self.poll_now();
        Ok(job_id)
    }
}

fn continuation_evidence(
    store: &Store,
    source: &Job,
    resume_unavailable: Option<&str>,
    use_source_context: bool,
) -> Result<String> {
    let mut runs = Vec::new();
    let mut earlier = Some(source.clone());
    while let Some(job) = earlier {
        let mut memory = ws::read_memory(store.workflow_connection(), job.id)?;
        if let Some(memory) = &mut memory
            && !(use_source_context && job.id == source.id)
        {
            // The separately reviewed current brief is authoritative for task history and Wiki
            // references. A superseded brief must not re-enter a cross-provider handoff.
            memory.context = None;
            memory.wiki_root = None;
        }
        runs.push(serde_json::json!({
            "run_id": job.id, "executor": job.executor, "mode": job.mode, "state": job.state,
            "finished_at": job.finished_at, "reason": job.reason, "summary": job.summary,
            "reported_result": job.report, "memory": memory,
        }));
        let parent = store
            .read_continuation(job.id)?
            .map(|intent| intent.source_job_id)
            .or(job.resume_job_id);
        earlier = match parent {
            Some(id) if id < job.id => store
                .get(id)?
                .filter(|parent| parent.issue_uuid == source.issue_uuid),
            _ => None,
        };
        anyhow::ensure!(
            runs.len() < 64 || earlier.is_none(),
            "the continuation history exceeds the supported size"
        );
    }
    let evidence = serde_json::to_string_pretty(&serde_json::json!({
        "resume_unavailable": resume_unavailable,
        "recorded_runs_newest_first": runs,
    }))?;
    anyhow::ensure!(
        evidence.len() <= 512 * 1024,
        "the continuation history exceeds the supported size"
    );
    Ok(evidence)
}
