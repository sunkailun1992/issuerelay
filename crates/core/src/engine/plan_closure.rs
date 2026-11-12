//! Closing a split plan (plan-closure design): what the direct children of a plan issue have come
//! to, kept in memory for the panel.
use std::time::Instant;

use super::*;
use crate::model::ChildIssue;

/// How many open plans each check reads the children of, most recently finished first. The
/// others are read when the user opens their details.
pub const PLAN_PROGRESS_LIMIT: usize = 20;

/// How long what a check read about a plan's children stands before a check reads them again,
/// as for blockers: Linear is asked about each open plan at most this often, whatever the poll
/// interval. Opening a plan's details and closing it always read Linear.
const PLAN_PROGRESS_TTL: Duration = Duration::from_secs(2 * 60);

/// What was read about one plan, and when (this machine's monotonic clock, for the time-to-live).
pub(super) struct PlanCheck {
    at: Instant,
    progress: PlanProgress,
}

impl PlanCheck {
    fn now(progress: PlanProgress) -> Self {
        Self {
            at: Instant::now(),
            progress,
        }
    }
}

/// What the direct children of a plan issue have come to, as last read from Linear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProgress {
    /// The planning run this progress belongs to.
    pub job_id: i64,
    pub issue_uuid: String,
    /// Children counted: every child but the deleted ones.
    pub total: u32,
    pub completed: u32,
    pub canceled: u32,
    /// Children that have not ended yet.
    pub open: u32,
    /// At least one child, none open and at least one completed: the plan can be closed.
    pub closable: bool,
    /// This machine's clock, for display only.
    pub checked_at: DateTime<Utc>,
    /// Why the children could not be read, cleaned for display; the counts are then all zero.
    pub error: Option<String>,
}

/// Counts the children of a plan issue. A child whose status is of kind `completed` or
/// `canceled` has ended; an archived child that has not ended was deleted and is not counted
/// (Linear only archives on its own what has ended). A child whose status kind Linear did not say
/// counts as open.
pub fn tally(
    job_id: i64,
    issue_uuid: &str,
    children: &[ChildIssue],
    checked_at: DateTime<Utc>,
) -> PlanProgress {
    let (mut completed, mut canceled, mut open) = (0, 0, 0);
    for child in children {
        match child.state_type.as_deref() {
            Some("completed") => completed += 1,
            Some("canceled") => canceled += 1,
            _ if child.archived => {}
            _ => open += 1,
        }
    }
    let total = completed + canceled + open;
    PlanProgress {
        job_id,
        issue_uuid: issue_uuid.to_string(),
        total,
        completed,
        canceled,
        open,
        closable: total > 0 && open == 0 && completed > 0,
        checked_at,
        error: None,
    }
}

/// A planning run that posted its plan and has ended.
fn posted_plan(job: &Job) -> bool {
    job.mode == RunMode::Plan
        && job.report.as_deref() == Some("planned")
        && job.finished_at.is_some()
}

impl Engine {
    /// Forgets what was read about the plans, so that the next check reads them again. For tests.
    #[doc(hidden)]
    pub fn forget_plan_progress(&self) {
        self.inner
            .plan_progress
            .lock()
            .expect("plan progress lock")
            .clear();
    }

    /// The progress last read for each open plan; reads nothing.
    pub fn plan_progress(&self) -> Vec<PlanProgress> {
        let mut progress: Vec<PlanProgress> = self
            .inner
            .plan_progress
            .lock()
            .expect("plan progress lock")
            .values()
            .map(|check| check.progress.clone())
            .collect();
        progress.sort_by_key(|p| p.job_id);
        progress
    }

    /// Reads the children of this planning run's issue now and keeps the result. The run must
    /// have posted its plan; an open plan and a closed one are both accepted. A failed read is
    /// the progress's `error`, not an error of the call.
    pub async fn refresh_plan_progress(&self, job_id: i64) -> Result<PlanProgress> {
        let job = self
            .store()
            .get(job_id)?
            .ok_or_else(|| anyhow!("job {job_id} not found"))?;
        anyhow::ensure!(
            posted_plan(&job) && matches!(job.state, JobState::NeedsInfo | JobState::Done),
            "job {job_id} is not a planning run that posted its plan"
        );
        let linear = self.linear();
        let progress = read_progress(linear.as_ref(), &job).await;
        self.inner
            .plan_progress
            .lock()
            .expect("plan progress lock")
            .insert(job_id, PlanCheck::now(progress.clone()));
        Ok(progress)
    }

    /// Closes a split plan once the user confirmed it in the panel: marks the plan issue done in
    /// Linear, the same single write as `mark_done`, then the planning run done here. Only the
    /// desktop panel calls this; IPC, CLI and MCP never expose it. Everything is checked again
    /// after the click: the run (newest, open, not claimed, archived when a Wiki is configured),
    /// the plan issue (still there, not canceled) and every child (none open, one completed).
    /// When Linear already shows the plan completed, only the run is recorded.
    pub async fn complete_plan(&self, job_id: i64) -> Result<()> {
        let _cycle = self.inner.cycle.lock().await;
        let settings = self.config().settings;
        let job = {
            let store = self.store();
            let job = store
                .get(job_id)?
                .ok_or_else(|| anyhow!("job {job_id} not found"))?;
            anyhow::ensure!(
                posted_plan(&job) && job.state == JobState::NeedsInfo,
                "job {job_id} is not a plan waiting to be closed"
            );
            anyhow::ensure!(
                store
                    .latest_for_issue(&job.issue_uuid)?
                    .map(|latest| latest.id)
                    == Some(job_id),
                "job {job_id} is not the latest run of {}",
                job.issue_identifier
            );
            let conn = store.workflow_connection();
            anyhow::ensure!(
                crate::workflow_store::read_claim(conn, job_id)?.is_none(),
                "release the handoff before closing the plan"
            );
            if let Some(root) = &settings.wiki_root {
                anyhow::ensure!(
                    Self::current_archive_receipt(conn, job_id, root)?.is_some(),
                    "archive this run in the configured Wiki before closing the plan"
                );
            }
            job
        };
        let linear = self.linear();
        let issue = linear
            .issue(&job.issue_uuid)
            .await?
            .filter(|issue| issue.uuid == job.issue_uuid)
            .ok_or_else(|| anyhow!("the issue no longer exists in Linear"))?;
        anyhow::ensure!(
            issue.state_type.as_deref() != Some("canceled"),
            "the plan is canceled in Linear; settle it there"
        );
        let children = linear.children(&job.issue_uuid).await?;
        let progress = tally(job_id, &job.issue_uuid, &children, Utc::now());
        anyhow::ensure!(
            progress.closable,
            "the plan cannot be closed yet: {} of {} children ended, {} completed",
            progress.completed + progress.canceled,
            progress.total,
            progress.completed
        );
        let done = settings.trigger.done;
        if issue.state_type.as_deref() != Some("completed") {
            linear.set_state(&job.issue_uuid, &done).await?;
        }
        // Conditional on the state checked above, as in `mark_done`.
        let closed =
            self.store()
                .set_state_if(job_id, &[JobState::NeedsInfo], JobState::Done, None)?;
        if !closed {
            bail!(
                "job {job_id} changed while the plan was being closed; Linear already shows the \
                 plan as {done:?}"
            );
        }
        self.inner
            .plan_progress
            .lock()
            .expect("plan progress lock")
            .insert(job_id, PlanCheck::now(progress));
        self.emit_job(job_id);
        Ok(())
    }

    /// The open plans: planning runs that posted their plan, still wait for the user and are the
    /// newest run of their issue, most recently finished first.
    fn open_plans(&self) -> Result<Vec<Job>> {
        let store = self.store();
        let mut plans = Vec::new();
        for job in store.list_by_state(&[JobState::NeedsInfo])? {
            if posted_plan(&job)
                && store
                    .latest_for_issue(&job.issue_uuid)?
                    .map(|latest| latest.id)
                    == Some(job.id)
            {
                plans.push(job);
            }
        }
        plans.sort_by_key(|job| std::cmp::Reverse(job.finished_at));
        Ok(plans)
    }

    /// Reads the children of up to `PLAN_PROGRESS_LIMIT` open plans after a check: those never read
    /// or read longer than `PLAN_PROGRESS_TTL` ago, most recently finished first. A plan whose
    /// children cannot be read keeps the reason as its `error`; nothing here fails the check.
    /// Plans that are no longer open drop out of the cache.
    pub(super) async fn refresh_open_plans(&self, linear: &dyn LinearApi) {
        let plans = match self.open_plans() {
            Ok(plans) => plans,
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "could not list the open plans");
                return;
            }
        };
        let due: Vec<&Job> = {
            let cache = self.inner.plan_progress.lock().expect("plan progress lock");
            plans
                .iter()
                .filter(|job| {
                    cache
                        .get(&job.id)
                        .is_none_or(|check| check.at.elapsed() >= PLAN_PROGRESS_TTL)
                })
                .take(PLAN_PROGRESS_LIMIT)
                .collect()
        };
        let mut fresh = HashMap::new();
        for job in due {
            fresh.insert(job.id, PlanCheck::now(read_progress(linear, job).await));
        }
        let open: HashSet<i64> = plans.iter().map(|job| job.id).collect();
        let mut cache = self.inner.plan_progress.lock().expect("plan progress lock");
        cache.retain(|id, _| open.contains(id));
        cache.extend(fresh);
    }
}

/// The progress of `job`'s plan as Linear shows it now.
async fn read_progress(linear: &dyn LinearApi, job: &Job) -> PlanProgress {
    let now = Utc::now();
    match linear.children(&job.issue_uuid).await {
        Ok(children) => tally(job.id, &job.issue_uuid, &children, now),
        Err(error) => PlanProgress {
            error: Some(clean_text(&format!("{error:#}"))),
            ..tally(job.id, &job.issue_uuid, &[], now)
        },
    }
}
