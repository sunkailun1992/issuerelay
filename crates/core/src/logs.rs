//! Log retention: run logs older than the retention period, and the oldest ones beyond the total
//! size limit, are deleted (design section 11; 30 days and 1 GB by default).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, ensure};

/// What one cleanup removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    /// How many logs were deleted.
    pub removed: usize,
    /// How many bytes they took.
    pub freed_bytes: u64,
}

/// Current run logs and completed private snapshots of previous attempts.
fn is_run_log(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".jsonl") else {
        return false;
    };
    let (id, attempt) = stem
        .split_once(".attempt-")
        .map_or((stem, None), |(id, suffix)| (id, Some(suffix)));
    !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_digit())
        && attempt.is_none_or(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn ordinary_file(meta: &std::fs::Metadata) -> bool {
    if !meta.file_type().is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return false;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

fn same_log(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return false;
        }
    }
    ordinary_file(after)
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

fn no_follow(options: &mut std::fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT; allow readers, but no writer or replacement while open.
        options.custom_flags(0x0020_0000).share_mode(1);
    }
}

/// Copies a closed attempt before the runner can truncate its current log. Pending copies are
/// not cleanup candidates; a completed copy has a unique name and shares the normal log budget.
pub(crate) fn preserve_previous(path: &Path) -> Result<Option<PathBuf>> {
    let before = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        ordinary_file(&before),
        "the previous log is not an ordinary file"
    );
    let id = path
        .file_stem()
        .and_then(|id| id.to_str())
        .context("invalid run log name")?;
    ensure!(
        !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()),
        "invalid run log name"
    );
    let parent = path.parent().context("run log has no parent")?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let mut input = options.open(path)?;
    ensure!(
        same_log(&before, &input.metadata()?),
        "the previous log changed before copying"
    );
    let mut copy = tempfile::Builder::new()
        .prefix(".pending-attempt-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    let copied = std::io::copy(&mut input, copy.as_file_mut())?;
    ensure!(
        copied == before.len()
            && same_log(&before, &input.metadata()?)
            && same_log(&before, &std::fs::symlink_metadata(path)?),
        "the previous log changed while copying"
    );
    copy.as_file().sync_all()?;
    let suffix = copy
        .path()
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix(".pending-attempt-"))
        .context("invalid pending log name")?;
    let destination = parent.join(format!("{id}.attempt-{suffix}.jsonl"));
    copy.persist_noclobber(&destination).map_err(|e| e.error)?;
    Ok(Some(destination.canonicalize()?))
}

/// Records the host's location pointer after the runner has closed its log, also when spawning
/// failed before the runner could write its opening notes.
pub(crate) fn record_previous(path: &Path, previous: &Path) -> Result<()> {
    use std::io::Write;
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        ensure!(
            ordinary_file(&meta),
            "the current log is not an ordinary file"
        );
    }
    let mut options = std::fs::OpenOptions::new();
    options.append(true).create(true);
    no_follow(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    ensure!(
        ordinary_file(&file.metadata()?),
        "the current log is not an ordinary file"
    );
    let mut note = serde_json::json!({"stream":"runner", "text":format!("Previous attempt log: {}", previous.display())}).to_string();
    note.push('\n');
    file.write_all(note.as_bytes())?;
    file.sync_all()?;
    Ok(())
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
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !ordinary_file(&meta) {
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

    #[test]
    fn previous_attempt_copies_are_unique_exact_and_leave_the_current_log_alone() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("12.jsonl");
        assert!(preserve_previous(&current).unwrap().is_none());
        std::fs::write(&current, b"first\n\0raw bytes\n").unwrap();
        let first = preserve_previous(&current).unwrap().unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"first\n\0raw bytes\n");
        assert_eq!(std::fs::read(&current).unwrap(), b"first\n\0raw bytes\n");
        std::fs::write(&current, b"second attempt\n").unwrap();
        let second = preserve_previous(&current).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"first\n\0raw bytes\n");
        assert_eq!(std::fs::read(second).unwrap(), b"second attempt\n");
    }

    #[test]
    fn previous_attempts_share_age_and_size_budgets_but_partial_and_other_files_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let old = log(dir.path(), "1.attempt-aB12.jsonl", 10, 31 * DAY, now);
        let older = log(dir.path(), "1.attempt-Cd34.jsonl", 10, 2 * DAY, now);
        let current = log(dir.path(), "1.jsonl", 10, DAY, now);
        let pending = log(dir.path(), ".pending-attempt-aB12.tmp", 10, 31 * DAY, now);
        let unrelated = log(
            dir.path(),
            "1.attempt-../secret.jsonl".replace('/', "_").as_str(),
            10,
            31 * DAY,
            now,
        );
        let report = cleanup(dir.path(), 30 * DAY, 10, &HashSet::new(), now).unwrap();
        assert_eq!(report.removed, 2);
        assert!(!old.exists() && !older.exists());
        assert!(current.exists() && pending.exists() && unrelated.exists());
    }

    #[test]
    fn recording_a_previous_attempt_encodes_a_single_host_note_without_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let current = dir.path().join("1.jsonl");
        std::fs::write(&current, "{\"type\":\"result\"}\n").unwrap();
        let previous = dir.path().join("synthetic \"quoted\" path");
        record_previous(&current, &previous).unwrap();
        let text = std::fs::read_to_string(current).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "{\"type\":\"result\"}");
        let note: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(note["stream"], "runner");
        assert_eq!(
            note["text"],
            format!("Previous attempt log: {}", previous.display())
        );
    }

    #[cfg(unix)]
    #[test]
    fn linked_previous_logs_are_neither_copied_nor_cleaned() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let current = dir.path().join("1.jsonl");
        std::fs::write(&source, b"unrelated bytes").unwrap();
        std::os::unix::fs::symlink(&source, &current).unwrap();
        assert!(preserve_previous(&current).is_err());
        assert!(record_previous(&current, &source).is_err());
        assert_eq!(
            cleanup(dir.path(), DAY, 0, &HashSet::new(), SystemTime::now())
                .unwrap()
                .removed,
            0
        );
        std::fs::remove_file(&current).unwrap();
        std::fs::hard_link(&source, &current).unwrap();
        assert!(preserve_previous(&current).is_err());
        assert_eq!(
            cleanup(dir.path(), DAY, 0, &HashSet::new(), SystemTime::now())
                .unwrap()
                .removed,
            0
        );
        assert_eq!(std::fs::read(source).unwrap(), b"unrelated bytes");
    }

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
