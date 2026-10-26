//! Spawn one plugin process, negotiate its version, and call its tools.
//!
//! The supervisor is synchronous. A `PluginProcess` owns one child process, one request id
//! sequence, and one deadline per call; there is no background call queue and no second way to
//! reach the plugin. `docs/PLUGIN_SPEC.md` §1 requires exactly one path from an action to a plugin,
//! and `TASK-074`'s gateway is the layer that adds permissions and approval in front of it.
//!
//! Every failure after the process starts poisons it: the child is killed, its pipes are closed,
//! and later calls return [`SupervisorError::NotRunning`]. That is deliberate. A stream that has
//! produced a malformed frame, timed out, or overflowed a cap cannot be assumed to be in sync with
//! the next request, and a supervisor that guessed would be trusting the process it just caught
//! misbehaving.
//!
//! The per-call timeout and the output cap both come from the manifest (`limits`), not from a
//! command-line flag, so a plugin cannot talk its way into a longer deadline. The output cap is
//! enforced in three places: one frame (the encoded line), the total notification traffic during a
//! call, and the depth of the decoded result (32 levels, `docs/PLUGIN_SPEC.md` §4).

use std::collections::VecDeque;
use std::io;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

use aicontext_plugin_sdk::{
    ApiVersion, ApprovalResult, CallToolParams, CallToolResult, FrameCodec, InitializeParams,
    InitializeResult, LogLevel, Method, PluginError, PluginManifest, Request, RequestId, Response,
    ToolDefinition, ToolsListResult, negotiate,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::json::{MAX_JSON_DEPTH, json_depth};

use super::error::SupervisorError;
use super::transport::{Incoming, StderrHandle, spawn_reader, spawn_stderr, spawn_writer};

/// The deepest result structure a plugin may return.
///
/// `docs/PLUGIN_SPEC.md` §4 bounds every result at 32 levels. Depth is counted with the root value
/// at 1, an object's members at 2, and so on. A result deeper than this is treated as an output
/// limit rather than a parse error: the bytes were readable, but the shape is more than the host
/// agreed to carry.
pub const MAX_RESULT_DEPTH: usize = MAX_JSON_DEPTH;

/// How many log entries are retained between calls.
const MAX_LOGS: usize = 64;

/// How much of one log message is retained.
const MAX_LOG_MESSAGE_BYTES: usize = 512;

/// How long a killed child is given to be reaped before the supervisor stops waiting.
const KILL_GRACE: Duration = Duration::from_secs(1);

/// How long a plugin is given to exit after a successful `shutdown` response.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);

/// How often a bounded wait checks whether the child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Start a plugin and complete the `initialize` handshake.
///
/// The manifest is validated again even if it came from [`PluginManifest::from_json`], because a
/// caller can construct the struct directly and an invalid limit must not reach a spawn.
///
/// # Errors
///
/// Before a process exists: [`SupervisorError::Manifest`] for an invalid manifest,
/// [`SupervisorError::ApiVersion`] (`PLG-004`) for a version this host does not implement, and
/// [`SupervisorError::Spawn`] (`PLG-001`) when the executable cannot be started.
///
/// After a process exists: [`SupervisorError::Handshake`] (`PLG-002`) for a missing or malformed
/// handshake, [`SupervisorError::Timeout`] (`PLG-003`) when it does not answer in time,
/// [`SupervisorError::Protocol`] (`PLG-005`) for a frame outside the protocol, and
/// [`SupervisorError::OutputLimit`] (`PLG-006`) when it writes more than the manifest allows.
pub fn start_plugin(manifest: &PluginManifest) -> Result<PluginProcess, SupervisorError> {
    manifest.validate().map_err(SupervisorError::Manifest)?;
    let negotiated = match manifest.negotiate(ApiVersion::CURRENT) {
        Ok(version) => version,
        Err(PluginError::ApiVersionMismatch { plugin, host }) => {
            return Err(SupervisorError::ApiVersion { plugin, host });
        }
        Err(error) => return Err(SupervisorError::Manifest(error)),
    };

    let max_output_bytes = usize::try_from(manifest.limits.max_output_bytes).unwrap_or(usize::MAX);
    let timeout = Duration::from_millis(manifest.limits.call_timeout_ms);

    let mut command = Command::new(&manifest.entry.command);
    command
        .args(&manifest.entry.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|source| SupervisorError::Spawn {
        program: manifest.entry.command.clone(),
        source,
    })?;

    let Some(stdin) = child.stdin.take() else {
        abort_child(&mut child);
        return Err(SupervisorError::Io {
            operation: "open the plugin's stdin",
            source: io::Error::other("the child has no piped stdin"),
        });
    };
    let Some(stdout) = child.stdout.take() else {
        abort_child(&mut child);
        return Err(SupervisorError::Io {
            operation: "open the plugin's stdout",
            source: io::Error::other("the child has no piped stdout"),
        });
    };
    let Some(stderr) = child.stderr.take() else {
        abort_child(&mut child);
        return Err(SupervisorError::Io {
            operation: "open the plugin's stderr",
            source: io::Error::other("the child has no piped stderr"),
        });
    };

    let (inbox_tx, inbox_rx) = sync_channel(super::transport::INBOX_CAPACITY);
    let (writer_tx, writer_rx) = channel();

    if let Err(source) = spawn_writer(stdin, writer_rx, inbox_tx.clone()) {
        abort_child(&mut child);
        return Err(SupervisorError::Io {
            operation: "start the plugin's stdin writer",
            source,
        });
    }
    if let Err(source) = spawn_reader(stdout, max_output_bytes, inbox_tx.clone()) {
        abort_child(&mut child);
        return Err(SupervisorError::Io {
            operation: "start the plugin's stdout reader",
            source,
        });
    }
    let stderr = match spawn_stderr(stderr) {
        Ok(tail) => tail,
        Err(source) => {
            abort_child(&mut child);
            return Err(SupervisorError::Io {
                operation: "start the plugin's stderr reader",
                source,
            });
        }
    };
    // The reader and writer each hold a clone. Dropping this one lets the supervisor see a
    // disconnected channel when both threads have ended.
    drop(inbox_tx);

    let mut process = PluginProcess {
        child,
        writer: Some(writer_tx),
        inbox: inbox_rx,
        stderr,
        codec: FrameCodec::new(max_output_bytes),
        timeout,
        max_output_bytes,
        next_id: 0,
        dead: false,
        logs: VecDeque::new(),
        capabilities: Vec::new(),
    };
    process.initialize(negotiated)?;
    Ok(process)
}

/// One running plugin process.
///
/// The process is killed when the value is dropped, so a caller cannot leak a plugin by returning
/// early. Call [`PluginProcess::shutdown`] for the graceful path: it sends `shutdown`, waits for
/// the response, closes stdin, and only kills if the plugin does not exit.
#[derive(Debug)]
pub struct PluginProcess {
    child: Child,
    writer: Option<Sender<Vec<u8>>>,
    inbox: Receiver<Incoming>,
    stderr: StderrHandle,
    codec: FrameCodec,
    timeout: Duration,
    max_output_bytes: usize,
    next_id: u64,
    dead: bool,
    logs: VecDeque<PluginLog>,
    capabilities: Vec<String>,
}

impl PluginProcess {
    /// Every capability the plugin reported during `initialize`.
    #[must_use]
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// Whether the child is still running.
    ///
    /// This reaps a child that has already exited; it does not wait. A process poisoned by an
    /// earlier failure reports `false` without touching the operating system.
    pub fn is_running(&mut self) -> bool {
        if self.dead {
            return false;
        }
        match self.child.try_wait() {
            Ok(Some(_)) => {
                self.dead = true;
                false
            }
            Ok(None) => true,
            Err(_) => {
                self.terminate();
                false
            }
        }
    }

    /// Drain the `log` notifications received so far.
    ///
    /// Logs are diagnostic data and are retained separately from call results. The queue holds at
    /// most 64 entries, each truncated to 512 bytes, and the oldest is discarded first. A caller
    /// that wants to relay them to stderr (`docs/PLUGIN_SPEC.md` §4) drains them after each call.
    pub fn take_logs(&mut self) -> Vec<PluginLog> {
        self.logs.drain(..).collect()
    }

    /// The tail of the plugin's stderr, with control characters replaced by spaces.
    ///
    /// This is for diagnosis after a failure. It is not protocol data and is never parsed.
    #[must_use]
    pub fn stderr_tail(&self) -> String {
        match self.stderr.lock() {
            Ok(tail) => tail.snapshot(),
            Err(poisoned) => poisoned.into_inner().snapshot(),
        }
    }

    /// Ask the plugin for its tool descriptors.
    ///
    /// # Errors
    ///
    /// Every supervisor failure in [`SupervisorError`], including a timeout (`PLG-003`), a
    /// malformed result (`PLG-005`), an output overflow (`PLG-006`), or a process that has already
    /// exited (`PLG-007`). A failure kills the process.
    pub fn list_tools(&mut self) -> Result<Vec<ToolDefinition>, SupervisorError> {
        let id = RequestId::from(self.next_id());
        let request = Request::tools_list(id.clone());
        let result: ToolsListResult = self.exchange(Method::ToolsList, &id, &request)?;
        Ok(result.tools)
    }

    /// Call one tool with already-validated arguments.
    ///
    /// Argument validation against `input_schema` is the gateway's job; this method checks only
    /// that the plugin answered, within its timeout and output limits, with the shape the protocol
    /// requires. A tool that reports `is_error: true` is a successful call whose result carries the
    /// failure, not a [`SupervisorError`]. The method is public because the supervisor is a library
    /// of its own, but [`ToolGateway`](crate::ToolGateway) is the only production caller;
    /// `no_bypass_path_to_plugin` enforces that invariant with a source scan.
    ///
    /// # Errors
    ///
    /// As [`PluginProcess::list_tools`].
    pub fn call_tool(
        &mut self,
        tool: &str,
        arguments: Value,
    ) -> Result<CallToolResult, SupervisorError> {
        let id = RequestId::from(self.next_id());
        let request = Request::call_tool(
            id.clone(),
            CallToolParams {
                name: tool.to_owned(),
                arguments,
            },
        );
        self.exchange(Method::ToolsCall, &id, &request)
    }

    /// Send `shutdown`, wait for its response, close stdin, and reap the process.
    ///
    /// A plugin that answers but does not exit within one second is killed; the call still
    /// succeeds because the process is gone, which is the promise `docs/PLUGIN_SPEC.md` §7 makes.
    /// A plugin that does not answer within `limits.call_timeout_ms` is killed and the timeout is
    /// returned.
    ///
    /// # Errors
    ///
    /// As [`PluginProcess::list_tools`]. Calling `shutdown` on a process that is already dead is
    /// not an error; there is nothing left to stop.
    pub fn shutdown(mut self) -> Result<(), SupervisorError> {
        if self.dead {
            return Ok(());
        }

        let id = RequestId::from(self.next_id());
        let request = Request::shutdown(id.clone());
        let _: Value = self.exchange(Method::Shutdown, &id, &request)?;
        self.finish_cleanly();
        Ok(())
    }

    fn initialize(&mut self, expected: ApiVersion) -> Result<(), SupervisorError> {
        let id = RequestId::from(self.next_id());
        let request = Request::initialize(
            id.clone(),
            InitializeParams {
                plugin_api_version: expected.to_string(),
                host_capabilities: Vec::new(),
            },
        );
        let result: InitializeResult = self.exchange(Method::Initialize, &id, &request)?;

        let Ok(reported) = ApiVersion::parse(&result.plugin_api_version) else {
            let detail = format!(
                "the plugin reported plugin_api_version `{}`, which is not a single decimal \
                 version",
                result.plugin_api_version
            );
            return Err(self.fail(SupervisorError::Handshake { detail }));
        };
        match negotiate(reported, ApiVersion::CURRENT) {
            Ok(version) if version == expected => {}
            Ok(version) => {
                let detail = format!(
                    "the plugin's manifest declares version {expected}, but the handshake reported \
                     {version}"
                );
                return Err(self.fail(SupervisorError::Handshake { detail }));
            }
            Err(PluginError::ApiVersionMismatch { plugin, host }) => {
                return Err(self.fail(SupervisorError::ApiVersion { plugin, host }));
            }
            Err(error) => {
                return Err(self.fail(SupervisorError::Handshake {
                    detail: error.to_string(),
                }));
            }
        }

        self.capabilities = result.capabilities;
        Ok(())
    }

    /// Send one request and wait for the response with the same id.
    ///
    /// The deadline covers writing the request and every notification that arrives before the
    /// response. A plugin cannot extend it by logging: notification traffic is counted against the
    /// manifest's output cap.
    fn exchange<P, T>(
        &mut self,
        method: Method,
        id: &RequestId,
        request: &Request<P>,
    ) -> Result<T, SupervisorError>
    where
        P: Serialize,
        T: DeserializeOwned,
    {
        if self.dead {
            return Err(SupervisorError::NotRunning);
        }

        let codec = self.codec;
        let line = match codec.encode_line(request) {
            Ok(line) => line,
            Err(error) => {
                let detail = format!("the `{method}` request could not be encoded: {error}");
                return Err(self.fail(SupervisorError::Protocol { detail }));
            }
        };
        self.send(line.into_bytes())?;

        let deadline = Instant::now() + self.timeout;
        let mut notification_bytes = 0_usize;
        loop {
            let now = Instant::now();
            let Some(remaining) = deadline.checked_duration_since(now) else {
                return Err(self.timeout(method));
            };
            let event = match self.inbox.recv_timeout(remaining) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => return Err(self.timeout(method)),
                Err(RecvTimeoutError::Disconnected) => return Err(self.exited()),
            };

            match event {
                Incoming::Response {
                    id: response_id,
                    result,
                } => {
                    if &response_id != id {
                        let detail = format!(
                            "the plugin answered `{method}` with id {response_id}, but the request \
                             id was {id}"
                        );
                        return Err(self.protocol(detail));
                    }
                    return match result {
                        Ok(value) => self.decode(method, value),
                        Err(error) => {
                            let detail = format!(
                                "the plugin rejected `{method}` with JSON-RPC error {}: {}",
                                error.code, error.message
                            );
                            match method {
                                Method::Initialize => {
                                    Err(self.fail(SupervisorError::Handshake { detail }))
                                }
                                _ => Err(self.protocol(detail)),
                            }
                        }
                    };
                }
                Incoming::Log { params, bytes } => {
                    notification_bytes = notification_bytes.saturating_add(bytes);
                    self.capture_log(params.level, &params.message);
                    if notification_bytes > self.max_output_bytes {
                        return Err(self.output_limit(
                            "the plugin wrote more log output during the call than the manifest \
                             allows",
                        ));
                    }
                }
                Incoming::Approval { id, params, bytes } => {
                    notification_bytes = notification_bytes.saturating_add(bytes);
                    if notification_bytes > self.max_output_bytes {
                        return Err(self.output_limit(
                            "the plugin wrote more approval output during the call than the \
                             manifest allows",
                        ));
                    }
                    self.deny_approval(id, &params)?;
                }
                Incoming::Protocol { detail } => return Err(self.protocol(detail)),
                Incoming::OutputLimit { detail } => return Err(self.output_limit(detail)),
                Incoming::Eof => return Err(self.exited()),
                Incoming::Io { operation, source } => return Err(self.io(operation, source)),
            }
        }
    }

    fn decode<T: DeserializeOwned>(
        &mut self,
        method: Method,
        value: Value,
    ) -> Result<T, SupervisorError> {
        if json_depth(&value) > MAX_RESULT_DEPTH {
            let detail = format!(
                "the `{method}` result nests deeper than the {MAX_RESULT_DEPTH}-level limit"
            );
            return Err(self.output_limit(detail));
        }
        match serde_json::from_value(value) {
            Ok(value) => Ok(value),
            Err(error) => {
                let detail = format!(
                    "the `{method}` result does not have the shape the protocol requires: {error}"
                );
                Err(self.protocol(detail))
            }
        }
    }

    fn deny_approval(
        &mut self,
        id: RequestId,
        params: &aicontext_plugin_sdk::ApprovalParams,
    ) -> Result<(), SupervisorError> {
        self.capture_log(
            LogLevel::Warning,
            &format!(
                "denied the plugin's request for approval of `{}`; the host decides before the \
                 call",
                params.tool
            ),
        );
        let response = Response::success(
            id,
            ApprovalResult {
                approved: false,
                reason: Some(
                    "plugin-initiated approval is not supported; the host decides before the call"
                        .to_owned(),
                ),
            },
        );
        let codec = self.codec;
        let line = match codec.encode_line(&response) {
            Ok(line) => line,
            Err(error) => {
                let detail = format!("the approval denial could not be encoded: {error}");
                return Err(self.protocol(detail));
            }
        };
        self.send(line.into_bytes())
    }

    fn send(&mut self, bytes: Vec<u8>) -> Result<(), SupervisorError> {
        let sent = match self.writer.as_ref() {
            Some(writer) => writer.send(bytes),
            None => return Err(SupervisorError::NotRunning),
        };
        if sent.is_err() {
            return Err(self.exited());
        }
        Ok(())
    }

    fn capture_log(&mut self, level: LogLevel, message: &str) {
        if self.logs.len() == MAX_LOGS {
            let _ = self.logs.pop_front();
        }
        self.logs.push_back(PluginLog {
            level,
            message: truncate_message(message),
        });
    }

    fn next_id(&mut self) -> u64 {
        self.next_id = self.next_id.saturating_add(1);
        self.next_id
    }

    fn finish_cleanly(&mut self) {
        self.dead = true;
        // Closing stdin is the first half of a clean stop: a plugin that reads its input to EOF
        // sees the end of the session rather than a broken pipe.
        self.writer.take();
        if !self.wait_for_exit(SHUTDOWN_GRACE) {
            let _ = self.child.kill();
            self.wait_for_exit(KILL_GRACE);
        }
        self.drain_inbox();
    }

    fn terminate(&mut self) {
        if self.dead {
            return;
        }
        self.dead = true;
        self.writer.take();
        let _ = self.child.kill();
        self.wait_for_exit(KILL_GRACE);
        self.drain_inbox();
    }

    /// Drain parsed events so a reader blocked on the bounded channel can notice that the process
    /// is gone. The channel is not part of the protocol state after a failure.
    fn drain_inbox(&mut self) {
        while self.inbox.try_recv().is_ok() {}
    }

    /// Wait for the child to exit without ever blocking indefinitely.
    fn wait_for_exit(&mut self, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(_) => return false,
            }
            let now = Instant::now();
            let Some(remaining) = deadline.checked_duration_since(now) else {
                return false;
            };
            thread::sleep(remaining.min(POLL_INTERVAL));
        }
    }

    fn timeout(&mut self, method: Method) -> SupervisorError {
        let timeout_ms = u64::try_from(self.timeout.as_millis()).unwrap_or(u64::MAX);
        self.terminate();
        SupervisorError::Timeout {
            method: method.as_str(),
            timeout_ms,
        }
    }

    fn output_limit(&mut self, detail: impl Into<String>) -> SupervisorError {
        let limit = self.max_output_bytes;
        self.terminate();
        SupervisorError::OutputLimit {
            limit,
            detail: detail.into(),
        }
    }

    fn protocol(&mut self, detail: impl Into<String>) -> SupervisorError {
        self.terminate();
        SupervisorError::Protocol {
            detail: detail.into(),
        }
    }

    fn exited(&mut self) -> SupervisorError {
        let status = match self.child.try_wait() {
            Ok(Some(status)) => match status.code() {
                Some(code) => format!("exit code {code}"),
                None => "terminated by a signal".to_owned(),
            },
            Ok(None) => "still running, but its stdout closed".to_owned(),
            Err(error) => format!("exit status unavailable: {error}"),
        };
        self.terminate();
        SupervisorError::Exited { status }
    }

    fn io(&mut self, operation: &'static str, source: io::Error) -> SupervisorError {
        self.terminate();
        SupervisorError::Io { operation, source }
    }

    /// Stop the process and return `error`, for a failure that leaves the stream unusable.
    fn fail(&mut self, error: SupervisorError) -> SupervisorError {
        self.terminate();
        error
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// One diagnostic line a plugin sent through the `log` notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginLog {
    level: LogLevel,
    message: String,
}

impl PluginLog {
    /// The level the plugin asked the host to use.
    #[must_use]
    pub const fn level(&self) -> LogLevel {
        self.level
    }

    /// The diagnostic text, truncated to 512 bytes.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Kill a child that could not be wired up. This path is unreachable when `Stdio::piped()` works,
/// but leaving a half-started process behind would be worse than an ignored error.
fn abort_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.try_wait();
}

fn truncate_message(message: &str) -> String {
    if message.len() <= MAX_LOG_MESSAGE_BYTES {
        return message.to_owned();
    }
    let mut end = MAX_LOG_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = message[..end].to_owned();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::{MAX_RESULT_DEPTH, truncate_message};

    #[test]
    fn a_long_log_message_is_truncated_on_a_character_boundary() {
        let message = "a".repeat(MAX_RESULT_DEPTH) + &"é".repeat(400);
        let truncated = truncate_message(&message);
        assert!(truncated.ends_with("..."));
        assert!(truncated.len() <= 512 + 3);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
