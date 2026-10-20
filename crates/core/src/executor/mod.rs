//! Builds the command line for one executor run. The prompt always travels on stdin and the
//! working directory is set on the process. The arguments are fixed strings except for a
//! validated model name, a reasoning-effort level (a fixed set of words) and, in a lean Claude
//! run, the path of the user-level `CLAUDE.md` that `lean::passable` checked (the third exception
//! of the fixed-arguments rule); this keeps `claude.cmd` / `codex.cmd` safe on Windows.
pub mod lean;
pub mod outcome;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::{Effort, ExecutorKind, PermissionProfile, is_valid_model_name};

/// One executor process: the program, its fixed arguments, the working directory, and the
/// prompt written to stdin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Written to the child's stdin, then stdin is closed.
    pub stdin: String,
    /// Variables this run sets on top of the executor environment (`env::ExecEnv`).
    pub env: Vec<(String, String)>,
    /// A lean run: the runner checks its output for signs that lean did not take effect
    /// (`lean::LeanCheck`).
    pub lean: bool,
    /// Lines the runner writes at the top of the run log, before the executor's output.
    pub notes: Vec<String>,
}

/// How one run starts: the permission profile, and the binding's model, reasoning effort and
/// lean configuration, when it has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions<'a> {
    pub profile: PermissionProfile,
    pub model: Option<&'a str>,
    pub effort: Option<Effort>,
    /// `Some` for a lean run.
    pub lean: Option<Lean<'a>>,
}

/// What a lean run needs besides the fixed arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lean<'a> {
    /// The user-level `CLAUDE.md` a lean Claude run leaves out, already checked with
    /// `lean::passable` (a path that does not pass is dropped again by `lean::claude_args`);
    /// `None` keeps it. Codex has no such argument and ignores it.
    pub user_instructions: Option<&'a Path>,
}

/// Resolves `claude` / `codex` on PATH, including `.cmd` shims on Windows.
pub fn resolve_program(kind: ExecutorKind) -> Result<PathBuf> {
    resolve_program_in(kind, None)
}

/// Resolves `claude` / `codex` on `path` (the environment check's resolved PATH), or on the
/// host's own PATH when `path` is `None`.
pub fn resolve_program_in(kind: ExecutorKind, path: Option<&str>) -> Result<PathBuf> {
    let name = kind.program_name();
    let found = match path {
        Some(path) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            which::which_in(name, Some(path), cwd)
        }
        None => which::which(name),
    };
    found.with_context(|| format!("`{name}` was not found on PATH"))
}

/// Whether `dir` is inside a git checkout: it, or a folder above it, has a `.git` directory or
/// file (worktrees use a file). A folder that cannot be read counts as not a checkout, which only
/// adds Codex's `--skip-git-repo-check`.
pub fn is_git_repo(dir: &Path) -> bool {
    dir.ancestors().any(|folder| folder.join(".git").exists())
}

/// Builds the command for one run. The arguments are fixed strings plus the model name (only
/// when it passes `is_valid_model_name`), the effort level and, for a lean run, `lean`'s
/// arguments; the prompt goes to stdin and `cwd` is set on the process, so neither ever appears
/// in the arguments.
pub fn build(
    kind: ExecutorKind,
    program: PathBuf,
    options: &RunOptions<'_>,
    cwd: &Path,
    prompt: String,
) -> CommandSpec {
    let model = options.model.filter(|m| is_valid_model_name(m));
    let mut args: Vec<String> = Vec::new();
    let mut env = Vec::new();
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
                match options.profile {
                    PermissionProfile::Full => "bypassPermissions",
                    PermissionProfile::Auto => "auto",
                    PermissionProfile::Personal => "dontAsk",
                }
                .into(),
            );
            if let Some(m) = model {
                args.extend(["--model".to_string(), m.to_string()]);
            }
            if let Some(effort) = options.effort {
                args.extend(["--effort".to_string(), effort.as_str().to_string()]);
            }
            if let Some(lean) = options.lean {
                args.extend(lean::claude_args(lean.user_instructions));
                let (name, value) = lean::DISABLE_AUTO_MEMORY;
                env.push((name.to_string(), value.to_string()));
            }
        }
        ExecutorKind::Codex => {
            args.extend(["exec", "--json"].map(String::from));
            match options.profile {
                PermissionProfile::Full => {
                    args.push("--dangerously-bypass-approvals-and-sandbox".into())
                }
                PermissionProfile::Auto => args.push("--approve-for-me".into()),
                PermissionProfile::Personal => {}
            }
            if !is_git_repo(cwd) {
                args.push("--skip-git-repo-check".into());
            }
            if options.lean.is_some() {
                args.extend(lean::codex_args());
            }
            if let Some(m) = model {
                args.extend(["-m".to_string(), m.to_string()]);
            }
            if let Some(effort) = options.effort {
                args.extend([
                    "-c".to_string(),
                    format!("model_reasoning_effort={}", effort.as_str()),
                ]);
            }
            args.push("-".into());
        }
    }
    CommandSpec {
        program,
        args,
        cwd: cwd.to_path_buf(),
        stdin: prompt,
        env,
        lean: options.lean.is_some(),
        notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ExecutorKind, PermissionProfile};
    use std::path::PathBuf;

    /// A run with only a profile and a model, as before lean runs and the effort existed.
    fn opts(profile: PermissionProfile, model: Option<&str>) -> RunOptions<'_> {
        RunOptions {
            profile,
            model,
            effort: None,
            lean: None,
        }
    }

    fn claude(profile: PermissionProfile, model: Option<&str>) -> CommandSpec {
        build(
            ExecutorKind::Claude,
            PathBuf::from("claude"),
            &opts(profile, model),
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
                &opts(profile, None),
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
        let tricky_dir = dir.path().join("目录 with 'quotes' & spaces");
        std::fs::create_dir(&tricky_dir).unwrap();
        let prompt = "line1\n\"quoted\" & | ^ % 中文 'x'".to_string();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let spec = build(
                kind,
                PathBuf::from("x"),
                &opts(PermissionProfile::Full, None),
                &tricky_dir,
                prompt.clone(),
            );
            assert_eq!(spec.stdin, prompt);
            assert_eq!(spec.cwd, tricky_dir);
            assert!(spec.args.iter().all(|a| {
                !a.contains("中文")
                    && !a.contains("目录")
                    && !a.contains("quotes")
                    && !a.contains('"')
            }));
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
    fn a_folder_inside_a_repository_counts_as_a_git_repository() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let nested = dir.path().join("packages").join("app");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(is_git_repo(&nested));
        let outside = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(outside.path()));
    }

    #[test]
    fn codex_keeps_its_repo_check_in_a_repository_subfolder() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let nested = repo.path().join("packages").join("app");
        std::fs::create_dir_all(&nested).unwrap();
        let args = build(
            ExecutorKind::Codex,
            PathBuf::from("codex"),
            &opts(PermissionProfile::Personal, None),
            &nested,
            "p".into(),
        )
        .args;
        assert_eq!(args, ["exec", "--json", "-"]);
    }

    #[test]
    fn a_git_file_in_a_folder_above_counts_too() {
        // A worktree or a submodule has a `.git` file, not a directory.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".git"), "gitdir: elsewhere").unwrap();
        let nested = dir.path().join("src");
        std::fs::create_dir(&nested).unwrap();
        assert!(is_git_repo(&nested));
    }

    #[test]
    fn executors_are_looked_up_on_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };
        let program = dir.path().join(name);
        std::fs::write(&program, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            resolve_program_in(ExecutorKind::Claude, Some(&path)).unwrap(),
            program
        );
        assert!(resolve_program_in(ExecutorKind::Codex, Some(&path)).is_err());
    }

    #[test]
    fn codex_model_comes_before_the_stdin_marker() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        let args = build(
            ExecutorKind::Codex,
            PathBuf::from("codex"),
            &opts(PermissionProfile::Full, Some("gpt-5.5")),
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
        assert!(is_valid_model_name("claude-sonnet-5-5"));
        assert!(is_valid_model_name("org:model_v1.2"));
        assert!(is_valid_model_name(&"a".repeat(100)));
        assert!(!is_valid_model_name(&"a".repeat(101)));
        assert!(!is_valid_model_name(""));
        assert!(!is_valid_model_name("two words"));
        assert!(!is_valid_model_name("模型"));
        assert!(!is_valid_model_name(
            "--dangerously-bypass-approvals-and-sandbox"
        ));
        assert!(!is_valid_model_name("-x"));
    }

    #[test]
    fn program_and_directory_pass_through_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let spec = build(
            ExecutorKind::Claude,
            PathBuf::from("tools").join("claude"),
            &opts(PermissionProfile::Full, None),
            dir.path(),
            "p".into(),
        );
        assert_eq!(spec.program, PathBuf::from("tools").join("claude"));
        assert_eq!(spec.cwd, dir.path());
    }

    #[test]
    fn unsafe_or_flag_like_models_are_dropped_for_both_executors() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            for model in [
                "x\" & calc",
                "--dangerously-bypass-approvals-and-sandbox",
                "-m",
                "a b",
                "gpt-5.5\n",
            ] {
                let args = build(
                    kind,
                    PathBuf::from("x"),
                    &opts(PermissionProfile::Full, Some(model)),
                    repo.path(),
                    "p".into(),
                )
                .args;
                assert!(
                    !args.iter().any(|a| a == "--model" || a == "-m"),
                    "{kind:?} {model:?}: {args:?}"
                );
            }
        }
    }

    #[test]
    fn shell_metacharacters_never_pass_as_a_model_name() {
        for bad in [
            "a&b", "a|b", "a^b", "a%b", "a<b", "a>b", "a(b", "a)b", "a!b", "a\"b", "a'b", "a;b",
            "a,b", "a=b", "a\\b", "a$b", "a\nb", "a b", "a/b", "a@b",
        ] {
            assert!(!is_valid_model_name(bad), "{bad:?}");
        }
    }

    /// The user-level CLAUDE.md a lean Claude run leaves out in these tests, an absolute path on
    /// this platform, and the `--settings` argument that leaves it out, written out.
    #[cfg(unix)]
    const DEMO_INSTRUCTIONS: &str = "/srv/demo-home/.claude/CLAUDE.md";
    #[cfg(unix)]
    const DEMO_SETTINGS: &str = r#"{"claudeMdExcludes":["/srv/demo-home/.claude/CLAUDE.md"]}"#;
    #[cfg(windows)]
    const DEMO_INSTRUCTIONS: &str = r"D:\srv\demo-home\.claude\CLAUDE.md";
    #[cfg(windows)]
    const DEMO_SETTINGS: &str =
        r#"{"claudeMdExcludes":["D:\\srv\\demo-home\\.claude\\CLAUDE.md"]}"#;

    /// The `--mcp-config` of a lean Claude run (lean-run design section 5.1), written out.
    const LEAN_MCP_CONFIG: &str =
        r#"{"mcpServers":{"linear-server":{"type":"http","url":"https://mcp.linear.app/mcp"}}}"#;

    fn lean(user_instructions: Option<&Path>) -> Option<Lean<'_>> {
        Some(Lean { user_instructions })
    }

    /// A folder with a `.git` directory, so Codex gets no `--skip-git-repo-check`.
    fn git_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        repo
    }

    #[test]
    fn claude_lean_arguments_follow_the_design() {
        let options = RunOptions {
            lean: lean(Some(Path::new(DEMO_INSTRUCTIONS))),
            ..opts(PermissionProfile::Full, None)
        };
        let spec = build(
            ExecutorKind::Claude,
            PathBuf::from("claude"),
            &options,
            Path::new("."),
            "p".into(),
        );
        assert_eq!(
            spec.args,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--permission-prompts",
                "none",
                "--permission-mode",
                "bypassPermissions",
                "--strict-mcp-config",
                "--mcp-config",
                LEAN_MCP_CONFIG,
                "--setting-sources",
                "project,local",
                "--settings",
                DEMO_SETTINGS,
            ]
        );
        assert!(spec.lean);
        assert!(spec.notes.is_empty());
    }

    #[test]
    fn claude_lean_without_a_passable_path_has_no_settings_argument() {
        let options = RunOptions {
            lean: lean(None),
            ..opts(PermissionProfile::Auto, None)
        };
        let args = build(
            ExecutorKind::Claude,
            PathBuf::from("claude"),
            &options,
            Path::new("."),
            "p".into(),
        )
        .args;
        assert_eq!(
            &args[7..],
            [
                "auto",
                "--strict-mcp-config",
                "--mcp-config",
                LEAN_MCP_CONFIG,
                "--setting-sources",
                "project,local",
            ]
        );
    }

    #[test]
    fn a_path_that_cannot_be_passed_never_reaches_the_arguments() {
        let relative = PathBuf::from("demo-home/.claude/CLAUDE.md");
        let with_percent = std::env::temp_dir().join("50%").join("CLAUDE.md");
        for path in [relative, with_percent] {
            let options = RunOptions {
                lean: lean(Some(&path)),
                ..opts(PermissionProfile::Full, None)
            };
            let args = build(
                ExecutorKind::Claude,
                PathBuf::from("claude"),
                &options,
                Path::new("."),
                "p".into(),
            )
            .args;
            assert_eq!(
                &args[8..],
                [
                    "--strict-mcp-config",
                    "--mcp-config",
                    LEAN_MCP_CONFIG,
                    "--setting-sources",
                    "project,local",
                ],
                "{path:?}"
            );
        }
    }

    #[test]
    fn the_lean_mcp_config_names_only_the_linear_server() {
        let options = RunOptions {
            lean: lean(None),
            ..opts(PermissionProfile::Full, None)
        };
        let args = build(
            ExecutorKind::Claude,
            PathBuf::from("claude"),
            &options,
            Path::new("."),
            "p".into(),
        )
        .args;
        let at = args.iter().position(|a| a == "--mcp-config").unwrap();
        let config: serde_json::Value = serde_json::from_str(&args[at + 1]).unwrap();
        assert_eq!(
            config,
            serde_json::json!({"mcpServers": {"linear-server": {
                "type": "http",
                "url": "https://mcp.linear.app/mcp"
            }}})
        );
    }

    /// A path with a space, Chinese, a quote and (on Unix) a backslash, and the JSON `--settings`
    /// argument that leaves it out.
    #[cfg(unix)]
    const TRICKY: [&str; 2] = [
        r"/srv/de mo/李四's\x/.claude/CLAUDE.md",
        r#"{"claudeMdExcludes":["/srv/de mo/李四's\\x/.claude/CLAUDE.md"]}"#,
    ];
    #[cfg(windows)]
    const TRICKY: [&str; 2] = [
        r"D:\srv\de mo\李四's\x\.claude\CLAUDE.md",
        r#"{"claudeMdExcludes":["D:\\srv\\de mo\\李四's\\x\\.claude\\CLAUDE.md"]}"#,
    ];

    #[test]
    fn the_settings_argument_encodes_the_path_as_json() {
        let [path, json] = TRICKY;
        let tricky = Path::new(path);
        let options = RunOptions {
            lean: lean(Some(tricky)),
            ..opts(PermissionProfile::Full, None)
        };
        let args = build(
            ExecutorKind::Claude,
            PathBuf::from("claude"),
            &options,
            Path::new("."),
            "p".into(),
        )
        .args;
        let settings = args.last().unwrap();
        assert_eq!(settings, json);
        let parsed: serde_json::Value = serde_json::from_str(settings).unwrap();
        assert_eq!(parsed["claudeMdExcludes"][0], path);
    }

    #[test]
    fn lean_claude_runs_turn_auto_memory_off_and_full_runs_set_nothing() {
        let repo = git_repo();
        let lean_options = RunOptions {
            lean: lean(None),
            ..opts(PermissionProfile::Full, None)
        };
        let spec = |kind, options: &RunOptions<'_>| {
            build(kind, PathBuf::from("x"), options, repo.path(), "p".into())
        };
        let lean_claude = spec(ExecutorKind::Claude, &lean_options);
        assert_eq!(
            lean_claude.env,
            [(
                "CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_string(),
                "1".to_string()
            )]
        );
        assert!(lean_claude.lean);
        let full_claude = spec(ExecutorKind::Claude, &opts(PermissionProfile::Full, None));
        assert!(full_claude.env.is_empty());
        assert!(!full_claude.lean);
        let lean_codex = spec(ExecutorKind::Codex, &lean_options);
        assert!(lean_codex.env.is_empty());
        assert!(lean_codex.lean);
        let full_codex = spec(ExecutorKind::Codex, &opts(PermissionProfile::Full, None));
        assert!(full_codex.env.is_empty());
        assert!(!full_codex.lean);
    }

    #[test]
    fn codex_lean_arguments_follow_the_design_and_keep_the_stdin_marker_last() {
        let repo = git_repo();
        let options = RunOptions {
            effort: Some(Effort::High),
            lean: lean(Some(Path::new(DEMO_INSTRUCTIONS))),
            ..opts(PermissionProfile::Full, Some("gpt-5.5"))
        };
        let args = build(
            ExecutorKind::Codex,
            PathBuf::from("codex"),
            &options,
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
                "--ignore-user-config",
                "--disable",
                "hooks",
                "--disable",
                "plugins",
                "--disable",
                "apps",
                "--disable",
                "memories",
                "--disable",
                "skill_mcp_dependency_install",
                "-c",
                "mcp_servers.linear.url=https://mcp.linear.app/mcp",
                "-c",
                "mcp_servers.linear.required=true",
                "-c",
                "mcp_servers.linear.startup_timeout_sec=30",
                "-c",
                "skills.include_instructions=false",
                "-m",
                "gpt-5.5",
                "-c",
                "model_reasoning_effort=high",
                "-",
            ]
        );
    }

    /// What `build` must give for every profile, with and without lean, a model and an effort,
    /// and for Codex inside and outside a git checkout, put together from the design's argument
    /// lists written out here (lean-run design, sections 5.1, 5.2 and 10): the lean arguments add
    /// to every profile without changing its permission arguments, and the model and the effort
    /// keep their places.
    #[test]
    fn every_combination_is_built_word_for_word() {
        const CLAUDE_LEAN: [&str; 7] = [
            "--strict-mcp-config",
            "--mcp-config",
            LEAN_MCP_CONFIG,
            "--setting-sources",
            "project,local",
            "--settings",
            DEMO_SETTINGS,
        ];
        const CODEX_LEAN: [&str; 19] = [
            "--ignore-user-config",
            "--disable",
            "hooks",
            "--disable",
            "plugins",
            "--disable",
            "apps",
            "--disable",
            "memories",
            "--disable",
            "skill_mcp_dependency_install",
            "-c",
            "mcp_servers.linear.url=https://mcp.linear.app/mcp",
            "-c",
            "mcp_servers.linear.required=true",
            "-c",
            "mcp_servers.linear.startup_timeout_sec=30",
            "-c",
            "skills.include_instructions=false",
        ];
        let repo = git_repo();
        let plain = tempfile::tempdir().unwrap();
        for profile in [
            PermissionProfile::Full,
            PermissionProfile::Auto,
            PermissionProfile::Personal,
        ] {
            for lean_run in [false, true] {
                for model in [None, Some("demo-model")] {
                    for effort in [None, Some(Effort::Max)] {
                        let options = RunOptions {
                            profile,
                            model,
                            effort,
                            lean: lean_run.then(|| Lean {
                                user_instructions: Some(Path::new(DEMO_INSTRUCTIONS)),
                            }),
                        };
                        let case = format!("{profile:?} lean={lean_run} {model:?} {effort:?}");

                        let mut expected = vec![
                            "-p",
                            "--output-format",
                            "stream-json",
                            "--verbose",
                            "--permission-prompts",
                            "none",
                            "--permission-mode",
                            match profile {
                                PermissionProfile::Full => "bypassPermissions",
                                PermissionProfile::Auto => "auto",
                                PermissionProfile::Personal => "dontAsk",
                            },
                        ];
                        if model.is_some() {
                            expected.extend(["--model", "demo-model"]);
                        }
                        if effort.is_some() {
                            expected.extend(["--effort", "max"]);
                        }
                        if lean_run {
                            expected.extend(CLAUDE_LEAN);
                        }
                        let spec = build(
                            ExecutorKind::Claude,
                            PathBuf::from("claude"),
                            &options,
                            repo.path(),
                            "p".into(),
                        );
                        assert_eq!(spec.args, expected, "claude {case}");
                        assert_eq!(spec.lean, lean_run, "claude {case}");
                        let memory_off = [(
                            "CLAUDE_CODE_DISABLE_AUTO_MEMORY".to_string(),
                            "1".to_string(),
                        )];
                        assert_eq!(spec.env, &memory_off[..usize::from(lean_run)], "{case}");

                        for (dir, in_git) in [(repo.path(), true), (plain.path(), false)] {
                            let mut expected = vec!["exec", "--json"];
                            match profile {
                                PermissionProfile::Full => {
                                    expected.push("--dangerously-bypass-approvals-and-sandbox")
                                }
                                PermissionProfile::Auto => expected.push("--approve-for-me"),
                                PermissionProfile::Personal => {}
                            }
                            if !in_git {
                                expected.push("--skip-git-repo-check");
                            }
                            if lean_run {
                                expected.extend(CODEX_LEAN);
                            }
                            if model.is_some() {
                                expected.extend(["-m", "demo-model"]);
                            }
                            if effort.is_some() {
                                expected.extend(["-c", "model_reasoning_effort=max"]);
                            }
                            expected.push("-");
                            let spec = build(
                                ExecutorKind::Codex,
                                PathBuf::from("codex"),
                                &options,
                                dir,
                                "p".into(),
                            );
                            assert_eq!(spec.args, expected, "codex {case} git={in_git}");
                            assert_eq!(spec.lean, lean_run, "codex {case}");
                            assert!(spec.env.is_empty(), "codex {case}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn effort_reaches_both_executors_in_every_profile() {
        let repo = git_repo();
        let levels = [
            (Effort::Low, "low"),
            (Effort::Medium, "medium"),
            (Effort::High, "high"),
            (Effort::Xhigh, "xhigh"),
            (Effort::Max, "max"),
        ];
        let count = |args: &[String], pair: [&str; 2]| {
            args.windows(2)
                .filter(|w| w[0] == pair[0] && w[1] == pair[1])
                .count()
        };
        for (effort, name) in levels {
            for profile in [
                PermissionProfile::Full,
                PermissionProfile::Auto,
                PermissionProfile::Personal,
            ] {
                for lean_run in [None, lean(None)] {
                    let options = RunOptions {
                        effort: Some(effort),
                        lean: lean_run,
                        ..opts(profile, Some("demo-model"))
                    };
                    let claude = build(
                        ExecutorKind::Claude,
                        PathBuf::from("claude"),
                        &options,
                        repo.path(),
                        "p".into(),
                    )
                    .args;
                    assert_eq!(count(&claude, ["--effort", name]), 1, "{claude:?}");
                    let codex = build(
                        ExecutorKind::Codex,
                        PathBuf::from("codex"),
                        &options,
                        repo.path(),
                        "p".into(),
                    )
                    .args;
                    let setting = format!("model_reasoning_effort={name}");
                    assert_eq!(count(&codex, ["-c", &setting]), 1, "{codex:?}");
                    assert_eq!(codex.last().unwrap(), "-");
                }
            }
        }
    }

    #[test]
    fn no_effort_means_no_effort_argument() {
        let repo = git_repo();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let args = build(
                kind,
                PathBuf::from("x"),
                &opts(PermissionProfile::Full, None),
                repo.path(),
                "p".into(),
            )
            .args;
            assert!(
                !args
                    .iter()
                    .any(|a| a == "--effort" || a.starts_with("model_reasoning_effort")),
                "{args:?}"
            );
        }
    }

    #[test]
    fn lean_args_hold_the_instructions_path_only_in_the_settings_argument() {
        let dir = tempfile::tempdir().unwrap();
        let tricky_dir = dir.path().join("目录 with 'quotes' & spaces");
        std::fs::create_dir(&tricky_dir).unwrap();
        let instructions = tricky_dir.join(".claude").join("CLAUDE.md");
        let prompt = "line1\n\"quoted\" & | ^ % 中文 'x'".to_string();
        let options = RunOptions {
            lean: lean(Some(&instructions)),
            ..opts(PermissionProfile::Full, None)
        };
        // The folder that holds the tricky one must not show up anywhere else either.
        let root = dir.path().to_str().unwrap();
        for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
            let spec = build(
                kind,
                PathBuf::from("x"),
                &options,
                &tricky_dir,
                prompt.clone(),
            );
            assert_eq!(spec.stdin, prompt);
            let settings = spec.args.iter().position(|a| a == "--settings");
            assert_eq!(settings.is_some(), kind == ExecutorKind::Claude);
            for (i, arg) in spec.args.iter().enumerate() {
                if settings.is_some_and(|at| i == at + 1) {
                    assert!(arg.contains("目录 with 'quotes' & spaces"), "{arg}");
                    continue;
                }
                assert!(
                    !arg.contains("中文")
                        && !arg.contains("目录")
                        && !arg.contains("quotes")
                        && !arg.contains(root),
                    "{kind:?}: {arg}"
                );
            }
        }
    }

    /// Runs the lean arguments through batch files like the npm and pnpm shims `claude.cmd` and
    /// `codex.cmd`, which hand `%*` to Node (pnpm's inside an `IF … ( … )` block, which an
    /// unquoted `)` would end early): what Node sees must be what `build` produced (lean-run
    /// design section 5.5). The path holds every character that `lean::passable` lets through
    /// and a Windows file name may hold. Needs Node on PATH, which the CI job installs; without
    /// it the test is skipped, except on CI, where that would hide the check.
    #[cfg(windows)]
    #[test]
    fn json_arguments_survive_npm_and_pnpm_style_cmd_shims() {
        if which::which("node").is_err() {
            assert!(
                std::env::var_os("CI").is_none(),
                "node is not on PATH, so the shims cannot be checked"
            );
            eprintln!("skipped: node is not on PATH");
            return;
        }
        let shim = tempfile::tempdir().unwrap();
        std::fs::write(
            shim.path().join("echo-args.js"),
            "process.stdout.write(JSON.stringify(process.argv.slice(2)))",
        )
        .unwrap();
        // Node from PATH, as npm's shim runs it when no node.exe sits next to it (and pnpm's
        // below): cmd reads a batch file in the console's code page, the OEM one by default, so an
        // absolute path written into it as UTF-8 would fail the test wherever Node's folder name
        // is not ASCII.
        std::fs::write(
            shim.path().join("npm-style.cmd"),
            concat!(
                "@SETLOCAL\r\n",
                "@SET PATHEXT=%PATHEXT:;.JS;=;%\r\n",
                "@\"node\" \"%~dp0echo-args.js\" %*\r\n",
            ),
        )
        .unwrap();
        std::fs::write(
            shim.path().join("pnpm-style.cmd"),
            concat!(
                "@SETLOCAL\r\n",
                "@IF EXIST \"%~dp0\\node.exe\" (\r\n",
                "  \"%~dp0\\node.exe\" \"%~dp0echo-args.js\" %*\r\n",
                ") ELSE (\r\n",
                "  @SET PATHEXT=%PATHEXT:;.JS;=;%\r\n",
                "  node \"%~dp0echo-args.js\" %*\r\n",
                ")\r\n",
            ),
        )
        .unwrap();
        let instructions = PathBuf::from(
            r"D:\Profiles\Demo User\李 o'brien & (x) #1 $a +b ,c ;d =e @f [g] {h} ~i `j\.claude\CLAUDE.md",
        );
        assert!(lean::passable(&instructions));
        let repo = git_repo();
        let options = RunOptions {
            effort: Some(Effort::Max),
            lean: lean(Some(&instructions)),
            ..opts(PermissionProfile::Full, Some("demo-model"))
        };
        for shim_name in ["npm-style.cmd", "pnpm-style.cmd"] {
            for kind in [ExecutorKind::Claude, ExecutorKind::Codex] {
                let spec = build(
                    kind,
                    shim.path().join(shim_name),
                    &options,
                    repo.path(),
                    "p".into(),
                );
                if kind == ExecutorKind::Claude {
                    assert!(
                        spec.args.iter().any(|a| a == "--settings"),
                        "the path went in: {:?}",
                        spec.args
                    );
                }
                let output = std::process::Command::new(&spec.program)
                    .args(&spec.args)
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{kind:?} {shim_name}: {output:?}");
                let echoed: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(echoed, spec.args, "{kind:?} {shim_name}");
            }
        }
    }
}
