//! Request / response types and the method table.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Methods the engine answers over the current user's local IPC endpoint.
/// Lifecycle writes use the typed prepare/commit pair.
pub const METHODS: [&str; 37] = [
    "app.info",
    "status",
    "pause",
    "resume",
    "poll_now",
    "list_jobs",
    "doctor",
    "bindings.list",
    "bindings.add",
    "bindings.remove",
    "logs",
    "workflow.run",
    "workflow.context",
    "workflow.set_context",
    "workflow.handoff",
    "workflow.claim",
    "workflow.release",
    "workflow.archive",
    "workflow.issues",
    "workflow.plan_progress",
    "workflow.refresh_plan_progress",
    "workflow.continuation_context",
    "workflow.supplement_continuation_context",
    "workflow.development_issue_url",
    "workflow.prepare_action",
    "workflow.commit_action",
    "settings.get",
    "projects.list",
    "labels.list",
    "models.catalog",
    "setup.control_registration",
    "setup.linear_named",
    "linear.reconnect",
    "desktop.autostart_status",
    "desktop.cli_path_status",
    "desktop.update_status",
    "desktop.check_update",
];

/// The request line was not a valid request.
pub const PARSE_ERROR: i64 = -32700;
/// The method is not in `METHODS`.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// The engine could not carry the request out; the message says why.
pub const SERVER_ERROR: i64 = -32000;

/// One request line. `params` is `null` when the caller sent none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// The `error` member of a failed response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// One response line: a `result` on success, an `error` on failure, never both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_writes_require_the_typed_action_pair() {
        assert!(METHODS.contains(&"workflow.prepare_action"));
        assert!(METHODS.contains(&"workflow.commit_action"));
        for forbidden in [
            "mark_done",
            "retry_job",
            "cancel_job",
            "record_local_completion",
            "complete_plan",
        ] {
            assert!(
                !METHODS.contains(&forbidden),
                "{forbidden} must not bypass the action pair"
            );
        }
        // Other spellings count too: `retry`, `job.cancel`, `markDone`, ...
        for method in METHODS {
            let lower = method.to_ascii_lowercase();
            for word in ["done", "retry", "cancel", "complete"] {
                assert!(
                    !lower.contains(word),
                    "`{method}` looks like a direct lifecycle write"
                );
            }
        }
    }

    #[test]
    fn wire_format() {
        let req = Request::new(7, "status", serde_json::json!({}));
        let text = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), req);
        let ok = serde_json::to_value(Response::ok(7, serde_json::json!(1))).unwrap();
        assert!(ok.get("error").is_none());
        let err = serde_json::to_value(Response::err(7, METHOD_NOT_FOUND, "nope")).unwrap();
        assert!(err.get("result").is_none());
        assert_eq!(err["error"]["code"], -32601);
        let bare: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"status"}"#).unwrap();
        assert!(bare.params.is_null());
    }
}
