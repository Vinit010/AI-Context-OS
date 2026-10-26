//! The pipes around one plugin process.
//!
//! Three reader threads keep a misbehaving plugin from blocking the host:
//!
//! - stdout is read line by line with a hard byte cap, parsed into [`Incoming`], and handed to the
//!   supervisor over a bounded channel. The cap is checked before the line is buffered, so a plugin
//!   that never writes a newline cannot grow the host's memory to the size of its output.
//! - stdin is written by its own thread, so a plugin that stops reading cannot block a call before
//!   its timeout starts.
//! - stderr is drained into a bounded tail for diagnostics. It is not protocol traffic, but a
//!   plugin that fills an unread stderr pipe would otherwise deadlock itself.
//!
//! The threads are detached rather than joined. A plugin can spawn a grandchild that inherits its
//! stdout, and then killing the plugin does not close the pipe; joining would make the host wait
//! for a process it does not control. Dropping the handles lets the threads end when the pipe ends,
//! and the supervisor never waits on them.

use std::io::{self, BufRead, Read, Write};
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use aicontext_plugin_sdk::{
    ApprovalParams, JSONRPC_VERSION, LogParams, Method, RequestId, RpcError,
};
use serde_json::Value;

/// How many parsed frames can wait for the supervisor before the reader blocks.
///
/// A small bound is deliberate: it gives the child backpressure while ensuring a flood cannot turn
/// into unbounded host memory. The supervisor drains the channel for the whole duration of a call,
/// so the bound is not on legitimate notification traffic, only on how far the reader can run
/// ahead.
pub(super) const INBOX_CAPACITY: usize = 16;

/// How much of a plugin's stderr is kept for diagnostics.
const STDERR_TAIL_BYTES: usize = 4096;

/// One event read from the plugin's stdout, or a failure of the pipe.
#[derive(Debug)]
pub(super) enum Incoming {
    /// A JSON-RPC response. The result is still untyped; the method's decoder owns its shape.
    Response {
        /// The id the plugin echoed.
        id: RequestId,
        /// The result or the plugin's JSON-RPC error.
        result: Result<Value, RpcError>,
    },
    /// A `log` notification. `bytes` is the whole wire line, counted against the call's flood
    /// budget.
    Log {
        /// The typed notification.
        params: LogParams,
        /// The size of the line that carried it.
        bytes: usize,
    },
    /// A `host/request_approval` request. The supervisor answers it, because the host is the
    /// decision authority even when the plugin asks.
    Approval {
        /// The request id to echo in the denial.
        id: RequestId,
        /// The typed request.
        params: ApprovalParams,
        /// The size of the line that carried it.
        bytes: usize,
    },
    /// The plugin wrote something that is not a frame this protocol allows.
    Protocol {
        /// The concrete violation.
        detail: String,
    },
    /// The plugin wrote a line over the manifest's cap.
    OutputLimit {
        /// The concrete violation.
        detail: String,
    },
    /// The plugin closed stdout.
    Eof,
    /// Reading or writing a pipe failed.
    Io {
        /// The operation that failed.
        operation: &'static str,
        /// The operating system's diagnosis.
        source: io::Error,
    },
}

/// Start the stdout reader. The handle is intentionally dropped by the caller.
pub(super) fn spawn_reader(
    stdout: ChildStdout,
    max_bytes: usize,
    outbox: SyncSender<Incoming>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("ac-plugin-out".to_owned())
        .spawn(move || {
            let mut reader = io::BufReader::new(stdout);
            loop {
                match next_frame(&mut reader, max_bytes) {
                    Ok(Frame::Eof) => {
                        let _ = outbox.send(Incoming::Eof);
                        break;
                    }
                    Ok(Frame::TooLong) => {
                        let _ = outbox.send(Incoming::OutputLimit {
                            detail: format!("a single line exceeds the {max_bytes}-byte cap"),
                        });
                        break;
                    }
                    Ok(Frame::Line(line)) => {
                        let bytes = line.len();
                        let event = parse_line(&line, bytes);
                        if outbox.send(event).is_err() {
                            break;
                        }
                    }
                    Ok(Frame::Truncated) => {
                        let _ = outbox.send(Incoming::Protocol {
                            detail: "the plugin closed stdout in the middle of a line".to_owned(),
                        });
                        break;
                    }
                    Err(source) => {
                        let _ = outbox.send(Incoming::Io {
                            operation: "read the plugin's stdout",
                            source,
                        });
                        break;
                    }
                }
            }
        })
}

/// Start the stdin writer. The handle is intentionally dropped by the caller.
pub(super) fn spawn_writer(
    stdin: ChildStdin,
    inbox: Receiver<Vec<u8>>,
    outbox: SyncSender<Incoming>,
) -> io::Result<thread::JoinHandle<()>> {
    thread::Builder::new()
        .name("ac-plugin-in".to_owned())
        .spawn(move || {
            let mut stdin = stdin;
            while let Ok(bytes) = inbox.recv() {
                if let Err(source) = stdin.write_all(&bytes).and_then(|()| stdin.flush()) {
                    let _ = outbox.send(Incoming::Io {
                        operation: "write to the plugin's stdin",
                        source,
                    });
                    break;
                }
            }
        })
}

/// Start the stderr drain. The handle is intentionally dropped by the caller.
pub(super) fn spawn_stderr(stderr: ChildStderr) -> io::Result<StderrHandle> {
    let tail = Arc::new(Mutex::new(StderrTail::default()));
    let writer = Arc::clone(&tail);
    thread::Builder::new()
        .name("ac-plugin-err".to_owned())
        .spawn(move || {
            let mut stderr = stderr;
            let mut chunk = [0_u8; 1024];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => match writer.lock() {
                        Ok(mut tail) => tail.push(&chunk[..read]),
                        Err(poisoned) => poisoned.into_inner().push(&chunk[..read]),
                    },
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        })?;
    Ok(tail)
}

/// The shared handle the supervisor reads for diagnostics.
pub(super) type StderrHandle = Arc<Mutex<StderrTail>>;

/// A bounded tail of the plugin's stderr.
#[derive(Debug, Default)]
pub(super) struct StderrTail {
    bytes: Vec<u8>,
}

impl StderrTail {
    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > STDERR_TAIL_BYTES {
            let excess = self.bytes.len() - STDERR_TAIL_BYTES;
            self.bytes.drain(..excess);
        }
    }

    /// The tail as text, with control characters replaced so it cannot forge a terminal line.
    pub(super) fn snapshot(&self) -> String {
        let lossy = String::from_utf8_lossy(&self.bytes);
        let clean: String = lossy
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        clean.trim().to_owned()
    }
}

/// One bounded read from stdout.
#[derive(Debug)]
enum Frame {
    Line(Vec<u8>),
    Truncated,
    TooLong,
    Eof,
}

/// Read one newline-terminated frame without ever buffering more than `max_bytes` plus one chunk.
///
/// `BufRead::read_until` cannot enforce a cap while reading: a hostile line would be fully buffered
/// before the caller could reject it. This loop checks the cap against each filled chunk first.
fn next_frame<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Frame> {
    let mut buffer = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if buffer.is_empty() {
                return Ok(Frame::Eof);
            }
            // EOF in the middle of a line is not a frame, even when the bytes happen to parse as
            // JSON: the protocol requires the newline terminator.
            return Ok(Frame::Truncated);
        }

        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            if buffer.len() + position > max_bytes {
                return Ok(Frame::TooLong);
            }
            buffer.extend_from_slice(&available[..=position]);
            reader.consume(position + 1);
            return Ok(Frame::Line(buffer));
        }

        if buffer.len() + available.len() > max_bytes {
            return Ok(Frame::TooLong);
        }
        buffer.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
    }
}

/// Parse one complete line into the event the supervisor understands.
///
/// The line is parsed as a generic JSON value first because a response's `result` may legitimately
/// be `null`, and `Option<Value>` would erase the difference between `"result": null` and a frame
/// with no `result` member at all.
fn parse_line(line: &[u8], bytes: usize) -> Incoming {
    let Ok(text) = std::str::from_utf8(line) else {
        return Incoming::Protocol {
            detail: "the plugin wrote a line that is not valid UTF-8".to_owned(),
        };
    };
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(source) => {
            return Incoming::Protocol {
                detail: format!("the plugin wrote a line that is not JSON: {source}"),
            };
        }
    };
    parse_value(&value, bytes)
}

fn parse_value(value: &Value, bytes: usize) -> Incoming {
    let Some(object) = value.as_object() else {
        return Incoming::Protocol {
            detail: "the plugin wrote a JSON value that is not an object".to_owned(),
        };
    };

    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Incoming::Protocol {
            detail: format!("the frame does not declare `jsonrpc` as {JSONRPC_VERSION:?}"),
        };
    }

    let id = match object.get("id") {
        None | Some(Value::Null) => None,
        Some(raw) => match serde_json::from_value::<RequestId>(raw.clone()) {
            Ok(id) => Some(id),
            Err(source) => {
                return Incoming::Protocol {
                    detail: format!("the frame's `id` is not a JSON-RPC id: {source}"),
                };
            }
        },
    };

    let method = match object.get("method") {
        None => None,
        Some(raw) => match serde_json::from_value::<Method>(raw.clone()) {
            Ok(method) => Some(method),
            Err(source) => {
                return Incoming::Protocol {
                    detail: format!(
                        "the frame's `method` is not one this protocol defines: {source}"
                    ),
                };
            }
        },
    };

    let params = object
        .get("params")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    match (method, id) {
        (Some(Method::Log), None) => match serde_json::from_value::<LogParams>(params) {
            Ok(params) => Incoming::Log { params, bytes },
            Err(source) => Incoming::Protocol {
                detail: format!(
                    "the `log` notification's params do not match the protocol: {source}"
                ),
            },
        },
        (Some(Method::RequestApproval), Some(id)) => {
            match serde_json::from_value::<ApprovalParams>(params) {
                Ok(params) => Incoming::Approval { id, params, bytes },
                Err(source) => Incoming::Protocol {
                    detail: format!(
                        "the `host/request_approval` params do not match the protocol: {source}"
                    ),
                },
            }
        }
        (Some(method), Some(_)) => Incoming::Protocol {
            detail: format!("the plugin sent the host-only method `{method}` as a request"),
        },
        (Some(method), None) => Incoming::Protocol {
            detail: format!("the plugin sent the request-only method `{method}` as a notification"),
        },
        (None, Some(id)) => match (object.get("result"), object.get("error")) {
            (Some(result), None) => Incoming::Response {
                id,
                result: Ok(result.clone()),
            },
            (None, Some(error)) => match serde_json::from_value::<RpcError>(error.clone()) {
                Ok(error) => Incoming::Response {
                    id,
                    result: Err(error),
                },
                Err(source) => Incoming::Protocol {
                    detail: format!(
                        "the response's `error` object does not match the protocol: {source}"
                    ),
                },
            },
            (Some(_), Some(_)) => Incoming::Protocol {
                detail: "a response carries both `result` and `error`".to_owned(),
            },
            (None, None) => Incoming::Protocol {
                detail: "a response carries neither `result` nor `error`".to_owned(),
            },
        },
        (None, None) => Incoming::Protocol {
            detail: "the frame is neither a request, a notification, nor a response".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{Frame, next_frame};

    fn frame(reader: &mut Cursor<&[u8]>, cap: usize) -> Frame {
        match next_frame(reader, cap) {
            Ok(frame) => frame,
            Err(error) => panic!("cursor reads cannot fail: {error}"),
        }
    }

    #[test]
    fn a_complete_line_is_returned_with_its_terminator() {
        let mut reader = Cursor::new(&b"{\"a\":1}\n{\"b\":2}\n"[..]);
        let first = frame(&mut reader, 64);
        assert!(matches!(first, Frame::Line(line) if line == b"{\"a\":1}\n"));
        let second = frame(&mut reader, 64);
        assert!(matches!(second, Frame::Line(line) if line == b"{\"b\":2}\n"));
        assert!(matches!(frame(&mut reader, 64), Frame::Eof));
    }

    #[test]
    fn a_line_over_the_cap_is_refused_before_it_is_buffered() {
        let mut reader = Cursor::new(&b"123456789\n"[..]);
        assert!(matches!(frame(&mut reader, 8), Frame::TooLong));
    }

    #[test]
    fn a_line_exactly_at_the_cap_is_accepted() {
        let mut reader = Cursor::new(&b"12345678\n"[..]);
        assert!(matches!(frame(&mut reader, 8), Frame::Line(line) if line == b"12345678\n"));
    }

    #[test]
    fn eof_without_a_line_is_clean() {
        let mut reader = Cursor::new(&b""[..]);
        assert!(matches!(frame(&mut reader, 8), Frame::Eof));
    }

    #[test]
    fn eof_mid_line_is_truncated_and_never_treated_as_a_frame() {
        let mut reader = Cursor::new(&b"{\"unfinished\":"[..]);
        assert!(matches!(frame(&mut reader, 64), Frame::Truncated));
    }

    #[test]
    fn crlf_is_kept_and_json_accepts_it_as_trailing_whitespace() {
        let mut reader = Cursor::new(&b"{\"a\":1}\r\n"[..]);
        let frame = frame(&mut reader, 64);
        assert!(matches!(frame, Frame::Line(line) if line == b"{\"a\":1}\r\n"));
    }
}
