//! Data types shared by the engine, the store and the IPC layer.
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::ExecutorKind;

/// The subset of a Linear issue IssueRelay needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub uuid: String,
    /// Human identifier such as `DEMO-1`.
    pub identifier: String,
    pub title: String,
    pub url: String,
    /// Status name, e.g. `Todo`.
    pub state: String,
    pub labels: Vec<String>,
    /// Linear project `uuid`, matching `Binding.project_id`; not the `P-<TEAM>-N` identifier.
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub assignee_id: Option<String>,
    pub parent_identifier: Option<String>,
    /// Linear priority: 0 none, 1 urgent … 4 low.
    pub priority: u8,
    pub created_at: DateTime<Utc>,
    /// Linear's own clock; the only timestamp used to decide re-queuing.
    pub updated_at: DateTime<Utc>,
    /// The kind of the status (`statusType`): `triage`, `backlog`, `unstarted`, `started`,
    /// `completed` or `canceled`; `None` when Linear did not say.
    #[serde(default)]
    pub state_type: Option<String>,
    /// The identifiers of the issues that block this one (`relations.blockedBy`); `None` when the
    /// reply did not carry relations, as `list_issues` replies never do.
    #[serde(default)]
    pub blocked_by: Option<Vec<String>>,
}

/// Local life cycle of one issue's run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    WaitingBinding,
    Queued,
    Running,
    AwaitingAcceptance,
    NeedsInfo,
    Done,
    Failed,
    Canceled,
}

impl JobState {
    /// Every state, in display order.
    pub const ALL: [JobState; 8] = [
        JobState::WaitingBinding,
        JobState::Queued,
        JobState::Running,
        JobState::AwaitingAcceptance,
        JobState::NeedsInfo,
        JobState::Done,
        JobState::Failed,
        JobState::Canceled,
    ];

    /// The stored name, shared by the database and IPC.
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::WaitingBinding => "waiting_binding",
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::AwaitingAcceptance => "awaiting_acceptance",
            JobState::NeedsInfo => "needs_info",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Canceled => "canceled",
        }
    }

    /// Parses a stored name (exact match); `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    /// Waiting, queued or running: the issue is still being handled.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            JobState::WaitingBinding | JobState::Queued | JobState::Running
        )
    }
}

/// What a run is for: carrying out the issue, or only writing a plan for it (run-improvements
/// design, section 10). A run is a planning run when its issue carries the plan label instead of
/// the trigger label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Implement,
    Plan,
}

impl RunMode {
    /// The value of the `mode` column: `plan`, or NULL for a usual run, which is also what rows
    /// written before planning runs existed hold.
    pub fn stored(self) -> Option<&'static str> {
        match self {
            RunMode::Implement => None,
            RunMode::Plan => Some("plan"),
        }
    }

    /// Reads the `mode` column; anything but `plan` is a usual run.
    pub fn from_stored(text: Option<&str>) -> Self {
        match text {
            Some("plan") => RunMode::Plan,
            _ => RunMode::Implement,
        }
    }
}

/// What a run used, as the executor reported it in its output (run-improvements design, section 7);
/// a figure it did not report is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunUsage {
    /// What this run cost, in millionths of a US dollar; only Claude reports a cost. Under a
    /// subscription it is Claude Code's estimate at API prices, not a charge.
    pub cost_micro_usd: Option<i64>,
    /// Claude's running total for the whole session, which a resumed run carries on; the engine
    /// works this run's own cost out of it.
    pub session_cost_micro_usd: Option<i64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Claude's `num_turns`; Codex reports nothing like it.
    pub turns: Option<u32>,
}

impl RunUsage {
    /// No figure at all.
    pub fn is_empty(&self) -> bool {
        *self == RunUsage::default()
    }
}

/// One run of one issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub issue_uuid: String,
    pub issue_identifier: String,
    pub title: String,
    pub binding_id: Option<String>,
    pub directory: Option<PathBuf>,
    pub executor: Option<ExecutorKind>,
    pub state: JobState,
    pub reason: Option<String>,
    pub priority: u8,
    pub issue_created_at: DateTime<Utc>,
    /// Linear's own `updatedAt` when the issue was discovered. The requeue rule compares
    /// only Linear's clocks, never this machine's.
    pub discovered_updated_at: DateTime<Utc>,
    /// Linear's own `updatedAt` recorded when the run was verified. When set, it replaces
    /// `discovered_updated_at` in the requeue rule.
    pub verified_updated_at: Option<DateTime<Utc>>,
    /// This machine's clock, for display only.
    pub created_at: DateTime<Utc>,
    /// This machine's clock, for display only.
    pub started_at: Option<DateTime<Utc>>,
    /// This machine's clock, for display only.
    pub finished_at: Option<DateTime<Utc>>,
    pub log_path: Option<PathBuf>,
    pub summary: Option<String>,
    /// A usual run or a planning run. Rows and hosts from before planning runs read as usual runs.
    #[serde(default)]
    pub mode: RunMode,
    /// The `result` of the AI's last line of JSON (`in_testing`, `needs_info`, `failed`,
    /// `planned`), when there was one.
    #[serde(default)]
    pub report: Option<String>,
    /// The executor's session: Claude's `session_id`, Codex's `thread_id`, when the run started one.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The earlier run whose session this run continues (run-improvements design, section 11).
    #[serde(default)]
    pub resume_job_id: Option<i64>,
    /// What the run used, when the executor reported anything.
    #[serde(default)]
    pub usage: Option<RunUsage>,
}

/// The executor process of a run, as recorded when it started: the PID and the kernel's start
/// time, `"<seconds>.<microseconds>"` since the epoch. A later host ends the process only while
/// that PID still has that start time; otherwise the PID belongs to someone else now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessTag {
    pub pid: u32,
    pub started: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_state_names_round_trip() {
        for state in JobState::ALL {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
        }
        assert_eq!(JobState::parse("nope"), None);
    }

    #[test]
    fn only_waiting_queued_running_are_active() {
        let active: Vec<_> = JobState::ALL
            .into_iter()
            .filter(|s| s.is_active())
            .collect();
        assert_eq!(
            active,
            vec![
                JobState::WaitingBinding,
                JobState::Queued,
                JobState::Running
            ]
        );
    }

    #[test]
    fn job_state_names_are_fixed() {
        let names: Vec<&str> = JobState::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "waiting_binding",
                "queued",
                "running",
                "awaiting_acceptance",
                "needs_info",
                "done",
                "failed",
                "canceled"
            ]
        );
    }

    #[test]
    fn serde_names_match_as_str() {
        #[derive(serde::Serialize)]
        struct Wrap {
            state: JobState,
        }
        for state in JobState::ALL {
            let text = toml::to_string(&Wrap { state }).unwrap();
            assert_eq!(text.trim(), format!("state = \"{}\"", state.as_str()));
        }
    }

    #[test]
    fn run_mode_is_stored_as_plan_or_nothing() {
        assert_eq!(RunMode::Plan.stored(), Some("plan"));
        assert_eq!(RunMode::Implement.stored(), None);
        for mode in [RunMode::Implement, RunMode::Plan] {
            assert_eq!(RunMode::from_stored(mode.stored()), mode);
        }
        assert_eq!(
            RunMode::from_stored(Some("a mode of a newer release")),
            RunMode::Implement
        );
    }

    #[test]
    fn a_job_from_an_older_host_reads_without_run_details() {
        // What a host from before the run improvements sends over IPC.
        let older = serde_json::json!({
            "id": 1, "issue_uuid": "u-1", "issue_identifier": "DEMO-1", "title": "Demo",
            "binding_id": null, "directory": null, "executor": null, "state": "failed",
            "reason": null, "priority": 0, "issue_created_at": "2026-10-01T08:00:00Z",
            "discovered_updated_at": "2026-10-01T08:00:00Z", "verified_updated_at": null,
            "created_at": "2026-10-01T08:00:00Z", "started_at": null, "finished_at": null,
            "log_path": null, "summary": null
        });
        let job: Job = serde_json::from_value(older).unwrap();
        assert_eq!(job.mode, RunMode::Implement);
        assert_eq!(
            (job.report, job.session_id, job.resume_job_id, job.usage),
            (None, None, None, None)
        );
        assert!(RunUsage::default().is_empty());
        assert!(
            !RunUsage {
                turns: Some(1),
                ..RunUsage::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn all_lists_every_variant_once() {
        // Adding a variant breaks this match until ALL is updated as well.
        for state in JobState::ALL {
            match state {
                JobState::WaitingBinding
                | JobState::Queued
                | JobState::Running
                | JobState::AwaitingAcceptance
                | JobState::NeedsInfo
                | JobState::Done
                | JobState::Failed
                | JobState::Canceled => {}
            }
        }
        let unique: std::collections::HashSet<JobState> = JobState::ALL.into_iter().collect();
        assert_eq!(unique.len(), JobState::ALL.len());
    }
}
