//! Which issues IssueRelay should pick up, and when an issue it already handled comes back.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{
    Binding, Config, ExecutorKind, LeanStatus, PermissionProfile, Route, TriggerNames, same_folder,
};
use crate::model::{Issue, Job, RunMode};

/// Why an issue that looks eligible has to wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitReason {
    ProjectNotBound,
    DirectoryMissing(PathBuf),
    ExecutorNotReady(ExecutorKind),
    /// The executor takes usual runs, but the binding's runs are lean and the lean checks fail.
    LeanNotReady(ExecutorKind),
    /// Issues that block this one in Linear and are neither completed nor canceled, by identifier.
    BlockedBy(Vec<String>),
    /// The issue carries the routing labels of several bindings of its project: these labels.
    LabelsMatchSeveralBindings(Vec<String>),
    /// The project's bindings all have routing labels, and the issue carries none of them.
    NoBindingForLabels,
    /// A planning run is always lean, which a binding of the home folder cannot be.
    PlanNeedsLean,
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
            WaitReason::LeanNotReady(kind) => {
                format!("executor {} is not ready for lean runs", kind.as_str())
            }
            WaitReason::BlockedBy(issues) => {
                format!("blocked by {}, which are not done yet", issues.join(", "))
            }
            WaitReason::LabelsMatchSeveralBindings(labels) => format!(
                "the labels {} match several bindings of the project",
                labels.join(", ")
            ),
            WaitReason::NoBindingForLabels => {
                "no binding of the project takes the issue's labels".into()
            }
            WaitReason::PlanNeedsLean => {
                "planning runs need a bound folder other than the home folder".into()
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
    LeanNotReady { executor: ExecutorKind },
    BlockedBy { issues: Vec<String> },
    LabelsMatchSeveralBindings { labels: Vec<String> },
    NoBindingForLabels,
    PlanNeedsLean,
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
            WaitReason::LeanNotReady(kind) => WaitReasonView::LeanNotReady { executor: *kind },
            WaitReason::BlockedBy(issues) => WaitReasonView::BlockedBy {
                issues: issues.clone(),
            },
            WaitReason::LabelsMatchSeveralBindings(labels) => {
                WaitReasonView::LabelsMatchSeveralBindings {
                    labels: labels.clone(),
                }
            }
            WaitReason::NoBindingForLabels => WaitReasonView::NoBindingForLabels,
            WaitReason::PlanNeedsLean => WaitReasonView::PlanNeedsLean,
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

    /// Whether only the lean checks keep `kind` from the runs of a binding whose lean status is
    /// `lean`: the runs are lean, and the executor is ready for usual runs but not for lean ones.
    pub fn held_back_by_lean(&self, kind: ExecutorKind, lean: LeanStatus) -> bool {
        lean.applies() && !self.lean.contains(&kind) && self.full.contains(&kind)
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

/// What a run of `issue` would be for, when it is a candidate at all: assigned to `me` and in the
/// Todo state, a usual run when it carries the trigger label, a planning run when it carries the
/// plan label instead (run-improvements design, section 10.1).
pub fn candidate_mode(issue: &Issue, names: &TriggerNames, me: &str) -> Option<RunMode> {
    if issue.assignee_id.as_deref() != Some(me) || !same_name(&issue.state, &names.todo) {
        return None;
    }
    let carries = |wanted: &str| issue.labels.iter().any(|label| same_name(label, wanted));
    if carries(&names.label) {
        Some(RunMode::Implement)
    } else if carries(&names.plan_label) {
        Some(RunMode::Plan)
    } else {
        None
    }
}

/// Assigned to `me`, in the Todo state, and carries the trigger label or the plan label.
pub fn is_candidate(issue: &Issue, names: &TriggerNames, me: &str) -> bool {
    candidate_mode(issue, names, me).is_some()
}

/// Checks the binding, that its directory exists (the only filesystem access in this
/// module, besides comparing that directory with `home` for its lean status), that the
/// executor is ready for the binding's runs, lean or not, and last that nothing blocks the issue:
/// `blockers` are the issues that block it and are not done yet, as the engine found them. A
/// planning run is always lean (it waits in the home folder, which cannot be) and is never held
/// back by blockers (run-improvements design, W8 and W9), except that a binding on personal
/// settings keeps them. `home` is the user's home folder; `None` when it is not known.
pub fn evaluate(
    issue: &Issue,
    cfg: &Config,
    me: &str,
    ready: &Readiness,
    home: Option<&Path>,
    blockers: &[String],
) -> Eligibility {
    let Some(mode) = candidate_mode(issue, &cfg.settings.trigger, me) else {
        return Eligibility::NotCandidate;
    };
    let binding = match cfg.route(issue) {
        Route::Bound(binding) => binding,
        Route::NotBound => return Eligibility::Waiting(WaitReason::ProjectNotBound),
        Route::SeveralMatch(labels) => {
            return Eligibility::Waiting(WaitReason::LabelsMatchSeveralBindings(labels));
        }
        Route::NoMatch => return Eligibility::Waiting(WaitReason::NoBindingForLabels),
    };
    if !binding.directory.is_dir() {
        return Eligibility::Waiting(WaitReason::DirectoryMissing(binding.directory.clone()));
    }
    let kind = cfg.executor_for(binding);
    let lean = match mode {
        RunMode::Implement => cfg.lean_for(binding, home),
        // A binding on personal settings keeps them, also in a planning run: the user's own rules
        // decide what the run may use besides reading.
        RunMode::Plan if cfg.permission_for(binding) == PermissionProfile::Personal => {
            LeanStatus::Personal
        }
        RunMode::Plan if home.is_some_and(|home| same_folder(&binding.directory, home)) => {
            return Eligibility::Waiting(WaitReason::PlanNeedsLean);
        }
        RunMode::Plan => LeanStatus::On,
    };
    if !ready.ready(kind, lean) {
        let reason = if ready.held_back_by_lean(kind, lean) {
            WaitReason::LeanNotReady(kind)
        } else {
            WaitReason::ExecutorNotReady(kind)
        };
        return Eligibility::Waiting(reason);
    }
    if mode == RunMode::Implement && !blockers.is_empty() {
        return Eligibility::Waiting(WaitReason::BlockedBy(blockers.to_vec()));
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
    use crate::model::{Issue, Job, JobState, RunMode};
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
            state_type: None,
            blocked_by: None,
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
            labels: Vec::new(),
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
            mode: RunMode::Implement,
            report: None,
            session_id: None,
            resume_job_id: None,
            usage: None,
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
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
        assert_eq!(
            evaluate(&issue(None), &cfg, "me", &all_ready(), None, &[]),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
    }

    #[test]
    fn missing_directory_waits() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        let cfg = config_with("proj-1", &missing, None);
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]),
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
            evaluate(&issue(Some("proj-1")), &cfg, "me", &only_claude, None, &[]),
            Eligibility::Waiting(WaitReason::ExecutorNotReady(ExecutorKind::Codex))
        );
    }

    #[test]
    fn bound_ready_issue_is_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        match evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]) {
            Eligibility::Eligible(binding) => assert_eq!(binding.project_id, "proj-1"),
            other => panic!("expected eligible, got {other:?}"),
        }
    }

    #[test]
    fn non_candidate_is_not_evaluated_further() {
        let cfg = Config::default();
        assert_eq!(
            evaluate(&issue(None), &cfg, "other", &all_ready(), None, &[]),
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
            (
                WaitReason::LeanNotReady(ExecutorKind::Claude),
                serde_json::json!({"code": "lean_not_ready", "executor": "claude"}),
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
        assert_eq!(
            WaitReason::LeanNotReady(ExecutorKind::Claude).describe(),
            "executor claude is not ready for lean runs"
        );
    }

    #[test]
    fn a_blocked_issue_waits_after_the_binding_checks() {
        let blockers = ["DEMO-4".to_string(), "DEMO-5".to_string()];
        // A problem with the setup is shown first: it needs the user, a blocker only time.
        assert_eq!(
            evaluate(
                &issue(Some("proj-1")),
                &Config::default(),
                "me",
                &all_ready(),
                None,
                &blockers
            ),
            Eligibility::Waiting(WaitReason::ProjectNotBound)
        );
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        assert_eq!(
            evaluate(
                &issue(Some("proj-1")),
                &cfg,
                "me",
                &all_ready(),
                None,
                &blockers
            ),
            Eligibility::Waiting(WaitReason::BlockedBy(blockers.to_vec()))
        );
        assert!(matches!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]),
            Eligibility::Eligible(_)
        ));
    }

    /// `cfg` with a binding of `proj-1` that takes the issues labeled `label` (to `dir`).
    fn add_label_binding(cfg: &mut Config, label: &str, dir: &Path) {
        cfg.bindings.push(Binding {
            id: format!("proj-1#{label}"),
            labels: vec![label.into()],
            directory: dir.to_path_buf(),
            ..cfg.bindings[0].clone()
        });
    }

    #[test]
    fn a_label_binding_takes_the_issues_with_its_label() {
        let default_dir = tempfile::tempdir().unwrap();
        let frontend_dir = tempfile::tempdir().unwrap();
        let mut cfg = config_with("proj-1", default_dir.path(), None);
        add_label_binding(&mut cfg, "Frontend", frontend_dir.path());
        let mut labeled = issue(Some("proj-1"));
        labeled.labels.push("frontend".into());
        match evaluate(&labeled, &cfg, "me", &all_ready(), None, &[]) {
            Eligibility::Eligible(binding) => assert_eq!(binding.id, "proj-1#Frontend"),
            other => panic!("expected the label binding, got {other:?}"),
        }
        match evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]) {
            Eligibility::Eligible(binding) => assert_eq!(binding.id, "proj-1"),
            other => panic!("expected the default binding, got {other:?}"),
        }
    }

    #[test]
    fn two_matching_label_bindings_make_the_issue_wait() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config_with("proj-1", dir.path(), None);
        add_label_binding(&mut cfg, "Frontend", dir.path());
        add_label_binding(&mut cfg, "Backend", dir.path());
        let mut both = issue(Some("proj-1"));
        both.labels
            .extend(["Frontend".to_string(), "Backend".to_string()]);
        assert_eq!(
            evaluate(&both, &cfg, "me", &all_ready(), None, &[]),
            Eligibility::Waiting(WaitReason::LabelsMatchSeveralBindings(vec![
                "Frontend".into(),
                "Backend".into()
            ]))
        );
    }

    #[test]
    fn no_default_and_no_matching_label_waits() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config_with("proj-1", dir.path(), None);
        add_label_binding(&mut cfg, "Frontend", dir.path());
        cfg.bindings.remove(0);
        assert_eq!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]),
            Eligibility::Waiting(WaitReason::NoBindingForLabels)
        );
    }

    #[test]
    fn routing_wait_reasons_have_codes_and_sentences() {
        let several =
            WaitReason::LabelsMatchSeveralBindings(vec!["Frontend".into(), "Backend".into()]);
        assert_eq!(
            several.describe(),
            "the labels Frontend, Backend match several bindings of the project"
        );
        assert_eq!(
            serde_json::to_value(several.view()).unwrap(),
            serde_json::json!({"code": "labels_match_several_bindings", "labels": ["Frontend", "Backend"]})
        );
        assert_eq!(
            WaitReason::NoBindingForLabels.describe(),
            "no binding of the project takes the issue's labels"
        );
        assert_eq!(
            serde_json::to_value(WaitReason::NoBindingForLabels.view()).unwrap(),
            serde_json::json!({"code": "no_binding_for_labels"})
        );
    }

    /// `issue(Some("proj-1"))` with the plan label instead of the trigger label.
    fn plan_issue() -> Issue {
        Issue {
            labels: vec!["ai-plan".into()],
            ..issue(Some("proj-1"))
        }
    }

    #[test]
    fn candidate_mode_prefers_a_usual_run_when_both_labels_are_there() {
        let names = TriggerNames::default();
        assert_eq!(
            candidate_mode(&issue(None), &names, "me"),
            Some(RunMode::Implement)
        );
        assert_eq!(
            candidate_mode(&plan_issue(), &names, "me"),
            Some(RunMode::Plan)
        );
        let mut both = plan_issue();
        both.labels.push(" AI-Ready ".into());
        assert_eq!(
            candidate_mode(&both, &names, "me"),
            Some(RunMode::Implement)
        );
        // Assigned to someone else, or not in Todo: no candidate whatever the labels.
        assert_eq!(candidate_mode(&plan_issue(), &names, "other"), None);
        let mut started = plan_issue();
        started.state = "In Progress".into();
        assert_eq!(candidate_mode(&started, &names, "me"), None);
        assert!(is_candidate(&plan_issue(), &names, "me"));
    }

    #[test]
    fn a_planning_run_is_lean_whatever_the_binding_says() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config_with("proj-1", dir.path(), None);
        cfg.bindings[0].lean = false;
        // Ready for usual runs only: a usual run of this binding (lean off) may start, a planning
        // run needs the lean checks.
        let ready = full_only(ExecutorKind::Claude);
        assert!(matches!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &ready, None, &[]),
            Eligibility::Eligible(_)
        ));
        assert_eq!(
            evaluate(&plan_issue(), &cfg, "me", &ready, None, &[]),
            Eligibility::Waiting(WaitReason::LeanNotReady(ExecutorKind::Claude))
        );
        assert!(matches!(
            evaluate(&plan_issue(), &cfg, "me", &all_ready(), None, &[]),
            Eligibility::Eligible(_)
        ));
    }

    #[test]
    fn a_planning_run_waits_in_the_home_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        assert_eq!(
            evaluate(
                &plan_issue(),
                &cfg,
                "me",
                &all_ready(),
                Some(dir.path()),
                &[]
            ),
            Eligibility::Waiting(WaitReason::PlanNeedsLean)
        );
        assert_eq!(
            WaitReason::PlanNeedsLean.describe(),
            "planning runs need a bound folder other than the home folder"
        );
        assert_eq!(
            serde_json::to_value(WaitReason::PlanNeedsLean.view()).unwrap(),
            serde_json::json!({"code": "plan_needs_lean"})
        );
    }

    #[test]
    fn a_planning_run_of_a_binding_on_personal_settings_keeps_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config_with("proj-1", dir.path(), None);
        cfg.bindings[0].permission = Some(PermissionProfile::Personal);
        // Not lean: the usual checks are enough, and the home folder is fine.
        let ready = full_only(ExecutorKind::Claude);
        assert!(matches!(
            evaluate(&plan_issue(), &cfg, "me", &ready, Some(dir.path()), &[]),
            Eligibility::Eligible(_)
        ));
    }

    #[test]
    fn blockers_do_not_hold_back_a_planning_run() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        let blockers = ["DEMO-4".to_string()];
        assert!(matches!(
            evaluate(&plan_issue(), &cfg, "me", &all_ready(), None, &blockers),
            Eligibility::Eligible(_)
        ));
    }

    #[test]
    fn blocked_by_has_a_code_and_a_sentence() {
        let reason = WaitReason::BlockedBy(vec!["DEMO-4".into(), "DEMO-5".into()]);
        assert_eq!(
            reason.describe(),
            "blocked by DEMO-4, DEMO-5, which are not done yet"
        );
        let json = serde_json::json!({"code": "blocked_by", "issues": ["DEMO-4", "DEMO-5"]});
        assert_eq!(serde_json::to_value(reason.view()).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<WaitReasonView>(json).unwrap(),
            reason.view()
        );
    }

    #[test]
    fn lean_bindings_wait_for_the_lean_checks() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config_with("proj-1", dir.path(), None);
        // Ready for usual runs: only the lean checks hold the binding back, and the reason says
        // so (one not ready at all still waits with `ExecutorNotReady`: see
        // `executor_not_ready_waits`).
        assert_eq!(
            evaluate(
                &issue(Some("proj-1")),
                &cfg,
                "me",
                &full_only(ExecutorKind::Claude),
                None,
                &[]
            ),
            Eligibility::Waiting(WaitReason::LeanNotReady(ExecutorKind::Claude))
        );
        assert!(matches!(
            evaluate(&issue(Some("proj-1")), &cfg, "me", &all_ready(), None, &[]),
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
                    evaluate(&issue(Some("proj-1")), &cfg, "me", &ready, home, &[]),
                    Eligibility::Eligible(_)
                ),
                "{:?}",
                cfg.bindings[0]
            );
        }
    }
}
