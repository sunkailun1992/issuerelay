//! Bounded read-only Git facts for a workflow checkpoint.

use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::text::strip_control;
use crate::workflow::{GitFacts, MAX_GIT_CHANGE_BYTES, MAX_GIT_CHANGES};

const GIT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_STATUS_BYTES: usize = 128 * 1024;
const EMPTY_GIT_CONFIG: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };
const GIT_PROGRAM: &str = if cfg!(windows) { "git.exe" } else { "git" };

/// Reads the HEAD, branch and status of the given worktree. An unborn branch has no HEAD and a
/// detached HEAD has no branch. If status cannot be read completely, no snapshot is returned.
pub async fn snapshot(directory: &Path) -> Option<GitFacts> {
    // Skip plain directories before starting Git. Worktrees and submodules have a .git file;
    // a binding to a subdirectory of a worktree still discovers the marker in an ancestor.
    if !directory
        .ancestors()
        .any(|parent| parent.join(".git").exists())
    {
        return None;
    }
    let program = which::which(GIT_PROGRAM).ok()?;
    // Git can invoke a worktree's configured clean/process filter even during status. Query
    // names only and proceed only on Git's documented "no matching keys" result.
    let (config_status, config_names) = read_git(
        &program,
        directory,
        &[
            "config",
            "--includes",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|process)$",
        ],
        1024,
    )
    .await?;
    if config_status.code() != Some(1) || !config_names.is_empty() {
        return None;
    }
    let (head, branch, status) = tokio::join!(
        read_git(&program, directory, &["rev-parse", "--verify", "HEAD"], 128),
        read_git(
            &program,
            directory,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            2048
        ),
        read_git(
            &program,
            directory,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=normal",
                "--ignore-submodules=all",
                "--find-renames",
            ],
            MAX_STATUS_BYTES,
        ),
    );
    let facts = GitFacts {
        head: successful_output(head).and_then(read_line),
        branch: successful_output(branch).and_then(read_line),
        changed: status_paths(&successful_output(status)?)?,
    };
    facts.validate().ok()?;
    Some(facts)
}

async fn read_git(
    program: &Path,
    directory: &Path,
    args: &[&str],
    limit: usize,
) -> Option<(ExitStatus, Vec<u8>)> {
    let mut command = Command::new(program);
    command
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
        ])
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", EMPTY_GIT_CONFIG)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    tokio::time::timeout(GIT_TIMEOUT, async {
        let mut child = command.spawn().ok()?;
        let mut stdout = child.stdout.take()?.take(limit as u64 + 1);
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).await.ok()?;
        if output.len() > limit {
            return None;
        }
        Some((child.wait().await.ok()?, output))
    })
    .await
    .ok()
    .flatten()
}

fn successful_output(result: Option<(ExitStatus, Vec<u8>)>) -> Option<Vec<u8>> {
    let (status, output) = result?;
    status.success().then_some(output)
}

fn read_line(output: Vec<u8>) -> Option<String> {
    let output = String::from_utf8(output).ok()?;
    let line = output.trim_end_matches(['\r', '\n']);
    (!line.is_empty()).then(|| line.to_owned())
}

fn status_paths(status: &[u8]) -> Option<Vec<String>> {
    if status.is_empty() {
        return Some(Vec::new());
    }
    let mut records = status.strip_suffix(&[0])?.split(|byte| *byte == 0);
    let mut changed = Vec::new();
    while let Some(record) = records.next() {
        if changed.len() == MAX_GIT_CHANGES {
            return None;
        }
        let (code, path) = record.split_at_checked(3)?;
        if code[2] != b' ' || path.is_empty() {
            return None;
        }
        let code = std::str::from_utf8(code).ok()?;
        let path = strip_control(&String::from_utf8_lossy(path));
        let line = if code.contains(['R', 'C']) {
            let original = records.next()?;
            if original.is_empty() {
                return None;
            }
            let original = strip_control(&String::from_utf8_lossy(original));
            format!("{code}{original} -> {path}")
        } else {
            format!("{code}{path}")
        };
        if line.len() > MAX_GIT_CHANGE_BYTES {
            return None;
        }
        changed.push(line);
    }
    Some(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn git(directory: &Path, args: &[&str]) -> String {
        let mut command = Command::new(which::which(GIT_PROGRAM).unwrap());
        command
            .env_clear()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", EMPTY_GIT_CONFIG)
            .env("GIT_TERMINAL_PROMPT", "0");
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        let output = command
            .args(["-c", "core.fsmonitor=false", "-c", "commit.gpgSign=false"])
            .args(args)
            .current_dir(directory)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[tokio::test]
    async fn snapshot_reports_head_branch_rename_and_untracked_path() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path();
        git(directory, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(directory.join("old name.txt"), "demo").unwrap();
        git(directory, &["add", "--", "old name.txt"]);
        git(
            directory,
            &[
                "-c",
                "user.name=Demo",
                "-c",
                "user.email=demo@example.org",
                "commit",
                "--quiet",
                "-m",
                "initial",
            ],
        );
        let head = git(directory, &["rev-parse", "HEAD"]);
        git(directory, &["mv", "--", "old name.txt", "new name.txt"]);
        std::fs::write(directory.join("untracked.txt"), "demo").unwrap();
        let before = std::fs::read(directory.join(".git/index")).unwrap();

        let facts = snapshot(directory).await.unwrap();
        assert_eq!(facts.head.as_deref(), Some(head.as_str()));
        assert_eq!(facts.branch.as_deref(), Some("main"));
        assert_eq!(
            facts.changed,
            ["R  old name.txt -> new name.txt", "?? untracked.txt"]
        );
        assert_eq!(std::fs::read(directory.join(".git/index")).unwrap(), before);
    }

    #[tokio::test]
    async fn snapshot_declines_repositories_with_configured_clean_filters() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
        git(
            root.path(),
            &[
                "config",
                "--local",
                "filter.demo.clean",
                "unused-demo-filter",
            ],
        );
        assert_eq!(snapshot(root.path()).await, None);
    }

    #[tokio::test]
    async fn snapshot_distinguishes_an_unborn_branch_from_a_missing_repo() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(snapshot(root.path()).await, None);
        git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
        let facts = snapshot(root.path()).await.unwrap();
        assert_eq!(facts.head, None);
        assert_eq!(facts.branch.as_deref(), Some("main"));
        assert!(facts.changed.is_empty());
    }
}
