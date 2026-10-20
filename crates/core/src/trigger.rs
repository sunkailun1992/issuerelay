//! Which issues IssueRelay should pick up, and when an issue it already handled comes back.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Binding, Config, ExecutorKind, LeanStatus, TriggerNames};
use crate::model::{Issue, Job};

/// Why an issue that looks eligible has to wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    ProjectNotBound,
    DirectoryMissing(PathBuf),
    ExecutorNotReady(ExecutorKind),
}

impl WaitReason {
    /// The text stored as the job's reason.
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

/// `WaitReason` as the panel reads it: a code it translates, with the folder or executor the
/// reason names. `describe` stays the text the job's reason and the CLI show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum WaitReasonView {
    ProjectNotBound,
    DirectoryMissing { directory: String },
    ExecutorNotReady { executor: ExecutorKind },
}

impl WaitReason {
    /// The structured form, for the panel's issue list.
    pub fn view(&self) -> WaitReasonView {
        match self {
            WaitReason::ProjectNotBound => WaitReasonView::ProjectNotBound,
            WaitReason::DirectoryMissing(dir) => WaitReasonView::DirectoryMissing {
                directory: dir.display().to_string(),
            },
            WaitReason::ExecutorNotReady(kind) => {
                WaitReasonView::ExecutorNotReady { executor: *kind }
            }
        }
    }
}

/// Which executors can take runs (lean-run design, section 7): `full` with the user's whole
/// configuration, `lean` also in lean runs, which need the Linear server and the sign-in that
/// lean runs use. `doctor::ready_set` builds it from the executor reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readiness {
    pub full: HashSet<ExecutorKind>,
    pub lean: HashSet<ExecutorKind>,
}

impl Readiness {
    /// Whether `kind` can take the runs of a binding whose lean status is `lean`: a binding whose
    /// runs are lean needs the lean checks, every other binding the usual ones.
    pub fn ready(&self, kind: ExecutorKind, lean: LeanStatus) -> bool {
        if lean.applies() {
            self.lean.contains(&kind)
        } else {
            self.full.contains(&kind)
        }
    }
}

/// The outcome of `evaluate`.
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

/// Assigned to `me`, carries the trigger label, and is in the Todo state.
pub fn is_candidate(issue: &Issue, names: &TriggerNames, me: &str) -> bool {
    issue.assignee_id.as_deref() == Some(me)
        && issue
            .labels
            .iter()
            .any(|label| same_name(label, &names.label))
        && same_name(&issue.state, &names.todo)
}

/// Checks the binding, that its directory exists (the only filesystem access in this
/// module, besides comparing that directory with `home` for its lean status) and that the
/// executor is ready for the binding's runs, lean or not. `home` is the user's home folder;
/// `None` when it is not known.
pub fn evaluate(
    issue: &Issue,
    cfg: &Config,
    me: &str,
    ready: &Readiness,
    home: Option<&Path>,
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
    if !ready.ready(kind, cfg.lean_for(binding, home)) {
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
    use crate::config::{Binding, Config, ExecutorKind, TriggerNames};
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
            lean: true,
            effort: None,
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

    fn all_ready() -> Readiness {
        let both: HashSet<_> = [ExecutorKind::Claude, ExecutorKind::Codex]
            .into_iter()
            .collect();
        Readiness {
            full: both.clone(),
            lean: both,
        }
    }

    /// `kind` is ready with the user's whole configuration only: its lean checks failed.
    fn full_only(kind: ExecutorKind) -> Readiness {
        Readiness {
            full: [kind].into_iter().collect(),
            lean: HashSet::new(),
        }
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
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
        assert_eq!(
            evaluate(&issue(None), &cfg, "me", &all_ready(), None),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
    }

    #[test]
    fn missing_directory_waits() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let cfg = config_with("proj-1", &missing, None);
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None),
            Eligibility::Waiting(WaitReason::DirectoryMissing(missing))
        );
    }

    #[test]
    fn executor_not_ready_waits() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), Some(ExecutorKind::Codex));
        let only_claude = Readiness {
            full: [ExecutorKind::Claude].into_iter().collect(),
            lean: [ExecutorKind::Claude].into_iter().collect(),
        };
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &only_claude, None),
            Eligibility::Waiting(WaitReason::ExecutorNotReady(ExecutorKind::Codex))
        );
    }

    #[test]
    fn bound_ready_issue_is_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        match evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None) {
            Eligibility::Eligible(binding) => assert_eq!(binding.project_id, "proj-1"),
            other => panic!("expected eligible, got {other:?}"),
        }
    }

    #[test]
    fn non_candidate_is_not_evaluated_further() {
        let cfg = Config::default();
        assert_eq!(
            evaluate(&issue(None), &cfg, "other", &all_ready(), None),
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

    #[test]
    fn active_jobs_never_requeue_even_after_linear_edits() {
        let mut edited = issue(None);
        edited.updated_at = t0() + Duration::hours(1);
        for state in JobState::ALL.into_iter().filter(|s| s.is_active()) {
            assert!(
                !should_enqueue(Some(&job(state, Some(t0()))), &edited),
                "{state:?}"
            );
            assert!(
                !should_enqueue(Some(&job(state, None)), &edited),
                "{state:?}"
            );
        }
    }

    #[test]
    fn every_finished_state_requeues_only_after_a_strictly_later_update() {
        for state in JobState::ALL.into_iter().filter(|s| !s.is_active()) {
            assert!(
                !should_enqueue(Some(&job(state, Some(t0()))), &issue(None)),
                "{state:?}"
            );
            let mut later = issue(None);
            later.updated_at = t0() + Duration::milliseconds(1);
            assert!(
                should_enqueue(Some(&job(state, Some(t0()))), &later),
                "{state:?}"
            );
        }
    }

    #[test]
    fn verified_time_wins_over_discovered_time() {
        // The run's own Linear writes moved updatedAt to t0 + 10s; that alone must not requeue.
        let finished = job(
            JobState::AwaitingAcceptance,
            Some(t0() + Duration::seconds(10)),
        );
        let mut at_verify = issue(None);
        at_verify.updated_at = t0() + Duration::seconds(10);
        assert!(!should_enqueue(Some(&finished), &at_verify));
        let mut edited_after = issue(None);
        edited_after.updated_at = t0() + Duration::seconds(11);
        assert!(should_enqueue(Some(&finished), &edited_after));
    }

    #[test]
    fn candidate_with_several_labels_none_or_no_assignee() {
        let names = TriggerNames::default();
        let mut several = issue(None);
        several.labels = vec!["bug".into(), "ai-ready".into()];
        assert!(is_candidate(&several, &names, "me"));
        let mut none = issue(None);
        none.labels.clear();
        assert!(!is_candidate(&none, &names, "me"));
        let mut unassigned = issue(None);
        unassigned.assignee_id = None;
        assert!(!is_candidate(&unassigned, &names, "me"));
    }

    #[test]
    fn same_name_trims_both_sides_and_folds_ascii_case_only() {
        assert!(same_name("  AI-Ready ", "ai-ready"));
        assert!(same_name("ai-ready", "  AI-READY\t"));
        assert!(same_name("In Testing", " in testing "));
        assert!(!same_name("É", "é"));
        assert!(!same_name("ai-ready", "ai ready"));
    }

    #[test]
    fn wait_reasons_have_a_code_the_panel_translates() {
        let shapes = [
            (
                WaitReason::ProjectNotBound,
                serde_json::json!({"code": "project_not_bound"}),
            ),
            (
                WaitReason::DirectoryMissing(PathBuf::from("demo-app")),
                serde_json::json!({"code": "directory_missing", "directory": "demo-app"}),
            ),
            (
                WaitReason::ExecutorNotReady(ExecutorKind::Codex),
                serde_json::json!({"code": "executor_not_ready", "executor": "codex"}),
            ),
        ];
        for (reason, json) in shapes {
            assert_eq!(serde_json::to_value(reason.view()).unwrap(), json);
            assert_eq!(
                serde_json::from_value::<WaitReasonView>(json).unwrap(),
                reason.view()
            );
        }
        // The stored job reason and the CLI keep the English sentence.
        assert_eq!(
            WaitReason::ProjectNotBound.describe(),
            "project is not bound to a local directory"
        );
    }

    #[test]
    fn wait_reasons_describe_themselves() {
        assert_eq!(
            WaitReason::ProjectNotBound.describe(),
            "project is not bound to a local directory"
        );
        assert_eq!(
            WaitReason::DirectoryMissing(std::path::PathBuf::from("missing-dir")).describe(),
            "bound directory does not exist: missing-dir"
        );
        assert_eq!(
            WaitReason::ExecutorNotReady(ExecutorKind::Codex).describe(),
            "executor codex is not ready"
        );
    }

    #[test]
    fn lean_bindings_wait_for_the_lean_checks() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        assert_eq!(
            evaluate(
                &issue(Some("proj-1")),
                &cfg,
                "me",
                &full_only(ExecutorKind::Claude),
                None
            ),
            Eligibility::Waiting(WaitReason::ExecutorNotReady(ExecutorKind::Claude))
        );
        assert!(matches!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None),
            Eligibility::Eligible(_)
        ));
    }

    #[test]
    fn personal_home_and_lean_off_bindings_use_the_full_checks() {
        let dir = tempfile::tempdir().unwrap();
        let ready = full_only(ExecutorKind::Claude);
        let mut personal = config_with("proj-1", dir.path(), None);
        personal.bindings[0].permission = Some(crate::config::PermissionProfile::Personal);
        let mut off = config_with("proj-1", dir.path(), None);
        off.bindings[0].lean = false;
        let at_home = config_with("proj-1", dir.path(), None);
        for (cfg, home) in [(personal, None), (off, None), (at_home, Some(dir.path()))] {
            assert!(
                matches!(
                    evaluate(&issue(Some("proj-1")), &cfg, "me", &ready, home),
                    Eligibility::Eligible(_)
                ),
                "{:?}",
                cfg.bindings[0]
            );
        }
    }
}
