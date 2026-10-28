//! The plugin's stdin/stdout server.
//!
//! `docs/PLUGIN_SPEC.md` §4 defines the six methods. This module implements the four a plugin
//! receives (`initialize`, `tools/list`, `tools/call`, `shutdown`) and rejects the two that travel
//! the other way (`log`, `host/request_approval`). stdout carries protocol frames only; the plugin
//! writes no progress text and no banner.
//!
//! The server is deliberately small and synchronous. One request is read, one response is written,
//! and the next request is not read until the previous call finished. That matches the supervisor's
//! one-deadline-per-call model and makes it impossible for a slow GitHub request to interleave with
//! another call.

use std::io::{self, BufRead, Write};

use aicontext_plugin_sdk::{
    ApiVersion, CallToolParams, CallToolResult, ContentBlock, FrameCodec, InitializeParams,
    InitializeResult, Method, RawRequest, RequestId, Response, RpcError, ToolsListResult,
};
use serde_json::{Map, Value, json};

use crate::client::GitHubClient;
use crate::error::GitHubError;
use crate::tools;

/// The largest request frame the plugin accepts.
///
/// The gateway caps arguments at 64 KiB, so this is a generous outer bound rather than a second
/// argument limit. It exists so a malformed or hostile caller cannot make the plugin allocate an
/// unbounded line before the protocol codec sees it.
pub const MAX_INPUT_FRAME_BYTES: usize = 1024 * 1024;

/// Serve the plugin protocol on `reader` and `writer`.
///
/// `max_output_bytes` is the manifest's `limits.max_output_bytes`. A tool result that cannot fit is
/// replaced with a `github.response_too_large` result rather than a frame the host would reject as
/// `PLG-006`; if even that result cannot fit, the call gets a JSON-RPC internal error.
///
/// # Errors
///
/// Returns an I/O error when the input stream cannot be read, a frame is larger than
/// [`MAX_INPUT_FRAME_BYTES`], a frame is not UTF-8, or a response cannot be written. A malformed
/// JSON-RPC request is a protocol error response, not an I/O error, so the stream stays usable.
pub fn serve<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    client: &GitHubClient,
    max_output_bytes: usize,
) -> io::Result<()> {
    let responses = FrameCodec::new(max_output_bytes);
    let mut initialized = false;

    loop {
        let Some(line) = read_frame(&mut reader, MAX_INPUT_FRAME_BYTES)? else {
            return Ok(());
        };

        match handle(&line, client, &mut initialized, max_output_bytes) {
            Handled::Reply(response) => write_response(&mut writer, responses, &response)?,
            Handled::Shutdown(response) => {
                write_response(&mut writer, responses, &response)?;
                return Ok(());
            }
        }
    }
}

/// One handled request.
enum Handled {
    /// Write the response and keep serving.
    Reply(Response<Value>),
    /// Write the response and stop.
    Shutdown(Response<Value>),
}

fn handle(
    line: &str,
    client: &GitHubClient,
    initialized: &mut bool,
    max_output_bytes: usize,
) -> Handled {
    let requests = FrameCodec::new(MAX_INPUT_FRAME_BYTES);
    let request: RawRequest = match requests.decode(line) {
        Ok(request) => request,
        Err(error) => {
            let id = request_id_from_line(line);
            return Handled::Reply(Response::error(
                id,
                RpcError::invalid_request(format!("the request frame is invalid: {error}")),
            ));
        }
    };

    let response = match request.method {
        Method::Initialize => initialize(request.id.clone(), &request, initialized),
        Method::ToolsList => {
            if *initialized {
                Response::success(
                    request.id,
                    json!(ToolsListResult {
                        tools: tools::tool_definitions()
                    }),
                )
            } else {
                not_initialized(request.id)
            }
        }
        Method::ToolsCall => call_tool_request(
            request.id.clone(),
            &request,
            client,
            *initialized,
            max_output_bytes,
        ),
        Method::Shutdown => {
            return Handled::Shutdown(Response::success(request.id, json!({})));
        }
        Method::Log | Method::RequestApproval => Response::error(
            Some(request.id),
            RpcError::invalid_request(format!(
                "`{}` is a plugin-to-host method and is not accepted by a plugin",
                request.method
            )),
        ),
    };
    Handled::Reply(response)
}

fn initialize(id: RequestId, request: &RawRequest, initialized: &mut bool) -> Response<Value> {
    if *initialized {
        return Response::error(
            Some(id),
            RpcError::invalid_request("initialize was already called"),
        );
    }

    match request.params_as::<InitializeParams>() {
        Ok(params) => match ApiVersion::parse(&params.plugin_api_version) {
            Ok(version) if version == ApiVersion::CURRENT => {
                *initialized = true;
                Response::success(
                    id,
                    json!(InitializeResult {
                        plugin_api_version: ApiVersion::CURRENT.to_string(),
                        capabilities: Vec::new(),
                    }),
                )
            }
            Ok(version) => Response::error(
                Some(id),
                RpcError::new(
                    -32602,
                    format!(
                        "the host asked for plugin API version {version}; this plugin implements \
                         {}",
                        ApiVersion::CURRENT
                    ),
                ),
            ),
            Err(error) => Response::error(Some(id), RpcError::invalid_params(error.to_string())),
        },
        Err(error) => Response::error(Some(id), RpcError::invalid_params(error.to_string())),
    }
}

fn call_tool_request(
    id: RequestId,
    request: &RawRequest,
    client: &GitHubClient,
    initialized: bool,
    max_output_bytes: usize,
) -> Response<Value> {
    if !initialized {
        return not_initialized(id);
    }

    let params = match request.params_as::<CallToolParams>() {
        Ok(params) => params,
        Err(error) => {
            return Response::error(Some(id), RpcError::invalid_params(error.to_string()));
        }
    };

    let result = call_tool(client, &params);
    let responses = FrameCodec::new(max_output_bytes);
    let response = Response::success(id.clone(), json!(result));
    if responses.encode(&response).is_ok() {
        return response;
    }

    let fallback = tool_error(&GitHubError::ResponseTooLarge {
        limit: max_output_bytes,
    });
    let fallback = Response::success(id.clone(), json!(fallback));
    if responses.encode(&fallback).is_ok() {
        fallback
    } else {
        Response::error(
            Some(id),
            RpcError::internal_error(
                "the tool result and its error replacement exceed the output limit",
            ),
        )
    }
}

fn call_tool(client: &GitHubClient, params: &CallToolParams) -> CallToolResult {
    match tools::dispatch(client, &params.name, &params.arguments) {
        Ok(value) => tool_result(&value, false),
        Err(error) => tool_error(&error),
    }
}

fn tool_result(value: &Value, is_error: bool) -> CallToolResult {
    CallToolResult {
        content: vec![ContentBlock::text(value.to_string())],
        is_error,
        meta: None,
        extra: Map::new(),
    }
}

fn tool_error(error: &GitHubError) -> CallToolResult {
    tool_result(&error.to_json(), true)
}

fn not_initialized(id: RequestId) -> Response<Value> {
    Response::error(
        Some(id),
        RpcError::invalid_request("initialize must be called before this method"),
    )
}

fn write_response<W: Write>(
    writer: &mut W,
    codec: FrameCodec,
    response: &Response<Value>,
) -> io::Result<()> {
    let line = codec.encode_line(response).map_err(|error| {
        io::Error::other(format!("the response frame could not be encoded: {error}"))
    })?;
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

/// Read one newline-terminated frame without allowing the buffer to grow past `max_bytes`.
fn read_frame<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Option<String>> {
    let mut buffer = Vec::new();

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if buffer.is_empty() {
                return Ok(None);
            }
            break;
        }

        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            if buffer.len() + position + 1 > max_bytes {
                return Err(io::Error::other(format!(
                    "the request frame exceeds {max_bytes} bytes"
                )));
            }
            buffer.extend_from_slice(&available[..=position]);
            reader.consume(position + 1);
            break;
        }

        if buffer.len() + available.len() > max_bytes {
            return Err(io::Error::other(format!(
                "the request frame exceeds {max_bytes} bytes"
            )));
        }
        let length = available.len();
        buffer.extend_from_slice(available);
        reader.consume(length);
    }

    String::from_utf8(buffer).map(Some).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the request frame is not UTF-8: {error}"),
        )
    })
}

/// Recover the id from a frame that failed to deserialize, so the error response can echo it.
fn request_id_from_line(line: &str) -> Option<RequestId> {
    let value: Value = serde_json::from_str(line).ok()?;
    let id = value.get("id")?;
    serde_json::from_value(id.clone()).ok()
}
