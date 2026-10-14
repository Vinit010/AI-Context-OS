//! Turns a streaming HTTP response into a `StreamEvent` stream.
//!
//! The bytes arrive on the reader's task, so a small task reads them and forwards parsed events
//! over a channel; dropping the returned stream drops the receiver, the next send fails, and the
//! task ends with the response. That is what makes cancellation and an abandoned stream both
//! bounded (`docs/AI_PROVIDER_SPEC.md` §6).

use std::collections::BTreeMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::cancel::CancellationToken;
use crate::provider::BoxStream;
use crate::types::StreamEvent;

use super::sse::SseParser;
use super::wire;

/// How many parsed events may wait before the reader is back-pressured.
const CAPACITY: usize = 32;

/// A [`StreamEvent`] stream backed by the reader task; the task owns the HTTP response.
pub(super) fn open(response: reqwest::Response, cancel: CancellationToken) -> BoxStream<StreamEvent> {
    let (sender, receiver) = mpsc::channel::<StreamEvent>(CAPACITY);
    tokio::spawn(async move {
        let mut response = response;
        let mut parser = SseParser::new();
        let mut tools = BTreeMap::new();
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                chunk = response.chunk() => match chunk {
                    Ok(Some(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        parser.push(&text);
                        while let Some(payload) = parser.next() {
                            let Some(events) = translate(&payload, &mut tools) else {
                                return;
                            };
                            for event in events {
                                if sender.send(event).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    // A clean end, a transport error, and a dropping receiver all end the stream:
                    // `StreamEvent` has no error item, so there is nothing to report and nothing to
                    // invent (docs/AI_PROVIDER_SPEC.md §2).
                    Ok(None) | Err(_) => return,
                },
            }
        }
    });
    Box::pin(EventReceiver { receiver })
}

/// The named tip of a tool call, repeated on each fragment so a reader keeps no state.
type ToolNames = BTreeMap<u64, (String, String)>;

/// Parses one `data:` payload, or `None` at `[DONE]` or a chunk that is not JSON.
fn translate(payload: &str, tools: &mut ToolNames) -> Option<Vec<StreamEvent>> {
    if payload.trim() == "[DONE]" {
        return None;
    }
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let mut events = Vec::new();
    if value.get("usage").is_some_and(|usage| !usage.is_null()) {
        events.push(StreamEvent::Usage(wire::parse_usage(value.get("usage"))));
    }
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(delta) = choice.get("delta") {
                if let Some(content) = delta.get("content").and_then(Value::as_str) {
                    if !content.is_empty() {
                        events.push(StreamEvent::TextDelta(content.to_owned()));
                    }
                }
                collect_tool_calls(delta, tools, &mut events);
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                events.push(StreamEvent::Finish(wire::finish_reason(reason)));
            }
        }
    }
    Some(events)
}

/// Appends one `ToolCallDelta` per wire fragment, filling the id and name from earlier fragments.
fn collect_tool_calls(delta: &Value, tools: &mut ToolNames, events: &mut Vec<StreamEvent>) {
    let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) else {
        return;
    };
    for call in calls {
        let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
        let named = tools.entry(index).or_default();
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                named.0 = id.to_owned();
            }
        }
        let function = call.get("function");
        if let Some(name) = function
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            if !name.is_empty() {
                named.1 = name.to_owned();
            }
        }
        let arguments = function
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        events.push(StreamEvent::ToolCallDelta {
            id: named.0.clone(),
            tool: named.1.clone(),
            arguments_delta: arguments.to_owned(),
        });
    }
}

/// A `Stream` over the receiver end of the reader channel.
struct EventReceiver {
    receiver: mpsc::Receiver<StreamEvent>,
}

impl Stream for EventReceiver {
    type Item = StreamEvent;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(context)
    }
}
