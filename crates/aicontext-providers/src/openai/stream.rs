//! Turns a chat-completions SSE stream into `StreamEvent`s.
//!
//! The pump — the reader task, the channel, the cancellation race — is [`crate::transport::stream`],
//! shared by all three adapters. What is here is what "chat completions" means on the wire: a
//! `choices[].delta` carrying a text fragment or a tool-call fragment, an optional `usage`, and the
//! `data: [DONE]` sentinel that ends the stream.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::provider::BoxStream;
use crate::transport::stream;
use crate::types::StreamEvent;

use super::wire;

/// Opens the pump over a chat-completions response, carrying the per-stream tool-name memory.
pub(super) fn open(
    response: reqwest::Response,
    cancel: crate::cancel::CancellationToken,
) -> BoxStream<StreamEvent> {
    // A tool call is named on its first fragment and its arguments follow, so a reader that keeps
    // no state still sees the name on every fragment (`docs/AI_PROVIDER_SPEC.md` §2).
    let mut tools = ToolNames::new();
    stream::open(response, cancel, move |payload| {
        translate(payload, &mut tools)
    })
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
                id.clone_into(&mut named.0);
            }
        }
        let function = call.get("function");
        if let Some(name) = function
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            if !name.is_empty() {
                name.clone_into(&mut named.1);
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

#[cfg(test)]
mod tests {
    use super::translate;
    use crate::types::{FinishReason, StreamEvent, Usage};

    fn one(payload: &str) -> Vec<StreamEvent> {
        let mut tools = super::ToolNames::new();
        translate(payload, &mut tools).expect("a parseable payload")
    }

    #[test]
    fn the_done_sentinel_ends_the_stream() {
        let mut tools = super::ToolNames::new();
        assert!(translate("[DONE]", &mut tools).is_none());
    }

    #[test]
    fn a_text_fragment_becomes_a_delta() {
        assert_eq!(
            one(r#"{"choices":[{"delta":{"content":"hi"}}]}"#),
            vec![StreamEvent::TextDelta("hi".to_owned())]
        );
    }

    #[test]
    fn usage_and_finish_are_reported() {
        let events = one(r#"{"choices":[{"delta":{},"finish_reason":"stop"}],
                 "usage":{"prompt_tokens":1,"completion_tokens":2}}"#);
        assert_eq!(
            events,
            vec![
                StreamEvent::Usage(Usage {
                    input_tokens: Some(1),
                    output_tokens: Some(2),
                    cached_input_tokens: None,
                    estimated: false,
                }),
                StreamEvent::Finish(FinishReason::Stop),
            ]
        );
    }

    #[test]
    fn a_tool_call_is_named_once_and_repeated_on_each_fragment() {
        let mut tools = super::ToolNames::new();
        let first = translate(
            r#"{"choices":[{"delta":{"tool_calls":[
                 {"index":0,"id":"call_1","function":{"name":"stop_ec2","arguments":""}}]}}]}"#,
            &mut tools,
        )
        .expect("first fragment");
        let second = translate(
            r#"{"choices":[{"delta":{"tool_calls":[
                 {"index":0,"function":{"arguments":"{\"id\":\"i-9\"}"}}]}}]}"#,
            &mut tools,
        )
        .expect("second fragment");
        assert_eq!(
            first,
            vec![StreamEvent::ToolCallDelta {
                id: "call_1".to_owned(),
                tool: "stop_ec2".to_owned(),
                arguments_delta: String::new(),
            }]
        );
        assert_eq!(
            second,
            vec![StreamEvent::ToolCallDelta {
                id: "call_1".to_owned(),
                tool: "stop_ec2".to_owned(),
                arguments_delta: "{\"id\":\"i-9\"}".to_owned(),
            }],
            "the id and name set on the first fragment must survive into the second"
        );
    }

    #[test]
    fn a_payload_that_is_not_json_ends_the_stream_rather_than_inventing_an_event() {
        let mut tools = super::ToolNames::new();
        assert!(translate("not json", &mut tools).is_none());
    }
}
