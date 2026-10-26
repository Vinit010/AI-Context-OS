//! A deliberately misbehaving plugin, driven by one mode argument.
//!
//! This is test support, not part of the library: it lives under `tests/fixtures` and is declared
//! as a `[[bin]]` so Cargo builds it for `tests/supervisor.rs` and exposes its path as
//! `CARGO_BIN_EXE_misbehaving-plugin`. Every mode speaks the real wire protocol from
//! `docs/PLUGIN_SPEC.md` §4; the failure is in how it answers, not in a private test hook.
//!
//! Modes:
//!
//! - `ok` (default): initialize, list one tool, echo the `text` argument, log, shut down.
//! - `crash`: exit with status 3 during `tools/call`.
//! - `hang`: stop answering during `tools/call`.
//! - `hang-handshake`: stop answering during `initialize`.
//! - `flood`: write one 64 KiB result frame, over any test manifest's cap.
//! - `flood-logs`: write 500 log notifications and never answer the call.
//! - `deep`: return a result nested 40 levels deep.
//! - `bad-version`: report plugin API version 2.
//! - `bad-handshake`: reject `initialize` with a JSON-RPC error.
//! - `garbage`: write a line that is not JSON during `tools/list`.
//! - `tool-error`: answer `tools/call` with a JSON-RPC error rather than `is_error`.
//! - `bad-result`: answer `tools/call` with the wrong result shape.
//! - `wrong-id`: answer `tools/call` with a mismatched id.
//! - `no-shutdown`: answer every call but ignore `shutdown`.

#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const TOOL: &str = "fixture.echo.run";

/// What the request loop should do after one frame.
enum Flow {
    Continue,
    Stop,
}

fn main() -> ExitCode {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "ok".to_owned());
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return ExitCode::FAILURE;
        };
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(_) => return ExitCode::from(2),
        };
        match handle(&mode, &request, &mut stdout) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Stop) => return ExitCode::SUCCESS,
            Err(_) => return ExitCode::FAILURE,
        }
    }

    ExitCode::SUCCESS
}

fn handle(mode: &str, request: &Value, stdout: &mut impl Write) -> io::Result<Flow> {
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let id = request.get("id").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => initialize(mode, &id, stdout)?,
        "tools/list" => tools_list(mode, &id, stdout)?,
        "tools/call" => tools_call(mode, request, &id, stdout)?,
        "shutdown" => {
            shutdown(mode, &id, stdout)?;
            return Ok(Flow::Stop);
        }
        _ => unknown_method(&id, stdout)?,
    }
    Ok(Flow::Continue)
}

fn initialize(mode: &str, id: &Value, stdout: &mut impl Write) -> io::Result<()> {
    if mode == "hang-handshake" {
        thread::sleep(Duration::from_secs(60));
    }
    if mode == "bad-handshake" {
        let error = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32603, "message": "fixture refuses initialize"}
        });
        return write_frame(stdout, &error);
    }
    let version = if mode == "bad-version" { "2" } else { "1" };
    let result = json!({
        "plugin_api_version": version,
        "capabilities": ["logging"]
    });
    write_result(stdout, id, &result)
}

fn tools_list(mode: &str, id: &Value, stdout: &mut impl Write) -> io::Result<()> {
    if mode == "garbage" {
        stdout.write_all(b"this is not json\n")?;
        return stdout.flush();
    }
    if mode == "flood" {
        return flood_frame(stdout, id);
    }
    let result = json!({
        "tools": [{
            "tool": TOOL,
            "summary": "Echo the text back",
            "effect": "write",
            "destructive": false,
            "input_schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["text"],
                "properties": {"text": {"type": "string"}}
            }
        }]
    });
    write_result(stdout, id, &result)
}

fn tools_call(mode: &str, request: &Value, id: &Value, stdout: &mut impl Write) -> io::Result<()> {
    match mode {
        "crash" => {
            eprintln!("fixture crashing on purpose");
            std::process::exit(3);
        }
        "hang" => {
            thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "flood" => flood_frame(stdout, id),
        "flood-logs" => flood_logs(stdout),
        "deep" => deep_result(stdout, id),
        "tool-error" => tool_error(stdout, id),
        "bad-result" => {
            let result = json!({"content": "not an array"});
            write_result(stdout, id, &result)
        }
        "wrong-id" => wrong_id(stdout),
        _ => echo_text(request, id, stdout),
    }
}

fn shutdown(mode: &str, id: &Value, stdout: &mut impl Write) -> io::Result<()> {
    if mode == "no-shutdown" {
        thread::sleep(Duration::from_secs(60));
    }
    write_result(stdout, id, &json!({}))
}

fn unknown_method(id: &Value, stdout: &mut impl Write) -> io::Result<()> {
    let error = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32601, "message": "fixture has no such method"}
    });
    write_frame(stdout, &error)
}

fn echo_text(request: &Value, id: &Value, stdout: &mut impl Write) -> io::Result<()> {
    let text = request
        .pointer("/params/arguments/text")
        .and_then(Value::as_str)
        .unwrap_or("ok");
    let notification = json!({
        "jsonrpc": "2.0",
        "method": "log",
        "params": {"level": "info", "message": "fixture handled the call"}
    });
    write_frame(stdout, &notification)?;
    let result = json!({
        "content": [{"type": "text", "text": text}],
        "is_error": false
    });
    write_result(stdout, id, &result)
}

fn flood_frame(stdout: &mut impl Write, id: &Value) -> io::Result<()> {
    let huge = "x".repeat(64 * 1024);
    let result = json!({
        "content": [{"type": "text", "text": huge}]
    });
    write_result(stdout, id, &result)
}

fn flood_logs(stdout: &mut impl Write) -> io::Result<()> {
    for index in 0..500 {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "log",
            "params": {
                "level": "info",
                "message": format!(
                    "flood line {index} ..........................................................."
                )
            }
        });
        write_frame(stdout, &notification)?;
    }
    thread::sleep(Duration::from_secs(60));
    Ok(())
}

fn deep_result(stdout: &mut impl Write, id: &Value) -> io::Result<()> {
    let mut nested = json!("bottom");
    for _ in 0..40 {
        nested = json!({"nested": nested});
    }
    let result = json!({
        "content": [{"type": "text", "text": "deep"}],
        "nested": nested
    });
    write_result(stdout, id, &result)
}

fn tool_error(stdout: &mut impl Write, id: &Value) -> io::Result<()> {
    let error = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32603, "message": "fixture tool failed"}
    });
    write_frame(stdout, &error)
}

fn wrong_id(stdout: &mut impl Write) -> io::Result<()> {
    let response = json!({
        "jsonrpc": "2.0",
        "id": 999_999,
        "result": {"content": [{"type": "text", "text": "wrong id"}]}
    });
    write_frame(stdout, &response)
}

fn write_result(stdout: &mut impl Write, id: &Value, result: &Value) -> io::Result<()> {
    let response = json!({"jsonrpc": "2.0", "id": id, "result": result});
    write_frame(stdout, &response)
}

fn write_frame(stdout: &mut impl Write, value: &Value) -> io::Result<()> {
    let line = serde_json::to_string(value).map_err(io::Error::other)?;
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}
