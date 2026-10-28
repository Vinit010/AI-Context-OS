//! Manifest and wire-protocol types for AI Context OS plugins.
//!
//! A plugin is an executable that exposes tools over JSON-RPC 2.0 on stdin/stdout
//! (`docs/PLUGIN_SPEC.md`). This crate is the part a plugin author shares with the host: the
//! [`PluginManifest`] declararation, the [`Request`]/[`Response`] types, and the
//! [`ApiVersion`] handshake. The SDK is optional; the wire protocol is the contract.
//!
//! # What this crate is
//!
//! - **Manifest types** matching `docs/PLUGIN_SPEC.md` §2, with the name, tool, `SemVer`, and
//!   cross-field checks that a JSON Schema cannot state.
//! - **Protocol types** for every method in §4, with newline-delimited framing and a byte cap.
//! - **Version negotiation** that rejects an incompatible plugin with `PLG-004` and names both
//!   versions.
//!
//! # What this crate is not
//!
//! There is no process spawning, no stream reading, no timeout, no approval decision, and no
//! network. Those belong to `aicontext-plugin-runtime` (`TASK-073`), which is why the boundary
//! test permits this crate to depend on `aicontext-core` and nothing else internal. The plugin
//! author never links the runtime or the CLI.
//!
//! The SDK also does not carry a runtime JSON Schema engine. The host validates the manifest
//! against `schemas/plugin-manifest.schema.json` at install and before execution; the SDK's
//! [`PluginManifest::from_json`] is the author-facing early check.
//!
//! # Example
//!
//! ```
//! use aicontext_plugin_sdk::{ApiVersion, PluginManifest};
//!
//! let raw = r#"{
//!   "manifest_version": 1,
//!   "plugin_api_version": "1",
//!   "name": "demo",
//!   "version": "1.0.0",
//!   "entry": { "command": "demo-plugin" },
//!   "capabilities": [{ "tool": "demo.items.read", "effect": "read" }],
//!   "filesystem": { "read": [], "write": [] },
//!   "network": { "egress": [] },
//!   "limits": { "call_timeout_ms": 1000, "max_output_bytes": 1024 }
//! }"#;
//!
//! let manifest = PluginManifest::from_json(raw)?;
//! assert_eq!(manifest.negotiate(ApiVersion::CURRENT)?, ApiVersion::CURRENT);
//! # Ok::<(), aicontext_plugin_sdk::PluginError>(())
//! ```

#![forbid(unsafe_code)]

pub mod manifest;
pub mod protocol;
pub mod version;

mod error;

pub use error::PluginError;
pub use manifest::{
    Auth, AuthKind, AuthReference, AuthSource, Capability, Effect, EntryPoint, FilesystemScope,
    Limits, MANIFEST_VERSION, MAX_MANIFEST_BYTES, MAX_MCP_TOOL_NAME_BYTES, NetworkScope,
    PluginManifest, PluginProtocol,
};
pub use protocol::{
    Annotations, ApprovalParams, ApprovalResult, CallToolParams, CallToolResult, ContentBlock,
    Direction, EmptyParams, ErrorResponse, FrameCodec, INTERNAL_ERROR, INVALID_PARAMS,
    INVALID_REQUEST, InitializeParams, InitializeResult, JSONRPC_VERSION, JsonRpcVersion, LogLevel,
    LogParams, METHOD_NOT_FOUND, Method, Notification, PARSE_ERROR, RawNotification, RawRequest,
    Request, RequestId, Response, ResultMeta, RpcError, SuccessResponse, ToolDefinition,
    ToolsListResult,
};
pub use version::{ApiVersion, PLUGIN_API_VERSION, negotiate, negotiate_raw};
