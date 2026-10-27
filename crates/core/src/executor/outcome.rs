//! Follows the executor's JSON event stream for what the engine keeps of a run: the final message,
//! the usage the executor reported and its session; and parses the one-line result JSON the prompt
//! asks for. Event shapes were confirmed in M0 (docs/gates) and in the probes of the run-improvements
//! design (section 4).

use serde_json::Value;

use crate::config::ExecutorKind;
use crate::model::RunUsage;

/// The AI's own claim about the run; the engine still verifies the issue state in Linear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    InTesting(String),
    NeedsInfo(String),
    Failed(String),
    /// A planning run posted its plan (run-improvements design, section 10.4).
    Planned(String),
}

impl Report {
    pub fn summary(&self) -> &str {
        match self {
            Report::InTesting(s)
            | Report::NeedsInfo(s)
            | Report::Failed(s)
            | Report::Planned(s) => s,
        }
    }

    /// The `result` word the AI wrote, as the run records keep it.
    pub fn as_str(&self) -> &'static str {
        match self {
            Report::InTesting(_) => "in_testing",
            Report::NeedsInfo(_) => "needs_info",
            Report::Failed(_) => "failed",
            Report::Planned(_) => "planned",
        }
    }
}

/// What `OutputTracker` kept of a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackedOutput {
    /// The executor's final message, when it printed one.
    pub final_text: Option<String>,
    /// What the executor reported about its usage; `None` when it reported nothing. The run's own
    /// cost (`RunUsage::cost_micro_usd`) is left to the engine, which knows whether the session
    /// was resumed.
    pub usage: Option<RunUsage>,
    /// The session the run started or continued, when the executor said so.
    pub session_id: Option<String>,
}

/// Follows the executor's events for the final message (Claude's `result` event, or Codex's last
/// `agent_message`, both confirmed in M0), the usage it reported and its session:
///
/// - Claude: the session comes from the `system/init` event, which opens every session that
///   starts or resumes; a `result` event also names one, but it does so when resuming a session
///   that is gone as well, which never started (design section 4.2). The `result` event carries
///   the session's running cost (`total_cost_usd`), this run's tokens (`usage`) and `num_turns`.
/// - Codex: the session is `thread.started`'s `thread_id`; each `turn.completed` carries the
///   tokens of its turn, which add up. Codex reports no cost and nothing like turns.
#[derive(Debug)]
pub struct OutputTracker {
    kind: ExecutorKind,
    last: Option<String>,
    usage: RunUsage,
    session_id: Option<String>,
}

/// A token count as the executor sent it: a non-negative integer, anything else counts as none.
fn count(value: &Value) -> Option<u64> {
    value.as_u64()
}

/// A cost in US dollars as millionths of a dollar: only a finite, non-negative amount of less than
/// a billion dollars counts.
fn micro_usd(value: &Value) -> Option<i64> {
    let dollars = value.as_f64()?;
    (dollars.is_finite() && (0.0..1e9).contains(&dollars)).then(|| (dollars * 1e6).round() as i64)
}

/// Adds `more` to `total`, which stays `None` while nothing was ever added.
fn add(total: &mut Option<u64>, more: Option<u64>) {
    if let Some(more) = more {
        *total = Some(total.unwrap_or(0).saturating_add(more));
    }
}

/// Whether `text` has the shape of the session ids Claude Code and Codex print: a UUID, `8-4-4-4-12`
/// hexadecimal digits. Only such a value may go into an executor's arguments (`--resume`).
pub fn is_session_id(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

impl OutputTracker {
    pub fn new(kind: ExecutorKind) -> Self {
        Self {
            kind,
            last: None,
            usage: RunUsage::default(),
            session_id: None,
        }
    }

    pub fn observe(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match self.kind {
            ExecutorKind::Claude => self.claude(&event),
            ExecutorKind::Codex => self.codex(&event),
        }
    }

    fn claude(&mut self, event: &Value) {
        match event["type"].as_str() {
            Some("system") if event["subtype"] == "init" => {
                self.keep_session(&event["session_id"]);
            }
            Some("result") => {
                if let Some(text) = event["result"].as_str() {
                    self.last = Some(text.to_string());
                }
                let usage = &event["usage"];
                self.usage = RunUsage {
                    cost_micro_usd: None,
                    session_cost_micro_usd: micro_usd(&event["total_cost_usd"]),
                    input_tokens: count(&usage["input_tokens"]),
                    output_tokens: count(&usage["output_tokens"]),
                    cache_read_tokens: count(&usage["cache_read_input_tokens"]),
                    cache_write_tokens: count(&usage["cache_creation_input_tokens"]),
                    turns: count(&event["num_turns"]).and_then(|n| u32::try_from(n).ok()),
                };
            }
            _ => {}
        }
    }

    fn codex(&mut self, event: &Value) {
        match event["type"].as_str() {
            Some("thread.started") => self.keep_session(&event["thread_id"]),
            Some("item.completed") if event["item"]["type"] == "agent_message" => {
                if let Some(text) = event["item"]["text"].as_str() {
                    self.last = Some(text.to_string());
                }
            }
            Some("turn.completed") => {
                let usage = &event["usage"];
                add(&mut self.usage.input_tokens, count(&usage["input_tokens"]));
                add(
                    &mut self.usage.output_tokens,
                    count(&usage["output_tokens"]),
                );
                add(
                    &mut self.usage.cache_read_tokens,
                    count(&usage["cached_input_tokens"]),
                );
                add(
                    &mut self.usage.cache_write_tokens,
                    count(&usage["cache_write_input_tokens"]),
                );
            }
            _ => {}
        }
    }

    /// The first session id that looks like one; later ones (a resumed run names the same) are
    /// ignored.
    fn keep_session(&mut self, id: &Value) {
        if self.session_id.is_none()
            && let Some(id) = id.as_str().filter(|id| is_session_id(id))
        {
            self.session_id = Some(id.to_string());
        }
    }

    pub fn final_text(&self) -> Option<&str> {
        self.last.as_deref()
    }

    pub fn into_output(self) -> TrackedOutput {
        TrackedOutput {
            final_text: self.last,
            usage: (!self.usage.is_empty()).then_some(self.usage),
            session_id: self.session_id,
        }
    }
}

/// Scans from the last line upward for a JSON object whose `result` is `in_testing`, `needs_info`,
/// `failed` or `planned`. Code fences are ignored.
pub fn parse_report(text: &str) -> Option<Report> {
    for line in text.lines().rev() {
        let line = line.trim().trim_matches('`').trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let summary = value["summary"].as_str().unwrap_or_default().to_string();
        let report = match value["result"].as_str() {
            Some("in_testing") => Report::InTesting(summary),
            Some("needs_info") => Report::NeedsInfo(summary),
            Some("failed") => Report::Failed(summary),
            Some("planned") => Report::Planned(summary),
            _ => continue,
        };
        return Some(report);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExecutorKind;

    #[test]
    fn claude_final_text_comes_from_result_event() {
        let mut t = OutputTracker::new(ExecutorKind::Claude);
        t.observe(r#"{"type":"system","subtype":"init"}"#);
        t.observe(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}"#,
        );
        t.observe("not json at all");
        t.observe(r#"{"type":"result","subtype":"success","result":"Done.\n{\"result\":\"in_testing\",\"summary\":\"pushed\"}"}"#);
        assert_eq!(
            parse_report(t.final_text().unwrap()),
            Some(Report::InTesting("pushed".into()))
        );
    }

    #[test]
    fn codex_final_text_is_last_agent_message() {
        let mut t = OutputTracker::new(ExecutorKind::Codex);
        t.observe(r#"{"type":"thread.started","thread_id":"0"}"#);
        t.observe(r#"{"type":"item.completed","item":{"type":"agent_message","text":"first"}}"#);
        t.observe(
            r#"{"type":"item.completed","item":{"type":"command_execution","command":"ls"}}"#,
        );
        t.observe(r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"result\":\"needs_info\",\"summary\":\"which repo?\"}"}}"#);
        t.observe(r#"{"type":"turn.completed"}"#);
        assert_eq!(
            t.into_output().final_text.as_deref(),
            Some(r#"{"result":"needs_info","summary":"which repo?"}"#)
        );
    }

    #[test]
    fn report_parsing_variants() {
        assert_eq!(
            parse_report("{\"result\":\"failed\",\"summary\":\"build broke\"}"),
            Some(Report::Failed("build broke".into()))
        );
        assert_eq!(
            parse_report("text\n```\n{\"result\":\"in_testing\",\"summary\":\"ok\"}\n```"),
            Some(Report::InTesting("ok".into()))
        );
        assert_eq!(
            parse_report("{\"result\":\"in_testing\"}"),
            Some(Report::InTesting(String::new()))
        );
        assert_eq!(parse_report("{\"result\":\"maybe\"}\nno json"), None);
        assert_eq!(parse_report(""), None);
        assert_eq!(Report::NeedsInfo("q".into()).summary(), "q");
    }

    #[test]
    fn planned_is_a_report() {
        let text =
            "Plan posted.\n```json\n{\"result\":\"planned\",\"summary\":\"three steps\"}\n```";
        let report = parse_report(text).unwrap();
        assert_eq!(report, Report::Planned("three steps".into()));
        assert_eq!(report.as_str(), "planned");
        assert_eq!(report.summary(), "three steps");
    }

    #[test]
    fn last_matching_json_line_wins() {
        let text = "{\"result\":\"failed\",\"summary\":\"old\"}\nmore\n{\"result\":\"in_testing\",\"summary\":\"new\"}";
        assert_eq!(parse_report(text), Some(Report::InTesting("new".into())));
    }

    #[test]
    fn real_m0_samples_yield_reports() {
        let report = |kind, sample: &str| {
            let mut t = OutputTracker::new(kind);
            sample.lines().for_each(|line| t.observe(line));
            t.final_text().and_then(parse_report)
        };
        let claude = report(
            ExecutorKind::Claude,
            include_str!("../../tests/fixtures/claude-stream.jsonl"),
        );
        assert!(
            matches!(&claude, Some(Report::InTesting(s)) if s.contains("hello.txt")),
            "{claude:?}"
        );
        let codex = report(
            ExecutorKind::Codex,
            include_str!("../../tests/fixtures/codex-exec.jsonl"),
        );
        assert!(
            matches!(&codex, Some(Report::InTesting(s)) if s.contains("README.md")),
            "{codex:?}"
        );
    }

    #[test]
    fn fenced_json_with_a_language_tag() {
        let text = "Summary below.\n```json\n{\"result\":\"needs_info\",\"summary\":\"which branch?\"}\n```\n";
        assert_eq!(
            parse_report(text),
            Some(Report::NeedsInfo("which branch?".into()))
        );
    }

    /// The `result` event of the resumed probe run (design section 4.2), its text shortened.
    const CLAUDE_RESUMED_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":3,"result":"PELICAN-27","session_id":"b88d7053-d27f-4825-b81c-80f6fbf109d0","total_cost_usd":0.0943946,"usage":{"input_tokens":4,"output_tokens":13,"cache_read_input_tokens":20786,"cache_creation_input_tokens":4364}}"#;

    #[test]
    fn claude_usage_comes_from_the_result_event() {
        let mut t = OutputTracker::new(ExecutorKind::Claude);
        t.observe(CLAUDE_RESUMED_RESULT);
        let usage = t.into_output().usage.expect("usage");
        assert_eq!(
            usage,
            RunUsage {
                cost_micro_usd: None,
                session_cost_micro_usd: Some(94_395),
                input_tokens: Some(4),
                output_tokens: Some(13),
                cache_read_tokens: Some(20_786),
                cache_write_tokens: Some(4_364),
                turns: Some(3),
            }
        );
    }

    #[test]
    fn claude_session_comes_from_init_only() {
        // A resume of a session that is gone: a `result` event names it, but no init came.
        let mut gone = OutputTracker::new(ExecutorKind::Claude);
        gone.observe(r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"00000000-0000-4000-8000-000000000000","total_cost_usd":0,"num_turns":0}"#);
        assert_eq!(gone.into_output().session_id, None);
        let mut started = OutputTracker::new(ExecutorKind::Claude);
        started.observe(r#"{"type":"system","subtype":"init","session_id":"b88d7053-d27f-4825-b81c-80f6fbf109d0"}"#);
        started.observe(CLAUDE_RESUMED_RESULT);
        let out = started.into_output();
        assert_eq!(
            out.session_id.as_deref(),
            Some("b88d7053-d27f-4825-b81c-80f6fbf109d0")
        );
        assert_eq!(out.final_text.as_deref(), Some("PELICAN-27"));
    }

    #[test]
    fn codex_usage_adds_up_every_turn() {
        let mut t = OutputTracker::new(ExecutorKind::Codex);
        t.observe(
            r#"{"type":"thread.started","thread_id":"01a17cd5-323b-7e51-8637-4bced9551eaa"}"#,
        );
        t.observe(r#"{"type":"turn.completed","usage":{"input_tokens":15088,"cached_input_tokens":12032,"cache_write_input_tokens":0,"output_tokens":5,"reasoning_output_tokens":0}}"#);
        t.observe(r#"{"type":"turn.completed","usage":{"input_tokens":30202,"cached_input_tokens":27008,"cache_write_input_tokens":0,"output_tokens":13,"reasoning_output_tokens":0}}"#);
        let out = t.into_output();
        assert_eq!(
            out.session_id.as_deref(),
            Some("01a17cd5-323b-7e51-8637-4bced9551eaa")
        );
        assert_eq!(
            out.usage,
            Some(RunUsage {
                cost_micro_usd: None,
                session_cost_micro_usd: None,
                input_tokens: Some(45_290),
                output_tokens: Some(18),
                cache_read_tokens: Some(39_040),
                cache_write_tokens: Some(0),
                turns: None,
            })
        );
    }

    #[test]
    fn odd_figures_are_left_out() {
        let mut t = OutputTracker::new(ExecutorKind::Claude);
        t.observe(r#"{"type":"result","total_cost_usd":-1,"num_turns":"3","usage":{"input_tokens":-4,"output_tokens":1.5}}"#);
        assert_eq!(t.into_output().usage, None);
        let mut codex = OutputTracker::new(ExecutorKind::Codex);
        codex.observe(r#"{"type":"turn.completed","usage":{"input_tokens":"many"}}"#);
        assert_eq!(codex.into_output().usage, None);
    }

    #[test]
    fn a_session_id_must_look_like_a_uuid() {
        assert!(is_session_id("01a17cd5-323b-7e51-8637-4bced9551eaa"));
        assert!(is_session_id("B88D7053-D27F-4825-B81C-80F6FBF109D0"));
        for text in [
            "",
            "b88d7053-d27f-4825-b81c-80f6fbf109d",
            "b88d7053-d27f-4825-b81c-80f6fbf109d0-",
            "b88d7053 d27f-4825-b81c-80f6fbf109d0",
            "--resume",
            "g88d7053-d27f-4825-b81c-80f6fbf109d0",
        ] {
            assert!(!is_session_id(text), "{text:?}");
        }
        // A session id that does not look like one is not kept.
        let mut t = OutputTracker::new(ExecutorKind::Codex);
        t.observe(r#"{"type":"thread.started","thread_id":"x; rm -rf"}"#);
        assert_eq!(t.into_output().session_id, None);
    }

    #[test]
    fn claude_error_result_without_text_leaves_no_final_text() {
        let mut t = OutputTracker::new(ExecutorKind::Claude);
        t.observe(r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#);
        assert_eq!(t.final_text(), None);
    }
}
