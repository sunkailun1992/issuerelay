//! Builds the command line for one executor run. The prompt always travels on stdin and the
//! working directory is set on the process, so no user-controlled text ever becomes an
//! argument (this keeps `claude.cmd` / `codex.cmd` safe on Windows).
pub mod outcome;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{ExecutorKind, PermissionProfile};

/// One executor process: the program, its fixed arguments, the working directory, and the
/// prompt written to stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Written to the child's stdin, then stdin is closed.
    pub stdin: String,
}

/// Resolves `claude` / `codex` on PATH, including `.cmd` shims on Windows.
pub fn resolve_program(kind: ExecutorKind) -> Result<PathBuf> {
    which::which(kind.program_name())
        .with_context(|| format!("`{}` was not found on PATH", kind.program_name()))
}

/// Whether `dir` is a git checkout: it has a `.git` directory, or a `.git` file as worktrees do.
pub fn is_git_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

/// A model name is passed as one argument only when it is 1 to 100 ASCII letters, digits, `.`,
/// `_`, `:` or `-` and does not start with `-`; anything else is dropped.
fn is_safe_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 100
        && !text.starts_with('-')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
}

/// Builds the command for one run. The arguments are fixed strings plus the model name (only
/// when it passes `is_safe_token`); the prompt goes to stdin and `cwd` is set on the process,
/// so neither ever appears in the arguments.
pub fn build(
    kind: ExecutorKind,
    program: PathBuf,
    profile: PermissionProfile,
    model: Option<&str>,
    cwd: &Path,
    prompt: String,
) -> CommandSpec {
    let model = model.filter(|m| is_safe_token(m));
    let mut args: Vec<String> = Vec::new();
    match kind {
        ExecutorKind::Claude => {
            args.extend(
                [
                    "-p",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-prompts",
                    "none",
                    "--permission-mode",
                ]
                .map(String::from),
            );
            args.push(
                match profile {
                    PermissionProfile::Full => "bypassPermissions",
                    PermissionProfile::Auto => "auto",
                    PermissionProfile::Personal => "dontAsk",
                }
                .into(),
            );
            if let Some(m) = model {
                args.extend(["--model".to_string(), m.to_string()]);
            }
        }
        ExecutorKind::Codex => {
            args.extend(["exec", "--json"].map(String::from));
            match profile {
                PermissionProfile::Full => {
                    args.push("--dangerously-bypass-approvals-and-sandbox".into())
                }
                PermissionProfile::Auto => args.push("--approve-for-me".into()),
                PermissionProfile::Personal => {}
            }
            if !is_git_repo(cwd) {
                args.push("--skip-git-repo-check".into());
            }
            if let Some(m) = model {
                args.extend(["-m".to_string(), m.to_string()]);
            }
            args.push("-".into());
        }
    }
    CommandSpec {
        program,
        args,
        cwd: cwd.to_path_buf(),
        stdin: prompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExecutorKind, PermissionProfile};
    use std::path::PathBuf;

    fn claude(profile: PermissionProfile, model: Option<&str>) -> CommandSpec {
        build(
            ExecutorKind::Claude,
            PathBuf::from("claude"),
            profile,
            model,
            Path::new("."),
            "p".into(),
        )
    }

    #[test]
    fn claude_full_profile_args() {
        assert_eq!(
            claude(PermissionProfile::Full, None).args,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-prompts",
                "none",
                "--permission-mode",
                "bypassPermissions",
            ]
        );
    }

    #[test]
    fn claude_other_profiles_and_model() {
        assert_eq!(
            claude(PermissionProfile::Auto, None).args.last().unwrap(),
            "auto"
        );
        assert_eq!(
            claude(PermissionProfile::Personal, None)
                .args
                .last()
                .unwrap(),
            "dontAsk"
        );
        let with_model = claude(PermissionProfile::Full, Some("claude-sonnet-5-5")).args;
        assert_eq!(
            &with_model[with_model.len() - 2..],
            ["--model", "claude-sonnet-5-5"]
        );
    }

    #[test]
    fn codex_profiles_and_git_check() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let plain = tempfile::tempdir().unwrap();
        let codex = |profile, dir: &Path| {
            build(
                ExecutorKind::Codex,
                PathBuf::from("codex"),
                profile,
                None,
                dir,
                "p".into(),
            )
            .args
        };
        assert_eq!(
            codex(PermissionProfile::Full, repo.path()),
            [
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "-"
            ]
        );
        assert_eq!(
            codex(PermissionProfile::Auto, repo.path()),
            ["exec", "--json", "--approve-for-me", "-"]
        );
        assert_eq!(
            codex(PermissionProfile::Personal, repo.path()),
            ["exec", "--json", "-"]
        );
        assert_eq!(
            codex(PermissionProfile::Full, plain.path()),
            [
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "--skip-git-repo-check",
                "-"
            ]
        );
    }

    #[test]
    fn args_never_contain_prompt_or_paths() {
        let dir = tempfile::tempdir().unwrap();
        let tricky_dir = dir.path().join("目录 with \"quotes\"");
        std::fs::create_dir(&tricky_dir).unwrap();
        let prompt = "line1\n\"quoted\" & | ^ % 中文 'x'".to_string();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let spec = build(
                kind,
                PathBuf::from("x"),
                PermissionProfile::Full,
                None,
                &tricky_dir,
                prompt.clone(),
            );
            assert_eq!(spec.stdin, prompt);
            assert_eq!(spec.cwd, tricky_dir);
            assert!(
                spec.args
                    .iter()
                    .all(|a| !a.contains("中文") && !a.contains("目录") && !a.contains('"'))
            );
        }
    }

    #[test]
    fn unsafe_model_names_are_dropped() {
        let args = claude(PermissionProfile::Full, Some("x\" & calc")).args;
        assert!(!args.contains(&"--model".to_string()));
    }

    #[test]
    fn git_repo_detection() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(dir.path()));
        std::fs::write(dir.path().join(".git"), "gitdir: elsewhere").unwrap();
        assert!(is_git_repo(dir.path()));
    }

    #[test]
    fn codex_model_comes_before_the_stdin_marker() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let args = build(
            ExecutorKind::Codex,
            PathBuf::from("codex"),
            PermissionProfile::Full,
            Some("gpt-5.5"),
            repo.path(),
            "p".into(),
        )
        .args;
        assert_eq!(
            args,
            [
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "-m",
                "gpt-5.5",
                "-"
            ]
        );
    }

    #[test]
    fn every_claude_profile_keeps_permission_prompts_off() {
        for profile in [
            PermissionProfile::Full,
            PermissionProfile::Auto,
            PermissionProfile::Personal,
        ] {
            let args = claude(profile, None).args;
            assert_eq!(
                &args[..7],
                [
                    "-p",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-prompts",
                    "none",
                    "--permission-mode"
                ]
            );
        }
    }

    #[test]
    fn model_names_are_limited_to_safe_tokens() {
        assert!(is_safe_token("claude-sonnet-5-5"));
        assert!(is_safe_token("org:model_v1.2"));
        assert!(is_safe_token(&"a".repeat(100)));
        assert!(!is_safe_token(&"a".repeat(101)));
        assert!(!is_safe_token(""));
        assert!(!is_safe_token("two words"));
        assert!(!is_safe_token("模型"));
        assert!(!is_safe_token("--dangerously-bypass-approvals-and-sandbox"));
        assert!(!is_safe_token("-x"));
    }

    #[test]
    fn program_and_directory_pass_through_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let spec = build(
            ExecutorKind::Claude,
            PathBuf::from("tools").join("claude"),
            PermissionProfile::Full,
            None,
            dir.path(),
            "p".into(),
        );
        assert_eq!(spec.program, PathBuf::from("tools").join("claude"));
        assert_eq!(spec.cwd, dir.path());
    }
}
