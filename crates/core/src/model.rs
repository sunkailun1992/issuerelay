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

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    /// Waiting, queued or running: the issue is still being tracked.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            JobState::WaitingBinding | JobState::Queued | JobState::Running
        )
    }
}

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
    pub discovered_updated_at: DateTime<Utc>,
    pub verified_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
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
}
