//! Integration tests for the MCP adapter.
//!
//! Every test drives the real `mcp-server` fixture over MCP's stdio transport. Nothing here
//! reaches into the adapter's internals: the server is a separate process that speaks the
//! protocol, and the host must complete MCP's handshake, follow pagination, translate names, and
//! map failures onto the documented `PLG` codes.

use std::fs;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use aicontext_core::ErrorCode;
use aicontext_plugin_runtime::supervisor::{PluginProcess, SupervisorError, start_plugin};
use aicontext_plugin_runtime::{Sandbox, SandboxRoots};
use aicontext_plugin_sdk::{
    Capability, Effect, EntryPoint, FilesystemScope, Limits, LogLevel, NetworkScope,
    PluginManifest, PluginProtocol,
};
use serde_json::json;

const READ: &str = "filesystem.file.read";
const WRITE: &str = "filesystem.file.write";

fn manifest(mode: &str) -> PluginManifest {
    PluginManifest {
        manifest_version: 1,
        plugin_api_version: "1".to_owned(),
        protocol: PluginProtocol::Mcp,
        name: "filesystem".to_owned(),
        version: "1.0.0".to_owned(),
        description: None,
        vendor: None,
        license: None,
        entry: EntryPoint {
            command: env!("CARGO_BIN_EXE_mcp-server").to_owned(),
            args: vec![mode.to_owned()],
        },
        minimum_aicontext_version: None,
        capabilities: vec![
            Capability {
                tool: READ.to_owned(),
                effect: Effect::Read,
                destructive: false,
                mcp_tool: Some("read_file".to_owned()),
            },
            Capability {
                tool: WRITE.to_owned(),
                effect: Effect::Write,
                destructive: false,
                mcp_tool: Some("write_file".to_owned()),
            },
        ],
        auth: None,
        filesystem: FilesystemScope {
            read: Vec::new(),
            write: Vec::new(),
        },
        network: NetworkScope { egress: Vec::new() },
        limits: Limits {
            call_timeout_ms: 2_000,
            max_output_bytes: 16 * 1024,
        },
    }
}

fn with_timeout(mut manifest: PluginManifest, timeout_ms: u64) -> PluginManifest {
    manifest.limits.call_timeout_ms = timeout_ms;
    manifest
}

/// A sandbox that outlives the test process: the fixture runs with this directory as its cwd.
fn sandbox(manifest: &PluginManifest) -> Sandbox {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("a temporary sandbox root"));
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = root.path().join(format!("case-{id}"));
    let project = directory.join("project");
    fs::create_dir_all(&project).expect("the project exists");
    let roots = SandboxRoots::new(directory.join("state"), project);
    Sandbox::prepare(manifest, &roots).expect("the sandbox is prepared")
}

fn start(manifest: &PluginManifest) -> Result<PluginProcess, SupervisorError> {
    let sandbox = sandbox(manifest);
    start_plugin(manifest, &sandbox)
}

#[test]
fn an_mcp_server_is_initialized_listed_called_and_shut_down() {
    let mut process = start(&manifest("ok")).expect("the MCP server starts");

    assert!(process.is_running());
    assert_eq!(
        process.capabilities().first().map(String::as_str),
        Some("tools"),
        "the server's capability names are reported for diagnostics"
    );

    let tools = process.list_tools().expect("tools/list succeeds");
    assert_eq!(tools.len(), 2, "both pages are followed");
    assert_eq!(tools[0].tool, READ);
    assert_eq!(tools[0].effect, Effect::Read);
    assert!(!tools[0].destructive);
    assert_eq!(tools[0].summary, "Read a file");
    assert_eq!(tools[1].tool, WRITE);
    assert_eq!(tools[1].effect, Effect::Write);
    assert_eq!(tools[1].summary, "Write a file");

    let result = process
        .call_tool(READ, json!({"text": "hello"}))
        .expect("tools/call succeeds");
    assert!(!result.is_error);
    assert_eq!(
        result.content.first().and_then(|block| block.as_text()),
        Some("read_file: hello"),
        "the host sent the server's name, not the plugin-facing name"
    );

    process
        .shutdown()
        .expect("shutdown closes stdin and the server exits");
}

#[test]
fn server_initiated_requests_are_answered_and_notifications_become_logs() {
    let mut process = start(&manifest("server-requests")).expect("the MCP server starts");
    process.list_tools().expect("tools/list succeeds");

    let result = process
        .call_tool(READ, json!({"text": "x"}))
        .expect("the call survives server-initiated traffic");
    assert_eq!(
        result.content.first().and_then(|block| block.as_text()),
        Some("ping=true;sampling=-32601"),
        "ping is answered with an empty result and sampling is refused with -32601"
    );

    let logs = process.take_logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].level(), LogLevel::Info);
    assert_eq!(logs[0].message(), "fixture: server request handled");

    process.shutdown().expect("the server exits on EOF");
}

#[test]
fn an_unsupported_mcp_version_is_plg_004_and_names_both_sides() {
    let error = start(&manifest("bad-version")).expect_err("the version is refused");

    assert_eq!(error.code(), ErrorCode::PLG_004);
    let message = error.to_string();
    assert!(message.contains("2026-01-01"), "{message}");
    assert!(message.contains("2025-06-18"), "{message}");
}

#[test]
fn a_tool_the_manifest_does_not_declare_is_plg_008() {
    let mut process = start(&manifest("undeclared")).expect("the MCP server starts");

    let error = process
        .list_tools()
        .expect_err("the undeclared tool is refused");
    assert_eq!(error.code(), ErrorCode::PLG_008);
    assert!(error.to_string().contains("secret_tool"), "{error}");
}

#[test]
fn a_descriptor_without_an_input_schema_is_plg_005() {
    let mut process = start(&manifest("bad-list")).expect("the MCP server starts");

    let error = process
        .list_tools()
        .expect_err("the descriptor is malformed");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(error.to_string().contains("inputSchema"), "{error}");
}

#[test]
fn a_json_rpc_error_from_the_server_is_plg_005() {
    let mut process = start(&manifest("json-error")).expect("the MCP server starts");
    process.list_tools().expect("tools/list succeeds");

    let error = process
        .call_tool(READ, json!({"text": "x"}))
        .expect_err("the server's error is a protocol failure");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(error.to_string().contains("-32603"), "{error}");
    assert!(
        error.to_string().contains("fixture refused the call"),
        "{error}"
    );
}

#[test]
fn is_error_is_a_result_not_a_runtime_failure() {
    let mut process = start(&manifest("is-error")).expect("the MCP server starts");
    process.list_tools().expect("tools/list succeeds");

    let result = process
        .call_tool(READ, json!({"text": "x"}))
        .expect("isError is data");
    assert!(result.is_error);
    assert_eq!(
        result.content.first().and_then(|block| block.as_text()),
        Some("boom")
    );
}

#[test]
fn a_server_that_does_not_answer_is_plg_003_and_is_killed() {
    let mut process = start(&with_timeout(manifest("hang"), 150)).expect("the MCP server starts");
    process.list_tools().expect("tools/list succeeds");

    let started = Instant::now();
    let error = process
        .call_tool(READ, json!({"text": "x"}))
        .expect_err("the call times out");
    assert_eq!(error.code(), ErrorCode::PLG_003);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the timeout is the manifest's, not the fixture's sleep"
    );
    assert!(!process.is_running(), "a timed-out process is killed");
}
