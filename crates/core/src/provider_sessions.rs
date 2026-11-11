//! Local preview of an IssueRelay-owned provider session, without starting an inference turn.
use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, ensure};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::env::ExecEnv;
use crate::executor::outcome::is_session_id;
use crate::runner::TreeGuard;
use crate::text::{clean_text, is_invisible, strip_ansi};

const QUERY_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REPLY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RPC_MESSAGES: usize = 128;
const PREVIEW_TURNS: usize = 12;
const PREVIEW_MESSAGES: usize = PREVIEW_TURNS * 2;
const MAX_MESSAGE_CHARS: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSessionRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeSessionMessage {
    pub role: NativeSessionRole,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NativeSessionPreview {
    pub session_id: String,
    pub title: Option<String>,
    pub messages: Vec<NativeSessionMessage>,
    pub has_more: bool,
}

/// Reads a recent text preview over the documented Codex App Server protocol.
/// The caller resolves the ID from an IssueRelay-owned Codex job before calling.
/// Provider responses are display data for the local user, never executor input.
/// https://learn.chatgpt.com/docs/app-server#read-a-stored-thread-without-resuming
pub async fn read_codex_session(
    program: &Path,
    env: &ExecEnv,
    session_id: &str,
) -> Result<NativeSessionPreview> {
    ensure!(is_session_id(session_id), "invalid Codex session ID");
    let mut command = CommandWrap::with_new(program, |command| {
        command
            .args([
                "app-server",
                "--listen",
                "stdio://",
                "-c",
                "analytics.enabled=false",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(home) = dirs::home_dir() {
            command.current_dir(home);
        }
        env.apply(command);
    });
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    {
        command.wrap(crate::runner::creation_flags());
        command.wrap(process_wrap::tokio::JobObject);
    }
    command.wrap(KillOnDrop);
    let mut child = command
        .spawn()
        .map_err(|_| anyhow!("could not start the Codex session reader"))?;
    let mut pipes = child.stdout().take().zip(child.stdin().take());
    let mut tree = TreeGuard::new(child);
    let result = match pipes.as_mut() {
        Some((stdout, stdin)) => tokio::time::timeout(
            QUERY_TIMEOUT,
            query_codex_session(stdout, stdin, session_id),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow!("Codex session reader timed out"))),
        None => Err(anyhow!("Codex session reader pipes are unavailable")),
    };
    // Keep the pipes open until the guard finds any descendants of the server.
    tree.end().await;
    result
}

async fn query_codex_session(
    stdout: impl AsyncRead + Unpin,
    mut stdin: impl AsyncWrite + Unpin,
    session_id: &str,
) -> Result<NativeSessionPreview> {
    let mut stdout = BufReader::new(stdout.take(MAX_REPLY_BYTES + 1));
    let mut messages_read = 0;
    send(
        &mut stdin,
        json!({
            "id": 1, "method": "initialize", "params": {"clientInfo": {
                "name": "issuerelay", "title": "IssueRelay", "version": env!("CARGO_PKG_VERSION")
            }}
        }),
    )
    .await?;
    receive(&mut stdout, 1, &mut messages_read).await?;
    send(&mut stdin, json!({"method": "initialized"})).await?;

    send(&mut stdin, json!({
        "id": 2, "method": "thread/read", "params": {"threadId": session_id, "includeTurns": false}
    })).await?;
    let metadata = receive(&mut stdout, 2, &mut messages_read).await?;
    let thread = &metadata["thread"];
    ensure!(
        thread["id"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(session_id)),
        "Codex returned a different session"
    );
    let title = thread["name"]
        .as_str()
        .map(clean_text)
        .filter(|name| !name.is_empty());

    send(&mut stdin, json!({
        "id": 3, "method": "thread/turns/list", "params": {
            "threadId": session_id, "limit": PREVIEW_TURNS, "sortDirection": "desc", "itemsView": "summary"
        }
    })).await?;
    let page = receive(&mut stdout, 3, &mut messages_read).await?;
    let turns = page["data"]
        .as_array()
        .ok_or_else(|| anyhow!("Codex returned an invalid session preview"))?;
    ensure!(
        turns.len() <= PREVIEW_TURNS,
        "Codex session preview exceeds its turn limit"
    );
    let mut has_more = page["nextCursor"]
        .as_str()
        .is_some_and(|cursor| !cursor.is_empty());
    let mut messages = VecDeque::new();
    for turn in turns.iter().rev() {
        let items = turn["items"]
            .as_array()
            .ok_or_else(|| anyhow!("Codex returned invalid turn items"))?;
        for item in items {
            if let Some((message, truncated)) = preview_message(item) {
                has_more |= truncated;
                if messages.len() == PREVIEW_MESSAGES {
                    messages.pop_front();
                    has_more = true;
                }
                messages.push_back(message);
            }
        }
    }
    Ok(NativeSessionPreview {
        session_id: session_id.into(),
        title,
        messages: messages.into(),
        has_more,
    })
}

async fn send(stdin: &mut (impl AsyncWrite + Unpin), request: Value) -> Result<()> {
    stdin.write_all(format!("{request}\n").as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

async fn receive(
    stdout: &mut BufReader<tokio::io::Take<impl AsyncRead + Unpin>>,
    id: u64,
    messages_read: &mut usize,
) -> Result<Value> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = stdout.read_until(b'\n', &mut line).await?;
        ensure!(count != 0, "Codex session reader closed before replying");
        *messages_read += 1;
        ensure!(
            stdout.get_ref().limit() != 0 && *messages_read <= MAX_RPC_MESSAGES,
            "Codex session reader output exceeds its limit"
        );
        let mut reply: Value = serde_json::from_slice(&line)
            .map_err(|_| anyhow!("Codex session reader returned invalid JSON"))?;
        if reply["id"].as_u64() == Some(id) && reply.get("method").is_none() {
            return reply
                .get_mut("result")
                .map(Value::take)
                .ok_or_else(|| anyhow!("Codex session reader rejected the request"));
        }
    }
}

fn preview_message(item: &Value) -> Option<(NativeSessionMessage, bool)> {
    let (role, text) = match item["type"].as_str()? {
        "userMessage" => (
            NativeSessionRole::User,
            item["content"]
                .as_array()?
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        "agentMessage" => (NativeSessionRole::Assistant, item["text"].as_str()?.into()),
        _ => return None,
    };
    let text: String = strip_ansi(&text)
        .chars()
        .filter_map(|character| match character {
            '\n' | '\t' => Some(character),
            character if character.is_control() => Some(' '),
            character if is_invisible(character) => None,
            character => Some(character),
        })
        .collect();
    let mut chars = text.trim().chars();
    let mut text: String = chars.by_ref().take(MAX_MESSAGE_CHARS).collect();
    let truncated = chars.next().is_some();
    if truncated {
        text.push('…');
    }
    (!text.is_empty()).then_some((NativeSessionMessage { role, text }, truncated))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const SESSION: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER_SESSION: &str = "00000000-0000-4000-8000-000000000002";

    fn json_lines(values: &[Value]) -> String {
        values.iter().map(|value| format!("{value}\n")).collect()
    }

    #[tokio::test]
    async fn invalid_id_is_rejected_before_starting_a_reader() {
        let error = read_codex_session(Path::new("unused-program"), &ExecEnv::inherit(), "--last")
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid Codex session ID");
    }

    #[tokio::test]
    async fn codex_preview_reads_only_the_exact_session_and_keeps_text_messages() {
        let fixture = json_lines(&[
            json!({"id": 1, "result": {}}),
            json!({"method": "thread/status/changed", "params": {"threadId": OTHER_SESSION}}),
            json!({"id": 2, "result": {"thread": {"id": SESSION, "name": "Previous run"}}}),
            json!({"id": 3, "result": {
                "nextCursor": "opaque",
                "data": [
                    {"id": "latest", "items": [
                        {"type": "agentMessage", "text": "Finished\nnext step\u{202e}."}
                    ]},
                    {"id": "older", "items": [
                        {"type": "userMessage", "content": [
                            {"type": "text", "text": "Continue the run"},
                            {"type": "localImage", "path": "/temporary/fixture.png"}
                        ]},
                        {"type": "commandExecution", "aggregatedOutput": "not a message"}
                    ]}
                ]
            }}),
        ]);
        let mut requests = Vec::new();
        let preview = query_codex_session(fixture.as_bytes(), &mut requests, SESSION)
            .await
            .expect("read the recorded session");

        let requests = String::from_utf8(requests).unwrap();
        let requests: Vec<Value> = requests
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            requests
                .iter()
                .map(|r| r["method"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "initialize",
                "initialized",
                "thread/read",
                "thread/turns/list"
            ]
        );
        assert_eq!(
            requests[2]["params"],
            json!({"threadId": SESSION, "includeTurns": false})
        );
        assert_eq!(requests[3]["params"]["threadId"], SESSION);
        assert_eq!(requests[3]["params"]["itemsView"], "summary");
        assert_eq!(preview.session_id, SESSION);
        assert_eq!(preview.title.as_deref(), Some("Previous run"));
        assert!(preview.has_more);
        assert_eq!(
            preview.messages,
            [
                NativeSessionMessage {
                    role: NativeSessionRole::User,
                    text: "Continue the run".into()
                },
                NativeSessionMessage {
                    role: NativeSessionRole::Assistant,
                    text: "Finished\nnext step.".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn codex_does_not_fetch_turns_for_a_different_session() {
        let fixture = json_lines(&[
            json!({"id": 1, "result": {}}),
            json!({"id": 2, "result": {"thread": {"id": OTHER_SESSION}}}),
        ]);
        let mut requests = Vec::new();
        let error = query_codex_session(fixture.as_bytes(), &mut requests, SESSION)
            .await
            .expect_err("reject a different session");
        assert_eq!(error.to_string(), "Codex returned a different session");
        assert!(
            !String::from_utf8(requests)
                .unwrap()
                .contains("thread/turns/list")
        );
    }

    #[tokio::test]
    async fn provider_error_does_not_echo_its_raw_detail() {
        let fixture = json_lines(&[
            json!({"id": 1, "result": {}}),
            json!({"id": 2, "error": {"code": -32600, "message": "private provider detail"}}),
        ]);
        let error = query_codex_session(fixture.as_bytes(), Vec::new(), SESSION)
            .await
            .expect_err("the provider refused the request");
        assert_eq!(
            error.to_string(),
            "Codex session reader rejected the request"
        );
    }
}
