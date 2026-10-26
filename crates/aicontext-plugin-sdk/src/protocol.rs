//! The plugin wire protocol: JSON-RPC 2.0 over newline-delimited stdin/stdout.
//!
//! `docs/PLUGIN_SPEC.md` §4 defines six methods. stdout carries protocol frames only; a plugin that
//! wants to say something to a human sends a [`LogParams`] notification, and the host relays it to
//! stderr. A result is untrusted data (`CONTEXT_SPEC.md` §7.6), so this module describes its shape
//! and leaves size, depth, ANSI, and control-character limits to the runtime that reads it.
//!
//! # MCP compatibility
//!
//! The tool descriptor and call result are deliberately close to MCP. On input,
//! [`ToolDefinition`] accepts MCP's `name`, `description`, and `inputSchema` field names as aliases
//! for the plugin contract's `tool`, `summary`, and `input_schema`; [`CallToolResult`] accepts
//! `isError` as an alias for `is_error`. Serialization uses the plugin contract's names from
//! `docs/PLUGIN_SPEC.md` §3. An MCP tool definition must still declare `effect`, which MCP does not
//! have; the manifest is the authority for that and for `destructive`.
//!
//! # Example
//!
//! ```
//! use aicontext_plugin_sdk::{CallToolParams, FrameCodec, Request, RequestId};
//!
//! let request = Request::call_tool(
//!     RequestId::from(1_u64),
//!     CallToolParams {
//!         name: "aws.ec2.stop".to_owned(),
//!         arguments: serde_json::json!({"instance_id": "i-0abc1234"}),
//!     },
//! );
//! let line = FrameCodec::new(1024 * 1024)
//!     .encode_line(&request)
//!     .expect("a request encodes");
//! assert!(line.ends_with('\n'));
//! assert!(line.contains("\"method\":\"tools/call\""));
//! ```

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::PluginError;

/// The only JSON-RPC version this protocol accepts.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC 2.0's parse-error code.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC 2.0's invalid-request code.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC 2.0's method-not-found code.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC 2.0's invalid-params code.
pub const INVALID_PARAMS: i64 = -32602;
/// JSON-RPC 2.0's internal-error code.
pub const INTERNAL_ERROR: i64 = -32603;

/// The `jsonrpc` member, fixed at `"2.0"`.
///
/// A unit enum rather than a `String` so a frame with any other value fails to deserialize at the
/// boundary instead of reaching a handler that has to remember to check it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum JsonRpcVersion {
    /// JSON-RPC 2.0.
    #[default]
    #[serde(rename = "2.0")]
    V2,
}

impl JsonRpcVersion {
    /// The wire form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "2.0"
    }
}

impl fmt::Display for JsonRpcVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A JSON-RPC request identifier: a non-negative integer or a string.
///
/// The protocol allows either; the runtime echoes exactly what it received. Fractional and negative
/// numbers are refused by the type, because an id exists to be echoed rather than calculated.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RequestId {
    /// A non-negative integer id.
    Number(u64),
    /// A string id.
    String(String),
}

impl RequestId {
    /// The numeric value, if this id is a number.
    #[must_use]
    pub const fn as_number(&self) -> Option<u64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::String(_) => None,
        }
    }

    /// The string value, if this id is a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Number(_) => None,
            Self::String(value) => Some(value),
        }
    }
}

impl From<u64> for RequestId {
    fn from(number: u64) -> Self {
        Self::Number(number)
    }
}

impl From<String> for RequestId {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for RequestId {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(number) => write!(formatter, "{number}"),
            Self::String(value) => formatter.write_str(value),
        }
    }
}

/// The six methods in `docs/PLUGIN_SPEC.md` §4.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Method {
    /// Host → plugin handshake.
    #[serde(rename = "initialize")]
    Initialize,
    /// Host → plugin tool discovery.
    #[serde(rename = "tools/list")]
    ToolsList,
    /// Host → plugin tool invocation.
    #[serde(rename = "tools/call")]
    ToolsCall,
    /// Host → plugin graceful stop.
    #[serde(rename = "shutdown")]
    Shutdown,
    /// Plugin → host diagnostic notification.
    #[serde(rename = "log")]
    Log,
    /// Plugin → host approval request.
    #[serde(rename = "host/request_approval")]
    RequestApproval,
}

impl Method {
    /// The wire form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::ToolsList => "tools/list",
            Self::ToolsCall => "tools/call",
            Self::Shutdown => "shutdown",
            Self::Log => "log",
            Self::RequestApproval => "host/request_approval",
        }
    }

    /// Which side is allowed to send this method.
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::Initialize | Self::ToolsList | Self::ToolsCall | Self::Shutdown => {
                Direction::HostToPlugin
            }
            Self::Log | Self::RequestApproval => Direction::PluginToHost,
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The direction a method may travel.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Direction {
    /// Sent by the host and handled by the plugin.
    HostToPlugin,
    /// Sent by the plugin and handled by the host.
    PluginToHost,
}

/// The empty params object used by `tools/list` and `shutdown`.
///
/// A braced empty struct serializes as `{}`, which is what JSON-RPC clients expect; a unit struct
/// would serialize as `null`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct EmptyParams {}

/// A typed JSON-RPC request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Request<P> {
    /// The protocol version, always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id the response must echo.
    pub id: RequestId,
    /// The method.
    pub method: Method,
    /// The method's parameters.
    pub params: P,
}

impl Request<InitializeParams> {
    /// A handshake request.
    #[must_use]
    pub fn initialize(id: impl Into<RequestId>, params: InitializeParams) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id: id.into(),
            method: Method::Initialize,
            params,
        }
    }
}

impl Request<EmptyParams> {
    /// A tool-discovery request.
    #[must_use]
    pub fn tools_list(id: impl Into<RequestId>) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id: id.into(),
            method: Method::ToolsList,
            params: EmptyParams {},
        }
    }

    /// A graceful-stop request.
    #[must_use]
    pub fn shutdown(id: impl Into<RequestId>) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id: id.into(),
            method: Method::Shutdown,
            params: EmptyParams {},
        }
    }
}

impl Request<CallToolParams> {
    /// A tool-invocation request.
    #[must_use]
    pub fn call_tool(id: impl Into<RequestId>, params: CallToolParams) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id: id.into(),
            method: Method::ToolsCall,
            params,
        }
    }
}

impl Request<ApprovalParams> {
    /// A request from a plugin to the host for approval.
    #[must_use]
    pub fn request_approval(id: impl Into<RequestId>, params: ApprovalParams) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            id: id.into(),
            method: Method::RequestApproval,
            params,
        }
    }
}

/// A request as it arrives, before the method's params are known.
///
/// The host reads one of these, checks [`Method::direction`], then calls [`RawRequest::params_as`]
/// with the params type for that method.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawRequest {
    /// The protocol version, always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id the response must echo.
    pub id: RequestId,
    /// The method.
    pub method: Method,
    /// The method's params, still untyped. An omitted params member becomes `{}`.
    #[serde(default = "empty_object")]
    pub params: Value,
}

impl RawRequest {
    /// Deserialize this request's params into `T`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::Params`] (`PLG-005`) naming the method when the object does not have
    /// the shape that method requires.
    pub fn params_as<T: DeserializeOwned>(&self) -> Result<T, PluginError> {
        serde_json::from_value(self.params.clone()).map_err(|source| PluginError::Params {
            method: self.method.as_str(),
            source,
        })
    }
}

/// A JSON-RPC notification: a request with no id and no response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Notification<P> {
    /// The protocol version, always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The method.
    pub method: Method,
    /// The method's parameters.
    pub params: P,
}

impl Notification<LogParams> {
    /// A diagnostic notification to the host.
    #[must_use]
    pub fn log(params: LogParams) -> Self {
        Self {
            jsonrpc: JsonRpcVersion::V2,
            method: Method::Log,
            params,
        }
    }
}

/// A notification as it arrives, before its params are known.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RawNotification {
    /// The protocol version, always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The method.
    pub method: Method,
    /// The method's params, still untyped. An omitted params member becomes `{}`.
    #[serde(default = "empty_object")]
    pub params: Value,
}

/// A successful JSON-RPC response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessResponse<R> {
    /// The protocol version, always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id from the request.
    pub id: RequestId,
    /// The method's result.
    pub result: R,
}

/// A JSON-RPC error response.
///
/// `id` is optional because a request whose id could not be parsed still gets an error frame, and
/// JSON-RPC requires the id to be `null` in that case.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    /// The protocol version, always `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// The id from the request, or `null` when it could not be read.
    #[serde(default)]
    pub id: Option<RequestId>,
    /// The error.
    pub error: RpcError,
}

/// Either a success or an error response.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Response<R> {
    /// A successful result.
    Success(SuccessResponse<R>),
    /// A typed failure.
    Error(ErrorResponse),
}

impl<R> Response<R> {
    /// A success response echoing `id`.
    #[must_use]
    pub fn success(id: impl Into<RequestId>, result: R) -> Self {
        Self::Success(SuccessResponse {
            jsonrpc: JsonRpcVersion::V2,
            id: id.into(),
            result,
        })
    }

    /// An error response, with `None` when the request id could not be read.
    #[must_use]
    pub fn error(id: Option<RequestId>, error: RpcError) -> Self {
        Self::Error(ErrorResponse {
            jsonrpc: JsonRpcVersion::V2,
            id,
            error,
        })
    }
}

/// A JSON-RPC error object.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RpcError {
    /// The JSON-RPC error code.
    pub code: i64,
    /// A human-readable message. The host never reformats it optimistically.
    pub message: String,
    /// Optional structured detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// An error with a code and message and no data.
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// A parse error (`-32700`).
    #[must_use]
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(PARSE_ERROR, message)
    }

    /// An invalid-request error (`-32600`).
    #[must_use]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message)
    }

    /// A method-not-found error (`-32601`).
    #[must_use]
    pub fn method_not_found(message: impl Into<String>) -> Self {
        Self::new(METHOD_NOT_FOUND, message)
    }

    /// An invalid-params error (`-32602`).
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    /// An internal error (`-32603`).
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, message)
    }
}

/// The `initialize` params sent by the host.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitializeParams {
    /// The host's plugin API version, negotiated against the plugin's manifest.
    pub plugin_api_version: String,
    /// Host capabilities the plugin may rely on. Empty means none beyond the base protocol.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_capabilities: Vec<String>,
}

/// The `initialize` result sent by the plugin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InitializeResult {
    /// The plugin's plugin API version, which the host compares with its own.
    pub plugin_api_version: String,
    /// Plugin capabilities, for diagnostics and future negotiation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// The `tools/list` result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolsListResult {
    /// Every tool the plugin implements.
    pub tools: Vec<ToolDefinition>,
}

/// The `tools/call` params.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CallToolParams {
    /// The tool to call.
    pub name: String,
    /// The arguments, validated by the gateway against the tool's `input_schema` before dispatch.
    #[serde(default = "empty_object")]
    pub arguments: Value,
}

/// The `tools/call` result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CallToolResult {
    /// The result content. MCP content types are carried as [`ContentBlock`]s.
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    /// Whether the tool failed. This is a result, not a JSON-RPC error: the call completed.
    #[serde(rename = "is_error", alias = "isError", default)]
    pub is_error: bool,
    /// Host-recorded metadata such as the request id and duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<ResultMeta>,
    /// Unknown MCP fields, carried through unchanged rather than dropped.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Metadata the host attaches to a tool result.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ResultMeta {
    /// The host's request id.
    #[serde(
        rename = "request_id",
        alias = "requestId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub request_id: Option<String>,
    /// The call's wall-clock duration in milliseconds.
    #[serde(
        rename = "duration_ms",
        alias = "durationMs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub duration_ms: Option<u64>,
    /// Unknown metadata, carried through unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One MCP-compatible content block.
///
/// Modelled as `type` plus a flat field map so MCP's text, image, audio, resource, and
/// resource-link blocks all survive the round trip. The runtime still treats every field as
/// untrusted and bounds the whole result before it reaches a prompt.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContentBlock {
    /// The MCP content type, for example `text`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The type-specific fields, for example `text` on a text block.
    #[serde(flatten)]
    pub fields: Map<String, Value>,
}

impl ContentBlock {
    /// A text block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        let mut fields = Map::new();
        fields.insert("text".to_owned(), Value::String(text.into()));
        Self {
            kind: "text".to_owned(),
            fields,
        }
    }

    /// The block's text, if it is a text block with a string `text` field.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        if self.kind == "text" {
            self.fields.get("text").and_then(Value::as_str)
        } else {
            None
        }
    }
}

/// A tool descriptor, as returned by `tools/list`.
///
/// The field names serialize as `docs/PLUGIN_SPEC.md` §3 shows. MCP's `name`, `description`, and
/// `inputSchema` are accepted on input; `effect` is required because MCP has no equivalent and a
/// missing read/write classification must not default to `read`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolDefinition {
    /// The tool name, `provider.service.action`.
    #[serde(rename = "tool", alias = "name")]
    pub tool: String,
    /// The approval-prompt summary. A safety surface, shown verbatim.
    #[serde(rename = "summary", alias = "description")]
    pub summary: String,
    /// Whether the tool reads or writes.
    pub effect: crate::manifest::Effect,
    /// Whether the tool deletes, terminates, or overwrites something.
    #[serde(default)]
    pub destructive: bool,
    /// The Draft 2020-12 input schema, enforced by the gateway before the call.
    #[serde(rename = "input_schema", alias = "inputSchema")]
    pub input_schema: Value,
    /// MCP tool annotations, carried through unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Annotations>,
    /// Unknown descriptor fields, carried through unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// MCP tool annotations.
///
/// The five documented hints are typed; anything a future MCP revision adds survives in `extra`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotations {
    /// A human-readable title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The tool does not modify state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    /// The tool may perform destructive updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    /// Repeated calls with the same arguments have no additional effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    /// The tool may interact with entities outside the host's knowledge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
    /// Unknown annotation keys, carried through unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `log` notification params.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogParams {
    /// The level the host should use when relaying to stderr.
    pub level: LogLevel,
    /// The diagnostic text. The host bounds it; the plugin must not include secrets.
    pub message: String,
}

/// A diagnostic level.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum LogLevel {
    /// Developer detail.
    Debug,
    /// Normal progress.
    Info,
    /// Something is wrong but the call may continue.
    Warning,
    /// Something failed.
    Error,
}

/// The `host/request_approval` params.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalParams {
    /// The tool the plugin wants to call.
    pub tool: String,
    /// The arguments it wants to call it with, so the human sees the real action.
    #[serde(default = "empty_object")]
    pub arguments: Value,
    /// Why the plugin thinks the call is needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The `host/request_approval` result.
///
/// The host is still the decision authority: a plugin may ask, but it cannot grant itself anything.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalResult {
    /// Whether the host approved this invocation.
    pub approved: bool,
    /// Why the host decided as it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Newline-delimited JSON framing with a byte cap.
///
/// The codec does not read or write a stream; it turns one message into one line and one line back
/// into a message. That keeps it pure and testable, and leaves the pipe, the timeout, and the
/// process to the runtime that owns them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameCodec {
    max_bytes: usize,
}

impl FrameCodec {
    /// A codec that refuses any single frame larger than `max_bytes`.
    ///
    /// The caller chooses the cap from the plugin's `limits.max_output_bytes` or the host's own
    /// policy. A zero cap refuses every frame rather than meaning "unlimited".
    #[must_use]
    pub const fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }

    /// The configured cap.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    /// Encode one message as a single line, without the trailing newline.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::EncodeJson`] if the value cannot be serialized, or
    /// [`PluginError::FrameRule`] (`PLG-005`) if the encoded frame exceeds the cap or would contain
    /// a line break.
    pub fn encode<T: Serialize>(&self, message: &T) -> Result<String, PluginError> {
        let json =
            serde_json::to_string(message).map_err(|source| PluginError::EncodeJson { source })?;

        if json.len() > self.max_bytes {
            return Err(PluginError::FrameRule {
                detail: format!(
                    "the encoded frame is {} bytes; the maximum is {}",
                    json.len(),
                    self.max_bytes
                ),
            });
        }
        if json.contains('\n') || json.contains('\r') {
            return Err(PluginError::FrameRule {
                detail: "a frame must not contain a line break".to_owned(),
            });
        }

        Ok(json)
    }

    /// Encode one message as a newline-terminated line.
    ///
    /// # Errors
    ///
    /// As [`FrameCodec::encode`].
    pub fn encode_line<T: Serialize>(&self, message: &T) -> Result<String, PluginError> {
        let mut line = self.encode(message)?;
        line.push('\n');
        Ok(line)
    }

    /// Decode one line, accepting an optional trailing `\n` or `\r\n`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::FrameRule`] (`PLG-005`) for an empty line, a frame over the cap, or a
    /// line that still contains a break after the terminator is removed; returns
    /// [`PluginError::FrameJson`] (`PLG-005`) when the line is not valid JSON.
    pub fn decode<T: DeserializeOwned>(&self, line: &str) -> Result<T, PluginError> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);

        if line.len() > self.max_bytes {
            return Err(PluginError::FrameRule {
                detail: format!(
                    "the frame is {} bytes; the maximum is {}",
                    line.len(),
                    self.max_bytes
                ),
            });
        }
        if line.trim().is_empty() {
            return Err(PluginError::FrameRule {
                detail: "the frame is empty".to_owned(),
            });
        }
        if line.contains('\n') || line.contains('\r') {
            return Err(PluginError::FrameRule {
                detail: "the frame contains more than one line".to_owned(),
            });
        }

        serde_json::from_str(line).map_err(|source| PluginError::FrameJson { source })
    }
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        Annotations, CallToolParams, CallToolResult, FrameCodec, JsonRpcVersion, LogLevel,
        LogParams, Method, Notification, RawRequest, Request, RequestId, Response, RpcError,
        ToolDefinition,
    };
    use crate::error::PluginError;

    fn codec() -> FrameCodec {
        FrameCodec::new(64 * 1024)
    }

    #[test]
    fn a_call_tool_request_has_the_documented_jsonrpc_shape() {
        let request = Request::call_tool(
            1_u64,
            CallToolParams {
                name: "aws.ec2.stop".to_owned(),
                arguments: json!({"instance_id": "i-0abc1234", "reason": "idle overnight"}),
            },
        );
        let value = serde_json::to_value(&request).expect("serialisable");

        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 1);
        assert_eq!(value["method"], "tools/call");
        assert_eq!(value["params"]["name"], "aws.ec2.stop");
        assert_eq!(value["params"]["arguments"]["instance_id"], "i-0abc1234");
    }

    #[test]
    fn string_ids_round_trip() {
        let request = Request::tools_list("req-8f2c");
        let line = codec().encode(&request).expect("encodes");
        let decoded: RawRequest = codec().decode(&line).expect("decodes");
        assert_eq!(decoded.id, RequestId::String("req-8f2c".to_owned()));
        assert_eq!(decoded.method, Method::ToolsList);
    }

    #[test]
    fn numeric_ids_round_trip() {
        let request = Request::shutdown(7_u64);
        let line = codec().encode(&request).expect("encodes");
        let decoded: RawRequest = codec().decode(&line).expect("decodes");
        assert_eq!(decoded.id, RequestId::Number(7));
    }

    #[test]
    fn empty_params_serialize_as_an_object_not_null() {
        let request = Request::tools_list(1_u64);
        let value = serde_json::to_value(&request).expect("serialisable");
        assert_eq!(value["params"], json!({}));
    }

    #[test]
    fn the_initialize_request_carries_the_host_version_and_capabilities() {
        let request = Request::initialize(
            1_u64,
            super::InitializeParams {
                plugin_api_version: "1".to_owned(),
                host_capabilities: vec!["approval".to_owned()],
            },
        );
        let value = serde_json::to_value(&request).expect("serialisable");
        assert_eq!(value["method"], "initialize");
        assert_eq!(value["params"]["plugin_api_version"], "1");
        assert_eq!(value["params"]["host_capabilities"][0], "approval");
    }

    #[test]
    fn an_unknown_method_is_rejected_at_the_boundary() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/nope","params":{}}"#;
        let error = codec().decode::<RawRequest>(line).expect_err("must reject");
        assert!(matches!(error, PluginError::FrameJson { .. }));
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
    }

    #[test]
    fn a_wrong_jsonrpc_version_is_rejected_at_the_boundary() {
        let line = r#"{"jsonrpc":"1.0","id":1,"method":"tools/list","params":{}}"#;
        let error = codec().decode::<RawRequest>(line).expect_err("must reject");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
    }

    #[test]
    fn params_that_do_not_match_the_method_are_plg_005() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":7}}"#;
        let raw: RawRequest = codec().decode(line).expect("the envelope is valid");
        let error = raw
            .params_as::<CallToolParams>()
            .expect_err("name is not a string");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
        assert!(error.to_string().contains("tools/call"));
    }

    #[test]
    fn a_success_response_round_trips() {
        let response = Response::success(
            RequestId::from(1_u64),
            CallToolResult {
                content: vec![super::ContentBlock::text("{\"previous\":\"running\"}")],
                is_error: false,
                meta: Some(super::ResultMeta {
                    request_id: Some("req-8f2c".to_owned()),
                    duration_ms: Some(412),
                    extra: serde_json::Map::new(),
                }),
                extra: serde_json::Map::new(),
            },
        );
        let line = codec().encode(&response).expect("encodes");
        let decoded: Response<CallToolResult> = codec().decode(&line).expect("decodes");
        assert_eq!(decoded, response);
    }

    #[test]
    fn an_error_response_round_trips_with_a_null_id() {
        let response: Response<()> =
            Response::error(None, RpcError::invalid_request("the request had no method"));
        let line = codec().encode(&response).expect("encodes");
        let value: serde_json::Value = serde_json::from_str(&line).expect("JSON");
        assert_eq!(value["id"], serde_json::Value::Null);
        assert_eq!(value["error"]["code"], -32600);

        let decoded: Response<()> = codec().decode(&line).expect("decodes");
        assert_eq!(decoded, response);
    }

    #[test]
    fn a_response_cannot_be_both_success_and_error() {
        let line =
            r#"{"jsonrpc":"2.0","id":1,"result":null,"error":{"code":-32603,"message":"x"}}"#;
        let error = codec()
            .decode::<Response<()>>(line)
            .expect_err("must reject");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
    }

    #[test]
    fn a_log_notification_has_no_id() {
        let notification = Notification::log(LogParams {
            level: LogLevel::Warning,
            message: "aws profile is not set".to_owned(),
        });
        let value = serde_json::to_value(&notification).expect("serialisable");
        assert_eq!(value["method"], "log");
        assert!(value.get("id").is_none());
        assert_eq!(value["params"]["level"], "warning");
    }

    #[test]
    fn a_tool_definition_accepts_the_mcp_field_names() {
        let mcp = json!({
            "name": "aws.ec2.stop",
            "description": "Stop a running EC2 instance",
            "effect": "write",
            "inputSchema": {"type": "object", "additionalProperties": false}
        });
        let tool: ToolDefinition = serde_json::from_value(mcp).expect("MCP shape");
        assert_eq!(tool.tool, "aws.ec2.stop");
        assert_eq!(tool.summary, "Stop a running EC2 instance");
        assert_eq!(tool.effect, crate::manifest::Effect::Write);
        assert_eq!(tool.input_schema["type"], "object");
    }

    #[test]
    fn a_tool_definition_round_trips_with_annotations_and_unknown_fields() {
        let tool = ToolDefinition {
            tool: "aws.ec2.stop".to_owned(),
            summary: "Stop a running EC2 instance".to_owned(),
            effect: crate::manifest::Effect::Write,
            destructive: false,
            input_schema: json!({"type": "object", "additionalProperties": false}),
            annotations: Some(Annotations {
                read_only_hint: Some(false),
                idempotent_hint: Some(false),
                ..Annotations::default()
            }),
            extra: serde_json::Map::new(),
        };
        let line = codec().encode(&tool).expect("encodes");
        let value: serde_json::Value = serde_json::from_str(&line).expect("JSON");
        assert_eq!(value["tool"], "aws.ec2.stop");
        assert_eq!(value["summary"], "Stop a running EC2 instance");
        assert_eq!(value["input_schema"]["type"], "object");
        assert_eq!(value["annotations"]["readOnlyHint"], false);

        let decoded: ToolDefinition = codec().decode(&line).expect("decodes");
        assert_eq!(decoded, tool);
    }

    #[test]
    fn a_call_result_accepts_the_mcp_is_error_alias() {
        let result: CallToolResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "boom"}],
            "isError": true
        }))
        .expect("MCP result shape");
        assert!(result.is_error);
        assert_eq!(
            result
                .content
                .first()
                .and_then(super::ContentBlock::as_text),
            Some("boom")
        );
    }

    #[test]
    fn every_method_has_the_documented_direction() {
        for (method, direction) in [
            (Method::Initialize, super::Direction::HostToPlugin),
            (Method::ToolsList, super::Direction::HostToPlugin),
            (Method::ToolsCall, super::Direction::HostToPlugin),
            (Method::Shutdown, super::Direction::HostToPlugin),
            (Method::Log, super::Direction::PluginToHost),
            (Method::RequestApproval, super::Direction::PluginToHost),
        ] {
            assert_eq!(method.direction(), direction, "{method}");
        }
    }

    #[test]
    fn the_codec_rejects_a_frame_over_its_cap() {
        let codec = FrameCodec::new(16);
        let error = codec
            .encode_line(&Request::tools_list(1_u64))
            .expect_err("too big");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
        assert!(error.to_string().contains("maximum"));
    }

    #[test]
    fn the_codec_rejects_two_objects_on_one_line() {
        let error = codec()
            .decode::<RawRequest>(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"} {}"#)
            .expect_err("must reject");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_005);
    }

    #[test]
    fn the_codec_accepts_one_trailing_newline_but_not_two_lines() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let with_newline = format!("{line}\n");
        let _: RawRequest = codec().decode(&with_newline).expect("one trailing newline");

        let two_lines = format!("{line}\n{line}\n");
        let error = codec()
            .decode::<RawRequest>(&two_lines)
            .expect_err("must reject");
        assert!(error.to_string().contains("more than one line"));
    }

    #[test]
    fn the_jsonrpc_version_is_two_point_zero() {
        assert_eq!(JsonRpcVersion::V2.as_str(), "2.0");
        assert_eq!(JsonRpcVersion::default(), JsonRpcVersion::V2);
    }

    #[test]
    fn built_in_error_constructors_use_the_standard_codes() {
        assert_eq!(RpcError::parse_error("x").code, super::PARSE_ERROR);
        assert_eq!(RpcError::invalid_request("x").code, super::INVALID_REQUEST);
        assert_eq!(
            RpcError::method_not_found("x").code,
            super::METHOD_NOT_FOUND
        );
        assert_eq!(RpcError::invalid_params("x").code, super::INVALID_PARAMS);
        assert_eq!(RpcError::internal_error("x").code, super::INTERNAL_ERROR);
    }
}
