//! Log retention: run logs older than the retention period, and the oldest ones beyond the total
//! size limit, are deleted (design section 11; 30 days and 1 GB by default).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};

/// What one cleanup removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// How many logs were deleted.
    pub removed: usize,
    /// How many bytes they took.
    pub freed_bytes: u64,
}

/// A run log as the engine names it: `<job id>.jsonl`.
fn is_run_log(name: &str) -> bool {
    name.strip_suffix(".jsonl")
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Deletes the run logs in `dir` last modified before `now - keep`, then the oldest of the rest
/// until they take at most `max_total` bytes. Logs in `in_use` (runs in progress) and files that
/// are not run logs are never deleted. A missing directory is nothing to clean; a log that cannot
/// be deleted is logged and skipped.
pub fn cleanup(
    dir: &Path,
    keep: Duration,
    max_total: u64,
    in_use: &HashSet<PathBuf>,
    now: SystemTime,
) -> Result<CleanupReport> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(CleanupReport::default()),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", dir.display())),
    };
    let mut logs: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot read {}", dir.display()))?;
        if !entry.file_name().to_str().is_some_and(is_run_log) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        logs.push((meta.modified().unwrap_or(now), meta.len(), entry.path()));
    }
    // Oldest first: expired logs come first, and the size limit removes the oldest.
    logs.sort_by_key(|(modified, _, _)| *modified);
    let cutoff = now.checked_sub(keep);
    let mut total: u64 = logs.iter().map(|(_, len, _)| *len).sum();
    let mut report = CleanupReport::default();
    for (modified, len, path) in &logs {
        let expired = cutoff.is_some_and(|cutoff| *modified < cutoff);
        if !(expired || total > max_total) || in_use.contains(path) {
            continue;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {
                report.removed += 1;
                report.freed_bytes += len;
                total -= len;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => total -= len,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not delete an old run log")
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: Duration = Duration::from_secs(86_400);

    /// Writes `bytes` bytes to `dir/name` and dates the file `age` before `now`.
    fn log(dir: &Path, name: &str, bytes: usize, age: Duration, now: SystemTime) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; bytes]).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(now - age).unwrap();
        path
    }

    #[test]
    fn old_logs_go_and_recent_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let old = log(dir.path(), "1.jsonl", 10, 31 * DAY, now);
        let recent = log(dir.path(), "2.jsonl", 10, 29 * DAY, now);
        let report = cleanup(dir.path(), 30 * DAY, u64::MAX, &HashSet::new(), now).unwrap();
        assert_eq!(
            report,
            CleanupReport {
                removed: 1,
                freed_bytes: 10
            }
        );
        assert!(!old.exists());
        assert!(recent.exists());
    }

    #[test]
    fn the_oldest_logs_go_until_the_rest_fits_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let a = log(dir.path(), "1.jsonl", 100, 3 * DAY, now);
        let b = log(dir.path(), "2.jsonl", 100, 2 * DAY, now);
        let c = log(dir.path(), "3.jsonl", 100, DAY, now);
        let report = cleanup(dir.path(), 30 * DAY, 150, &HashSet::new(), now).unwrap();
        assert_eq!(report.removed, 2);
        assert!(!a.exists() && !b.exists());
        assert!(c.exists(), "the newest log fits the limit on its own");
    }

    // The expired log is deleted for its age, so the size limit starts from what is left.
    #[test]
    fn deleting_expired_logs_counts_toward_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let expired = log(dir.path(), "1.jsonl", 100, 31 * DAY, now);
        let older = log(dir.path(), "2.jsonl", 100, 2 * DAY, now);
        let newest = log(dir.path(), "3.jsonl", 100, DAY, now);
        let report = cleanup(dir.path(), 30 * DAY, 150, &HashSet::new(), now).unwrap();
        assert_eq!(
            report,
            CleanupReport {
                removed: 2,
                freed_bytes: 200
            }
        );
        assert!(!expired.exists() && !older.exists());
        assert!(newest.exists());
    }

    #[test]
    fn logs_of_runs_in_progress_and_other_files_are_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let running = log(dir.path(), "7.jsonl", 100, 40 * DAY, now);
        let note = log(dir.path(), "notes.txt", 100, 40 * DAY, now);
        let odd = log(dir.path(), "x7.jsonl", 100, 40 * DAY, now);
        let in_use: HashSet<PathBuf> = [running.clone()].into();
        let report = cleanup(dir.path(), 30 * DAY, 0, &in_use, now).unwrap();
        assert_eq!(report.removed, 0);
        assert!(running.exists() && note.exists() && odd.exists());
    }

    #[test]
    fn a_missing_directory_is_nothing_to_clean() {
        let dir = tempfile::tempdir().unwrap();
        let report = cleanup(
            &dir.path().join("logs"),
            DAY,
            0,
            &HashSet::new(),
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(report, CleanupReport::default());
    }
}
