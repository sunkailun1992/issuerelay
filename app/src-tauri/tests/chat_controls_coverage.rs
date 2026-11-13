use std::collections::{BTreeMap, BTreeSet};

#[test]
fn every_panel_command_has_one_chat_disposition() {
    let declaration = include_str!("../build.rs")
        .split_once("const PANEL_COMMANDS: &[&str] = &[")
        .expect("panel command list")
        .1
        .split_once("];")
        .expect("panel command list terminator")
        .0;
    let registered: BTreeSet<_> = declaration
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .strip_prefix('"')
                .and_then(|entry| entry.strip_suffix('"'))
                .expect("panel command string literal")
        })
        .collect();
    assert!(!registered.is_empty(), "panel command list is empty");

    let matrix = include_str!("../../../docs/chat-controls.md")
        .split_once("<!-- panel-commands:start -->")
        .expect("chat coverage matrix start")
        .1
        .split_once("<!-- panel-commands:end -->")
        .expect("chat coverage matrix end")
        .0;
    let mcp = include_str!("../../../crates/cli/src/mcp.rs");
    let actions = mcp
        .split_once("const TASK_ACTIONS:")
        .expect("task action list")
        .1
        .split_once("];")
        .expect("task action list terminator")
        .0;
    let mut documented = BTreeMap::new();
    for row in matrix
        .lines()
        .map(str::trim)
        .filter(|row| row.starts_with('|'))
    {
        let cells: Vec<_> = row.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 3, "invalid chat coverage row: {row}");
        if matches!(cells[0], "面板命令" | "---") {
            continue;
        }
        let command = cells[0]
            .strip_prefix('`')
            .and_then(|cell| cell.strip_suffix('`'))
            .expect("chat coverage command must be inline code");
        assert!(
            matches!(cells[1], "已支持" | "本轮接入" | "桌面" | "系统授权"),
            "missing or unknown disposition for {command}: {}",
            cells[1]
        );
        assert!(
            !cells[2].is_empty(),
            "missing reason or equivalent for {command}"
        );
        if matches!(cells[1], "已支持" | "本轮接入") {
            let has_control = cells[2].split('`').skip(1).step_by(2).any(|name| {
                mcp.contains(&format!("async fn {name}("))
                    || actions.contains(&format!("\"{name}\""))
            });
            assert!(
                has_control,
                "document an existing chat control for {command}"
            );
        }
        assert!(
            documented.insert(command, cells[1]).is_none(),
            "duplicate chat coverage row for {command}"
        );
    }
    assert_eq!(
        registered,
        documented.into_keys().collect(),
        "update docs/chat-controls.md when changing the desktop command surface"
    );
}
