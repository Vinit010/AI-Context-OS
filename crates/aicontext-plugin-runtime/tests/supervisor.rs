//! Integration tests for the plugin supervisor.
//!
//! Every test drives the real `misbehaving-plugin` binary over the wire protocol. Nothing here
//! reaches into the supervisor's internals: a plugin that crashes, hangs, floods, lies about its
//! version, or answers with the wrong shape must produce the documented `PLG` code and a dead
//! process, which is exactly the contract `docs/PLUGIN_SPEC.md` §4 and §7 state.

use std::fs;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use aicontext_core::ErrorCode;
use aicontext_plugin_runtime::supervisor::{PluginProcess, SupervisorError, start_plugin};
use aicontext_plugin_runtime::{Sandbox, SandboxRoots};
use aicontext_plugin_sdk::{
    Capability, Effect, EntryPoint, FilesystemScope, Limits, LogLevel, NetworkScope, PluginManifest,
};
use serde_json::json;

const TOOL: &str = "fixture.echo.run";
const MISSING_COMMAND: &str = "aicontext-plugin-that-does-not-exist-6f2c";

fn manifest(mode: &str) -> PluginManifest {
    PluginManifest {
        manifest_version: 1,
        plugin_api_version: "1".to_owned(),
        name: "fixture".to_owned(),
        version: "1.0.0".to_owned(),
        description: None,
        vendor: None,
        license: None,
        entry: EntryPoint {
            command: env!("CARGO_BIN_EXE_misbehaving-plugin").to_owned(),
            args: vec![mode.to_owned()],
        },
        minimum_aicontext_version: None,
        capabilities: vec![Capability {
            tool: TOOL.to_owned(),
            effect: Effect::Write,
            destructive: false,
        }],
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

fn with_limits(
    mut manifest: PluginManifest,
    timeout_ms: u64,
    max_output_bytes: u64,
) -> PluginManifest {
    manifest.limits.call_timeout_ms = timeout_ms;
    manifest.limits.max_output_bytes = max_output_bytes;
    manifest
}

fn with_version(mut manifest: PluginManifest, version: &str) -> PluginManifest {
    version.clone_into(&mut manifest.plugin_api_version);
    manifest
}

fn with_command(mut manifest: PluginManifest, command: &str) -> PluginManifest {
    command.clone_into(&mut manifest.entry.command);
    manifest
}

/// A sandbox that outlives the test process: the fixture runs with this directory as its cwd.
///
/// One shared temporary root is removed when the test binary exits. Each call gets its own
/// `case-N` subtree so tests running in parallel cannot share a plugin tree.
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
fn a_well_behaved_plugin_is_started_listed_called_and_logged() {
    let mut process = start(&manifest("ok")).expect("the fixture starts");

    assert!(process.is_running());
    assert_eq!(process.capabilities().len(), 1);
    assert_eq!(process.capabilities()[0], "logging");

    let tools = process.list_tools().expect("tools/list succeeds");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool, TOOL);
    assert_eq!(tools[0].effect, Effect::Write);

    let result = process
        .call_tool(TOOL, json!({"text": "hello"}))
        .expect("tools/call succeeds");
    assert!(!result.is_error);
    assert_eq!(
        result.content.first().and_then(|block| block.as_text()),
        Some("hello")
    );

    let logs = process.take_logs();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].level(), LogLevel::Info);
    assert_eq!(logs[0].message(), "fixture handled the call");

    process.shutdown().expect("shutdown succeeds");
}

#[test]
fn a_plugin_that_exits_during_a_call_is_plg_007_and_is_not_retried() {
    let mut process = start(&manifest("crash")).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the fixture exits");
    assert_eq!(error.code(), ErrorCode::PLG_007);
    assert!(matches!(error, SupervisorError::Exited { .. }));
    assert!(!process.is_running());

    let second = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the pipe is poisoned");
    assert_eq!(second.code(), ErrorCode::PLG_007);
    assert!(matches!(second, SupervisorError::NotRunning));
}

#[test]
fn a_plugin_that_hangs_is_killed_at_the_per_call_timeout() {
    let mut process =
        start(&with_limits(manifest("hang"), 200, 16 * 1024)).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the fixture hangs");
    assert_eq!(error.code(), ErrorCode::PLG_003);
    assert!(
        matches!(
            error,
            SupervisorError::Timeout {
                method: "tools/call",
                timeout_ms: 200
            }
        ),
        "{error}"
    );
    assert!(!process.is_running());
}

#[test]
fn a_plugin_that_floods_one_frame_is_plg_006_and_is_killed() {
    let mut process =
        start(&with_limits(manifest("flood"), 2_000, 4_096)).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the fixture floods");
    assert_eq!(error.code(), ErrorCode::PLG_006);
    assert!(
        matches!(error, SupervisorError::OutputLimit { limit: 4_096, .. }),
        "{error}"
    );
    assert!(!process.is_running());
}

#[test]
fn a_plugin_that_floods_log_notifications_is_plg_006_and_is_killed() {
    let mut process =
        start(&with_limits(manifest("flood-logs"), 5_000, 4_096)).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the fixture floods the log channel");
    assert_eq!(error.code(), ErrorCode::PLG_006);
    assert!(!process.is_running());
}

#[test]
fn a_result_deeper_than_the_protocol_cap_is_plg_006() {
    let mut process =
        start(&with_limits(manifest("deep"), 2_000, 16 * 1024)).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the result is too deep");
    assert_eq!(error.code(), ErrorCode::PLG_006);
    assert!(!process.is_running());
}

#[test]
fn a_handshake_that_reports_another_api_version_is_plg_004() {
    let error = start(&manifest("bad-version")).expect_err("version 2 is not supported");

    assert_eq!(error.code(), ErrorCode::PLG_004);
    assert!(
        matches!(error, SupervisorError::ApiVersion { plugin, host } if plugin.number() == 2 && host.number() == 1),
        "{error}"
    );
}

#[test]
fn a_manifest_for_another_api_version_is_refused_before_the_process_starts() {
    let manifest = with_command(with_version(manifest("ok"), "2"), MISSING_COMMAND);

    let error = start(&manifest).expect_err("version 2 is not supported");
    assert_eq!(error.code(), ErrorCode::PLG_004);
    assert!(
        matches!(error, SupervisorError::ApiVersion { .. }),
        "the version must be refused before the spawn is attempted: {error}"
    );
}

#[test]
fn a_handshake_error_response_is_plg_002() {
    let error = start(&manifest("bad-handshake")).expect_err("the handshake is rejected");

    assert_eq!(error.code(), ErrorCode::PLG_002);
    assert!(
        matches!(error, SupervisorError::Handshake { ref detail } if detail.contains("fixture refuses initialize")),
        "{error}"
    );
}

#[test]
fn a_non_json_frame_is_plg_005_and_is_killed() {
    let mut process = start(&manifest("garbage")).expect("the fixture starts");

    let error = process.list_tools().expect_err("the frame is not JSON");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(matches!(error, SupervisorError::Protocol { .. }));
    assert!(!process.is_running());
}

#[test]
fn a_tool_error_response_is_plg_005_rather_than_a_result() {
    let mut process = start(&manifest("tool-error")).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the plugin used a JSON-RPC error");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(error.to_string().contains("fixture tool failed"), "{error}");
}

#[test]
fn a_result_with_the_wrong_shape_is_plg_005() {
    let mut process = start(&manifest("bad-result")).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("content is not an array");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(!process.is_running());
}

#[test]
fn a_response_with_the_wrong_id_is_plg_005() {
    let mut process = start(&manifest("wrong-id")).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the id does not match");
    assert_eq!(error.code(), ErrorCode::PLG_005);
    assert!(error.to_string().contains("999999"), "{error}");
}

#[test]
fn an_executable_that_does_not_exist_is_plg_001() {
    let manifest = with_command(manifest("ok"), MISSING_COMMAND);

    let error = start(&manifest).expect_err("there is no such executable");
    assert_eq!(error.code(), ErrorCode::PLG_001);
    assert!(
        matches!(error, SupervisorError::Spawn { ref program, .. } if program == MISSING_COMMAND),
        "{error}"
    );
}

#[test]
fn a_plugin_that_never_answers_initialize_is_plg_003() {
    let error = start(&with_limits(manifest("hang-handshake"), 200, 16 * 1024))
        .expect_err("the handshake hangs");

    assert_eq!(error.code(), ErrorCode::PLG_003);
    assert!(
        matches!(
            error,
            SupervisorError::Timeout {
                method: "initialize",
                timeout_ms: 200
            }
        ),
        "{error}"
    );
}

#[test]
fn a_plugin_that_ignores_shutdown_is_killed_and_reports_plg_003() {
    let process =
        start(&with_limits(manifest("no-shutdown"), 200, 16 * 1024)).expect("the fixture starts");

    let error = process.shutdown().expect_err("shutdown times out");
    assert_eq!(error.code(), ErrorCode::PLG_003);
    assert!(
        matches!(
            error,
            SupervisorError::Timeout {
                method: "shutdown",
                timeout_ms: 200
            }
        ),
        "{error}"
    );
}

#[test]
fn a_crashing_plugin_keeps_its_stderr_for_diagnosis() {
    let mut process = start(&manifest("crash")).expect("the fixture starts");

    let error = process
        .call_tool(TOOL, json!({"text": "x"}))
        .expect_err("the fixture exits");
    assert_eq!(error.code(), ErrorCode::PLG_007);

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if process
            .stderr_tail()
            .contains("fixture crashing on purpose")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "stderr tail did not contain the marker: {:?}",
            process.stderr_tail()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
