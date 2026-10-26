//! End-to-end tests for `aicontext agent run` in plan mode (`TASK-055`).
//!
//! These run the binary against a real localhost socket, because what a unit test behind the planner
//! port cannot show is exactly what plan mode promises: one real call to the configured provider,
//! a plan written under `.aicontext/plans/`, and nothing else in the tree touched.
//!
//! Every success path is `auth: none`, so no test here can reach the developer's OS keychain
//! (`RULES.md` §8), and a local or free provider is enough to run the command at all.

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

/// A chat completion whose content is a schema-valid `action_proposal`.
const PLAN: &str = r#"{"id":"cmpl-fake","object":"chat.completion","model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"{\"kind\":\"action_proposal\",\"version\":1,\"actions\":[{\"type\":\"plan\",\"summary\":\"Update the register and stop.\",\"files_modify\":[\".ai/TASKS.md\"],\"files_create\":[],\"files_delete\":[],\"risks\":[\"the instance may be in use\"]}]}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1200,"completion_tokens":180,"total_tokens":1380}}"#;

/// A chat completion claiming to be a proposal at a version this build does not know.
const MALFORMED: &str = r#"{"id":"cmpl-fake","object":"chat.completion","model":"test-model","choices":[{"index":0,"message":{"role":"assistant","content":"{\"kind\":\"action_proposal\",\"version\":2,\"actions\":[]}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
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

/// A temporary project with the `.ai/` tree `agent run` assembles from.
fn project() -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().to_path_buf();
    fs::create_dir_all(root.join(".ai")).expect("create .ai");
    fs::write(root.join(".ai/AI.md"), "# AI\n\nThe entry point.\n").expect("write AI.md");
    fs::write(root.join(".ai/RULES.md"), "# Rules\n\nBe correct.\n").expect("write RULES.md");
    fs::write(
        root.join(".ai/TASKS.md"),
        "# Tasks\n\n**Current phase:** Phase 1\n**Current task:** TASK-001\n\n\
         ### TASK-001 — Write the fixture plan\n\n```yaml\nid: TASK-001\n\
         title: Write the fixture plan\nstatus: TODO\npriority: HIGH\n\
         touches: [\"crates/example/**\"]\n```\n",
    )
    .expect("write TASKS.md");
    (outer, root)
}

/// The profile of a free local provider pointed at `base_url`, with no credential of any kind.
fn profile_yaml(base_url: &str) -> String {
    format!(
        "version: 1\ndefault_provider: local\nproviders:\n  - id: local\n    \
         adapter: openai-compatible\n    base_url: {base_url}/v1\n    model: test-model\n    \
         auth:\n      type: none\n"
    )
}

/// The same profile with list prices, and optionally a per-run ceiling.
fn priced_profile_yaml(base_url: &str, ceiling: Option<&str>) -> String {
    let budget = ceiling.map_or_else(String::new, |ceiling| {
        format!("budget:\n  max_run_usd: {ceiling}\n")
    });
    format!(
        "{budget}version: 1\ndefault_provider: local\nproviders:\n  - id: local\n    \
         adapter: openai-compatible\n    base_url: {base_url}/v1\n    model: test-model\n    \
         auth:\n      type: none\n    pricing:\n      input_per_mtok: 2.5\n      \
         output_per_mtok: 10\n"
    )
}

/// Writes the profile directly, for the tests whose subject is not `connect`.
fn write_profile(root: &Path, base_url: &str) {
    fs::write(root.join(".ai/ai.yaml"), profile_yaml(base_url)).expect("write ai.yaml");
}

/// Every file under `root`, as `/`-separated relative paths, for before/after comparison.
fn files_under(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, found: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, found);
            } else {
                let relative = path.strip_prefix(base).expect("under the base");
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut found = Vec::new();
    walk(root, root, &mut found);
    found.sort();
    found
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
            "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
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

/// A port nothing is listening on: bound, read, and released.
fn dead_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let address = listener.local_addr().expect("a bound listener");
    drop(listener);
    format!("http://{address}")
}

#[test]
fn connect_then_agent_run_plans_the_task_and_touches_nothing_else() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);

    // `connect` proves the provider and writes the profile; `agent run` reads it back.
    let mut connect = vec![
        "connect",
        "--provider",
        "local",
        "--base-url",
        &server.url,
        "--model",
        "test-model",
    ];
    connect.push("--json");
    let connected = run_in(&root, &connect);
    assert_eq!(connected.code, 0, "stderr: {}", connected.stderr);
    let profile_before = fs::read_to_string(root.join(".ai/ai.yaml")).expect("the profile");
    let tree_before = files_under(&root);

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001", "--json"]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["command"], "agent");
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["task"], "TASK-001");
    assert_eq!(body["data"]["task_title"], "Write the fixture plan");
    assert_eq!(body["data"]["structured"], true, "a schema-valid proposal");
    assert_eq!(body["data"]["dry_run"], false);
    assert_eq!(body["data"]["truncated"], false);
    assert_eq!(body["data"]["usage"]["input_tokens"], 1200);
    assert_eq!(body["data"]["usage"]["output_tokens"], 180);
    assert_eq!(body["data"]["usage"]["estimated"], false);
    let plan = body["data"]["plan"].as_str().expect("a plan path");
    assert!(plan.ends_with(".aicontext/plans/TASK-001.md"), "{plan}");

    // The plan file is written, under the directory `init` git-ignores.
    let document = fs::read_to_string(root.join(".aicontext/plans/TASK-001.md"))
        .expect("the plan was written");
    assert!(document.contains("# Plan — TASK-001: Write the fixture plan"));
    assert!(document.contains("## Files to modify"));
    assert!(document.contains("- `.ai/TASKS.md`"));
    assert!(document.contains("in 1200, out 180, cached 0 — provider-reported"));

    // One real call: the assembled context went out with the output ceiling and the task in the
    // instruction region.
    let requests = server.seen();
    assert_eq!(requests.len(), 2, "one connect verification, one plan call");
    let request = &requests[1];
    assert!(
        request.contains("\"max_tokens\":4000"),
        "the plan ceiling is part of the request: {request}"
    );
    assert!(request.contains("TASK-001"), "{request}");
    assert!(request.contains("plan mode"), "{request}");
    assert!(
        request.contains("Be correct."),
        "RULES.md is mandatory policy: {request}"
    );

    // The working tree outside `.aicontext/` is byte-for-byte what it was.
    let tree_after = files_under(&root);
    let new_files: Vec<&String> = tree_after
        .iter()
        .filter(|file| !tree_before.contains(file))
        .collect();
    assert!(!new_files.is_empty(), "the plan itself is new");
    for file in new_files {
        assert!(
            file.starts_with(".aicontext/"),
            "a run wrote outside `.aicontext/`: {file}"
        );
    }
    assert!(
        fs::read_to_string(root.join(".ai/ai.yaml")).expect("the profile") == profile_before,
        "the profile is read, never rewritten"
    );
}

#[test]
fn a_priced_run_reports_its_cost_in_the_json_and_the_plan() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    fs::write(
        root.join(".ai/ai.yaml"),
        priced_profile_yaml(&server.url, None),
    )
    .expect("write ai.yaml");

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001", "--json"]);

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["data"]["usage"]["input_tokens"], 1200);
    assert_eq!(
        body["data"]["cost_usd"], "0.004800",
        "1,200 input at $2.50/M plus 180 output at $10/M"
    );
    assert_eq!(body["data"]["budget_usd"], Value::Null);
    assert_eq!(body["data"]["projected_cost_usd"], Value::Null);
    assert_eq!(
        server.seen().len(),
        1,
        "one plan call, no verification call"
    );

    let document = fs::read_to_string(root.join(".aicontext/plans/TASK-001.md"))
        .expect("the plan was written");
    assert!(
        document.contains("cost $0.004800"),
        "the plan records its own cost: {document}"
    );
}

#[test]
fn a_per_run_ceiling_refuses_the_call_before_any_spend() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    fs::write(
        root.join(".ai/ai.yaml"),
        priced_profile_yaml(&server.url, Some("0.000001")),
    )
    .expect("write ai.yaml");

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001"]);

    assert_eq!(run.code, 3, "stderr: {}", run.stderr);
    assert!(run.stderr.contains("AGENT-012"), "{}", run.stderr);
    assert!(
        run.stderr.contains("no call was made"),
        "the refusal is before the provider is touched: {}",
        run.stderr
    );
    assert!(
        server.seen().is_empty(),
        "an over-budget request is refused before it reaches the socket"
    );
    assert!(!root.join(".aicontext/plans/TASK-001.md").exists());
}

#[test]
fn a_ceiling_without_prices_exits_three_and_makes_no_call() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    let profile = format!(
        "budget:\n  max_run_usd: 1.00\n{}",
        profile_yaml(&server.url)
    );
    fs::write(root.join(".ai/ai.yaml"), profile).expect("write ai.yaml");

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001"]);

    assert_eq!(run.code, 3, "stderr: {}", run.stderr);
    assert!(run.stderr.contains("AGENT-011"), "{}", run.stderr);
    assert!(
        run.stderr.contains("pricing"),
        "the missing block is named: {}",
        run.stderr
    );
    assert!(server.seen().is_empty());
}

#[test]
fn a_dry_run_reports_the_projected_cost_and_the_ceiling() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    fs::write(
        root.join(".ai/ai.yaml"),
        priced_profile_yaml(&server.url, Some("1.00")),
    )
    .expect("write ai.yaml");

    let run = run_in(
        &root,
        &["agent", "run", "--task", "TASK-001", "--dry-run", "--json"],
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(server.seen().is_empty(), "a dry run makes no call");
    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["data"]["dry_run"], true);
    assert_eq!(body["data"]["usage"], Value::Null);
    assert_eq!(body["data"]["cost_usd"], Value::Null);
    assert_eq!(body["data"]["budget_usd"], "1.000000");
    assert!(
        body["data"]["projected_cost_usd"].is_string(),
        "the projection is a priced estimate: {}",
        body["data"]["projected_cost_usd"]
    );
}

#[test]
fn an_actual_cost_over_the_ceiling_exits_three_after_writing_the_plan() {
    let (_outer, root) = project();
    let body = PLAN.replace(r#""prompt_tokens":1200"#, r#""prompt_tokens":10000000"#);
    let server = Server::start(200, &body);
    fs::write(
        root.join(".ai/ai.yaml"),
        priced_profile_yaml(&server.url, Some("0.10")),
    )
    .expect("write ai.yaml");

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001", "--json"]);

    assert_eq!(run.code, 3, "stderr: {}", run.stderr);
    let result: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(result["ok"], false);
    assert_eq!(result["exit_code"], 3);
    assert_eq!(
        result["data"]["cost_usd"], "25.001800",
        "10,000,000 input at $2.50/M plus 180 output at $10/M"
    );
    assert_eq!(result["findings"][0]["code"], "AGENT-013");
    assert_eq!(result["findings"][0]["severity"], "error");
    assert!(
        root.join(".aicontext/plans/TASK-001.md").exists(),
        "the spend happened; the plan it bought is still worth writing"
    );
}

#[test]
fn a_provider_that_reports_no_usage_leaves_the_ceiling_unverified() {
    let (_outer, root) = project();
    let body = PLAN.replace(
        r#","usage":{"prompt_tokens":1200,"completion_tokens":180,"total_tokens":1380}"#,
        "",
    );
    let server = Server::start(200, &body);
    fs::write(
        root.join(".ai/ai.yaml"),
        priced_profile_yaml(&server.url, Some("1.00")),
    )
    .expect("write ai.yaml");

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001", "--json"]);

    assert_eq!(run.code, 0, "a warning is not a failure: {}", run.stderr);
    let result: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(result["data"]["usage"]["input_tokens"], Value::Null);
    assert_eq!(result["data"]["cost_usd"], Value::Null);
    assert_eq!(result["findings"][0]["code"], "AGENT-014");
    assert_eq!(result["findings"][0]["severity"], "warning");
    assert!(root.join(".aicontext/plans/TASK-001.md").exists());
}

#[test]
fn a_provider_that_cannot_be_reached_exits_six_and_writes_no_plan() {
    let (_outer, root) = project();
    write_profile(&root, &dead_url());

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001"]);

    assert_eq!(
        run.code, 6,
        "an unreachable provider is the provider's failure: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("PROVIDER_"),
        "the provider layer's code, not a new one: {}",
        run.stderr
    );
    assert!(
        !root.join(".aicontext/plans/TASK-001.md").exists(),
        "a run that never heard back must not write a plan"
    );
}

#[test]
fn a_dry_run_makes_no_call_and_writes_nothing() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    write_profile(&root, &server.url);
    let tree_before = files_under(&root);

    let run = run_in(
        &root,
        &["agent", "run", "--task", "TASK-001", "--dry-run", "--json"],
    );

    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        server.seen().is_empty(),
        "a dry run assembles and reports; it makes no call"
    );
    // The scan may warm the index cache, which lives under the git-ignored `.aicontext/` like the
    // plan would; the working tree itself must be untouched.
    let tree_after = files_under(&root);
    let written: Vec<&String> = tree_after
        .iter()
        .filter(|file| !tree_before.contains(file))
        .collect();
    for file in written {
        assert!(
            file.starts_with(".aicontext/"),
            "a dry run wrote outside `.aicontext/`: {file}"
        );
    }
    assert!(
        !root.join(".aicontext/plans").exists(),
        "a dry run writes no plan"
    );
    let body: Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["data"]["dry_run"], true);
    assert_eq!(body["data"]["plan"], serde_json::Value::Null);
    assert_eq!(body["data"]["usage"], serde_json::Value::Null);
}

#[test]
fn an_unknown_task_exits_two_before_any_call() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    write_profile(&root, &server.url);

    let run = run_in(&root, &["agent", "run", "--task", "TASK-999"]);

    assert_eq!(run.code, 2, "{}", run.stderr);
    assert!(run.stderr.contains("AGENT-002"), "{}", run.stderr);
    assert!(
        run.stderr.contains("TASK-001"),
        "the register is named: {}",
        run.stderr
    );
    assert!(
        server.seen().is_empty(),
        "the task is checked before anything is assembled or called"
    );
    assert!(!root.join(".aicontext/plans/TASK-999.md").exists());
}

#[test]
fn execution_mode_is_not_accepted_rather_than_ignored() {
    let (_outer, root) = project();
    let server = Server::start(200, PLAN);
    write_profile(&root, &server.url);

    for flag in ["--execute", "--allow"] {
        let run = run_in(&root, &["agent", "run", "--task", "TASK-001", flag]);
        assert_eq!(
            run.code, 2,
            "{flag} must be a usage error, not a silently ignored flag: {}",
            run.stderr
        );
    }
    assert!(
        server.seen().is_empty(),
        "a refused command opens no socket"
    );
}

#[test]
fn a_malformed_proposal_exits_three_and_writes_no_plan() {
    let (_outer, root) = project();
    let server = Server::start(200, MALFORMED);
    write_profile(&root, &server.url);

    let run = run_in(&root, &["agent", "run", "--task", "TASK-001"]);

    assert_eq!(run.code, 3, "{}", run.stderr);
    assert!(run.stderr.contains("AGENT-005"), "{}", run.stderr);
    assert!(
        run.stderr.contains("version"),
        "the problem is named: {}",
        run.stderr
    );
    assert!(
        !root.join(".aicontext/plans/TASK-001.md").exists(),
        "a proposal that fails validation is reported, not written"
    );
}

#[test]
fn a_bare_agent_and_an_unknown_subcommand_are_usage_errors() {
    let (_outer, root) = project();

    for args in [
        vec!["agent"],
        vec!["agent", "list"],
        vec!["agent", "plan", "--task", "TASK-001"],
    ] {
        let run = run_in(&root, &args);
        assert_eq!(
            run.code, 2,
            "{args:?} must be a usage error: {}",
            run.stderr
        );
    }
}
