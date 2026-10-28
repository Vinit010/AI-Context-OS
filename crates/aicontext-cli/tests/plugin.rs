//! `aicontext plugin list|install|remove` against the real binary.
//!
//! The unit tests in `src/plugin/` drive the flow with a fake prompter; these tests drive the
//! process, so they cover the parts a fake cannot: `--yes`, a pipe where no human can answer, the
//! files the command leaves behind, and the JSON a script branches on.
//!
//! Every install here passes `--yes`. A test that runs without it is asserting the refusal, because
//! the test harness has no terminal and `docs/PLUGIN_SPEC.md` §8 requires a confirmation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A temporary project with the `.ai/` tree `install` requires.
fn project() -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().to_path_buf();
    fs::create_dir_all(root.join(".ai")).expect("create .ai");
    (outer, root)
}

/// A temporary project with no `.ai/` tree.
fn bare_project() -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().to_path_buf();
    (outer, root)
}

/// Runs the binary in `root`.
fn run_in(root: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_aicontext"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("the binary runs");
    Run {
        code: output.status.code().expect("the process exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The one plugin every test installs, with one read tool, one read path, and one auth reference.
const MANIFEST: &str = r#"{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "name": "demo",
  "version": "1.0.0",
  "description": "A demo plugin",
  "entry": { "command": "demo-plugin", "args": ["serve"] },
  "capabilities": [
    { "tool": "demo.items.read", "effect": "read" }
  ],
  "auth": {
    "type": "env_or_credential_command",
    "references": [{ "name": "DEMO_TOKEN", "source": "env" }],
    "instructions": "set DEMO_TOKEN"
  },
  "filesystem": { "read": ["docs"], "write": [] },
  "network": { "egress": [] },
  "limits": { "call_timeout_ms": 5000, "max_output_bytes": 1048576 }
}"#;

/// The same plugin at a new version, requesting a write tool instead of the read one.
const MANIFEST_UPGRADE: &str = r#"{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "name": "demo",
  "version": "1.1.0",
  "description": "A demo plugin",
  "entry": { "command": "demo-plugin", "args": ["serve"] },
  "capabilities": [
    { "tool": "demo.items.write", "effect": "write" }
  ],
  "filesystem": { "read": ["docs"], "write": [] },
  "network": { "egress": [] },
  "limits": { "call_timeout_ms": 5000, "max_output_bytes": 1048576 }
}"#;

/// A third-party MCP server: the manifest is the registration, and the tool name the model sees is
/// mapped onto the name the server lists.
const MCP_MANIFEST: &str = r#"{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "protocol": "mcp",
  "name": "demo",
  "version": "1.0.0",
  "description": "A third-party MCP server",
  "entry": {
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "."]
  },
  "capabilities": [
    { "tool": "demo.files.read", "effect": "read", "mcp_tool": "read_file" }
  ],
  "filesystem": { "read": ["."], "write": [] },
  "network": { "egress": [] },
  "limits": { "call_timeout_ms": 30000, "max_output_bytes": 1048576 }
}"#;

/// Writes `plugins/demo/manifest.json` under `root` and returns the relative reference to install.
fn write_plugin(root: &Path, manifest: &str) -> String {
    let directory = root.join("plugins").join("demo");
    fs::create_dir_all(&directory).expect("create plugin directory");
    fs::write(directory.join("manifest.json"), manifest).expect("write manifest");
    "plugins/demo".to_owned()
}

/// The lockfile as parsed JSON.
fn lockfile(root: &Path) -> serde_json::Value {
    let text =
        fs::read_to_string(root.join(".aicontext/plugins.lock.json")).expect("read lockfile");
    serde_json::from_str(&text).expect("lockfile is JSON")
}

#[test]
fn install_without_yes_refuses_when_no_terminal_can_answer() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);

    let run = run_in(&root, &["plugin", "install", &reference]);

    assert_eq!(run.code, 5, "an unconfirmed install is an approval failure");
    assert!(
        run.stderr.contains("PLUGIN-004"),
        "the refusal must carry its code: {}",
        run.stderr
    );
    assert!(
        !root.join(".aicontext").exists(),
        "a refusal must write no state"
    );
}

#[test]
fn install_prints_the_permission_diff_and_auth_before_asking() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);

    let run = run_in(&root, &["plugin", "install", &reference]);

    // The preview is printed before the prompt, so a human can see what they are approving even
    // when no human is there to answer.
    assert!(run.stdout.contains("plugin `demo` 1.0.0"), "{}", run.stdout);
    assert!(
        run.stdout.contains("signature   unsigned"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("permissions (3 added, 0 removed, 0 unchanged)"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("+ tool `demo.items.read` (read)"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("+ filesystem read `docs`"),
        "{}",
        run.stdout
    );
    assert!(run.stdout.contains("auth requirements:"), "{}", run.stdout);
    assert!(run.stdout.contains("DEMO_TOKEN (env)"), "{}", run.stdout);
    assert!(run.stdout.contains("set DEMO_TOKEN"), "{}", run.stdout);
}

#[test]
fn install_with_yes_writes_the_manifest_copy_and_a_sorted_lockfile() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);

    let run = run_in(&root, &["plugin", "install", &reference, "--yes"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    let copy = root.join(".aicontext/plugins/demo/manifest.json");
    assert!(copy.is_file(), "the validated bytes are kept");
    assert_eq!(fs::read_to_string(&copy).expect("read copy"), MANIFEST);

    let record = lockfile(&root);
    assert_eq!(record["lockfile_version"], 1);
    assert_eq!(record["plugins"][0]["name"], "demo");
    assert_eq!(record["plugins"][0]["version"], "1.0.0");
    assert_eq!(record["plugins"][0]["signature"], "unsigned");
    let permissions = record["plugins"][0]["permissions"]
        .as_array()
        .expect("permissions is an array");
    assert_eq!(permissions.len(), 3);
    assert_eq!(permissions[0]["kind"], "tool");
    assert_eq!(permissions[0]["tool"], "demo.items.read");
    assert_eq!(permissions[0]["effect"], "read");
    assert_eq!(permissions[1]["kind"], "filesystem");
    assert_eq!(permissions[1]["access"], "read");
    assert_eq!(permissions[1]["path"], "docs");
    assert_eq!(permissions[2]["kind"], "auth");
    assert_eq!(permissions[2]["name"], "DEMO_TOKEN");
}

#[test]
fn a_reinstall_reports_the_permission_it_adds_and_the_one_it_revokes() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);
    let first = run_in(&root, &["plugin", "install", &reference, "--yes"]);
    assert_eq!(first.code, 0, "{}", first.stderr);

    write_plugin(&root, MANIFEST_UPGRADE);
    let second = run_in(&root, &["plugin", "install", &reference, "--yes", "--json"]);

    assert_eq!(second.code, 0, "{}", second.stderr);
    let body: serde_json::Value = serde_json::from_str(&second.stdout).expect("one JSON object");
    assert_eq!(body["command"], "plugin");
    assert_eq!(body["data"]["action"], "install");
    assert_eq!(body["data"]["plugin"]["version"], "1.1.0");
    assert_eq!(
        body["data"]["diff"]["added"]
            .as_array()
            .expect("added")
            .len(),
        1
    );
    assert_eq!(
        body["data"]["diff"]["removed"]
            .as_array()
            .expect("removed")
            .len(),
        2,
        "the read tool and the auth reference are both revoked"
    );
    assert_eq!(body["data"]["diff"]["added"][0]["tool"], "demo.items.write");
    assert_eq!(
        body["data"]["diff"]["removed"][0]["tool"],
        "demo.items.read"
    );
    assert_eq!(body["data"]["diff"]["removed"][1]["kind"], "auth");

    let record = lockfile(&root);
    assert_eq!(record["plugins"][0]["version"], "1.1.0");
    assert_eq!(
        record["plugins"][0]["permissions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn remove_revokes_the_permissions_and_deletes_the_state_it_owned() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);
    assert_eq!(
        run_in(&root, &["plugin", "install", &reference, "--yes"]).code,
        0
    );

    let run = run_in(&root, &["plugin", "remove", "demo", "--json"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["data"]["action"], "remove");
    assert_eq!(body["data"]["plugin"]["name"], "demo");
    assert_eq!(
        body["data"]["revoked"].as_array().expect("revoked").len(),
        3
    );
    assert_eq!(body["data"]["state_removed"], true);
    assert_eq!(body["data"]["remaining"], 0);
    assert!(
        !root.join(".aicontext/plugins/demo").exists(),
        "the state tree is deleted"
    );
    assert!(
        !root.join(".aicontext/plugins.lock.json").exists(),
        "the record is deleted when the last plugin goes"
    );
}

#[test]
fn removing_a_plugin_that_is_not_installed_is_a_named_usage_error() {
    let (_outer, root) = project();

    let run = run_in(&root, &["plugin", "remove", "ghost"]);

    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("PLUGIN-010"), "{}", run.stderr);
    assert!(run.stderr.contains("ghost"), "{}", run.stderr);
}

#[test]
fn list_without_a_lockfile_reports_an_empty_record() {
    let (_outer, root) = project();

    let run = run_in(&root, &["plugin", "list", "--json"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["command"], "plugin");
    assert_eq!(body["data"]["action"], "list");
    assert_eq!(body["data"]["lockfile"], ".aicontext/plugins.lock.json");
    assert_eq!(
        body["data"]["plugins"].as_array().expect("plugins").len(),
        0
    );
}

#[test]
fn list_shows_the_installed_plugin_and_its_permissions() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);
    assert_eq!(
        run_in(&root, &["plugin", "install", &reference, "--yes"]).code,
        0
    );

    let run = run_in(&root, &["plugin", "list", "--json"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    let plugins = body["data"]["plugins"].as_array().expect("plugins");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0]["name"], "demo");
    assert_eq!(plugins[0]["signature"], "unsigned");
    assert_eq!(
        plugins[0]["permissions"]
            .as_array()
            .expect("permissions")
            .len(),
        3
    );
}

#[test]
fn a_malformed_lockfile_is_refused_rather_than_treated_as_empty() {
    let (_outer, root) = project();
    fs::create_dir_all(root.join(".aicontext")).expect("create .aicontext");
    fs::write(root.join(".aicontext/plugins.lock.json"), "not json").expect("write");

    let run = run_in(&root, &["plugin", "list"]);

    assert_eq!(run.code, 3, "a malformed record is a validation failure");
    assert!(run.stderr.contains("PLUGIN-006"), "{}", run.stderr);
}

#[test]
fn an_unknown_permission_kind_in_the_lockfile_is_refused() {
    let (_outer, root) = project();
    fs::create_dir_all(root.join(".aicontext")).expect("create .aicontext");
    let record = format!(
        r#"{{
  "lockfile_version": 1,
  "plugins": [
    {{
      "name": "demo",
      "version": "1.0.0",
      "plugin_api_version": "1",
      "manifest_digest": "{}",
      "source": "plugins/demo",
      "signature": "unsigned",
      "permissions": [{{ "kind": "kernel", "module": "vmlinux" }}]
    }}
  ]
}}"#,
        "a".repeat(64)
    );
    fs::write(root.join(".aicontext/plugins.lock.json"), record).expect("write");

    let run = run_in(&root, &["plugin", "list"]);

    assert_eq!(run.code, 3);
    assert!(run.stderr.contains("PLUGIN-006"), "{}", run.stderr);
    assert!(
        run.stderr.contains("kernel"),
        "the unknown kind must be named: {}",
        run.stderr
    );
}

#[test]
fn an_unknown_permission_in_the_manifest_is_refused() {
    let (_outer, root) = project();
    let manifest = MANIFEST.replace("\"effect\": \"read\"", "\"effect\": \"delete\"");
    let reference = write_plugin(&root, &manifest);

    let run = run_in(&root, &["plugin", "install", &reference, "--yes"]);

    assert_eq!(run.code, 3, "an invalid manifest is a validation failure");
    assert!(run.stderr.contains("PLG-011"), "{}", run.stderr);
    assert!(
        !root.join(".aicontext").exists(),
        "a refused manifest writes nothing"
    );
}

#[test]
fn a_registry_reference_is_refused_by_name() {
    let (_outer, root) = project();

    let run = run_in(&root, &["plugin", "install", "demo", "--yes"]);

    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("PLUGIN-003"), "{}", run.stderr);
    assert!(run.stderr.contains("registry reference"), "{}", run.stderr);
}

#[test]
fn a_missing_local_manifest_is_refused_by_name() {
    let (_outer, root) = project();

    let run = run_in(&root, &["plugin", "install", "plugins/missing", "--yes"]);

    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("PLUGIN-002"), "{}", run.stderr);
}

#[test]
fn install_requires_an_ai_tree() {
    let (_outer, root) = bare_project();
    let reference = write_plugin(&root, MANIFEST);

    let run = run_in(&root, &["plugin", "install", &reference, "--yes"]);

    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("PROJ-001"), "{}", run.stderr);
    assert!(!root.join(".aicontext").exists());
}

#[test]
fn the_json_install_shape_names_the_lockfile_the_plugin_and_the_diff() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);

    let run = run_in(&root, &["plugin", "install", &reference, "--yes", "--json"]);

    assert_eq!(run.code, 0, "{}", run.stderr);
    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["command"], "plugin");
    assert_eq!(body["ok"], true);
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["data"]["action"], "install");
    assert_eq!(body["data"]["lockfile"], ".aicontext/plugins.lock.json");
    assert_eq!(body["data"]["plugin"]["name"], "demo");
    assert_eq!(body["data"]["plugin"]["signature"], "unsigned");
    assert_eq!(
        body["data"]["diff"]["added"]
            .as_array()
            .expect("added")
            .len(),
        3
    );
    assert_eq!(
        body["data"]["diff"]["removed"]
            .as_array()
            .expect("removed")
            .len(),
        0
    );
    assert!(
        run.stderr.contains("permissions (3 added"),
        "under --json the human preview still reaches stderr: {}",
        run.stderr
    );
}

#[test]
fn an_mcp_server_is_registered_with_the_plugin_flag_and_records_its_protocol() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MCP_MANIFEST);

    let run = run_in(
        &root,
        &[
            "plugin",
            "install",
            &reference,
            "--as-plugin",
            "--yes",
            "--json",
        ],
    );

    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(
        run.stderr.contains("protocol    mcp"),
        "the preview names the protocol before the prompt: {}",
        run.stderr
    );

    let record = lockfile(&root);
    assert_eq!(record["plugins"][0]["protocol"], "mcp");
    assert_eq!(
        record["plugins"][0]["permissions"][0]["tool"],
        "demo.files.read"
    );

    let list = run_in(&root, &["plugin", "list", "--json"]);
    assert_eq!(list.code, 0, "{}", list.stderr);
    let body: serde_json::Value = serde_json::from_str(&list.stdout).expect("one JSON object");
    assert_eq!(body["data"]["plugins"][0]["protocol"], "mcp");

    let human = run_in(&root, &["plugin", "list"]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        human.stdout.contains("mcp"),
        "the human list names the protocol: {}",
        human.stdout
    );
}

#[test]
fn an_mcp_manifest_without_the_plugin_flag_is_plugin_012() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MCP_MANIFEST);

    let run = run_in(&root, &["plugin", "install", &reference, "--yes"]);

    assert_eq!(run.code, 2, "a protocol mismatch is a usage error");
    assert!(run.stderr.contains("PLUGIN-012"), "{}", run.stderr);
    assert!(run.stderr.contains("--as-plugin"), "{}", run.stderr);
    assert!(
        !root.join(".aicontext").exists(),
        "a refusal must write no state"
    );
}

#[test]
fn the_plugin_flag_on_a_native_manifest_is_plugin_012() {
    let (_outer, root) = project();
    let reference = write_plugin(&root, MANIFEST);

    let run = run_in(
        &root,
        &["plugin", "install", &reference, "--as-plugin", "--yes"],
    );

    assert_eq!(run.code, 2, "a protocol mismatch is a usage error");
    assert!(run.stderr.contains("PLUGIN-012"), "{}", run.stderr);
    assert!(run.stderr.contains("aiplugin"), "{}", run.stderr);
    assert!(
        !root.join(".aicontext").exists(),
        "a refusal must write no state"
    );
}
