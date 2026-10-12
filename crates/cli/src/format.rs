//! Human-readable output for CLI commands; `--json` prints the raw result instead.
//!
//! Everything printed here can hold text from Linear, an executor or the engine, none of which is
//! trusted: control and invisible characters never reach the terminal, whether in the summaries,
//! in the JSON or in the error line.
use std::fmt::Write as _;

use issuerelay_core::text::is_invisible;
use serde_json::Value;

/// What `issuerelay` prints for a result: the JSON itself with `--json`, otherwise the summary
/// of what `method` returned.
pub fn render(method: &str, value: &Value, as_json: bool) -> String {
    if as_json {
        json(value)
    } else {
        human(method, value)
    }
}

/// Renders an engine result as the text `issuerelay` prints without `--json`.
///
/// Every string taken from `value` goes through [`clean`], so an escape sequence in an issue
/// title or a log line (an OSC 52 clipboard write, say) cannot reach the terminal. Only the
/// line breaks between lines that this module adds are left.
pub fn human(method: &str, value: &Value) -> String {
    match method {
        "status" => status(value),
        "list_jobs" => jobs(value),
        "doctor" => doctor(value),
        "bindings.list" => bindings(value),
        "logs" => logs(value),
        "pause" => "Monitoring paused (running jobs continue).".into(),
        "resume" => "Monitoring resumed.".into(),
        "poll_now" => "Checking Linear now.".into(),
        "bindings.add" => "Binding saved.".into(),
        "bindings.remove" if value["removed"] == true => "Binding removed.".into(),
        "bindings.remove" => "No binding with that id.".into(),
        // Compact JSON has no raw line breaks, but serde_json leaves DEL and the C1 range
        // (U+0080..=U+009F) unescaped.
        _ => clean(&value.to_string()),
    }
}

/// The result as indented JSON that is safe to print. serde_json escapes everything below
/// U+0020 inside strings, but not DEL or the C1 controls, and not zero-width or
/// direction-changing characters; those are written as `\uXXXX` escapes, which keeps the output
/// valid JSON that parses back to the same value.
pub fn json(value: &Value) -> String {
    let mut out = String::new();
    for c in format!("{value:#}").chars() {
        // Inside strings serde has escaped every line break already, so a raw one is layout.
        if (c.is_control() && c != '\n') || is_invisible(c) {
            let _ = write!(out, "\\u{:04x}", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// The line printed when a command fails: the error and its causes, cleaned.
pub fn error_line(error: &anyhow::Error) -> String {
    format!("Error: {}", clean(&format!("{error:#}")))
}

/// Text from Linear, an executor or the engine as it may be printed: every control character
/// (line breaks and tabs included) becomes a space, and zero-width and direction-changing
/// characters are removed. Unlike the engine's `clean_text` it does not trim or cut the text,
/// because log lines are long and keep their indentation.
pub fn clean(raw: &str) -> String {
    raw.chars()
        .filter_map(|c| {
            if c.is_control() {
                Some(' ')
            } else if is_invisible(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect()
}

fn text(value: &Value, key: &str) -> String {
    clean(value.get(key).and_then(Value::as_str).unwrap_or("-"))
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(clean)
        .collect()
}

fn status(v: &Value) -> String {
    let monitoring = if v["paused"] == true { "paused" } else { "on" };
    let linear = if v["linear_ok"] == true {
        format!("connected as {}", text(&v["viewer"], "name"))
    } else {
        format!("not connected ({})", text(v, "last_error"))
    };
    let mut out = format!(
        "Monitoring: {monitoring}\nLinear: {linear}\nRunning: {}  Queued: {}  Waiting for binding: {}\nLast check: {}",
        v["running"],
        v["queued"],
        v["waiting"],
        text(v, "last_poll_at")
    );
    // While Linear is connected this says why a job could not start; otherwise it repeats the
    // reason in the Linear line.
    if let Some(error) = v.get("last_error").and_then(Value::as_str) {
        let error = clean(error);
        if !error.trim().is_empty() {
            let _ = write!(out, "\nLast error: {error}");
        }
    }
    out
}

fn jobs(v: &Value) -> String {
    let items = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return "No runs yet.".into();
    }
    items
        .iter()
        .map(|j| {
            format!(
                "#{:<5} {:<12} {:<20} {}",
                j["id"].as_i64().unwrap_or_default(),
                text(j, "issue_identifier"),
                text(j, "state"),
                text(j, "title")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn doctor(v: &Value) -> String {
    let mut out = Vec::new();
    match v.get("viewer").filter(|x| !x.is_null()) {
        Some(viewer) => out.push(format!("Linear: connected as {}", text(viewer, "name"))),
        None => out.push(format!(
            "Linear: not connected ({})",
            text(v, "linear_error")
        )),
    }
    for e in v["executors"].as_array().into_iter().flatten() {
        let ready = e["program"].is_string() && e["logged_in"] == true && e["linear_mcp"] == true;
        let problems = strings(&e["problems"]);
        let detail = if ready {
            "ready".to_string()
        } else if problems.is_empty() {
            "not ready".to_string()
        } else {
            format!("not ready ({})", problems.join("; "))
        };
        out.push(format!("{}: {detail}", text(e, "kind")));
    }
    if let Some(ws) = v.get("workspace").filter(|x| !x.is_null()) {
        let label = if ws["label_ok"] == true {
            "label ok"
        } else {
            "label missing"
        };
        let missing = strings(&ws["missing_states"]);
        out.push(if missing.is_empty() {
            format!("Workspace: {label}, statuses ok")
        } else {
            format!(
                "Workspace: {label}, missing statuses: {}",
                missing.join(", ")
            )
        });
    }
    for b in v["bindings"].as_array().into_iter().flatten() {
        let mut wrong = Vec::new();
        if b["directory_exists"] != true {
            wrong.push("directory missing");
        }
        if b["executor_ready"] != true {
            wrong.push("executor not ready");
        }
        let verdict = if wrong.is_empty() {
            "ok".to_string()
        } else {
            wrong.join(", ")
        };
        out.push(format!(
            "Binding {} -> {}: {verdict}",
            text(b, "project_name"),
            text(b, "executor"),
        ));
    }
    out.join("\n")
}

fn bindings(v: &Value) -> String {
    let items = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return "No bindings. Add one with `issuerelay bind add`.".into();
    }
    items
        .iter()
        .map(|b| {
            let executor = clean(
                b.get("executor")
                    .and_then(Value::as_str)
                    .unwrap_or("default"),
            );
            format!(
                "{}  {}  ->  {}  [{executor}]",
                text(b, "id"),
                text(b, "project_name"),
                text(b, "directory")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn logs(v: &Value) -> String {
    let lines = strings(v);
    if lines.is_empty() {
        return "No log lines yet.".into();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_lines() {
        let v = json!({"paused": false, "linear_ok": true, "viewer": {"id": "u", "name": "Jane Doe"},
            "last_poll_at": "2026-10-01T08:00:00Z", "last_error": null, "running": 1, "queued": 2, "waiting": 0});
        let text = human("status", &v);
        assert!(text.contains("Monitoring: on"));
        assert!(text.contains("Linear: connected as Jane Doe"));
        assert!(text.contains("Running: 1  Queued: 2  Waiting for binding: 0"));
        assert!(text.contains("Last check: 2026-10-01T08:00:00Z"), "{text}");
        assert!(!text.contains("Last error"), "{text}");
        let down = json!({"paused": true, "linear_ok": false, "last_error": "timeout", "running": 0, "queued": 0, "waiting": 0});
        let text = human("status", &down);
        assert!(text.contains("Monitoring: paused") && text.contains("not connected (timeout)"));
        assert!(text.contains("Last check: -\n"), "{text}");
        assert!(text.ends_with("Last error: timeout"), "{text}");
        // While Linear is connected, the last error says why a job could not start.
        let blocked = json!({"paused": false, "linear_ok": true, "viewer": {"id": "u", "name": "Jane Doe"},
            "last_poll_at": "2026-10-01T08:00:00Z", "last_error": "DEMO-1: the folder is missing", "running": 0, "queued": 1, "waiting": 0});
        let text = human("status", &blocked);
        assert!(text.contains("Linear: connected as Jane Doe"), "{text}");
        assert!(
            text.ends_with("Last error: DEMO-1: the folder is missing"),
            "{text}"
        );
        // Linear's error text is untrusted: no escape sequence may reach the terminal.
        let hostile = json!({"paused": false, "linear_ok": false, "last_error": "bad\u{1b}[2J\u{9b}x\nnext", "running": 0, "queued": 0, "waiting": 0});
        let text = human("status", &hostile);
        assert!(text.contains("not connected (bad [2J x next)"), "{text:?}");
        assert!(text.ends_with("Last error: bad [2J x next"), "{text:?}");
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
    }

    #[test]
    fn jobs_table() {
        assert_eq!(human("list_jobs", &json!([])), "No runs yet.");
        let text = human(
            "list_jobs",
            &json!([{"id": 3, "issue_identifier": "DEMO-1", "state": "running", "title": "Demo task"}]),
        );
        assert!(
            text.starts_with("#3")
                && text.contains("DEMO-1")
                && text.contains("running")
                && text.ends_with("Demo task")
        );
        let hostile = human(
            "list_jobs",
            &json!([{"id": 4, "issue_identifier": "DEMO-2", "state": "queued", "title": "a\u{1b}]52;c;ZXZpbA==\u{7}b\nc"}]),
        );
        assert!(!hostile.chars().any(char::is_control), "{hostile:?}");
        assert!(hostile.ends_with("a ]52;c;ZXZpbA== b c"), "{hostile:?}");
        // Zero-width and direction-changing characters are dropped, not turned into spaces.
        let spoofed = human(
            "list_jobs",
            &json!([{"id": 5, "issue_identifier": "DEMO-3", "state": "queued", "title": "x\u{202e}y\u{200b}z\u{feff}"}]),
        );
        assert!(spoofed.ends_with("xyz"), "{spoofed:?}");
    }

    #[test]
    fn doctor_summary() {
        let v = json!({
            "viewer": {"id": "u", "name": "Jane Doe"},
            "linear_error": null,
            "executors": [
                {"kind": "claude", "program": "/x/claude", "logged_in": true, "linear_mcp": true, "problems": []},
                {"kind": "codex", "program": null, "logged_in": false, "linear_mcp": false, "problems": ["`codex` is not installed or not on PATH"]}
            ],
            "workspace": {"label_ok": true, "missing_states": ["Demo: In Testing"]},
            "bindings": [{"binding_id": "p", "project_name": "Demo Project", "directory_exists": true, "executor": "claude", "executor_ready": true}]
        });
        let text = human("doctor", &v);
        assert!(text.contains("Linear: connected as Jane Doe"));
        assert!(text.contains("claude: ready"));
        assert!(text.contains("codex: not ready (`codex` is not installed or not on PATH)"));
        assert!(text.contains("Workspace: label ok, missing statuses: Demo: In Testing"));
        assert!(text.contains("Binding Demo Project -> claude: ok"));
        // Everything Linear or an executor can influence is untrusted text.
        let hostile = human(
            "doctor",
            &json!({
                "viewer": null,
                "linear_error": "e\u{1b}[31m",
                "executors": [{"kind": "codex", "program": null, "logged_in": false, "linear_mcp": false,
                    "problems": ["p\u{7}1", "p\u{1b}]52;c;eA==\u{7}2"]}],
                "workspace": {"label_ok": false, "missing_states": ["Demo:\u{1b}x"]},
                "bindings": [{"binding_id": "p", "project_name": "N\u{1b}]0;t\u{7}M", "directory_exists": true, "executor": "claude", "executor_ready": true}]
            }),
        );
        assert!(
            !hostile.chars().any(|c| c.is_control() && c != '\n'),
            "{hostile:?}"
        );
        assert!(
            hostile.contains("not connected (e [31m)")
                && hostile.contains("not ready (p 1; p ]52;c;eA== 2)")
        );
        assert!(
            hostile.contains("missing statuses: Demo: x")
                && hostile.contains("Binding N ]0;t M -> claude: ok")
        );
    }

    #[test]
    fn doctor_lines_for_linear_workspace_and_executors() {
        let report = |workspace: Value, executors: Value| {
            let report = json!({"viewer": null, "linear_error": "offline", "executors": executors,
                "workspace": workspace, "bindings": []});
            human("doctor", &report)
        };
        let none = json!([]);
        assert_eq!(
            report(Value::Null, none.clone()),
            "Linear: not connected (offline)"
        );
        let ok = json!({"label_ok": true, "missing_states": []});
        assert!(report(ok, none.clone()).ends_with("Workspace: label ok, statuses ok"));
        let no_label = json!({"label_ok": false, "missing_states": []});
        assert!(report(no_label, none).ends_with("Workspace: label missing, statuses ok"));
        // An executor that is not ready without a reason is still not ready; each of the three
        // checks has to pass.
        let executor = |program: Value, logged_in: bool, linear_mcp: bool| {
            let one = json!([{"kind": "codex", "program": program, "logged_in": logged_in,
                "linear_mcp": linear_mcp, "problems": []}]);
            report(Value::Null, one).lines().last().unwrap().to_string()
        };
        assert_eq!(executor(json!("/x/codex"), true, true), "codex: ready");
        assert_eq!(executor(json!("/x/codex"), true, false), "codex: not ready");
        assert_eq!(executor(json!("/x/codex"), false, true), "codex: not ready");
        assert_eq!(executor(Value::Null, true, true), "codex: not ready");
    }

    #[test]
    fn a_binding_line_says_what_is_wrong() {
        let line = |directory_exists: bool, executor_ready: bool| {
            let report = json!({"viewer": null, "linear_error": "offline", "executors": [], "workspace": null,
                "bindings": [{"binding_id": "p", "project_name": "Demo Project", "directory_exists": directory_exists,
                    "executor": "claude", "executor_ready": executor_ready}]});
            human("doctor", &report).lines().last().unwrap().to_string()
        };
        assert_eq!(line(true, true), "Binding Demo Project -> claude: ok");
        assert_eq!(
            line(false, true),
            "Binding Demo Project -> claude: directory missing"
        );
        assert_eq!(
            line(true, false),
            "Binding Demo Project -> claude: executor not ready"
        );
        assert_eq!(
            line(false, false),
            "Binding Demo Project -> claude: directory missing, executor not ready"
        );
    }

    #[test]
    fn bindings_logs_and_simple_messages() {
        assert!(human("bindings.list", &json!([])).contains("issuerelay bind add"));
        let text = human(
            "bindings.list",
            &json!([{"id": "p1", "project_name": "Demo Project", "directory": "demo"}]),
        );
        assert!(text.contains("p1") && text.contains("Demo Project") && text.contains("[default]"));
        let named = human(
            "bindings.list",
            &json!([{"id": "p1", "project_name": "Demo Project", "directory": "demo", "executor": "claude"}]),
        );
        assert_eq!(named, "p1  Demo Project  ->  demo  [claude]");
        assert_eq!(human("logs", &json!(["a", "b"])), "a\nb");
        assert_eq!(human("logs", &json!([])), "No log lines yet.");
        assert_eq!(
            human("resume", &json!({"paused": false})),
            "Monitoring resumed."
        );
        assert_eq!(
            human("poll_now", &json!({"ok": true})),
            "Checking Linear now."
        );
        assert_eq!(
            human("bindings.add", &json!({"ok": true})),
            "Binding saved."
        );
        assert_eq!(
            human("bindings.remove", &json!({"removed": true})),
            "Binding removed."
        );
        assert_eq!(
            human("pause", &json!({"paused": true})),
            "Monitoring paused (running jobs continue)."
        );
        assert_eq!(
            human("bindings.remove", &json!({"removed": false})),
            "No binding with that id."
        );
        // Log lines are the executor's raw output; names and directories come from Linear and the user.
        assert_eq!(
            human("logs", &json!(["a\u{1b}[8mb", "c\rd\u{9b}e"])),
            "a [8mb\nc d e"
        );
        let listed = human(
            "bindings.list",
            &json!([{"id": "p\u{1b}1", "project_name": "N\u{7}M", "directory": "d\u{9b}e", "executor": "co\u{1b}dex"}]),
        );
        assert_eq!(listed, "p 1  N M  ->  d e  [co dex]");
        // serde_json leaves C1 controls unescaped, so the fallback has to clean them itself.
        assert_eq!(human("some.other", &json!("a\u{9b}b")), "\"a b\"");
    }

    #[test]
    fn clean_turns_controls_into_spaces_and_drops_invisible_characters() {
        // Tab, line breaks, ESC, BEL, DEL and a C1 control each become one space.
        assert_eq!(
            clean("a\tb\nc\r\nd\u{1b}e\u{7}f\u{7f}g\u{9b}h"),
            "a b c  d e f g h"
        );
        // Zero-width and direction-changing characters vanish.
        assert_eq!(clean("a\u{200b}b\u{202e}c\u{2028}d\u{feff}e"), "abcde");
        // Everything else stays: indentation, other scripts, a long line.
        assert_eq!(
            clean("  indented \u{65e5}\u{672c}\u{8a9e} \u{e9} "),
            "  indented \u{65e5}\u{672c}\u{8a9e} \u{e9} "
        );
        let long = "x".repeat(2000);
        assert_eq!(clean(&long), long);
    }

    #[test]
    fn json_output_escapes_what_serde_leaves_raw_and_stays_valid_json() {
        let v = json!([{"id": 1, "title": "a\u{9b}b\u{7f}c\u{202e}d\u{1b}e\nf\u{200b}g"}]);
        let text = json(&v);
        // Only the line breaks of the indentation are raw control characters.
        assert!(
            !text
                .chars()
                .any(|c| (c.is_control() && c != '\n') || is_invisible(c)),
            "{text:?}"
        );
        assert!(
            text.contains("a\\u009bb\\u007fc\\u202ed\\u001be\\nf\\u200bg"),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), v);
        // Plain data is printed as serde_json prints it.
        let plain = json!({"paused": false, "n": [1, 2]});
        assert_eq!(json(&plain), serde_json::to_string_pretty(&plain).unwrap());
    }

    #[test]
    fn json_mode_prints_the_raw_result_and_the_other_mode_the_summary() {
        let v = json!({"paused": true});
        let raw = render("pause", &v, true);
        assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), v);
        assert_eq!(
            render("pause", &v, false),
            "Monitoring paused (running jobs continue)."
        );
    }

    #[test]
    fn the_error_line_keeps_the_cause_chain_and_is_clean() {
        let error = anyhow::anyhow!("inner").context("outer");
        assert_eq!(error_line(&error), "Error: outer: inner");
        let hostile = anyhow::anyhow!("bad\u{1b}]52;c;ZXZpbA==\u{7}\u{202e}end").context("outer\n");
        assert_eq!(
            error_line(&hostile),
            "Error: outer : bad ]52;c;ZXZpbA== end"
        );
    }
}
