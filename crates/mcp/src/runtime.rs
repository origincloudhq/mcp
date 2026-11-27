//! What the generated tools share: their entries in the tool tables, and
//! the decoding, answering and waiting every tool does.

use std::time::Duration;

use origincloud_core::{Operation, Service};
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
    /// Changes nothing, but answers with secret material that then sits in
    /// the conversation, a secret's plaintext: behind the review gate, a
    /// session offers it only when set to allow sensitive tools, and never
    /// when read-only.
    Sensitive,
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

/// What a tool answers with.
#[derive(Debug, PartialEq)]
pub(crate) enum Reply {
    /// A message in proto3 JSON: the response, or the operation.
    Message(Value),
    /// A file: the answer of a method answering with a
    /// `google.api.HttpBody`, the file itself rather than JSON. Only the
    /// tools of such methods answer with one, and an API may have none.
    #[allow(dead_code)]
    File(File),
}

/// A file a tool answers with.
#[derive(Debug, PartialEq)]
pub(crate) struct File {
    /// Where the file was read from: the URL of the method's call, such as
    /// `https://billing.origincloudapis.eu/v1beta1/legalDocuments/terms/versions/3:download`.
    pub uri: String,
    /// Its media type, such as `application/pdf`; empty when the API named
    /// none.
    pub content_type: String,
    /// Its bytes.
    pub data: Vec<u8>,
}

/// `response` as the tool's result, in proto3 JSON.
pub(crate) fn reply<T: Serialize>(response: &T) -> Result<Reply, ToolError> {
    json(response).map(Reply::Message)
}

/// `value` in proto3 JSON.
fn json<T: Serialize>(value: &T) -> Result<Value, ToolError> {
    serde_json::to_value(value)
        .map_err(|e| ToolError::Arguments(format!("the response can't be written: {e}")))
}

/// The file a call of `service` at `path`, the escaped path of its HTTP
/// binding, answered with: its media type and bytes, from the SDK's
/// `HttpBody`. The tools of such methods call it, and an API may have none.
#[allow(dead_code)]
pub(crate) fn file(
    service: &Service,
    path: &str,
    content_type: String,
    data: Vec<u8>,
) -> Result<Reply, ToolError> {
    let uri = match service.base_url() {
        Some(base) => format!("{}{path}", base.as_str().trim_end_matches('/')),
        None => path.to_owned(),
    };
    Ok(Reply::File(File {
        uri,
        content_type,
        data,
    }))
}

/// The operation a call started, once it is done, or as it stands after
/// [`OPERATION_WAIT`]: a failed one is the tool's error.
pub(crate) async fn finish<T: DeserializeOwned>(
    mut operation: Operation<T>,
) -> Result<Reply, ToolError> {
    let waited = tokio::time::timeout(OPERATION_WAIT, operation.wait()).await;
    let raw = json(operation.raw())?;
    match waited {
        Ok(Err(_)) if operation.done() => Err(ToolError::Failed(raw)),
        Ok(Err(error)) => Err(ToolError::Call(error)),
        Ok(Ok(_)) | Err(_) => Ok(Reply::Message(raw)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file is named by the URL of the call that read it: the service's
    /// endpoint and the path of the method's binding.
    #[test]
    fn a_file_is_named_by_the_url_it_was_read_from() {
        let client = origincloud_core::Client::builder()
            .endpoint("http://127.0.0.1:8080/")
            .credentials(origincloud_core::AccessToken::new("token"))
            .build()
            .unwrap();
        let reply = file(
            &client.service("billing", false),
            "/v1beta1/legalDocuments/terms/versions/3:download",
            "application/pdf".to_owned(),
            b"%PDF-1.7".to_vec(),
        )
        .unwrap();
        assert_eq!(
            reply,
            Reply::File(File {
                uri: "http://127.0.0.1:8080/v1beta1/legalDocuments/terms/versions/3:download"
                    .to_owned(),
                content_type: "application/pdf".to_owned(),
                data: b"%PDF-1.7".to_vec(),
            })
        );
    }
}
