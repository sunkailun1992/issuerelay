//! Pulls the executor's final message out of its JSON event stream and parses the one-line
//! result JSON the prompt asks for. Event shapes were confirmed in M0 (docs/gates).

use serde_json::Value;

use crate::config::ExecutorKind;

/// The AI's own claim about the run; the engine still verifies the issue state in Linear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    InTesting(String),
    NeedsInfo(String),
    Failed(String),
}

impl Report {
    pub fn summary(&self) -> &str {
        match self {
            Report::InTesting(s) | Report::NeedsInfo(s) | Report::Failed(s) => s,
        }
    }
}

/// Keeps the executor's final message: Claude's `result` event, or Codex's last `agent_message`.
/// Both formats were confirmed in M0.
#[derive(Debug)]
pub struct FinalTextTracker {
    kind: ExecutorKind,
    last: Option<String>,
}

impl FinalTextTracker {
    pub fn new(kind: ExecutorKind) -> Self {
        Self { kind, last: None }
    }

    pub fn observe(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let text = match self.kind {
            ExecutorKind::Claude if event["type"] == "result" => event["result"].as_str(),
            ExecutorKind::Codex
                if event["type"] == "item.completed"
                    && event["item"]["type"] == "agent_message" =>
            {
                event["item"]["text"].as_str()
            }
            _ => None,
        };
        if let Some(text) = text {
            self.last = Some(text.to_string());
        }
    }

    pub fn final_text(&self) -> Option<&str> {
        self.last.as_deref()
    }

    pub fn into_final_text(self) -> Option<String> {
        self.last
    }
}

/// Scans from the last line upward for a JSON object whose `result` is `in_testing`, `needs_info`
/// or `failed`. Code fences are ignored.
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
        let mut t = FinalTextTracker::new(ExecutorKind::Claude);
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
        let mut t = FinalTextTracker::new(ExecutorKind::Codex);
        t.observe(r#"{"type":"thread.started","thread_id":"0"}"#);
        t.observe(r#"{"type":"item.completed","item":{"type":"agent_message","text":"first"}}"#);
        t.observe(
            r#"{"type":"item.completed","item":{"type":"command_execution","command":"ls"}}"#,
        );
        t.observe(r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"result\":\"needs_info\",\"summary\":\"which repo?\"}"}}"#);
        t.observe(r#"{"type":"turn.completed"}"#);
        assert_eq!(
            t.into_final_text().as_deref(),
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
    fn last_matching_json_line_wins() {
        let text = "{\"result\":\"failed\",\"summary\":\"old\"}\nmore\n{\"result\":\"in_testing\",\"summary\":\"new\"}";
        assert_eq!(parse_report(text), Some(Report::InTesting("new".into())));
    }

    #[test]
    fn real_m0_samples_yield_reports() {
        let report = |kind, sample: &str| {
            let mut t = FinalTextTracker::new(kind);
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

    #[test]
    fn claude_error_result_without_text_leaves_no_final_text() {
        let mut t = FinalTextTracker::new(ExecutorKind::Claude);
        t.observe(r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#);
        assert_eq!(t.final_text(), None);
    }
}
