//! Lean runs (lean-run design, sections 5.1 and 5.2): the arguments that keep an unattended run
//! to Linear's MCP server and the target repository's own rules and settings, without the user's
//! other MCP servers, plugins, hooks, auto memory and user-level instructions.
//!
//! The arguments are fixed strings, with one exception (the third of the fixed-arguments rule in
//! `AGENTS.md`): the path of the user-level `CLAUDE.md` that Claude Code is told to leave out,
//! which `passable` checks before it goes in.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::ExecutorKind;
use crate::linear::auth::LINEAR_MCP_URL;

/// The name of Linear's MCP server in Claude Code, as IssueRelay's one-click setup registers it
/// (design section 8). Claude Code keeps an OAuth sign-in per server name, so a lean run has to
/// use this name to reuse it: under another name the server reports `needs-auth` (measured
/// 2026-10-19, Claude Code 2.1.285).
pub const CLAUDE_LINEAR_SERVER: &str = "linear-server";

/// The same for Codex, which keys a stored sign-in by the server's name and URL (codex-cli
/// 0.159.2, measured 2026-10-19: under another name the server reports `AuthRequired`).
pub const CODEX_LINEAR_SERVER: &str = "linear";

/// Set in the environment of a lean Claude run: Claude Code then neither reads nor writes its
/// auto memory (its documentation says so; measured with Claude Code 2.1.285 on 2026-10-19, with
/// the same effect as the setting `autoMemoryEnabled: false`).
pub const DISABLE_AUTO_MEMORY: (&str, &str) = ("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1");

/// What a lean Claude run adds after its other arguments: only Linear's server, no user settings
/// (so neither the user's plugins nor their hooks and skills), and `user_instructions`, the
/// user-level `CLAUDE.md`, left out when it is given. A path that is not `passable` never goes
/// into the arguments: the caller checks it first to say so in the run log, and it is checked
/// again here.
pub fn claude_args(user_instructions: Option<&Path>) -> Vec<String> {
    let servers = json!({
        "mcpServers": { CLAUDE_LINEAR_SERVER: { "type": "http", "url": LINEAR_MCP_URL } }
    });
    let mut args = vec![
        "--strict-mcp-config".to_string(),
        // `--mcp-config` takes any number of values (`--mcp-config <configs...>` in the help of
        // Claude Code 2.1.285, read 2026-10-20), so an option has to follow its one value: any
        // other argument there would be read as one more configuration.
        "--mcp-config".to_string(),
        servers.to_string(),
        "--setting-sources".to_string(),
        "project,local".to_string(),
    ];
    if let Some(path) = user_instructions
        .filter(|path| passable(path))
        .and_then(Path::to_str)
    {
        args.push("--settings".to_string());
        args.push(json!({ "claudeMdExcludes": [path] }).to_string());
    }
    args
}

/// What a lean Codex run adds before its model and the closing `-`: no user configuration, no
/// hooks, plugins, ChatGPT connectors or memories, and Linear's server as the only (required)
/// MCP server. The skill list is not shown to the model: user-level skills cannot be left out on
/// their own in codex-cli 0.159.2 (measured 2026-10-19).
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
/// characters and of `"`, `%`, `!` and `^`. On Windows `claude` and `codex` are often the npm or
/// pnpm shims `claude.cmd` and `codex.cmd`: Rust quotes an argument for a batch file so that cmd
/// hands it on intact, `&`, `(` and `)` included, and the shims pass it to Node as it is
/// (`json_arguments_survive_npm_and_pnpm_style_cmd_shims`). The four characters kept out are a
/// margin on top of that: cmd can still act on them inside quotes, for example in a shim that
/// parses its arguments again (`call`, delayed expansion). A home folder almost never has them;
/// when it does, the run keeps the user-level `CLAUDE.md` and its log says so.
pub fn passable(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|text| {
            !text
                .chars()
                .any(|c| c.is_control() || matches!(c, '"' | '%' | '!' | '^'))
        })
}

/// Watches a lean run's output for signs that lean did not take effect (lean-run design section
/// 8), so the run can be stopped instead of carrying on with the user's whole configuration.
/// Claude Code names its MCP servers and its auto-memory folder in its `system/init` event, which
/// comes before any work: only `system` events, such as a project's session-start hooks, may come
/// first (Claude Code 2.1.285, measured 2026-10-19). Codex names nothing up front, so the server
/// of each of its MCP calls is checked as it starts: every `mcp_tool_call` item names its `server`
/// (codex-cli 0.159.2, from its source; seen in the sandbox acceptance on 2026-10-20).
pub struct LeanCheck {
    kind: ExecutorKind,
    /// Set by Claude's init event, whatever it showed: the lines after it are not checked (an init
    /// that did not pass has been reported already).
    done: bool,
}

impl LeanCheck {
    pub fn new(kind: ExecutorKind) -> Self {
        Self { kind, done: false }
    }

    /// What `line`, one line of the executor's stdout, shows to be wrong; `None` when nothing is.
    /// A line that is not JSON is not looked at.
    pub fn observe(&mut self, line: &str) -> Option<String> {
        if self.done {
            return None;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return None;
        };
        match self.kind {
            ExecutorKind::Claude => self.claude(&event),
            ExecutorKind::Codex => codex_violation(&event),
        }
    }

    fn claude(&mut self, event: &Value) -> Option<String> {
        let kind = event["type"].as_str().unwrap_or("unknown");
        if kind != "system" {
            return Some(format!("a `{kind}` event came before the init event"));
        }
        if event["subtype"] != "init" {
            return None;
        }
        self.done = true;
        let Some(servers) = event["mcp_servers"].as_array() else {
            return Some("the init event has no `mcp_servers` list".into());
        };
        let mut names: Vec<&str> = servers
            .iter()
            .map(|server| server["name"].as_str().unwrap_or("unnamed"))
            .collect();
        names.sort_unstable();
        if names != [CLAUDE_LINEAR_SERVER] {
            let listed = if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            };
            return Some(format!(
                "the init event lists the MCP servers {listed} instead of only `{CLAUDE_LINEAR_SERVER}`"
            ));
        }
        // Not a documented field (Claude Code 2.1.285 and 2.1.299 sent it with auto memory on,
        // measured 2026-10-19; the lean runs of the 2026-10-20 sandbox acceptance sent none): no
        // field is fine, a field that names a folder is not.
        memory_is_on(&event["memory_paths"]).then(|| "auto memory is on".to_string())
    }
}

/// A Codex event that starts or ends a call to an MCP server other than Linear's.
fn codex_violation(event: &Value) -> Option<String> {
    if !matches!(
        event["type"].as_str(),
        Some("item.started" | "item.completed")
    ) {
        return None;
    }
    let item = &event["item"];
    if item["type"] != "mcp_tool_call" {
        return None;
    }
    let server = item["server"].as_str().unwrap_or("unnamed");
    (server != CODEX_LINEAR_SERVER).then(|| format!("a call to the MCP server `{server}`"))
}

/// Whether `paths` (Claude's `memory_paths`) names a folder: any non-empty string in it.
fn memory_is_on(paths: &Value) -> bool {
    match paths {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => items.iter().any(memory_is_on),
        Value::Object(map) => map.values().any(memory_is_on),
        _ => false,
    }
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

    // `claudeMdExcludes` holds glob patterns matched against absolute paths (Claude Code's
    // documentation, lean-run design section 5.5), so the string matters, not only the path it
    // names: one with a `/` where Windows puts `\` equals the expected `PathBuf` all the same.
    #[test]
    fn the_derived_path_goes_into_the_settings_json_with_the_platform_separator() {
        let home = absolute("srv/demo-home");
        let config_dir = absolute("opt/demo-claude");
        let (from_home, from_config_dir) = if cfg!(windows) {
            (
                r#"{"claudeMdExcludes":["D:\\srv\\demo-home\\.claude\\CLAUDE.md"]}"#,
                r#"{"claudeMdExcludes":["D:\\opt\\demo-claude\\CLAUDE.md"]}"#,
            )
        } else {
            (
                r#"{"claudeMdExcludes":["/srv/demo-home/.claude/CLAUDE.md"]}"#,
                r#"{"claudeMdExcludes":["/opt/demo-claude/CLAUDE.md"]}"#,
            )
        };
        for (path, settings) in [
            (user_instructions_path(Some(&home), None), from_home),
            (
                user_instructions_path(Some(&home), Some(&config_dir)),
                from_config_dir,
            ),
        ] {
            let args = claude_args(path.as_deref());
            assert_eq!(args[args.len() - 2..], ["--settings", settings]);
        }
    }

    #[test]
    fn passable_paths() {
        for rest in [
            "srv/demo-home/.claude/CLAUDE.md",
            "srv/李 四/.claude/CLAUDE.md",
            "srv/o'brien/.claude/CLAUDE.md",
            "srv/Demo User/.claude/CLAUDE.md",
            // Punctuation that a Windows file name may hold and the shims pass on intact, as in
            // `json_arguments_survive_npm_and_pnpm_style_cmd_shims`: `passable` must not stop it.
            "srv/Demo & Co (x) #1 $a +b ,c ;d =e @f [g] {h} ~i `j/.claude/CLAUDE.md",
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

    /// An absolute path that is not valid Unicode: a byte that is not UTF-8 on Unix, an unpaired
    /// surrogate on Windows.
    #[cfg(unix)]
    fn not_unicode() -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(
            b"/srv/demo-\xff/.claude/CLAUDE.md",
        ))
    }

    #[cfg(windows)]
    fn not_unicode() -> PathBuf {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = r"D:\srv\demo-".encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(r"\.claude\CLAUDE.md".encode_utf16());
        PathBuf::from(std::ffi::OsString::from_wide(&wide))
    }

    #[test]
    fn a_path_that_is_not_unicode_never_goes_in() {
        let path = not_unicode();
        assert!(path.is_absolute(), "{path:?}");
        assert!(!passable(&path), "{path:?}");
        assert!(
            !claude_args(Some(&path)).contains(&"--settings".to_string()),
            "{path:?}"
        );
    }

    const LINEAR_ONLY_INIT: &str = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"linear-server","status":"connected"}]}"#;

    fn first_violation(kind: ExecutorKind, lines: &[&str]) -> Option<String> {
        let mut check = LeanCheck::new(kind);
        lines.iter().find_map(|line| check.observe(line))
    }

    #[test]
    fn the_recorded_claude_stream_passes() {
        let lines: Vec<&str> = include_str!("../../tests/fixtures/claude-stream.jsonl")
            .lines()
            .collect();
        assert_eq!(first_violation(ExecutorKind::Claude, &lines), None);
    }

    #[test]
    fn the_recorded_codex_stream_passes() {
        let lines: Vec<&str> = include_str!("../../tests/fixtures/codex-exec.jsonl")
            .lines()
            .collect();
        assert_eq!(first_violation(ExecutorKind::Codex, &lines), None);
    }

    #[test]
    fn another_mcp_server_in_init_is_a_violation() {
        let init = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"linear-server","status":"connected"},{"name":"demo-tools","status":"connected"}]}"#;
        let detail = first_violation(ExecutorKind::Claude, &[init]).unwrap();
        assert!(detail.contains("demo-tools"), "{detail}");
    }

    #[test]
    fn an_init_without_mcp_servers_is_a_violation() {
        let detail = first_violation(
            ExecutorKind::Claude,
            &[r#"{"type":"system","subtype":"init"}"#],
        )
        .unwrap();
        assert!(detail.contains("mcp_servers"), "{detail}");
        let empty = r#"{"type":"system","subtype":"init","mcp_servers":[]}"#;
        assert!(first_violation(ExecutorKind::Claude, &[empty]).is_some());
    }

    #[test]
    fn a_memory_path_with_a_value_is_a_violation() {
        let init = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"linear-server","status":"connected"}],"memory_paths":{"auto":"/srv/demo-home/.claude/projects/x/memory/"}}"#;
        assert_eq!(
            first_violation(ExecutorKind::Claude, &[init]).as_deref(),
            Some("auto memory is on")
        );
    }

    #[test]
    fn an_empty_memory_path_passes() {
        for memory in [r#"{}"#, r#"{"auto":""}"#] {
            let init = format!(
                r#"{{"type":"system","subtype":"init","mcp_servers":[{{"name":"linear-server","status":"needs-auth"}}],"memory_paths":{memory}}}"#
            );
            assert_eq!(
                first_violation(ExecutorKind::Claude, &[&init]),
                None,
                "{init}"
            );
        }
    }

    #[test]
    fn hook_events_before_init_pass() {
        let lines = [
            r#"{"type":"system","subtype":"hook_started","hook_event":"SessionStart"}"#,
            r#"{"type":"system","subtype":"hook_response","hook_event":"SessionStart"}"#,
            LINEAR_ONLY_INIT,
        ];
        assert_eq!(first_violation(ExecutorKind::Claude, &lines), None);
    }

    #[test]
    fn an_assistant_event_before_init_is_a_violation() {
        let lines = [r#"{"type":"assistant","message":{}}"#, LINEAR_ONLY_INIT];
        let detail = first_violation(ExecutorKind::Claude, &lines).unwrap();
        assert!(detail.contains("assistant"), "{detail}");
    }

    #[test]
    fn an_event_without_a_type_before_init_is_a_violation() {
        let lines = [
            r#"{"subtype":"init","mcp_servers":[{"name":"linear-server","status":"connected"}]}"#,
            LINEAR_ONLY_INIT,
        ];
        let detail = first_violation(ExecutorKind::Claude, &lines).unwrap();
        assert!(detail.contains("`unknown`"), "{detail}");
    }

    #[test]
    fn a_result_before_init_is_a_violation() {
        let lines = [
            r#"{"type":"result","subtype":"success","result":"done"}"#,
            LINEAR_ONLY_INIT,
        ];
        let detail = first_violation(ExecutorKind::Claude, &lines).unwrap();
        assert!(detail.contains("`result`"), "{detail}");
    }

    #[test]
    fn lines_after_init_are_not_checked() {
        let lines = [
            LINEAR_ONLY_INIT,
            r#"{"type":"system","subtype":"init","mcp_servers":[]}"#,
            r#"{"type":"assistant","message":{}}"#,
        ];
        assert_eq!(first_violation(ExecutorKind::Claude, &lines), None);
    }

    #[test]
    fn non_json_lines_are_ignored() {
        let lines = ["not json", "", LINEAR_ONLY_INIT];
        assert_eq!(first_violation(ExecutorKind::Claude, &lines), None);
    }

    #[test]
    fn a_codex_call_to_another_server_is_a_violation() {
        let started = r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","server":"demo-tools","tool":"search"}}"#;
        let detail = first_violation(ExecutorKind::Codex, &[started]).unwrap();
        assert!(detail.contains("demo-tools"), "{detail}");
        let completed = r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"demo-tools","tool":"search","status":"completed"}}"#;
        assert!(first_violation(ExecutorKind::Codex, &[completed]).is_some());
        let linear = r#"{"type":"item.started","item":{"id":"item_2","type":"mcp_tool_call","server":"linear","tool":"get_issue"}}"#;
        assert_eq!(first_violation(ExecutorKind::Codex, &[linear]), None);
    }

    #[test]
    fn a_codex_call_that_names_no_server_is_a_violation() {
        let started = r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","tool":"get_issue"}}"#;
        let detail = first_violation(ExecutorKind::Codex, &[started]).unwrap();
        assert!(detail.contains("`unnamed`"), "{detail}");
    }
}
