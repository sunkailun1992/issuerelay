//! Which issues IssueRelay should pick up, and when an issue it already handled comes back.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::{Binding, Config, ExecutorKind, TriggerNames};
use crate::model::{Issue, Job};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    ProjectNotBound,
    DirectoryMissing(PathBuf),
    ExecutorNotReady(ExecutorKind),
}

impl WaitReason {
    pub fn describe(&self) -> String {
        match self {
            WaitReason::ProjectNotBound => "project is not bound to a local directory".into(),
            WaitReason::DirectoryMissing(dir) => {
                format!("bound directory does not exist: {}", dir.display())
            }
            WaitReason::ExecutorNotReady(kind) => {
                format!("executor {} is not ready", kind.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    NotCandidate,
    Waiting(WaitReason),
    Eligible(Binding),
}

/// Linear names are compared ignoring surrounding whitespace and ASCII case.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

pub fn is_candidate(issue: &Issue, names: &TriggerNames, me: &str) -> bool {
    issue.assignee_id.as_deref() == Some(me)
        && issue
            .labels
            .iter()
            .any(|label| same_name(label, &names.label))
        && same_name(&issue.state, &names.todo)
}

pub fn evaluate(
    issue: &Issue,
    cfg: &Config,
    me: &str,
    ready: &HashSet<ExecutorKind>,
) -> Eligibility {
    if !is_candidate(issue, &cfg.settings.trigger, me) {
        return Eligibility::NotCandidate;
    }
    let Some(binding) = issue
        .project_id
        .as_deref()
        .and_then(|p| cfg.binding_for_project(p))
    else {
        return Eligibility::Waiting(WaitReason::ProjectNotBound);
    };
    if !binding.directory.is_dir() {
        return Eligibility::Waiting(WaitReason::DirectoryMissing(binding.directory.clone()));
    }
    let kind = cfg.executor_for(binding);
    if !ready.contains(&kind) {
        return Eligibility::Waiting(WaitReason::ExecutorNotReady(kind));
    }
    Eligibility::Eligible(binding.clone())
}

/// A tracked issue (waiting, queued, running) is never enqueued again. A finished one comes
/// back only when Linear's own `updatedAt` moved past the value recorded when the last run
/// was verified, so clock skew on this machine cannot cause duplicates or misses.
pub fn should_enqueue(latest: Option<&Job>, issue: &Issue) -> bool {
    match latest {
        None => true,
        Some(job) if job.state.is_active() => false,
        Some(job) => {
            let seen = job.verified_updated_at.unwrap_or(job.discovered_updated_at);
            issue.updated_at > seen
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Binding, Config, ExecutorKind};
    use crate::model::{Issue, Job, JobState};
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use std::collections::HashSet;
    use std::path::Path;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap()
    }

    fn issue(project: Option<&str>) -> Issue {
        Issue {
            uuid: "u-1".into(),
            identifier: "DEMO-1".into(),
            title: "Demo".into(),
            url: "https://linear.app/demo/issue/DEMO-1".into(),
            state: "Todo".into(),
            labels: vec!["ai-ready".into()],
            project_id: project.map(str::to_string),
            project_name: Some("Demo Project".into()),
            assignee_id: Some("me".into()),
            parent_identifier: None,
            priority: 0,
            created_at: t0(),
            updated_at: t0(),
        }
    }

    fn config_with(project: &str, dir: &Path, executor: Option<ExecutorKind>) -> Config {
        let mut cfg = Config::default();
        cfg.bindings.push(Binding {
            id: project.into(),
            project_id: project.into(),
            project_name: "Demo Project".into(),
            directory: dir.to_path_buf(),
            executor,
            permission: None,
            model: None,
            prompt_override: None,
        });
        cfg
    }

    fn job(state: JobState, verified: Option<DateTime<Utc>>) -> Job {
        Job {
            id: 1,
            issue_uuid: "u-1".into(),
            issue_identifier: "DEMO-1".into(),
            title: "Demo".into(),
            binding_id: None,
            directory: None,
            executor: None,
            state,
            reason: None,
            priority: 0,
            issue_created_at: t0(),
            discovered_updated_at: t0(),
            verified_updated_at: verified,
            created_at: t0(),
            started_at: None,
            finished_at: None,
            log_path: None,
            summary: None,
        }
    }

    fn all_ready() -> HashSet<ExecutorKind> {
        [ExecutorKind::Claude, ExecutorKind::Codex]
            .into_iter()
            .collect()
    }

    #[test]
    fn candidate_needs_assignee_label_and_todo() {
        let names = Config::default().settings.trigger;
        assert!(is_candidate(&issue(None), &names, "me"));
        assert!(!is_candidate(&issue(None), &names, "someone-else"));
        let mut other_label = issue(None);
        other_label.labels = vec!["bug".into()];
        assert!(!is_candidate(&other_label, &names, "me"));
        let mut in_progress = issue(None);
        in_progress.state = "In Progress".into();
        assert!(!is_candidate(&in_progress, &names, "me"));
    }

    #[test]
    fn names_compare_case_insensitively() {
        let names = Config::default().settings.trigger;
        let mut i = issue(None);
        i.labels = vec![" AI-Ready ".into()];
        i.state = "todo".into();
        assert!(is_candidate(&i, &names, "me"));
    }

    #[test]
    fn unbound_project_waits() {
        let cfg = Config::default();
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready()),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
        assert_eq!(
            evaluate(&issue(None), &cfg, "me", &all_ready()),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
    }

    #[test]
    fn missing_directory_waits() {
        let cfg = config_with("proj-1", Path::new("/definitely/not/here"), None);
        assert!(matches!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready()),
            Eligibility::Waiting(WaitReason::DirectoryMissing(_))
        ));
    }

    #[test]
    fn executor_not_ready_waits() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), Some(ExecutorKind::Codex));
        let only_claude: HashSet<_> = [ExecutorKind::Claude].into_iter().collect();
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &only_claude),
            Eligibility::Waiting(WaitReason::ExecutorNotReady(ExecutorKind::Codex))
        );
    }

    #[test]
    fn bound_ready_issue_is_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        match evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready()) {
            Eligibility::Eligible(binding) => assert_eq!(binding.project_id, "proj-1"),
            other => panic!("expected eligible, got {other:?}"),
        }
    }

    #[test]
    fn non_candidate_is_not_evaluated_further() {
        let cfg = Config::default();
        assert_eq!(
            evaluate(&issue(None), &cfg, "other", &all_ready()),
            Eligibility::NotCandidate
        );
    }

    #[test]
    fn enqueue_new_issue_but_not_tracked_one() {
        assert!(should_enqueue(None, &issue(None)));
        for state in [
            JobState::WaitingBinding,
            JobState::Queued,
            JobState::Running,
        ] {
            assert!(!should_enqueue(Some(&job(state, None)), &issue(None)));
        }
    }

    #[test]
    fn finished_issue_requeues_only_after_linear_update() {
        let finished = job(JobState::Failed, Some(t0()));
        assert!(!should_enqueue(Some(&finished), &issue(None)));
        let mut touched = issue(None);
        touched.updated_at = t0() + Duration::seconds(1);
        assert!(should_enqueue(Some(&finished), &touched));
    }

    #[test]
    fn should_enqueue_compares_linear_clock_only() {
        // Local timestamps far in the future must not matter; only Linear's updatedAt does.
        let far_future = Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap();
        let mut finished = job(JobState::NeedsInfo, Some(t0()));
        finished.created_at = far_future;
        finished.finished_at = Some(far_future);
        let mut touched = issue(None);
        touched.updated_at = t0() + Duration::seconds(1);
        assert!(should_enqueue(Some(&finished), &touched));
        assert!(!should_enqueue(Some(&finished), &issue(None)));
    }

    #[test]
    fn falls_back_to_discovered_time_when_never_verified() {
        let canceled = job(JobState::Canceled, None);
        assert!(!should_enqueue(Some(&canceled), &issue(None)));
        let mut touched = issue(None);
        touched.updated_at = t0() + Duration::seconds(5);
        assert!(should_enqueue(Some(&canceled), &touched));
    }
}
