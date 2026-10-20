//! End-to-end tests for `aicontext connect` and `aicontext disconnect` (`TASK-053`).
//!
//! These run the binary against a real localhost socket, because the two things a unit test behind
//! the verifier port cannot show are exactly what `connect` is for: that it makes one real call to a
//! real address, and that a failure on that call is what stops it writing anything.
//!
//! Every success path here is `auth: none`, which stores no credential, so no test in this file can
//! reach the developer's OS keychain (`RULES.md` §8). The paths that *would* store one run behind the
//! keychain port in `src/connect/`, which is the only place a keychain can be faked.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;
use tempfile::TempDir;

/// What a provider answers when the verification call is legitimate.
const OK: &str = r#"{"id":"cmpl-fake","object":"chat.completion","model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":1,"total_tokens":4}}"#;

/// The error body a provider answers an unauthenticated call with.
const REFUSED: &str = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#;

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A temporary project with an `.ai/` tree, which `connect` requires before it will spend a call.
fn project() -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().to_path_buf();
    fs::create_dir_all(root.join(".ai")).expect("create .ai");
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

/// The `connect` flags every test here needs, minus the credential (see the module docs).
fn connect_args(base_url: &str) -> Vec<&str> {
    vec![
        "connect",
        "--provider",
        "local",
        "--base-url",
        base_url,
        "--model",
        "test-model",
    ]
}

/// A fake provider on an ephemeral localhost port.
///
/// `std`-only on purpose: the test needs a socket, not a web framework, and the point of the test is
/// that the binary under test makes a real HTTP call without leaving the machine.
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts a server that answers every request with `status` and `body`.
    fn start(status: u16, body: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        listener.set_nonblocking(true).expect("non-blocking");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len(),
            reason = reason(status)
        );

        let handle = {
            let seen = Arc::clone(&seen);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => serve(stream, &seen, &response),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            url: format!("http://{address}"),
            seen,
            stop,
            handle: Some(handle),
        }
    }

    /// The request bodies received so far.
    fn seen(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("the request log is not poisoned")
            .clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Reads one HTTP/1.1 request off `stream`, records its body, and answers it.
fn serve(mut stream: std::net::TcpStream, seen: &Arc<Mutex<Vec<String>>>, response: &str) {
    // A socket accepted by a non-blocking listener can inherit that mode on Windows, which would
    // make the first read return `WouldBlock` and close the connection.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };

    let mut content_length = 0usize;
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    for line in head.split("\r\n").skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let wanted = header_end + content_length;
    while buffer.len() < wanted {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let body_end = wanted.min(buffer.len());
    let body = String::from_utf8_lossy(&buffer[header_end..body_end]).into_owned();
    seen.lock()
        .expect("the request log is not poisoned")
        .push(body);

    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// The first index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A reason phrase for a status.
fn reason(status: u16) -> &'static str {
    match status {
        200..=299 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        _ => "Status",
    }
}

/// Reads `.ai/ai.yaml` as it stands after a run.
fn profile(root: &Path) -> String {
    fs::read_to_string(root.join(".ai/ai.yaml")).expect("the profile was written")
}

/// Checks a written profile against the schema other tools read it by.
///
/// This is the assertion that makes "stores a provider profile" mean something: the file is not just
/// present, it is the shape `schemas/ai-config.schema.json` commits to, so a later reader that
/// trusts the schema trusts what `connect` wrote.
fn assert_matches_schema(text: &str) {
    let instance: Value = yaml_serde::from_str(text).expect("the profile is valid YAML");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/ai-config.schema.json");
    let source = fs::read_to_string(path).expect("the ai-config schema is in the repository");
    let schema: Value = serde_json::from_str(&source).expect("the schema is valid JSON");
    let validator = jsonschema::validator_for(&schema).expect("the schema compiles");
    if let Err(error) = validator.validate(&instance) {
        panic!("connect wrote a profile the schema rejects: {error}");
    }
}

/// Whether a JSON flag in the payload is `true`.
fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or(false)
}

#[test]
fn connect_proves_the_provider_then_writes_a_schema_valid_profile() {
    let (_outer, root) = project();
    let server = Server::start(200, OK);

    let run = run_in(&root, &connect_args(&server.url));
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    let requests = server.seen();
    assert_eq!(requests.len(), 1, "one minimal call, exactly once");
    let request = &requests[0];
    assert!(
        request.contains("Reply with the single word"),
        "the call asks for as little as it can: {request}"
    );
    assert!(
        request.contains("\"max_tokens\":8"),
        "the ceiling is part of asking for little: {request}"
    );
    assert!(
        !request.contains("\"stream\":true"),
        "a verification call must not open a stream: {request}"
    );

    let text = profile(&root);
    assert!(text.contains("id: local"), "{text}");
    assert!(text.contains("adapter: openai-compatible"), "{text}");
    assert!(text.contains("model: test-model"), "{text}");
    assert!(text.contains("type: none"), "{text}");
    assert!(text.contains("default_provider: local"), "{text}");
    assert!(
        !text.contains("keychain_entry"),
        "no credential was given, so no reference is recorded: {text}"
    );
    assert_matches_schema(&text);
}

#[test]
fn connect_reports_the_profile_as_one_json_object() {
    let (_outer, root) = project();
    let server = Server::start(200, OK);

    let mut args = connect_args(&server.url);
    args.push("--json");
    let run = run_in(&root, &args);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);

    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["command"], "connect");
    assert_eq!(body["exit_code"], 0);
    assert!(flag(&body["ok"]), "{}", body);
    assert_eq!(body["data"]["provider"], "local");
    assert_eq!(body["data"]["adapter"], "openai-compatible");
    assert_eq!(body["data"]["auth"], "none");
    assert!(!flag(&body["data"]["keychain_stored"]), "{}", body);
    assert!(flag(&body["data"]["verified"]), "{}", body);
    assert!(
        body["data"]["config_path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".ai/ai.yaml")),
        "{}",
        body["data"]["config_path"]
    );

    // The payload's shape is a contract a script branches on, so it is listed rather than sampled:
    // a field appearing without a decision to add it is a change no test would have noticed.
    let mut keys: Vec<&str> = body["data"]
        .as_object()
        .expect("data is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "adapter",
            "auth",
            "base_url",
            "config_path",
            "env",
            "keychain_entry",
            "keychain_stored",
            "model",
            "provider",
            "verified",
        ]
    );
}

#[test]
fn a_provider_that_rejects_the_call_exits_six_and_writes_nothing() {
    let (_outer, root) = project();
    let server = Server::start(401, REFUSED);

    let run = run_in(&root, &connect_args(&server.url));

    assert_eq!(
        run.code, 6,
        "an unproven provider is the provider's failure: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("PROVIDER_UNAUTHORIZED"),
        "{}",
        run.stderr
    );
    assert!(
        !root.join(".ai/ai.yaml").exists(),
        "a profile that never answered must not be committed"
    );
    assert!(!server.seen().is_empty(), "the call was attempted");
    assert!(
        run.stdout.is_empty(),
        "the report belongs on stderr: {}",
        run.stdout
    );
}

#[test]
fn offline_refuses_before_opening_a_socket_or_writing_a_file() {
    let (_outer, root) = project();
    let server = Server::start(200, OK);

    let mut args = connect_args(&server.url);
    args.push("--offline");
    let run = run_in(&root, &args);

    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("CONNECT-008"), "{}", run.stderr);
    assert!(
        server.seen().is_empty(),
        "offline has to mean no socket was opened"
    );
    assert!(!root.join(".ai/ai.yaml").exists());
}

#[test]
fn a_project_without_an_ai_tree_is_refused_before_the_call() {
    let outer = TempDir::new().expect("temp dir");
    let server = Server::start(200, OK);

    let run = run_in(outer.path(), &connect_args(&server.url));

    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("PROJ-001"), "{}", run.stderr);
    assert!(run.stderr.contains("aicontext init"), "{}", run.stderr);
    assert!(
        server.seen().is_empty(),
        "nothing is called for a root we cannot write"
    );
}

#[test]
fn connect_without_the_flags_it_needs_is_a_usage_error() {
    let (_outer, root) = project();
    let run = run_in(&root, &["connect", "--provider", "local"]);

    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("--base-url"), "{}", run.stderr);
    assert!(run.stderr.contains("--model"), "{}", run.stderr);
}

#[test]
fn disconnect_removes_the_last_provider_and_the_file_it_lived_in() {
    let (_outer, root) = project();
    let server = Server::start(200, OK);
    let connected = run_in(&root, &connect_args(&server.url));
    assert_eq!(connected.code, 0, "stderr: {}", connected.stderr);

    let run = run_in(&root, &["disconnect", "--provider", "local", "--json"]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        !root.join(".ai/ai.yaml").exists(),
        "`providers: minItems 1` makes an emptied list unschemaable, so the file goes instead"
    );
    assert!(root.join(".ai").is_dir(), "the tree is `init`'s to manage");
    assert_eq!(
        server.seen().len(),
        1,
        "disconnect reads and writes a file; it makes no call"
    );

    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["command"], "disconnect");
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["data"]["provider"], "local");
    assert_eq!(body["data"]["config_deleted"], serde_json::json!(true));
    assert!(!flag(&body["data"]["keychain_deleted"]), "{}", body);
    assert_eq!(body["data"]["remaining_providers"], 0);
}

#[test]
fn disconnect_keeps_the_providers_that_remain_and_a_schema_valid_file() {
    let (_outer, root) = project();
    fs::write(
        root.join(".ai/ai.yaml"),
        "version: 1\ndefault_provider: hosted\nproviders:\n  - id: hosted\n    adapter: openai-compatible\n    auth:\n      type: none\n  - id: local\n    adapter: openai-compatible\n    auth:\n      type: none\n",
    )
    .expect("write the fixture");

    let run = run_in(&root, &["disconnect", "--provider", "local", "--json"]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let text = profile(&root);
    assert!(!text.contains("id: local"), "{text}");
    assert!(text.contains("id: hosted"), "{text}");
    assert!(
        text.contains("default_provider: hosted"),
        "a default naming a provider that is still there must survive: {text}"
    );
    assert_matches_schema(&text);

    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["data"]["config_deleted"], serde_json::json!(false));
    assert_eq!(body["data"]["remaining_providers"], 1);
}

#[test]
fn disconnect_of_an_unknown_provider_exits_two_and_names_the_ones_that_are_there() {
    let (_outer, root) = project();
    fs::write(
        root.join(".ai/ai.yaml"),
        "version: 1\ndefault_provider: local\nproviders:\n  - id: local\n    adapter: openai-compatible\n    auth:\n      type: none\n",
    )
    .expect("write the fixture");

    let run = run_in(&root, &["disconnect", "--provider", "hosted"]);

    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("CONNECT-009"), "{}", run.stderr);
    assert!(run.stderr.contains("local"), "{}", run.stderr);
    assert!(
        profile(&root).contains("id: local"),
        "a mistyped id must not change anything"
    );
}
