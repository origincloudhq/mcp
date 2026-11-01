//! What the generated tools share: their entries in the tool tables, and
//! the decoding, answering and waiting every tool does.

use std::time::Duration;

use origincloud_core::Operation;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

/// How long a tool answering with an operation waits for it before
/// answering with the operation as it stands, not done: well inside the
/// time MCP clients give a call, and long enough for most operations to
/// finish within it.
pub(crate) const OPERATION_WAIT: Duration = Duration::from_secs(30);

/// A tool, as the generator writes it in a service's table.
#[derive(Debug)]
pub(crate) struct ToolDef {
    /// `pvs_create_volume`.
    pub name: &'static str,
    /// `Create volume`.
    pub title: &'static str,
    /// What the method does, from its comment, and what the tool answers.
    pub description: &'static str,
    /// The JSON schema of the arguments, the request message in proto3
    /// JSON.
    pub input_schema: &'static str,
    pub effect: Effect,
}

/// What calling a tool does to the platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Changes nothing: a read, or waiting for an operation.
    Read,
    /// Changes something.
    Write {
        /// A delete, or a change that can't be undone.
        destructive: bool,
        /// A repeat with the same arguments does nothing more: its request
        /// has a request id.
        idempotent: bool,
    },
}

/// A service's tools, which `--toolsets` and its like select by name.
#[derive(Debug)]
pub(crate) struct Toolset {
    /// The service: `pvs`.
    pub name: &'static str,
    pub tools: &'static [ToolDef],
}

/// Why a tool call failed, as the agent reads it.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolError {
    /// The arguments don't make a request.
    #[error("{0}")]
    Arguments(String),
    /// The API refused the call, or couldn't be reached.
    #[error(transparent)]
    Call(#[from] origincloud_core::Error),
    /// The operation the call started failed: the operation, its `error`
    /// saying why.
    #[error("the operation failed")]
    Failed(Value),
}

impl ToolError {
    /// The error as the tool's structured result.
    pub(crate) fn to_json(&self) -> Value {
        match self {
            ToolError::Arguments(message) => json!({
                "reason": "INVALID_ARGUMENTS",
                "message": message,
            }),
            ToolError::Call(error) => match error.api() {
                Some(api) => json!({
                    "reason": api.reason,
                    "status": api.status,
                    "message": api.message,
                    "requestId": api.request_id,
                    "fieldViolations": api
                        .field_violations
                        .iter()
                        .map(|v| json!({ "field": v.field, "description": v.description }))
                        .collect::<Vec<_>>(),
                }),
                None => json!({ "reason": "UNAVAILABLE", "message": error.to_string() }),
            },
            ToolError::Failed(operation) => operation.clone(),
        }
    }
}

/// The request a tool's `arguments` make, once they have been checked
/// against its input schema.
pub(crate) fn request<T: DeserializeOwned>(arguments: Map<String, Value>) -> Result<T, ToolError> {
    serde_json::from_value(Value::Object(arguments))
        .map_err(|e| ToolError::Arguments(format!("the arguments don't make a request: {e}")))
}

/// `response` as the tool's result, in proto3 JSON.
pub(crate) fn reply<T: Serialize>(response: &T) -> Result<Value, ToolError> {
    serde_json::to_value(response)
        .map_err(|e| ToolError::Arguments(format!("the response can't be written: {e}")))
}

/// The operation a call started, once it is done, or as it stands after
/// [`OPERATION_WAIT`]: a failed one is the tool's error.
pub(crate) async fn finish<T: DeserializeOwned>(
    mut operation: Operation<T>,
) -> Result<Value, ToolError> {
    let waited = tokio::time::timeout(OPERATION_WAIT, operation.wait()).await;
    let raw = reply(operation.raw())?;
    match waited {
        Ok(Err(_)) if operation.done() => Err(ToolError::Failed(raw)),
        Ok(Err(error)) => Err(ToolError::Call(error)),
        Ok(Ok(_)) | Err(_) => Ok(raw),
    }
}
