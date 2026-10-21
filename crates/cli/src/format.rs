//! Human-readable output for CLI commands; `--json` prints the raw result instead.
//!
//! Everything printed here can hold text from Linear, an executor or the engine, none of which is
//! trusted: control and invisible characters never reach the terminal, whether in the summaries,
//! in the JSON or in the error line.
use std::fmt::Write as _;

use issuerelay_core::{
    config::LeanStatus,
    env::{EnvSource, mask_credentials},
    text::is_invisible,
};
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

/// The most characters of a reason or summary that the jobs table shows.
const WHY_CHARS: usize = 60;

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
    // Only while the host waits for its runs before it exits; older hosts send no such field.
    let draining = if v["draining"] == true {
        "\nDraining: finishing runs, starting nothing new"
    } else {
        ""
    };
    let linear = if v["linear_ok"] == true {
        format!("connected as {}", text(&v["viewer"], "name"))
    } else {
        format!("not connected ({})", text(v, "last_error"))
    };
    let mut out = format!(
        "Monitoring: {monitoring}{draining}\nLinear: {linear}\nRunning: {}  Queued: {}  Waiting for binding: {}\nLast check: {}",
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
            let mut line = format!(
                "#{:<5} {:<12} {:<20} {}",
                j["id"].as_i64().unwrap_or_default(),
                text(j, "issue_identifier"),
                text(j, "state"),
                text(j, "title")
            );
            let why = why(j);
            if !why.is_empty() {
                let _ = write!(line, " — {why}");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Why a job is where it is: its reason, else its summary, cleaned and cut to `WHY_CHARS`
/// characters (`…` marks a cut); empty when it has neither.
fn why(job: &Value) -> String {
    let text = ["reason", "summary"]
        .into_iter()
        .filter_map(|key| job.get(key).and_then(Value::as_str))
        .map(|text| clean(text).trim().to_string())
        .find(|text| !text.is_empty())
        .unwrap_or_default();
    match text.char_indices().nth(WHY_CHARS) {
        Some((cut, _)) => format!("{}…", text[..cut].trim_end()),
        None => text,
    }
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
        let installed = e["program"].is_string() && e["logged_in"] == true;
        let ready = installed && e["linear_mcp"] == true;
        out.push(format!(
            "{}: {}",
            text(e, "kind"),
            readiness(ready, &strings(&e["problems"]))
        ));
        // A host from before lean runs sends no lean checks; then nothing is said about them.
        if e.get("lean_linear_mcp").is_some() {
            let ready = installed && e["lean_linear_mcp"] == true && e["lean_logged_in"] == true;
            out.push(format!(
                "{} (lean runs): {}",
                text(e, "kind"),
                readiness(ready, &strings(&e["lean_problems"]))
            ));
        }
    }
    if let Some(env) = v.get("env").filter(|x| x.is_object()) {
        out.extend(environment(env));
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
        // Which teams the statuses were looked up in; a host from before the field existed
        // sends none, and then nothing is said about teams.
        if ws["teams_checked"].is_array() {
            let teams = strings(&ws["teams_checked"]);
            out.push(format!(
                "Teams checked: {}",
                if teams.is_empty() {
                    "none".to_string()
                } else {
                    teams.join(", ")
                }
            ));
        }
    }
    for b in v["bindings"].as_array().into_iter().flatten() {
        let mut wrong = Vec::new();
        if b["directory_exists"] != true {
            wrong.push("directory missing");
        }
        if b["executor_ready"] != true {
            // As the binding's issues say when they wait. A host from before `lean_not_ready`
            // sends none, and then the general words stay.
            wrong.push(if b["lean_not_ready"] == true {
                "executor not ready for lean runs"
            } else {
                "executor not ready"
            });
        }
        let verdict = if wrong.is_empty() {
            "ok".to_string()
        } else {
            wrong.join(", ")
        };
        // The effort IssueRelay passes to the runs; nothing when no level is set (the executor's
        // own setting applies) or the host is from before the field.
        let effort = b
            .get("effort")
            .and_then(Value::as_str)
            .map(|effort| format!(", effort {}", clean(effort)))
            .unwrap_or_default();
        out.push(format!(
            "Binding {} -> {}{}{effort}: {verdict}",
            text(b, "project_name"),
            text(b, "executor"),
            lean_label(&b["lean"]),
        ));
    }
    out.join("\n")
}

/// `ready`, or `not ready` with the problems that say why, when there are any.
fn readiness(ready: bool, problems: &[String]) -> String {
    if ready {
        "ready".to_string()
    } else if problems.is_empty() {
        "not ready".to_string()
    } else {
        format!("not ready ({})", problems.join("; "))
    }
}

/// What a binding report's `lean` means, after the executor's name; nothing when the host sent
/// none (it is from before lean runs) or one this version does not know.
fn lean_label(value: &Value) -> &'static str {
    match serde_json::from_value::<LeanStatus>(value.clone()) {
        Ok(LeanStatus::On) => " (lean)",
        Ok(LeanStatus::Off) => " (full configuration)",
        Ok(LeanStatus::Personal) => " (full configuration: personal settings)",
        Ok(LeanStatus::HomeDirectory) => " (full configuration: home folder)",
        Err(_) => "",
    }
}

/// The lines for the environment executors get: where the PATH and the proxy come from, the
/// proxy variables with any user name and password in a URL masked, and what could not be read.
fn environment(env: &Value) -> Vec<String> {
    let mut lines = vec![
        format!("PATH: {}", source_label(&env["path_source"])),
        format!("Proxy: {}", source_label(&env["proxy_source"])),
    ];
    for pair in env["proxy"].as_array().into_iter().flatten() {
        if let Some([name, value]) = pair.as_array().map(Vec::as_slice)
            && let (Some(name), Some(value)) = (name.as_str(), value.as_str())
        {
            lines.push(format!(
                "  {}={}",
                clean(name),
                clean(&mask_credentials(value))
            ));
        }
    }
    lines.extend(
        strings(&env["problems"])
            .into_iter()
            .map(|p| format!("  ! {p}")),
    );
    lines
}

/// What an `EnvSource` as the engine sent it means; a value this version does not know is not
/// echoed.
fn source_label(value: &Value) -> &'static str {
    match serde_json::from_value::<EnvSource>(value.clone()) {
        Ok(EnvSource::Host) => "inherited from the host",
        Ok(EnvSource::LoginShell) => "login shell",
        Ok(EnvSource::SystemSettings) => "system settings",
        Ok(EnvSource::None) => "none",
        Err(_) => "unknown",
    }
}

fn bindings(v: &Value) -> String {
    let items = v.as_array().map(Vec::as_slice).unwrap_or_default();
    if items.is_empty() {
        return "No bindings. Add one with `issuerelay bind add`.".into();
    }
    items
        .iter()
        .map(|b| {
            let mut tags = vec![clean(
                b.get("executor")
                    .and_then(Value::as_str)
                    .unwrap_or("default"),
            )];
            // Only what differs from the defaults: the permission profile and the effort from
            // the settings, lean runs on. A binding that sets the personal profile itself (never
            // lean) shows it here; one that follows a personal default, or binds the home folder,
            // looks like a lean one, and `doctor` says whether its runs are lean.
            if let Some(permission) = b.get("permission").and_then(Value::as_str) {
                tags.push(format!("permission {}", clean(permission)));
            }
            if b["lean"] == false {
                tags.push("lean off".into());
            }
            if let Some(effort) = b.get("effort").and_then(Value::as_str) {
                tags.push(format!("effort {}", clean(effort)));
            }
            format!(
                "{}  {}  ->  {}  [{}]",
                text(b, "id"),
                text(b, "project_name"),
                text(b, "directory"),
                tags.join(", ")
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
    use issuerelay_core::{
        engine::StatusSnapshot,
        env::{EnvSource, ExecEnv},
    };
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
    fn status_says_when_the_host_is_draining() {
        let draining = StatusSnapshot {
            draining: true,
            ..StatusSnapshot::default()
        };
        let text = human("status", &serde_json::to_value(draining).unwrap());
        assert!(
            text.contains("Monitoring: on\nDraining: finishing runs, starting nothing new\n"),
            "{text}"
        );
        let text = human(
            "status",
            &serde_json::to_value(StatusSnapshot::default()).unwrap(),
        );
        assert!(!text.contains("Draining"), "{text}");
        // A host from before `draining` existed sends no such field.
        let older =
            json!({"paused": false, "linear_ok": false, "running": 0, "queued": 0, "waiting": 0});
        assert!(!human("status", &older).contains("Draining"));
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
        // The last column says why: the reason, else the summary.
        let table = human(
            "list_jobs",
            &json!([
                {"id": 3, "issue_identifier": "DEMO-1", "state": "running", "title": "Demo task"},
                {"id": 2, "issue_identifier": "DEMO-2", "state": "failed", "title": "Other task",
                    "reason": "run exceeded the time limit", "summary": null}
            ]),
        );
        assert_eq!(
            table,
            "#3     DEMO-1       running              Demo task\n\
             #2     DEMO-2       failed               Other task — run exceeded the time limit"
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
    fn the_jobs_table_ends_with_the_reason_or_the_summary() {
        let line = |job: Value| human("list_jobs", &json!([job]));
        // The reason says why a run failed or stopped; it comes before the summary.
        let failed = line(
            json!({"id": 6, "issue_identifier": "DEMO-4", "state": "failed",
            "title": "Demo task", "reason": "executor exited with code 1", "summary": "tried twice"}),
        );
        assert!(
            failed.ends_with("Demo task — executor exited with code 1"),
            "{failed}"
        );
        let accepted = line(json!({"id": 7, "issue_identifier": "DEMO-5",
            "state": "awaiting_acceptance", "title": "Demo task", "reason": null,
            "summary": "pushed to test"}));
        assert!(
            accepted.ends_with("Demo task — pushed to test"),
            "{accepted}"
        );
        let blank_reason = line(
            json!({"id": 8, "issue_identifier": "DEMO-6", "state": "failed",
            "title": "Demo task", "reason": "  ", "summary": "the last words"}),
        );
        assert!(blank_reason.ends_with("— the last words"), "{blank_reason}");
        let neither = line(
            json!({"id": 9, "issue_identifier": "DEMO-7", "state": "queued",
            "title": "Demo task", "reason": null, "summary": null}),
        );
        assert!(neither.ends_with("Demo task"), "{neither}");
        // Cleaned and cut to 60 characters, `…` marking the cut.
        let long = format!("x\u{1b}[2Jy\n{}", "z".repeat(100));
        let cut = line(
            json!({"id": 10, "issue_identifier": "DEMO-8", "state": "failed",
            "title": "Demo task", "reason": long}),
        );
        let (_, why) = cut.split_once(" — ").unwrap();
        assert!(!cut.chars().any(char::is_control), "{cut:?}");
        assert!(why.starts_with("x [2Jy z"), "{why}");
        assert_eq!(why.chars().count(), 61, "{why}");
        assert!(why.ends_with('…'), "{why}");
        let exactly = "w".repeat(60);
        let full = line(
            json!({"id": 11, "issue_identifier": "DEMO-9", "state": "failed",
            "title": "Demo task", "reason": exactly}),
        );
        assert!(full.ends_with(&format!(" — {exactly}")), "{full}");
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
            "workspace": {"label_ok": true, "missing_states": ["Demo: In Testing"], "teams_checked": ["Demo", "Other"]},
            "bindings": [{"binding_id": "p", "project_name": "Demo Project", "directory_exists": true, "executor": "claude", "executor_ready": true}]
        });
        let text = human("doctor", &v);
        assert!(text.contains("Linear: connected as Jane Doe"));
        assert!(text.contains("claude: ready"));
        assert!(text.contains("codex: not ready (`codex` is not installed or not on PATH)"));
        assert!(text.contains(
            "Workspace: label ok, missing statuses: Demo: In Testing\nTeams checked: Demo, Other\n"
        ));
        assert!(text.contains("Binding Demo Project -> claude: ok"));
        // Everything Linear or an executor can influence is untrusted text.
        let hostile = human(
            "doctor",
            &json!({
                "viewer": null,
                "linear_error": "e\u{1b}[31m",
                "executors": [{"kind": "codex", "program": null, "logged_in": false, "linear_mcp": false,
                    "problems": ["p\u{7}1", "p\u{1b}]52;c;eA==\u{7}2"]}],
                "workspace": {"label_ok": false, "missing_states": ["Demo:\u{1b}x"], "teams_checked": ["Te\u{1b}]52;c;eA==\u{7}am", "Se\u{202e}cond\nforged line"]},
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
        // Team names come from Linear as well.
        assert!(
            hostile.contains("Teams checked: Te ]52;c;eA== am, Second forged line"),
            "{hostile:?}"
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
        assert!(report(no_label, none.clone()).ends_with("Workspace: label missing, statuses ok"));
        // The teams the statuses were looked up in follow the verdict. A host from before the
        // field existed sends none, and then nothing is said about teams.
        let teams = |teams: Value| {
            let workspace = json!({"label_ok": true, "missing_states": [], "teams_checked": teams});
            report(workspace, none.clone())
        };
        assert_eq!(
            teams(json!(["Demo"])),
            "Linear: not connected (offline)\nWorkspace: label ok, statuses ok\nTeams checked: Demo"
        );
        assert!(teams(json!(["Demo", "Beta"])).ends_with("\nTeams checked: Demo, Beta"));
        // No team at all (a workspace without one) is said, not left out.
        assert!(teams(json!([])).ends_with("\nTeams checked: none"));
        let older = json!({"label_ok": true, "missing_states": []});
        assert!(!report(older, none).contains("Teams checked"));
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

    /// A `doctor` result whose environment is `env`, as it arrives over IPC.
    fn doctor_with_env(env: &ExecEnv) -> Value {
        json!({"viewer": null, "linear_error": "offline", "executors": [], "workspace": null,
            "bindings": [], "env": serde_json::to_value(env).unwrap()})
    }

    #[test]
    fn doctor_shows_where_path_and_proxy_come_from_with_credentials_masked() {
        let report = doctor_with_env(&ExecEnv {
            path: Some("/opt/demo/bin:/usr/bin".into()),
            path_source: EnvSource::LoginShell,
            proxy: vec![(
                "HTTPS_PROXY".into(),
                "http://jane:s3cret@proxy.example:8080".into(),
            )],
            proxy_source: EnvSource::SystemSettings,
            problems: vec!["could not read the login shell's environment: timed out".into()],
        });
        let text = human("doctor", &report);
        assert!(text.contains("PATH: login shell"), "{text}");
        assert!(text.contains("Proxy: system settings"), "{text}");
        assert!(
            text.contains("HTTPS_PROXY=http://***@proxy.example:8080"),
            "{text}"
        );
        assert!(!text.contains("s3cret"), "{text}");
        assert!(text.contains("timed out"), "{text}");
    }

    #[test]
    fn doctor_names_every_source_and_stays_quiet_without_an_environment() {
        let source = |path_source, proxy_source| {
            let report = doctor_with_env(&ExecEnv {
                path_source,
                proxy_source,
                ..ExecEnv::inherit()
            });
            let text = human("doctor", &report);
            text.lines()
                .filter(|line| line.starts_with("PATH:") || line.starts_with("Proxy:"))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        assert_eq!(
            source(EnvSource::Host, EnvSource::None),
            "PATH: inherited from the host | Proxy: none"
        );
        assert_eq!(
            source(EnvSource::SystemSettings, EnvSource::LoginShell),
            "PATH: system settings | Proxy: login shell"
        );
        // The probe has no environment, or an older engine does not send one: nothing is shown.
        for env in [json!(null), json!("odd")] {
            let report = json!({"viewer": null, "linear_error": "offline", "executors": [],
                "workspace": null, "bindings": [], "env": env});
            assert_eq!(human("doctor", &report), "Linear: not connected (offline)");
        }
        // A source this version does not know is not echoed.
        let report = json!({"viewer": null, "linear_error": "offline", "executors": [],
            "workspace": null, "bindings": [],
            "env": {"path_source": "from\u{1b}[2Jsomewhere", "proxy_source": 3, "proxy": [], "problems": []}});
        let text = human("doctor", &report);
        assert!(text.contains("PATH: unknown\nProxy: unknown"), "{text}");
    }

    #[test]
    fn the_environment_of_a_doctor_result_is_cleaned_like_everything_else() {
        // The proxy and the problems come from the host's environment, a login shell and the
        // system settings; none of that may move the cursor or forge a line.
        let report = doctor_with_env(&ExecEnv {
            proxy: vec![(
                "HTTPS_PROXY".into(),
                "http://jane:s3cret@proxy.example\u{1b}[2J:8080".into(),
            )],
            proxy_source: EnvSource::LoginShell,
            problems: vec!["bad\u{1b}]52;c;ZXZpbA==\u{7}\nforged line".into()],
            ..ExecEnv::inherit()
        });
        let text = human("doctor", &report);
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{text:?}"
        );
        assert!(!text.contains("s3cret"), "{text}");
        assert!(
            text.contains("  HTTPS_PROXY=http://***@proxy.example [2J:8080"),
            "{text}"
        );
        assert!(
            text.contains("  ! bad ]52;c;ZXZpbA==  forged line"),
            "{text}"
        );
        assert_eq!(text.lines().count(), 5, "{text}");
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
    fn a_binding_held_back_only_by_the_lean_checks_says_so() {
        let line = |lean_not_ready: Option<bool>| {
            let mut binding = json!({"binding_id": "p", "project_name": "Demo Project",
                "directory_exists": true, "executor": "codex", "executor_ready": false,
                "lean": "on"});
            if let Some(value) = lean_not_ready {
                binding["lean_not_ready"] = json!(value);
            }
            let report = json!({"viewer": null, "linear_error": "offline", "executors": [],
                "workspace": null, "bindings": [binding]});
            human("doctor", &report).lines().last().unwrap().to_string()
        };
        assert_eq!(
            line(Some(true)),
            "Binding Demo Project -> codex (lean): executor not ready for lean runs"
        );
        assert_eq!(
            line(Some(false)),
            "Binding Demo Project -> codex (lean): executor not ready"
        );
        // A host from before the field says nothing about it: the general words.
        assert_eq!(
            line(None),
            "Binding Demo Project -> codex (lean): executor not ready"
        );
    }

    #[test]
    fn binding_lines_name_the_permission_lean_off_and_effort() {
        let line = |fields: &[(&str, Value)]| {
            let mut binding = json!({"id": "p1", "project_name": "Demo Project",
                "directory": "demo", "executor": "claude"});
            for (name, value) in fields {
                binding[*name] = value.clone();
            }
            human("bindings.list", &json!([binding]))
        };
        assert_eq!(line(&[]), "p1  Demo Project  ->  demo  [claude]");
        assert_eq!(
            line(&[("lean", json!(true))]),
            "p1  Demo Project  ->  demo  [claude]"
        );
        assert_eq!(
            line(&[("lean", json!(false)), ("effort", json!("max"))]),
            "p1  Demo Project  ->  demo  [claude, lean off, effort max]"
        );
        assert_eq!(
            line(&[("effort", json!("low"))]),
            "p1  Demo Project  ->  demo  [claude, effort low]"
        );
        // The binding's own permission profile, so a personal binding (never lean) does not
        // look like a lean one.
        assert_eq!(
            line(&[("permission", json!("personal"))]),
            "p1  Demo Project  ->  demo  [claude, permission personal]"
        );
        assert_eq!(
            line(&[
                ("permission", json!("auto")),
                ("lean", json!(false)),
                ("effort", json!("max"))
            ]),
            "p1  Demo Project  ->  demo  [claude, permission auto, lean off, effort max]"
        );
        // What the host sends is cleaned like everything else.
        assert_eq!(
            line(&[("effort", json!("m\u{1b}ax"))]),
            "p1  Demo Project  ->  demo  [claude, effort m ax]"
        );
        assert_eq!(
            line(&[("permission", json!("pers\u{9b}onal"))]),
            "p1  Demo Project  ->  demo  [claude, permission pers onal]"
        );
    }

    #[test]
    fn doctor_binding_lines_say_whether_runs_are_lean() {
        let line = |lean: Option<&str>| {
            let mut binding = json!({"binding_id": "p", "project_name": "Demo Project",
                "directory_exists": true, "executor": "claude", "executor_ready": true});
            if let Some(lean) = lean {
                binding["lean"] = json!(lean);
            }
            let report = json!({"viewer": null, "linear_error": "offline", "executors": [],
                "workspace": null, "bindings": [binding]});
            human("doctor", &report).lines().last().unwrap().to_string()
        };
        // A host from before lean runs sends no `lean`, and a value this version does not know
        // is not echoed.
        assert_eq!(line(None), "Binding Demo Project -> claude: ok");
        assert_eq!(line(Some("newer")), "Binding Demo Project -> claude: ok");
        assert_eq!(
            line(Some("on")),
            "Binding Demo Project -> claude (lean): ok"
        );
        assert_eq!(
            line(Some("off")),
            "Binding Demo Project -> claude (full configuration): ok"
        );
        assert_eq!(
            line(Some("personal")),
            "Binding Demo Project -> claude (full configuration: personal settings): ok"
        );
        assert_eq!(
            line(Some("home_directory")),
            "Binding Demo Project -> claude (full configuration: home folder): ok"
        );
    }

    #[test]
    fn doctor_binding_lines_name_the_effort() {
        let line = |effort: Option<Value>| {
            let mut binding = json!({"binding_id": "p", "project_name": "Demo Project",
                "directory_exists": true, "executor": "claude", "executor_ready": true,
                "lean": "on"});
            if let Some(effort) = effort {
                binding["effort"] = effort;
            }
            let report = json!({"viewer": null, "linear_error": "offline", "executors": [],
                "workspace": null, "bindings": [binding]});
            human("doctor", &report).lines().last().unwrap().to_string()
        };
        assert_eq!(
            line(Some(json!("max"))),
            "Binding Demo Project -> claude (lean), effort max: ok"
        );
        // No level set (the executor's own setting applies), or a host from before the field.
        assert_eq!(
            line(Some(Value::Null)),
            "Binding Demo Project -> claude (lean): ok"
        );
        assert_eq!(line(None), "Binding Demo Project -> claude (lean): ok");
        // What the host sends is cleaned like everything else.
        assert_eq!(
            line(Some(json!("m\u{1b}ax"))),
            "Binding Demo Project -> claude (lean), effort m ax: ok"
        );
    }

    #[test]
    fn doctor_prints_the_lean_readiness_of_each_executor() {
        let executor = |kind: &str, lean: Option<(bool, bool, Value)>| {
            let mut e = json!({"kind": kind, "program": format!("/x/{kind}"), "logged_in": true,
                "linear_mcp": true, "problems": []});
            if let Some((linear, logged_in, problems)) = lean {
                e["lean_linear_mcp"] = json!(linear);
                e["lean_logged_in"] = json!(logged_in);
                e["lean_problems"] = problems;
            }
            e
        };
        let lines = |executors: Value| {
            let report = json!({"viewer": null, "linear_error": "offline",
                "executors": executors, "workspace": null, "bindings": []});
            human("doctor", &report)
                .lines()
                .skip(1)
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            lines(json!([
                executor("claude", Some((true, true, json!([])))),
                executor(
                    "codex",
                    Some((
                        false,
                        true,
                        json!([
                            "lean runs use the Linear MCP server named `linear` in codex",
                            "p\u{7}2"
                        ])
                    ))
                ),
            ])),
            [
                "claude: ready",
                "claude (lean runs): ready",
                "codex: ready",
                "codex (lean runs): not ready (lean runs use the Linear MCP server named `linear` in codex; p 2)",
            ]
        );
        // A lean check that a failed usual check left unjudged adds no problem of its own.
        let mut signed_out = executor("codex", Some((true, false, json!([]))));
        signed_out["logged_in"] = json!(false);
        signed_out["problems"] = json!(["codex is not logged in"]);
        assert_eq!(
            lines(json!([signed_out])),
            [
                "codex: not ready (codex is not logged in)",
                "codex (lean runs): not ready"
            ]
        );
        // A host from before lean runs sends none of the lean fields.
        assert_eq!(lines(json!([executor("claude", None)])), ["claude: ready"]);
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
