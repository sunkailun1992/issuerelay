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
