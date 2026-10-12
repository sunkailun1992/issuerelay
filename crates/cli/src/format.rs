//! Human-readable output for CLI commands; `--json` prints the raw result instead.
use serde_json::Value;

/// Renders an engine result as the text `issuerelay` prints without `--json`.
///
/// Issue titles, Linear errors, executor problems and log lines are untrusted: every string
/// taken from `value` loses its control characters first, so an escape sequence in it (an
/// OSC 52 clipboard write, say) cannot reach the terminal. Only the newlines between lines
/// that this module adds are left.
pub fn human(method: &str, value: &Value) -> String {
    match method {
        "status" => status(value),
        "list_jobs" => jobs(value),
        "doctor" => doctor(value),
        "bindings.list" => bindings(value),
        "logs" => strings(value).join("\n"),
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

/// Drops every control character: ESC, BEL, CR, line breaks, DEL and the C1 range.
fn clean(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).collect()
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
    format!(
        "Monitoring: {monitoring}\nLinear: {linear}\nRunning: {}  Queued: {}  Waiting for binding: {}\nLast check: {}",
        v["running"],
        v["queued"],
        v["waiting"],
        text(v, "last_poll_at")
    )
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
        let ok = b["directory_exists"] == true && b["executor_ready"] == true;
        out.push(format!(
            "Binding {} -> {}: {}",
            text(b, "project_name"),
            text(b, "executor"),
            if ok { "ok" } else { "not ready" }
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
        let down = json!({"paused": true, "linear_ok": false, "last_error": "timeout", "running": 0, "queued": 0, "waiting": 0});
        let text = human("status", &down);
        assert!(text.contains("Monitoring: paused") && text.contains("not connected (timeout)"));
        // Linear's error text is untrusted: no escape sequence may reach the terminal.
        let hostile = json!({"paused": false, "linear_ok": false, "last_error": "bad\u{1b}[2J\u{9b}x\nnext", "running": 0, "queued": 0, "waiting": 0});
        let text = human("status", &hostile);
        assert!(text.contains("not connected (bad[2Jxnext)"), "{text:?}");
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
        assert!(hostile.ends_with("a]52;c;ZXZpbA==bc"));
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
        assert!(text.contains("missing statuses: Demo: In Testing"));
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
                "bindings": [{"binding_id": "p", "project_name": "N\u{1b}]0;t\u{7}", "directory_exists": true, "executor": "claude", "executor_ready": true}]
            }),
        );
        assert!(
            !hostile.chars().any(|c| c.is_control() && c != '\n'),
            "{hostile:?}"
        );
        assert!(
            hostile.contains("not connected (e[31m)")
                && hostile.contains("not ready (p1; p]52;c;eA==2)")
        );
        assert!(
            hostile.contains("missing statuses: Demo:x")
                && hostile.contains("Binding N]0;t -> claude: ok")
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
        assert_eq!(human("logs", &json!(["a", "b"])), "a\nb");
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
            human("logs", &json!(["a\u{1b}[8mb", "c\rd\u{9b}"])),
            "a[8mb\ncd"
        );
        let listed = human(
            "bindings.list",
            &json!([{"id": "p\u{1b}1", "project_name": "N\u{7}", "directory": "d\u{9b}", "executor": "codex\u{1b}"}]),
        );
        assert_eq!(listed, "p1  N  ->  d  [codex]");
        // serde_json leaves C1 controls unescaped, so the fallback has to clean them itself.
        assert_eq!(human("some.other", &json!("a\u{9b}b")), "\"ab\"");
    }
}
