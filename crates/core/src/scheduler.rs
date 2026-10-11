//! Picks which queued jobs start next: one run per directory, at most `max` overall,
//! urgent first, then oldest.

use std::collections::HashSet;
use std::path::PathBuf;

use chrono::{DateTime, Utc};

/// A queued job the scheduler may start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Store id of the job; the lowest id breaks the final tie.
    pub job_id: i64,
    /// The bound directory; at most one run per directory at a time. Directories are compared
    /// as literal `Path`s, so callers must pass one canonical spelling (the engine canonicalizes
    /// both sides).
    pub directory: PathBuf,
    /// Linear priority, valid range 0..=4: 1 urgent … 4 low, 0 none (sorted last).
    pub priority: u8,
    /// When the issue was created in Linear; older goes first.
    pub issue_created_at: DateTime<Utc>,
}

/// Linear priority 1 (urgent) … 4 (low) first; 0 means "no priority" and goes last.
fn rank(priority: u8) -> u8 {
    if priority == 0 { 5 } else { priority }
}

/// Returns the job ids to start now, in start order: urgent first (Linear priority 1…4, then
/// 0 = none), then the oldest issue, then the lowest job id. At most one run per directory
/// (counting `running_dirs`), and never more than `max` runs in total (counting `running`).
pub fn pick(
    queued: &[Candidate],
    running_dirs: &HashSet<PathBuf>,
    running: usize,
    max: usize,
) -> Vec<i64> {
    let mut ordered: Vec<&Candidate> = queued.iter().collect();
    ordered.sort_by_key(|c| (rank(c.priority), c.issue_created_at, c.job_id));
    let mut busy = running_dirs.clone();
    let mut picked = Vec::new();
    for candidate in ordered {
        if running + picked.len() >= max {
            break;
        }
        if busy.insert(candidate.directory.clone()) {
            picked.push(candidate.job_id);
        }
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn c(id: i64, dir: &str, priority: u8, minutes: i64) -> Candidate {
        Candidate {
            job_id: id,
            directory: PathBuf::from(dir),
            priority,
            issue_created_at: Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap()
                + Duration::minutes(minutes),
        }
    }

    #[test]
    fn respects_global_cap() {
        let queued = [c(1, "a", 0, 0), c(2, "b", 0, 1), c(3, "c", 0, 2)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 2), vec![1, 2]);
        assert_eq!(pick(&queued, &HashSet::new(), 1, 2), vec![1]);
        assert!(pick(&queued, &HashSet::new(), 2, 2).is_empty());
    }

    #[test]
    fn one_run_per_directory() {
        let queued = [c(1, "a", 0, 0), c(2, "a", 0, 1), c(3, "b", 0, 2)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 5), vec![1, 3]);
        let busy: HashSet<PathBuf> = [PathBuf::from("a")].into_iter().collect();
        assert_eq!(pick(&queued, &busy, 1, 5), vec![3]);
    }

    #[test]
    fn urgent_first_then_oldest_and_none_last() {
        let queued = [
            c(1, "a", 0, 0),
            c(2, "b", 3, 5),
            c(3, "c", 1, 9),
            c(4, "d", 3, 1),
        ];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 4), vec![3, 4, 2, 1]);
    }

    #[test]
    fn zero_cap_starts_nothing() {
        assert!(pick(&[c(1, "a", 0, 0)], &HashSet::new(), 0, 0).is_empty());
    }

    #[test]
    fn same_priority_and_time_break_ties_by_job_id() {
        // Ids and directories are in opposite order on purpose; a directory tie-break fails.
        let queued = [c(9, "a", 2, 0), c(4, "b", 2, 0)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 2), vec![4, 9]);
    }

    #[test]
    fn every_priority_level_in_order() {
        let queued = [
            c(1, "a", 4, 0),
            c(2, "b", 0, 0),
            c(3, "c", 2, 0),
            c(4, "d", 1, 0),
            c(5, "e", 3, 0),
        ];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 5), vec![4, 3, 5, 1, 2]);
    }

    #[test]
    fn more_running_than_the_cap_starts_nothing() {
        assert!(pick(&[c(1, "a", 0, 0)], &HashSet::new(), 3, 2).is_empty());
    }

    #[test]
    fn no_priority_goes_after_low_priority() {
        // The no-priority job is older and has the lower id on purpose: ranking 0 like 4 fails.
        let queued = [c(1, "a", 0, 0), c(2, "b", 4, 5)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 2), vec![2, 1]);
    }

    #[test]
    fn a_skipped_busy_directory_does_not_use_up_a_slot() {
        // Job 2 shares job 1's directory and is skipped; job 3 must still get the second slot.
        let queued = [c(1, "a", 1, 0), c(2, "a", 2, 1), c(3, "b", 3, 2)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 2), vec![1, 3]);
    }

    #[test]
    fn the_best_ranked_job_takes_a_contested_directory() {
        // Listed first but low priority: the urgent job listed later must win directory "a".
        let queued = [c(1, "a", 4, 0), c(2, "a", 1, 1)];
        assert_eq!(pick(&queued, &HashSet::new(), 0, 2), vec![2]);
    }

    #[test]
    fn a_directory_with_a_running_job_does_not_use_up_a_slot() {
        let queued = [c(1, "a", 1, 0), c(2, "b", 2, 1)];
        let busy: HashSet<PathBuf> = [PathBuf::from("a")].into_iter().collect();
        assert_eq!(pick(&queued, &busy, 1, 2), vec![2]);
    }
}
