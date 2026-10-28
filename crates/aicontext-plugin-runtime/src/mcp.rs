//! The Model Context Protocol adapter used when a manifest declares `protocol: "mcp"`.
//!
//! `docs/PLUGIN_SPEC.md` §10 registers a third-party MCP server as a plugin: the manifest remains
//! the permission declaration, and this module translates the parts of MCP that differ from the
//! plugin contract. MCP is JSON-RPC 2.0 over stdio, and its `initialize`, `tools/list`, and
//! `tools/call` methods already have the same names as ours, so the adapter is narrow:
//!
//! - `initialize` carries `protocolVersion`, `capabilities`, and `clientInfo` instead of
//!   `plugin_api_version`, and the host sends `notifications/initialized` after the response.
//! - A tool descriptor uses `name`, `description`, and `inputSchema`. MCP has no `effect` or
//!   `destructive`, so those come from the manifest's [`Capability`], which is also where the
//!   plugin-facing `provider.service.action` name lives. [`Capability::mcp_tool`] is the
//!   translation sent back in `tools/call`.
//! - MCP servers may send notifications and server-initiated requests. The supervisor ignores
//!   `notifications/*`, records `notifications/message` as a log, answers `ping`, and refuses every
//!   other server-initiated request with `-32601` rather than leaving the server waiting.
//!
//! The permission model is deliberately outside the protocol. MCP cannot express a permission
//! floor, and a hint such as `readOnlyHint` is untrusted data, so the manifest declares the effect
//! and the gateway resolves every call against `.ai/permissions/permissions.yaml` as usual. This is
//! the resolution of `ARCHITECTURE.md` §13 U-2.
//!
//! # Deliberate limits
//!
//! The transport is stdio only. HTTP, SSE, and streamable HTTP transports, MCP resources, prompts,
//! sampling, roots, and elicitation are not implemented; §10 of the specification lists the limits
//! in one place. A protocol version this build does not know is refused with `PLG-004` naming both
//! versions rather than guessed at.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use aicontext_plugin_sdk::{Annotations, Capability, ToolDefinition};

/// The MCP revision the host asks for during `initialize`.
///
/// `2025-06-18` is the newest revision this build implements. The server may answer with an older
/// revision it supports; [`SUPPORTED_PROTOCOL_VERSIONS`] is the set this adapter can read.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Every MCP revision this adapter understands.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

/// How many `tools/list` pages the adapter follows before refusing the server.
pub const MAX_PAGES: usize = 64;

/// How many tools the adapter will collect across all pages.
pub const MAX_TOOLS: usize = 1024;

/// The `initialize` params the host sends to an MCP server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// The MCP revision the host asks for.
    pub protocol_version: String,
    /// Client capabilities. Empty: this host supports no sampling, roots, or elicitation.
    #[serde(default)]
    pub capabilities: Map<String, Value>,
    /// Who is asking, for the server's diagnostics.
    pub client_info: ClientInfo,
}

impl Default for InitializeParams {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_owned(),
            capabilities: Map::new(),
            client_info: ClientInfo {
                name: "aicontext".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        }
    }
}

/// The client identification MCP's `initialize` requires.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClientInfo {
    /// The client's name.
    pub name: String,
    /// The client's version.
    pub version: String,
}

/// The `initialize` result an MCP server sends.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InitializeResult {
    /// The revision the server will speak. It may be older than the one the host asked for.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    /// The server's capabilities. Carried for diagnostics; this host uses none of them.
    #[serde(default)]
    pub capabilities: Map<String, Value>,
    /// The server's name and version, for the report and diagnostics.
    #[serde(rename = "serverInfo", default)]
    pub server_info: Option<ServerInfo>,
    /// Optional usage instructions. Untrusted data; this host does not show them in a prompt.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Unknown fields, carried through rather than refused so a newer revision can add one.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The server identification MCP's `initialize` result carries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ServerInfo {
    /// The server's name.
    pub name: String,
    /// The server's version.
    pub version: String,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The `tools/list` params.
///
/// MCP paginates with an opaque cursor the server chose. The host echoes it unchanged and never
/// interprets it; `None` asks for the first page.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolsListParams {
    /// The cursor from the previous page, or `None` for the first page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// The `tools/list` result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolsListResult {
    /// The page's tools.
    #[serde(default)]
    pub tools: Vec<Tool>,
    /// The cursor for the next page, when the server paginates.
    #[serde(rename = "nextCursor", default)]
    pub next_cursor: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One MCP tool descriptor.
///
/// Only the fields the host enforces are typed. `title` is used as a summary when `description` is
/// absent; the rest, including `outputSchema`, survive in `extra`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Tool {
    /// The server's tool name, sent back in `tools/call`.
    pub name: String,
    /// A human-readable title.
    #[serde(default)]
    pub title: Option<String>,
    /// The approval-prompt summary. MCP's `description` is shown verbatim, as a safety surface.
    #[serde(default)]
    pub description: Option<String>,
    /// The tool's input schema. MCP requires it; a missing value is a protocol failure.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// MCP annotations, carried through unchanged. Hints are not a permission floor.
    #[serde(default)]
    pub annotations: Option<Annotations>,
    /// Unknown fields, carried through unchanged.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The params of an MCP `notifications/message` log notification.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LogMessage {
    /// One of MCP's eight levels.
    pub level: String,
    /// The logger that emitted the message, when the server names one.
    #[serde(default)]
    pub logger: Option<String>,
    /// The payload. MCP allows any JSON value; the host bounds and sanitises the rendered form.
    pub data: Value,
}

impl LogMessage {
    /// The host's four-level vocabulary for this message.
    ///
    /// MCP has `debug`, `info`, `notice`, `warning`, `error`, `critical`, `alert`, and `emergency`.
    /// `None` means the level is not one MCP defines, which the supervisor refuses as a malformed
    /// frame rather than silently downgrading.
    #[must_use]
    pub fn severity(&self) -> Option<aicontext_plugin_sdk::LogLevel> {
        use aicontext_plugin_sdk::LogLevel;

        match self.level.as_str() {
            "debug" => Some(LogLevel::Debug),
            "info" | "notice" => Some(LogLevel::Info),
            "warning" => Some(LogLevel::Warning),
            "error" | "critical" | "alert" | "emergency" => Some(LogLevel::Error),
            _ => None,
        }
    }

    /// The message as one line, with the logger prefix when present.
    #[must_use]
    pub fn render(&self) -> String {
        let data = match &self.data {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        match &self.logger {
            Some(logger) => format!("{logger}: {data}"),
            None => data,
        }
    }
}

/// The name the MCP server uses for a declared capability.
#[must_use]
pub fn server_name(capability: &Capability) -> &str {
    capability
        .mcp_tool
        .as_deref()
        .unwrap_or(capability.tool.as_str())
}

/// Translate one MCP descriptor into the plugin contract's descriptor.
///
/// The permission fields come from the manifest, never from the server's annotations. The summary
/// falls back to `title` and then the server's name, because the approval prompt must always have
/// something to show.
#[must_use]
pub fn definition(capability: &Capability, tool: Tool) -> ToolDefinition {
    let summary = tool
        .description
        .or(tool.title)
        .unwrap_or_else(|| tool.name.clone());
    ToolDefinition {
        tool: capability.tool.clone(),
        summary,
        effect: capability.effect,
        destructive: capability.destructive,
        input_schema: tool.input_schema,
        annotations: tool.annotations,
        extra: tool.extra,
    }
}

/// Whether the server's chosen revision is one this adapter can read.
#[must_use]
pub fn supports(version: &str) -> bool {
    SUPPORTED_PROTOCOL_VERSIONS.contains(&version)
}

/// The supported revisions as one human-readable list.
#[must_use]
pub fn supported_versions() -> String {
    SUPPORTED_PROTOCOL_VERSIONS.join(", ")
}

#[cfg(test)]
mod tests {
    use aicontext_plugin_sdk::{Effect, LogLevel};
    use serde_json::json;

    use super::{
        LogMessage, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS, Tool, definition, server_name,
        supports,
    };
    use aicontext_plugin_sdk::Capability;

    fn capability() -> Capability {
        Capability {
            tool: "filesystem.file.read".to_owned(),
            effect: Effect::Read,
            destructive: false,
            mcp_tool: Some("read_file".to_owned()),
        }
    }

    #[test]
    fn the_adapter_speaks_the_newest_revision_it_knows() {
        assert_eq!(PROTOCOL_VERSION, "2025-06-18");
        assert!(supports(PROTOCOL_VERSION));
        for version in SUPPORTED_PROTOCOL_VERSIONS {
            assert!(supports(version), "{version}");
        }
        assert!(!supports("2026-01-01"));
    }

    #[test]
    fn the_server_name_defaults_to_the_plugin_name() {
        let mut capability = capability();
        assert_eq!(server_name(&capability), "read_file");
        capability.mcp_tool = None;
        assert_eq!(server_name(&capability), "filesystem.file.read");
    }

    #[test]
    fn a_descriptor_takes_its_permissions_from_the_manifest_not_the_server() {
        let tool: Tool = serde_json::from_value(json!({
            "name": "read_file",
            "description": "Read a file",
            "inputSchema": {"type": "object", "additionalProperties": false},
            "annotations": {"readOnlyHint": false, "destructiveHint": true}
        }))
        .expect("an MCP descriptor");

        let definition = definition(&capability(), tool);
        assert_eq!(definition.tool, "filesystem.file.read");
        assert_eq!(definition.summary, "Read a file");
        assert_eq!(definition.effect, Effect::Read);
        assert!(!definition.destructive);
        assert_eq!(
            definition
                .annotations
                .expect("annotations")
                .destructive_hint,
            Some(true),
            "the hint is carried as data, not as a permission"
        );
    }

    #[test]
    fn a_missing_description_falls_back_to_the_title_then_the_name() {
        let with_title: Tool = serde_json::from_value(json!({
            "name": "read_file",
            "title": "Read a file",
            "inputSchema": {}
        }))
        .expect("a descriptor");
        assert_eq!(definition(&capability(), with_title).summary, "Read a file");

        let bare: Tool = serde_json::from_value(json!({"name": "read_file", "inputSchema": {}}))
            .expect("a descriptor");
        assert_eq!(definition(&capability(), bare).summary, "read_file");
    }

    #[test]
    fn mcp_levels_map_onto_the_four_level_vocabulary() {
        let level = |name: &str| {
            LogMessage {
                level: name.to_owned(),
                logger: None,
                data: json!("x"),
            }
            .severity()
        };
        assert_eq!(level("debug"), Some(LogLevel::Debug));
        assert_eq!(level("info"), Some(LogLevel::Info));
        assert_eq!(level("notice"), Some(LogLevel::Info));
        assert_eq!(level("warning"), Some(LogLevel::Warning));
        assert_eq!(level("error"), Some(LogLevel::Error));
        assert_eq!(level("emergency"), Some(LogLevel::Error));
        assert_eq!(level("verbose"), None);
    }

    #[test]
    fn a_log_message_renders_its_logger_and_data() {
        let message = LogMessage {
            level: "info".to_owned(),
            logger: Some("files".to_owned()),
            data: json!({"path": "a.txt"}),
        };
        assert_eq!(message.render(), "files: {\"path\":\"a.txt\"}");
    }
}
