//! The plugin's wire loop, driven without a child process.
//!
//! `server::serve` is the same function `main` calls with stdin and stdout. Feeding it a `Cursor`
//! exercises the real framing, handshake, tool list, call, and shutdown while keeping the test
//! deterministic. `tests/gateway.rs` then proves the host starts the real binary and wraps the
//! result in its untrusted envelope.

mod common;

use std::io::Cursor;

use aicontext_plugin_github::client::GitHubClient;
use aicontext_plugin_github::{server, tools};
use common::{MockResponse, MockServer};
use serde_json::{Value, json};

fn client(server: &MockServer) -> GitHubClient {
    GitHubClient::new(Some(&server.base), None).expect("the loopback base is accepted")
}

fn response(lines: &[&str], index: usize) -> Value {
    serde_json::from_str(lines[index]).expect("every stdout line is a JSON-RPC frame")
}

#[test]
fn the_server_handles_the_documented_lifecycle() {
    let github = MockServer::start(|request| {
        assert_eq!(request.path, "/repos/octocat/hello-world");
        MockResponse::json(&json!({
            "full_name": "octocat/hello-world",
            "default_branch": "main",
            "html_url": "https://github.com/octocat/hello-world"
        }))
    });

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"plugin_api_version": "1"}
    });
    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
    let call = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": tools::REPO_READ,
            "arguments": {"owner": "octocat", "repo": "hello-world"}
        }
    });
    let shutdown = json!({"jsonrpc": "2.0", "id": 4, "method": "shutdown", "params": {}});
    let input = format!("{initialize}\n{list}\n{call}\n{shutdown}\n");

    let mut output = Vec::new();
    server::serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &client(&github),
        1024 * 1024,
    )
    .expect("the lifecycle completes");

    let text = String::from_utf8(output).expect("stdout is UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4, "stdout must carry protocol frames only");

    let handshake = response(&lines, 0);
    assert_eq!(handshake["jsonrpc"], "2.0");
    assert_eq!(handshake["id"], 1);
    assert_eq!(handshake["result"]["plugin_api_version"], "1");

    let listed = response(&lines, 1);
    let definitions = listed["result"]["tools"]
        .as_array()
        .expect("tools is an array");
    assert_eq!(definitions.len(), 7);
    let names: Vec<&str> = definitions
        .iter()
        .map(|tool| tool["tool"].as_str().expect("a tool name"))
        .collect();
    assert_eq!(names, tools::TOOL_NAMES);

    let called = response(&lines, 2);
    assert_eq!(called["result"]["is_error"], false);
    assert_eq!(called["result"]["content"][0]["type"], "text");
    let payload: Value = serde_json::from_str(
        called["result"]["content"][0]["text"]
            .as_str()
            .expect("the result is a text block"),
    )
    .expect("the text block is JSON");
    assert_eq!(payload["trust"], "untrusted");
    assert_eq!(payload["repository"]["full_name"], "octocat/hello-world");

    let stopped = response(&lines, 3);
    assert_eq!(stopped["result"], json!({}));

    assert_eq!(github.requests().len(), 1);
}

#[test]
fn tools_call_before_initialize_is_a_jsonrpc_error() {
    let github = MockServer::start(|_| MockResponse::json(&json!({})));
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}});
    let shutdown = json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": {}});
    let input = format!("{list}\n{shutdown}\n");

    let mut output = Vec::new();
    server::serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &client(&github),
        1024 * 1024,
    )
    .expect("the server reports the misuse and continues");

    let text = String::from_utf8(output).expect("stdout is UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    let refused = response(&lines, 0);
    assert_eq!(refused["id"], 1);
    assert_eq!(refused["error"]["code"], -32600);
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("initialize"))
    );
    assert_eq!(response(&lines, 1)["result"], json!({}));
}

#[test]
fn a_malformed_frame_gets_an_error_and_the_stream_continues() {
    let github = MockServer::start(|_| MockResponse::json(&json!({})));
    let shutdown = json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": {}});
    let input = format!("this is not JSON\n{shutdown}\n");

    let mut output = Vec::new();
    server::serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &client(&github),
        1024 * 1024,
    )
    .expect("the malformed frame is a protocol error, not an I/O failure");

    let text = String::from_utf8(output).expect("stdout is UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    let refused = response(&lines, 0);
    assert!(refused["id"].is_null());
    assert_eq!(refused["error"]["code"], -32600);
    assert_eq!(response(&lines, 1)["result"], json!({}));
}

#[test]
fn a_tool_error_is_a_result_not_a_jsonrpc_error() {
    let github = MockServer::start(|_| {
        MockResponse::status(
            403,
            r#"{"message":"API rate limit exceeded for 127.0.0.1."}"#,
        )
        .with_header("x-ratelimit-remaining", "0")
        .with_header("x-ratelimit-limit", "60")
        .with_header("x-ratelimit-reset", "4000000000")
    });

    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"plugin_api_version": "1"}
    });
    let call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tools::REPO_READ,
            "arguments": {"owner": "o", "repo": "r"}
        }
    });
    let shutdown = json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": {}});
    let input = format!("{initialize}\n{call}\n{shutdown}\n");

    let mut output = Vec::new();
    server::serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &client(&github),
        1024 * 1024,
    )
    .expect("the call completes");

    let text = String::from_utf8(output).expect("stdout is UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    let called = response(&lines, 1);
    assert!(
        called.get("error").is_none(),
        "a GitHub failure is a tool result, not a JSON-RPC error"
    );
    assert_eq!(called["result"]["is_error"], true);

    let payload: Value = serde_json::from_str(
        called["result"]["content"][0]["text"]
            .as_str()
            .expect("the error is a text block"),
    )
    .expect("the text block is JSON");
    assert_eq!(payload["trust"], "untrusted");
    assert_eq!(payload["error"]["code"], "github.rate_limited");
    assert!(payload["error"]["remediation"].is_string());
}

#[test]
fn an_unknown_tool_is_a_typed_tool_error() {
    let github = MockServer::start(|_| MockResponse::json(&json!({})));
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"plugin_api_version": "1"}
    });
    let call = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "github.nope.read", "arguments": {}}
    });
    let shutdown = json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": {}});
    let input = format!("{initialize}\n{call}\n{shutdown}\n");

    let mut output = Vec::new();
    server::serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &client(&github),
        1024 * 1024,
    )
    .expect("the call completes");

    let text = String::from_utf8(output).expect("stdout is UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    let called = response(&lines, 1);
    assert_eq!(called["result"]["is_error"], true);
    let payload: Value = serde_json::from_str(
        called["result"]["content"][0]["text"]
            .as_str()
            .expect("the error is a text block"),
    )
    .expect("the text block is JSON");
    assert_eq!(payload["error"]["code"], "github.unknown_tool");
}

#[test]
fn initialize_twice_is_refused() {
    let github = MockServer::start(|_| MockResponse::json(&json!({})));
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"plugin_api_version": "1"}
    });
    let shutdown = json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown", "params": {}});
    let input = format!("{initialize}\n{initialize}\n{shutdown}\n");

    let mut output = Vec::new();
    server::serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &client(&github),
        1024 * 1024,
    )
    .expect("the second handshake is a protocol error");

    let text = String::from_utf8(output).expect("stdout is UTF-8");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(response(&lines, 0)["result"]["plugin_api_version"], "1");
    assert_eq!(response(&lines, 1)["error"]["code"], -32600);
    assert_eq!(response(&lines, 2)["result"], json!({}));
}

#[test]
fn a_frame_over_the_input_cap_is_an_io_error() {
    let github =
        GitHubClient::new(Some("http://127.0.0.1:9"), None).expect("the loopback base is accepted");
    let input = "x".repeat(server::MAX_INPUT_FRAME_BYTES + 1);
    let error = server::serve(
        Cursor::new(input.into_bytes()),
        &mut Vec::new(),
        &github,
        1024 * 1024,
    )
    .expect_err("the frame is over the cap");

    assert_eq!(error.kind(), std::io::ErrorKind::Other);
}
