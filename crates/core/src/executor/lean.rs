//! Lean runs (lean-run design, sections 5.1 and 5.2): the arguments that keep an unattended run
//! to Linear's MCP server and the target repository's own rules and settings, without the user's
//! other MCP servers, plugins, hooks, auto memory and user-level instructions.
//!
//! The arguments are fixed strings, with one exception (the third of the fixed-arguments rule in
//! `AGENTS.md`): the path of the user-level `CLAUDE.md` that Claude Code is told to leave out,
//! which `passable` checks before it goes in.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::linear::auth::LINEAR_MCP_URL;

/// The name of Linear's MCP server in Claude Code, as IssueRelay's one-click setup registers it
/// (design section 8). Claude Code keeps an OAuth sign-in per server name, so a lean run has to
/// use this name to reuse it: under another name the server reports `needs-auth` (measured
/// 2026-10-19, Claude Code 2.1.285).
pub const CLAUDE_LINEAR_SERVER: &str = "linear-server";

/// The same for Codex, which keys a stored sign-in by the server's name and URL (codex-cli
/// 0.159.2: under another name the server reports `AuthRequired`).
pub const CODEX_LINEAR_SERVER: &str = "linear";

/// Set in the environment of a lean Claude run: Claude Code then neither reads nor writes its
/// auto memory.
pub const DISABLE_AUTO_MEMORY: (&str, &str) = ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");

/// What a lean Claude run adds after its other arguments: only Linear's server, no user settings
/// (so neither the user's plugins nor their hooks and skills), and `user_instructions`, the
/// user-level `CLAUDE.md`, left out when it is given.
pub fn claude_args(user_instructions: Option<&Path>) -> Vec<String> {
    let servers = json!({
        "mcpServers": { CLAUDE_LINEAR_SERVER: { "type": "http", "url": LINEAR_MCP_URL } }
    });
    let mut args = vec![
        "--strict-mcp-config".to_string(),
        "--mcp-config".to_string(),
        servers.to_string(),
        "--setting-sources".to_string(),
        "project,local".to_string(),
    ];
    if let Some(path) = user_instructions.and_then(Path::to_str) {
        args.push("--settings".to_string());
        args.push(json!({ "claudeMdExcludes": [path] }).to_string());
    }
    args
}

/// What a lean Codex run adds before its model and the closing `-`: no user configuration, no
/// hooks, plugins, ChatGPT connectors or memories, and Linear's server as the only (required)
/// MCP server. The skill list is not shown to the model: user-level skills cannot be left out on
/// their own in codex-cli 0.159.2.
pub fn codex_args() -> Vec<String> {
    let server = CODEX_LINEAR_SERVER;
    let mut args: Vec<String> = [
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
    ]
    .map(String::from)
    .into();
    for setting in [
        format!("mcp_servers.{server}.url={LINEAR_MCP_URL}"),
        format!("mcp_servers.{server}.required=true"),
        format!("mcp_servers.{server}.startup_timeout_sec=30"),
        "skills.include_instructions=false".to_string(),
    ] {
        args.push("-c".to_string());
        args.push(setting);
    }
    args
}

/// Where Claude Code keeps the user-level `CLAUDE.md`: in `CLAUDE_CONFIG_DIR` when that is set,
/// otherwise in `.claude` in the home folder; `None` when neither is known. The file need not
/// exist: leaving out a missing file changes nothing.
pub fn user_instructions_path(
    home: Option<&Path>,
    claude_config_dir: Option<&Path>,
) -> Option<PathBuf> {
    match (claude_config_dir, home) {
        (Some(dir), _) => Some(dir.join("CLAUDE.md")),
        (None, Some(home)) => Some(home.join(".claude").join("CLAUDE.md")),
        (None, None) => None,
    }
}

/// Whether `path` may go into the arguments: absolute, valid UTF-8, and free of control
/// characters and of `"`, `%`, `!` and `^`, which a Windows batch file (the npm shims
/// `claude.cmd` and `codex.cmd`) would interpret. A home folder almost never has them; when it
/// does, the run keeps the user-level `CLAUDE.md` and its log says so.
pub fn passable(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|text| {
            !text
                .chars()
                .any(|c| c.is_control() || matches!(c, '"' | '%' | '!' | '^'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path on this platform: `rest` under the root, or under a drive on Windows.
    fn absolute(rest: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"D:\{}", rest.replace('/', r"\")))
        } else {
            PathBuf::from(format!("/{rest}"))
        }
    }

    #[test]
    fn user_instructions_path_prefers_claude_config_dir() {
        let home = absolute("srv/demo-home");
        let config_dir = absolute("opt/demo-claude");
        assert_eq!(
            user_instructions_path(Some(&home), Some(&config_dir)),
            Some(config_dir.join("CLAUDE.md"))
        );
        assert_eq!(
            user_instructions_path(None, Some(&config_dir)),
            Some(config_dir.join("CLAUDE.md"))
        );
        assert_eq!(
            user_instructions_path(Some(&home), None),
            Some(home.join(".claude").join("CLAUDE.md"))
        );
        assert_eq!(user_instructions_path(None, None), None);
    }

    #[test]
    fn passable_paths() {
        for rest in [
            "srv/demo-home/.claude/CLAUDE.md",
            "srv/李 四/.claude/CLAUDE.md",
            "srv/o'brien/.claude/CLAUDE.md",
            "srv/Demo User/.claude/CLAUDE.md",
        ] {
            assert!(passable(&absolute(rest)), "{rest}");
        }
        assert!(!passable(Path::new("srv/demo-home/.claude/CLAUDE.md")));
        for bad in [
            "srv/a\nb",
            "srv/a\"b",
            "srv/a%b",
            "srv/a!b",
            "srv/a^b",
            "srv/a\u{7}b",
        ] {
            assert!(!passable(&absolute(bad)), "{bad:?}");
        }
    }
}
