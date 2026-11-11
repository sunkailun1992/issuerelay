//! Model and reasoning options for the binding editor. The executor catalog is advisory: a
//! model can appear in it without being available for inference on the signed-in account.

use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail, ensure};
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};

use crate::config::{ExecutorKind, is_valid_model_name};
use crate::env::ExecEnv;
use crate::runner::TreeGuard;
use crate::text::{clean_text, strip_ansi};

const MAX_MODELS: usize = 128;
const MAX_LABEL_BYTES: usize = 256;
const MAX_EFFORTS: usize = 16;
const MAX_EFFORT_BYTES: usize = 32;
const MAX_CURSOR_BYTES: usize = 1024;
const RECOMMENDED_MODELS: usize = 3;
const PAGE_SIZE: usize = 32;
const MAX_PAGES: usize = 8;
const MAX_STDOUT_BYTES: u64 = 512 * 1024;
const MAX_RPC_MESSAGES: usize = 128;
const QUERY_TIMEOUT: Duration = Duration::from_secs(8);

const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogSource {
    Native,
    Cache,
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub executor: ExecutorKind,
    pub source: ModelCatalogSource,
    pub models: Vec<ModelOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub efforts: Vec<String>,
    pub default_effort: Option<String>,
    pub recommended: bool,
    pub is_default: bool,
}

pub fn recommended_default(kind: ExecutorKind) -> &'static str {
    match kind {
        ExecutorKind::Codex => "gpt-6.1-sol",
        ExecutorKind::Claude => "fable",
    }
}

/// Recommendations for when no native catalog is available (checked 2026-11-11).
/// Codex's three entries and efforts were verified with CLI 0.159.2 `model/list`.
/// Claude aliases follow the installed CLI and provider, so they carry no pinned version or default effort.
/// https://learn.chatgpt.com/docs/models
/// https://code.claude.com/docs/en/model-config#model-aliases
pub fn builtin_catalog(kind: ExecutorKind) -> ModelCatalog {
    let entries: &[(&str, &str)] = match kind {
        ExecutorKind::Codex => &[
            ("gpt-6.1-sol", "GPT-6.1 Sol"),
            ("gpt-6-astra", "GPT-6 Astra"),
            ("gpt-6.1-terra", "GPT-6.1 Terra"),
        ],
        ExecutorKind::Claude => &[
            ("fable", "Claude Fable"),
            ("opus", "Claude Opus"),
            ("sonnet", "Claude Sonnet"),
            ("haiku", "Claude Haiku"),
        ],
    };
    let efforts = match kind {
        ExecutorKind::Codex => CODEX_EFFORTS,
        ExecutorKind::Claude => EFFORTS,
    };
    let models = entries
        .iter()
        .enumerate()
        .map(|(index, (id, label))| ModelOption {
            id: (*id).into(),
            label: (*label).into(),
            efforts: efforts.iter().map(|effort| (*effort).into()).collect(),
            default_effort: None,
            recommended: index < RECOMMENDED_MODELS,
            is_default: *id == recommended_default(kind),
        })
        .collect();
    ModelCatalog {
        executor: kind,
        source: ModelCatalogSource::Builtin,
        models,
    }
}

/// Takes only the documented `model/list` fields or the matching fields of a Codex model cache.
/// https://learn.chatgpt.com/docs/app-server#list-models-modellist
pub fn parse_codex_catalog(value: &Value) -> Option<ModelCatalog> {
    let (rows, source) = if let Some(result) = value.get("result") {
        (result.get("data")?.as_array()?, ModelCatalogSource::Native)
    } else if let Some(data) = value.get("data") {
        (data.as_array()?, ModelCatalogSource::Native)
    } else {
        (value.get("models")?.as_array()?, ModelCatalogSource::Cache)
    };
    let mut seen = HashSet::new();
    let mut models = rows
        .iter()
        .take(MAX_MODELS)
        .filter_map(|row| parse_model(row, source))
        .filter(|model| seen.insert(model.id.clone()))
        .collect::<Vec<_>>();
    if models.is_empty() {
        return None;
    }
    // The picker puts the native default first and keeps the vendor order for the rest.
    models.sort_by_key(|model| !model.is_default);
    for model in models.iter_mut().take(RECOMMENDED_MODELS) {
        model.recommended = true;
    }
    Some(ModelCatalog {
        executor: ExecutorKind::Codex,
        source,
        models,
    })
}

fn parse_model(row: &Value, source: ModelCatalogSource) -> Option<ModelOption> {
    let (id, label, efforts, effort_key, default_effort, is_default) = match source {
        ModelCatalogSource::Native if row.get("hidden").and_then(Value::as_bool) != Some(true) => (
            "model",
            "displayName",
            "supportedReasoningEfforts",
            "reasoningEffort",
            "defaultReasoningEffort",
            row.get("isDefault").and_then(Value::as_bool) == Some(true),
        ),
        ModelCatalogSource::Cache
            if row.get("visibility").and_then(Value::as_str) == Some("list") =>
        {
            (
                "slug",
                "display_name",
                "supported_reasoning_levels",
                "effort",
                "default_reasoning_level",
                false,
            )
        }
        _ => return None,
    };
    let id = row.get(id)?.as_str()?;
    if !is_valid_model_name(id) {
        return None;
    }
    let label = row
        .get(label)
        .and_then(Value::as_str)
        .filter(|label| label.len() <= MAX_LABEL_BYTES)
        .map(|label| clean_text(&strip_ansi(label)))
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| id.to_owned());
    let mut names = Vec::new();
    for effort in row
        .get(efforts)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_EFFORTS)
    {
        if let Some(name) = effort
            .get(effort_key)
            .and_then(Value::as_str)
            .filter(|name| safe_effort(name))
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    let default_effort = row
        .get(default_effort)
        .and_then(Value::as_str)
        .filter(|name| names.contains(name))
        .map(str::to_owned);
    Some(ModelOption {
        id: id.into(),
        label,
        efforts: names.into_iter().map(str::to_owned).collect(),
        default_effort,
        recommended: false,
        is_default,
    })
}

fn safe_effort(name: &str) -> bool {
    name.len() <= MAX_EFFORT_BYTES
        && name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

/// Requests model metadata over the installed CLI's stdio protocol. The command shares the
/// environment of the job runner and sends no thread or turn requests.
pub async fn discover_codex(program: &Path, env: &ExecEnv) -> Result<ModelCatalog> {
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
        .map_err(|_| anyhow!("could not start the Codex model query"))?;
    let mut pipes = child.stdout().take().zip(child.stdin().take());
    let mut tree = TreeGuard::new(child);
    let result = match pipes.as_mut() {
        Some((stdout, stdin)) => tokio::time::timeout(QUERY_TIMEOUT, query_codex(stdout, stdin))
            .await
            .unwrap_or_else(|_| Err(anyhow!("Codex model query timed out"))),
        None => Err(anyhow!("Codex model query pipes are unavailable")),
    };
    // Keep stdin open until the guard has found any descendants of the server.
    tree.end().await;
    result
}

async fn query_codex(
    stdout: impl AsyncRead + Unpin,
    mut stdin: impl AsyncWrite + Unpin,
) -> Result<ModelCatalog> {
    let mut stdout = BufReader::new(stdout.take(MAX_STDOUT_BYTES + 1));
    let mut read = 0;
    let mut messages = 0;
    send_request(
        &mut stdin,
        json!({
            "id": 1, "method": "initialize", "params": {"clientInfo": {
                "name": "issuerelay", "title": "IssueRelay", "version": env!("CARGO_PKG_VERSION")
            }}
        }),
    )
    .await?;
    read_reply(&mut stdout, 1, &mut read, &mut messages).await?;
    send_request(&mut stdin, json!({"method": "initialized"})).await?;

    let mut rows = Vec::new();
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    for page in 0..MAX_PAGES {
        let id = (page + 2) as u64;
        let mut params = json!({"limit": PAGE_SIZE, "includeHidden": false});
        if let Some(value) = cursor.take() {
            params["cursor"] = Value::String(value);
        }
        send_request(
            &mut stdin,
            json!({"id": id, "method": "model/list", "params": params}),
        )
        .await?;
        let mut result = read_reply(&mut stdout, id, &mut read, &mut messages).await?;
        let Some(data) = result.get_mut("data").and_then(Value::as_array_mut) else {
            bail!("Codex model query returned an invalid catalog page");
        };
        ensure!(
            data.len() <= PAGE_SIZE,
            "Codex model catalog page exceeds its limit"
        );
        rows.extend(data.drain(..).take(MAX_MODELS - rows.len()));
        match result.get("nextCursor") {
            None | Some(Value::Null) => break,
            Some(Value::String(value)) if !value.is_empty() && value.len() <= MAX_CURSOR_BYTES => {
                ensure!(
                    seen_cursors.insert(value.clone()),
                    "Codex model catalog repeated a page cursor"
                );
                cursor = Some(value.clone());
            }
            _ => bail!("Codex model query returned an invalid page cursor"),
        }
        if rows.len() == MAX_MODELS {
            break;
        }
        ensure!(
            page + 1 < MAX_PAGES,
            "Codex model catalog exceeds the page limit"
        );
    }
    parse_codex_catalog(&json!({"data": rows}))
        .ok_or_else(|| anyhow!("Codex model catalog is empty"))
}

async fn send_request(stdin: &mut (impl AsyncWrite + Unpin), request: Value) -> Result<()> {
    stdin.write_all(format!("{request}\n").as_bytes()).await?;
    stdin.flush().await?;
    Ok(())
}

async fn read_reply(
    stdout: &mut (impl AsyncBufRead + Unpin),
    id: u64,
    read: &mut u64,
    messages: &mut usize,
) -> Result<Value> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = stdout.read_until(b'\n', &mut line).await? as u64;
        ensure!(count != 0, "Codex model query closed before replying");
        *read += count;
        *messages += 1;
        ensure!(
            *read <= MAX_STDOUT_BYTES && *messages <= MAX_RPC_MESSAGES,
            "Codex model query output exceeds its limit"
        );
        let mut message: Value = serde_json::from_slice(&line)
            .map_err(|_| anyhow!("Codex model query returned invalid JSON"))?;
        if message.get("id").and_then(Value::as_u64) != Some(id) || message.get("method").is_some()
        {
            continue;
        }
        return message
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| anyhow!("Codex model query rejected the request"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn model(id: &str, efforts: &[&str], is_default: bool) -> Value {
        json!({
            "id": id, "model": id, "displayName": id, "hidden": false,
            "defaultReasoningEffort": "low", "isDefault": is_default,
            "supportedReasoningEfforts": efforts.iter().map(|effort| {
                json!({"reasoningEffort": effort, "description": "Model effort"})
            }).collect::<Vec<_>>()
        })
    }

    #[test]
    fn native_catalog_keeps_advertised_efforts_and_uses_the_model_slug() {
        let mut sol = model("gpt-6.1-sol", &["low", "max", "ultra", "future-mode"], true);
        sol["id"] = json!("catalog-id");
        let luna = model("gpt-6-luna", &["low", "max"], false);
        let catalog = parse_codex_catalog(&json!({"id": 2, "result": {"data": [sol, luna]}}))
            .expect("native catalog");
        assert_eq!(catalog.source, ModelCatalogSource::Native);
        assert_eq!(catalog.models[0].id, "gpt-6.1-sol");
        assert!(catalog.models[0].is_default);
        assert_eq!(catalog.models[0].default_effort.as_deref(), Some("low"));
        assert_eq!(
            catalog.models[0].efforts,
            ["low", "max", "ultra", "future-mode"]
        );
        assert_eq!(catalog.models[1].efforts, ["low", "max"]);
    }

    #[test]
    fn native_catalog_rejects_unsafe_tokens_and_ignores_hidden_rows() {
        let mut hidden = model("hidden-model", &["low"], false);
        hidden["hidden"] = json!(true);
        let mut visible = model("new-model", &["max", "x\nfoo=1", "--flag", "max"], true);
        visible["displayName"] = json!("\u{202e}\tNew model\n");
        let catalog = parse_codex_catalog(&json!({"data": [
            model("--flag", &["max"], false), hidden, visible,
            model("new-model", &["low"], false)
        ]}))
        .expect("one safe model");
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].label, "New model");
        assert_eq!(catalog.models[0].efforts, ["max"]);
        assert_eq!(catalog.models[0].default_effort, None);
        assert!(parse_codex_catalog(&json!({"data": []})).is_none());
    }

    #[test]
    fn cache_catalog_uses_only_visible_model_metadata() {
        let cache = json!({"identity": "unused", "models": [
            {"slug": "listed", "display_name": "Listed model", "visibility": "list",
             "default_reasoning_level": "max", "supported_reasoning_levels": [{"effort": "max"}]},
            {"slug": "hidden", "display_name": "Hidden model", "visibility": "hide"},
        ]});
        let catalog = parse_codex_catalog(&cache).expect("one visible cached model");
        assert_eq!(catalog.source, ModelCatalogSource::Cache);
        assert_eq!(catalog.models.len(), 1);
        assert_eq!(catalog.models[0].id, "listed");
        assert_eq!(catalog.models[0].default_effort.as_deref(), Some("max"));
        assert!(parse_codex_catalog(&json!({"result": {"models": cache["models"]}})).is_none());
    }

    #[test]
    fn claude_recommendations_use_aliases_and_only_claude_efforts() {
        let catalog = builtin_catalog(ExecutorKind::Claude);
        assert_eq!(recommended_default(ExecutorKind::Claude), "fable");
        assert_eq!(catalog.source, ModelCatalogSource::Builtin);
        assert_eq!(
            catalog
                .models
                .iter()
                .filter(|model| model.recommended)
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["fable", "opus", "sonnet"]
        );
        assert!(
            catalog
                .models
                .iter()
                .all(|model| !model.efforts.iter().any(|effort| effort == "ultra"))
        );
    }

    #[tokio::test]
    async fn native_query_collects_pages_without_starting_a_session() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (client_read, client_write) = tokio::io::split(client);
        let (server_read, mut server_write) = tokio::io::split(server);
        let fake_server = async {
            let mut requests = BufReader::new(server_read).lines();
            let responses = [
                ("initialize", json!({"id": 1, "result": {}})),
                ("initialized", Value::Null),
                (
                    "model/list",
                    json!({"id": 2, "result": {
                        "data": [model("sol", &["low", "ultra"], true)], "nextCursor": "page-2"
                    }}),
                ),
                (
                    "model/list",
                    json!({"id": 3, "result": {
                        "data": [model("sol", &["low"], false), model("luna", &["low"], false)],
                        "nextCursor": null
                    }}),
                ),
            ];
            for (method, reply) in responses {
                let request: Value =
                    serde_json::from_str(&requests.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(request["method"], method);
                if request["id"] == 3 {
                    assert_eq!(request["params"]["cursor"], "page-2");
                }
                if !reply.is_null() {
                    server_write
                        .write_all(format!("{reply}\n").as_bytes())
                        .await
                        .unwrap();
                }
            }
            assert!(requests.next_line().await.unwrap().is_none());
        };
        let (catalog, ()) = tokio::join!(query_codex(client_read, client_write), fake_server);
        let catalog = catalog.expect("both pages returned");
        assert_eq!(
            catalog
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["sol", "luna"]
        );
        assert_eq!(catalog.models[0].efforts, ["low", "ultra"]);
    }
}
