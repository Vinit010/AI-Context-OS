//! A small MCP server for `tests/mcp.rs`, driven by one mode argument.
//!
//! This is test support, not part of the library: it lives under `tests/fixtures` and is declared
//! as a `[[bin]]` so Cargo builds it for the integration tests and exposes its path as
//! `CARGO_BIN_EXE_mcp-server`. It speaks MCP's stdio transport rather than the plugin contract, so
//! the adapter is exercised over the wire: the `initialize` exchange, `notifications/initialized`,
//! paginated `tools/list`, `tools/call`, server-initiated requests, `notifications/message`, and
//! EOF shutdown.
//!
//! Modes:
//!
//! - `ok` (default): two tools across two pages; `tools/call` echoes `name: text`.
//! - `is-error`: `tools/call` returns `isError: true` with text `boom`.
//! - `json-error`: `tools/call` returns a JSON-RPC error.
//! - `bad-version`: `initialize` selects a revision this host does not support.
//! - `undeclared`: `tools/list` returns `secret_tool`, which the manifest does not declare.
//! - `bad-list`: `tools/list` omits `inputSchema`.
//! - `hang`: never answers `tools/call`.
//! - `server-requests`: during `tools/call`, sends `ping`, `sampling/createMessage`, and a
//!   `notifications/message` log, then reports what the host answered.

use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "ok".to_owned());
    if let Err(error) = run(&mode) {
        let _ = writeln!(io::stderr(), "mcp-server fixture: {error}");
        std::process::exit(1);
    }
}

fn run(mode: &str) -> io::Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut initialized = false;
    let mut line = String::new();

    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let frame: Value = match serde_json::from_str(trimmed) {
            Ok(frame) => frame,
            Err(error) => {
                write_error(
                    &mut output,
                    &Value::Null,
                    -32700,
                    &format!("fixture could not parse the frame: {error}"),
                )?;
                continue;
            }
        };

        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
        let id = frame.get("id").cloned();
        match method {
            "initialize" => initialize(&mut output, id.as_ref(), mode)?,
            "notifications/initialized" => initialized = true,
            "tools/list" => {
                if !initialized {
                    write_error(
                        &mut output,
                        &id.unwrap_or(Value::Null),
                        -32000,
                        "notifications/initialized was not received before tools/list",
                    )?;
                    continue;
                }
                tools_list(&mut output, id.as_ref(), mode, &frame)?;
            }
            "tools/call" => {
                if !initialized {
                    write_error(
                        &mut output,
                        &id.unwrap_or(Value::Null),
                        -32000,
                        "notifications/initialized was not received before tools/call",
                    )?;
                    continue;
                }
                tools_call(&mut input, &mut output, id.as_ref(), mode, &frame)?;
            }
            "shutdown" => write_result(&mut output, id.as_ref(), &json!({}))?,
            "exit" => return Ok(()),
            _ => {
                if let Some(id) = id {
                    write_error(&mut output, &id, -32601, "method not found")?;
                }
            }
        }
    }
}

fn initialize(output: &mut impl Write, id: Option<&Value>, mode: &str) -> io::Result<()> {
    let version = if mode == "bad-version" {
        "2026-01-01"
    } else {
        "2025-06-18"
    };
    write_result(
        output,
        id,
        &json!({
            "protocolVersion": version,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fixture-mcp", "version": "1.0.0"}
        }),
    )
}

fn tools_list(
    output: &mut impl Write,
    id: Option<&Value>,
    mode: &str,
    frame: &Value,
) -> io::Result<()> {
    if mode == "undeclared" {
        return write_result(
            output,
            id,
            &json!({"tools": [tool("secret_tool", "Not declared", &schema())]}),
        );
    }
    if mode == "bad-list" {
        return write_result(
            output,
            id,
            &json!({"tools": [{"name": "read_file", "description": "Read a file"}]}),
        );
    }

    let cursor = frame
        .get("params")
        .and_then(|params| params.get("cursor"))
        .and_then(Value::as_str);
    match cursor {
        None => write_result(
            output,
            id,
            &json!({
                "tools": [tool("read_file", "Read a file", &schema())],
                "nextCursor": "page-2"
            }),
        ),
        Some("page-2") => write_result(
            output,
            id,
            &json!({"tools": [tool("write_file", "Write a file", &schema())]}),
        ),
        Some(other) => write_error(
            output,
            &id.cloned().unwrap_or(Value::Null),
            -32602,
            &format!("fixture does not know the cursor `{other}`"),
        ),
    }
}

fn tools_call(
    input: &mut impl BufRead,
    output: &mut impl Write,
    id: Option<&Value>,
    mode: &str,
    frame: &Value,
) -> io::Result<()> {
    let name = frame
        .get("params")
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let arguments = frame
        .get("params")
        .and_then(|params| params.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    match mode {
        "json-error" => write_error(
            output,
            &id.cloned().unwrap_or(Value::Null),
            -32603,
            "fixture refused the call",
        ),
        "is-error" => write_result(
            output,
            id,
            &json!({"content": [{"type": "text", "text": "boom"}], "isError": true}),
        ),
        "hang" => {
            thread::sleep(Duration::from_secs(60));
            Ok(())
        }
        "server-requests" => server_requests(input, output, id),
        _ => {
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            write_result(
                output,
                id,
                &json!({
                    "content": [{"type": "text", "text": format!("{name}: {text}")}],
                    "isError": false
                }),
            )
        }
    }
}

fn server_requests(
    input: &mut impl BufRead,
    output: &mut impl Write,
    id: Option<&Value>,
) -> io::Result<()> {
    write_frame(
        output,
        &json!({"jsonrpc": "2.0", "id": "srv-ping", "method": "ping"}),
    )?;
    let ping = read_response(input)?;
    let ping_ok = ping
        .get("result")
        .is_some_and(|result| result.as_object().is_some_and(serde_json::Map::is_empty));

    write_frame(
        output,
        &json!({
            "jsonrpc": "2.0",
            "id": "srv-sampling",
            "method": "sampling/createMessage",
            "params": {}
        }),
    )?;
    let sampling = read_response(input)?;
    let sampling_code = sampling
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    write_frame(
        output,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/message",
            "params": {"level": "info", "logger": "fixture", "data": "server request handled"}
        }),
    )?;

    write_result(
        output,
        id,
        &json!({
            "content": [{
                "type": "text",
                "text": format!("ping={ping_ok};sampling={sampling_code}")
            }],
            "isError": false
        }),
    )
}

fn read_response(input: &mut impl BufRead) -> io::Result<Value> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Err(io::Error::other(
                "the host closed stdin while the fixture awaited a response",
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return serde_json::from_str(trimmed).map_err(io::Error::other);
    }
}

fn tool(name: &str, description: &str, input_schema: &Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {"text": {"type": "string"}},
        "required": ["text"]
    })
}

fn write_result(output: &mut impl Write, id: Option<&Value>, result: &Value) -> io::Result<()> {
    let id = id.cloned().unwrap_or(Value::Null);
    write_frame(
        output,
        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
}

fn write_error(output: &mut impl Write, id: &Value, code: i64, message: &str) -> io::Result<()> {
    write_frame(
        output,
        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
}

fn write_frame(output: &mut impl Write, frame: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, frame).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}
