//! Integration tests for the plugin sandbox.
//!
//! These tests drive the real `misbehaving-plugin` binary with the `stats` mode, so the working
//! directory and environment are what the operating system actually gave the child rather than what
//! the sandbox intended. The remaining assertions cover the manifest checks that refuse a declared
//! escape, the default-deny network policy, and the read/write allowlists.
//!
//! The OS-level boundary is deliberately not overclaimed: a plugin that opens a path directly is
//! constrained by the platform, not by these tests. What is asserted here is the host-mediated
//! policy and every control the runtime can establish structurally (`docs/SECURITY.md` §12, R-3).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use aicontext_core::ErrorCode;
use aicontext_plugin_runtime::supervisor::{SupervisorError, start_plugin};
use aicontext_plugin_runtime::{Sandbox, SandboxError, SandboxRoots};
use aicontext_plugin_sdk::{
    Auth, AuthKind, AuthReference, AuthSource, Capability, Effect, EntryPoint, FilesystemScope,
    Limits, NetworkScope, PluginManifest, PluginProtocol,
};
use serde_json::{Value, json};

const TOOL: &str = "fixture.echo.run";
const AMBIENT: &str = "AICONTEXT_SANDBOX_AMBIENT_PROBE";

fn manifest(mode: &str) -> PluginManifest {
    PluginManifest {
        manifest_version: 1,
        plugin_api_version: "1".to_owned(),
        protocol: PluginProtocol::AiPlugin,
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
            mcp_tool: None,
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

fn temporary_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary root")
}

fn roots(project: &Path) -> SandboxRoots {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let root = ROOT.get_or_init(|| tempfile::tempdir().expect("a temporary sandbox root"));
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let directory = root.path().join(format!("case-{id}"));
    let state = directory.join("state");
    fs::create_dir_all(state.join("plugins")).expect("the state root exists");
    SandboxRoots::new(state, project)
}

fn project() -> tempfile::TempDir {
    let directory = temporary_root();
    fs::create_dir(directory.path().join("src")).expect("the read directory exists");
    fs::create_dir(directory.path().join("build")).expect("the write directory exists");
    directory
}

#[test]
fn the_plugin_runs_in_the_sandbox_with_a_filtered_environment() {
    let manifest = manifest("stats");
    let project = project();
    let roots = roots(project.path());
    let sandbox = Sandbox::prepare(&manifest, &roots).expect("the sandbox is prepared");
    let mut process = start_plugin(&manifest, &sandbox).expect("the fixture starts");

    let result = process
        .call_tool(TOOL, json!({"text": "ignored"}))
        .expect("the fixture answers");
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .expect("the stats are a text block");
    let stats: Value = serde_json::from_str(text).expect("the stats are JSON");

    let cwd = stats["cwd"].as_str().expect("the child reports its cwd");
    assert_eq!(
        fs::canonicalize(cwd).expect("the cwd exists"),
        sandbox.work()
    );
    assert_eq!(
        stats["cwd_entries"],
        json!(0),
        "the work directory is empty"
    );
    assert_eq!(
        stats["ambient_present"],
        json!(false),
        "no ambient variable"
    );
    assert_eq!(stats["path_present"], json!(true), "the base PATH is set");

    let home = stats["home"].as_str().expect("HOME is set");
    assert_eq!(
        fs::canonicalize(home).expect("the home exists"),
        sandbox.home()
    );
    assert_eq!(stats["userprofile"], json!(home));
    for key in ["tmpdir", "tmp", "temp"] {
        let value = stats[key].as_str().expect("the temporary variable is set");
        assert_eq!(
            fs::canonicalize(value).expect("the temporary directory exists"),
            sandbox.temp(),
            "{key}"
        );
    }

    if let Some(host_home) = std::env::var_os("HOME") {
        assert_ne!(
            PathBuf::from(home),
            PathBuf::from(host_home),
            "the host home must not be inherited"
        );
    }
    assert_ne!(sandbox.home(), project.path(), "the project is not HOME");

    process.shutdown().expect("shutdown succeeds");
}

#[test]
fn network_egress_is_denied_by_default_and_limited_to_declared_hosts() {
    let project = project();
    let mut manifest = manifest("ok");
    let roots = roots(project.path());
    let sandbox = Sandbox::prepare(&manifest, &roots).expect("the sandbox is prepared");
    assert!(sandbox.network().is_deny_all());
    let error = sandbox
        .check_host("api.example.com")
        .expect_err("an empty egress list means no network");
    assert_eq!(error.code(), ErrorCode::PLG_009);
    assert!(matches!(error, SandboxError::Denied { .. }), "{error}");

    manifest.network.egress = vec!["API.Example.com".to_owned(), "s3.example.com".to_owned()];
    let sandbox = Sandbox::prepare(&manifest, &roots).expect("the sandbox is prepared");
    sandbox
        .check_host("api.example.com")
        .expect("the declared host is allowed");
    sandbox
        .check_host("S3.EXAMPLE.COM")
        .expect("host matching is case-insensitive");
    for denied in ["example.com", "evil.example.com", "api.example.com.evil"] {
        let error = sandbox
            .check_host(denied)
            .expect_err("the host is outside the egress list");
        assert_eq!(error.code(), ErrorCode::PLG_009, "{denied}");
    }
}

#[test]
fn host_mediated_paths_are_checked_against_read_and_write_lists() {
    let project = project();
    fs::write(project.path().join("src/main.rs"), "fn main() {}").expect("the file exists");
    let mut manifest = manifest("ok");
    manifest.filesystem.read = vec!["src".to_owned()];
    manifest.filesystem.write = vec!["build".to_owned()];
    let roots = roots(project.path());
    let sandbox = Sandbox::prepare(&manifest, &roots).expect("the sandbox is prepared");

    let read = sandbox
        .check_read("src/main.rs")
        .expect("the file is in the read scope");
    assert_eq!(
        read,
        fs::canonicalize(project.path())
            .unwrap()
            .join("src/main.rs")
    );
    assert!(sandbox.check_write("build/out.bin").is_ok());

    for denied in ["Cargo.toml", "src/../Cargo.toml", "../outside"] {
        let error = sandbox
            .check_read(denied)
            .expect_err("the path is outside the read scope");
        assert!(
            matches!(
                error,
                SandboxError::Denied { .. } | SandboxError::Escape { .. }
            ),
            "`{denied}`: {error}"
        );
    }
    let error = sandbox
        .check_write("src/main.rs")
        .expect_err("read access is not write access");
    assert!(matches!(error, SandboxError::Denied { .. }), "{error}");
}

#[test]
fn a_declared_escape_or_bad_host_is_refused_with_a_typed_error() {
    let project = project();
    let roots = roots(project.path());

    let mut escaping = manifest("ok");
    escaping.filesystem.read = vec!["../outside".to_owned()];
    let error = Sandbox::prepare(&escaping, &roots).expect_err("the path escapes the project");
    assert_eq!(error.code(), ErrorCode::PLG_009);
    assert!(matches!(error, SandboxError::Escape { .. }), "{error}");

    let mut reserved = manifest("ok");
    reserved.filesystem.write = vec!["CON.txt".to_owned()];
    let error = Sandbox::prepare(&reserved, &roots).expect_err("the name is not portable");
    assert_eq!(error.code(), ErrorCode::PLG_011);
    assert!(matches!(error, SandboxError::InvalidPath { .. }), "{error}");

    let mut bad_host = manifest("ok");
    bad_host.network.egress = vec!["https://example.com".to_owned()];
    let error = Sandbox::prepare(&bad_host, &roots).expect_err("a URL is not a host");
    assert_eq!(error.code(), ErrorCode::PLG_011);
    assert!(matches!(error, SandboxError::InvalidHost { .. }), "{error}");

    let mut controlled = manifest("ok");
    controlled.auth = Some(Auth {
        kind: AuthKind::EnvOrCredentialCommand,
        references: vec![AuthReference {
            name: "HOME".to_owned(),
            source: AuthSource::Env,
        }],
        instructions: None,
    });
    let error = Sandbox::prepare(&controlled, &roots).expect_err("HOME is the sandbox's own");
    assert_eq!(error.code(), ErrorCode::PLG_011);
    assert!(matches!(error, SandboxError::Environment { .. }), "{error}");
}

#[test]
fn a_sandbox_cannot_start_a_different_manifest() {
    let project = project();
    let roots = roots(project.path());
    let prepared = manifest("ok");
    let sandbox = Sandbox::prepare(&prepared, &roots).expect("the sandbox is prepared");

    let mut other = manifest("ok");
    other.name = "other".to_owned();
    other.capabilities[0].tool = "other.echo.run".to_owned();
    let error = start_plugin(&other, &sandbox).expect_err("the sandbox is for another plugin");
    assert_eq!(error.code(), ErrorCode::PLG_009);
    assert!(matches!(error, SupervisorError::Sandbox(_)), "{error}");
}

#[cfg(unix)]
#[test]
fn a_symlink_that_leaves_the_project_is_refused() {
    let project = project();
    let outside = temporary_root();
    std::os::unix::fs::symlink(outside.path(), project.path().join("link"))
        .expect("the symlink is created");
    let mut manifest = manifest("ok");
    manifest.filesystem.read = vec!["link".to_owned()];
    let roots = roots(project.path());

    let error = Sandbox::prepare(&manifest, &roots).expect_err("the link leaves the project");
    assert_eq!(error.code(), ErrorCode::PLG_009);
    assert!(matches!(error, SandboxError::Escape { .. }), "{error}");
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_refused_before_it_can_be_followed() {
    let project = project();
    let outside = temporary_root();
    std::os::unix::fs::symlink(
        outside.path().join("appears-later"),
        project.path().join("link"),
    )
    .expect("the symlink is created");
    let mut manifest = manifest("ok");
    manifest.filesystem.read = vec!["link".to_owned()];
    let roots = roots(project.path());

    let error = Sandbox::prepare(&manifest, &roots)
        .expect_err("a link whose target does not exist is not an ordinary missing path");
    assert_eq!(error.code(), ErrorCode::PLG_009);
    assert!(matches!(error, SandboxError::Io { .. }), "{error}");
}

#[test]
fn the_ambient_probe_is_not_set_by_the_test_environment() {
    // The stats fixture reports whether this name exists. It must not, or the environment test
    // above would be unable to distinguish a filtered child from an inherited one.
    assert!(std::env::var_os(AMBIENT).is_none(), "{AMBIENT} is set");
}
