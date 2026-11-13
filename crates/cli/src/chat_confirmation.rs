//! Receives exact one-line confirmation commands from the opted-in Codex input hook.
use std::{future::Future, io::Read};

use anyhow::{Result, ensure};
use issuerelay_core::executor::outcome::is_session_id;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_INPUT_BYTES: u64 = 256 * 1024;
const HOOK_EVENT: &str = "UserPromptSubmit";

#[derive(Deserialize)]
struct HookInput {
    hook_event_name: Option<String>,
    prompt: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    agent_id: Option<Value>,
    agent_type: Option<Value>,
}

fn confirmation_code(prompt: &str) -> Option<&str> {
    let code = prompt
        .strip_prefix("确认 ")
        .or_else(|| prompt.strip_prefix("confirm "))?;
    (code.len() == 32
        && code
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(code)
}

pub(super) async fn handle<Fut>(
    input: impl Read,
    send: impl FnOnce(&'static str, Value) -> Fut,
) -> Result<Option<Value>>
where
    Fut: Future<Output = Result<Value>>,
{
    let mut bytes = Vec::new();
    input.take(MAX_INPUT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_INPUT_BYTES {
        return Ok(None);
    }
    let event: HookInput = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid Codex confirmation hook input"))?;
    let Some(code) = event.prompt.as_deref().and_then(confirmation_code) else {
        return Ok(None);
    };
    ensure!(
        event.hook_event_name.as_deref() == Some(HOOK_EVENT),
        "a typed confirmation requires a UserPromptSubmit event"
    );
    ensure!(
        event.agent_id.is_none() && event.agent_type.is_none(),
        "a typed confirmation requires a main Codex chat"
    );
    let session_id = event.session_id.as_deref().filter(|id| is_session_id(id));
    let turn_id = event.turn_id.as_deref().filter(|id| is_session_id(id));
    let (Some(session_id), Some(turn_id)) = (session_id, turn_id) else {
        anyhow::bail!("a typed confirmation requires valid Codex session and turn identifiers");
    };

    let result = send(
        "workflow.confirm_chat_action",
        json!({"session_id": session_id, "turn_id": turn_id, "code": code}),
    )
    .await;
    let outcome = result
        .as_ref()
        .ok()
        .and_then(|value| value.get("result"))
        .and_then(Value::as_str);
    let message = match outcome {
        Some("dispatch_approved") => {
            "IssueRelay received the typed confirmation. Dispatch approved."
        }
        Some("continuation_queued") => {
            "IssueRelay received the typed confirmation. Continuation queued."
        }
        Some("retry_queued_for_review") => {
            "IssueRelay received the typed confirmation. Retry queued for review."
        }
        Some("stop_requested") => "IssueRelay received the typed confirmation. Stop requested.",
        Some("local_completion_recorded") => {
            "IssueRelay received the typed confirmation. Local completion recorded."
        }
        Some("done") => "IssueRelay received the typed confirmation. Issue marked done.",
        Some("plan_done") => "IssueRelay received the typed confirmation. Plan marked done.",
        _ => {
            "IssueRelay could not verify the result of this typed confirmation. Check the current IssueRelay run or status in this chat before retrying."
        }
    };
    Ok(Some(json!({
        "hookSpecificOutput": {
            "hookEventName": HOOK_EVENT,
            "additionalContext": message
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "00000000-0000-4000-8000-000000000001";
    const TURN: &str = "00000000-0000-4000-8000-000000000002";
    const CODE: &str = "0123456789abcdef0123456789abcdef";

    fn event(prompt: &str) -> Value {
        json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": SESSION,
            "turn_id": TURN,
            "prompt": prompt,
            "cwd": "/synthetic/workspace",
            "transcript_path": "/synthetic/transcript.jsonl",
            "agent_id": null
        })
    }

    #[tokio::test]
    async fn exact_confirmation_sends_only_the_three_review_fields() {
        for prefix in ["确认 ", "confirm "] {
            let input = event(&format!("{prefix}{CODE}")).to_string();
            let output = handle(input.as_bytes(), |method, params| async move {
                assert_eq!(method, "workflow.confirm_chat_action");
                assert_eq!(
                    params,
                    json!({"session_id": SESSION, "turn_id": TURN, "code": CODE})
                );
                Ok(json!({
                    "result": "dispatch_approved",
                    "issue_identifier": "Never echo this host text"
                }))
            })
            .await
            .unwrap()
            .unwrap();
            let context = &output["hookSpecificOutput"];
            assert_eq!(context["hookEventName"], "UserPromptSubmit");
            assert!(
                context["additionalContext"]
                    .as_str()
                    .unwrap()
                    .contains("Dispatch approved")
            );
            assert!(!output.to_string().contains("Never echo"));
        }
    }

    #[tokio::test]
    async fn near_matches_and_overlong_input_are_silent_and_never_send() {
        for prompt in [
            format!("Confirm {CODE}"),
            format!("confirm  {CODE}"),
            format!(" confirm {CODE}"),
            format!("confirm {CODE}\n"),
            format!("confirm {CODE} extra"),
            format!("> 确认 {CODE}"),
            format!("确认\u{3000}{CODE}"),
            format!("confirm {}", CODE.to_ascii_uppercase()),
            format!("confirm {}g", &CODE[..31]),
            format!("confirm {}", &CODE[..31]),
        ] {
            let input = event(&prompt).to_string();
            assert!(
                handle(input.as_bytes(), |_, _| async { panic!("must not send") })
                    .await
                    .unwrap()
                    .is_none(),
                "{prompt:?}",
            );
        }
        let mut input = event(&format!("confirm {CODE}")).to_string().into_bytes();
        input.resize(MAX_INPUT_BYTES as usize + 1, b' ');
        assert!(
            handle(input.as_slice(), |_, _| async { panic!("must not send") })
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn exact_command_with_invalid_event_does_not_send() {
        let prompt = format!("确认 {CODE}");
        for (field, value) in [
            ("session_id", json!("not-a-uuid")),
            ("turn_id", Value::Null),
            ("hook_event_name", json!("PostToolUse")),
            ("agent_id", json!(TURN)),
            ("agent_type", json!("assistant")),
        ] {
            let mut input = event(&prompt);
            input[field] = value;
            assert!(
                handle(input.to_string().as_bytes(), |_, _| async {
                    panic!("must not send")
                })
                .await
                .is_err(),
                "{field}",
            );
        }
    }

    #[tokio::test]
    async fn failure_receipts_do_not_pass_host_error_or_unknown_result_into_chat() {
        let input = event(&format!("confirm {CODE}")).to_string();
        for response in [
            Err(anyhow::anyhow!("Never echo this host error")),
            Ok(json!({"result": "Never echo this host result"})),
        ] {
            let output = handle(input.as_bytes(), |_, _| async { response })
                .await
                .unwrap()
                .unwrap();
            assert!(!output.to_string().contains("Never echo"));
            assert!(
                output["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
                    .contains(
                        "Check the current IssueRelay run or status in this chat before retrying."
                    )
            );
        }
    }
}
