//! Turns a `streamGenerateContent` SSE stream into `StreamEvent`s.
//!
//! The pump is [`crate::transport::stream`], shared by all three adapters. What is here is what
//! Gemini's stream means: each `data:` line is a whole response *chunk* — a candidate with parts, a
//! `finishReason`, and a `usageMetadata` — rather than an increment, so a `functionCall` arrives
//! complete in one chunk and its arguments are serialized into one delta rather than accumulated.
//!
//! The finish reason carries the same wrinkle as the non-streaming path: `STOP` with a function
//! call in the stream is tool use. Because the call and the reason usually arrive in different
//! chunks, the translator remembers whether it has seen a call.

use serde_json::Value;

use crate::provider::BoxStream;
use crate::transport::stream;
use crate::types::{StreamEvent, Usage};

use super::wire;

/// Opens the pump over a `streamGenerateContent` response, carrying the per-stream tool memory.
pub(super) fn open(
    response: reqwest::Response,
    cancel: crate::cancel::CancellationToken,
) -> BoxStream<StreamEvent> {
    let mut state = State::default();
    stream::open(response, cancel, move |payload| {
        translate(payload, &mut state)
    })
}

/// What the stream has revealed so far, for the one fact a later chunk needs.
#[derive(Default)]
struct State {
    /// Whether any chunk has carried a `functionCall`, so a `STOP` later means tool use.
    saw_tool_call: bool,
}

/// Parses one `data:` payload; a chunk with no candidates contributes nothing.
fn translate(payload: &str, state: &mut State) -> Option<Vec<StreamEvent>> {
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let mut events = Vec::new();
    let candidate = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first());
    if let Some(candidate) = candidate {
        if let Some(parts) = candidate.pointer("/content/parts").and_then(Value::as_array) {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        events.push(StreamEvent::TextDelta(text.to_owned()));
                    }
                }
                if let Some(call) = part.get("functionCall") {
                    state.saw_tool_call = true;
                    // The arguments arrive as a whole object, so one delta carries all of it; the
                    // JSON is re-rendered compactly so a reader concatenating fragments gets valid
                    // JSON without caring how it was spaced on the wire.
                    let tool = call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let arguments = call
                        .get("args")
                        .map(|args| serde_json::to_string(args).unwrap_or_default())
                        .unwrap_or_default();
                    events.push(StreamEvent::ToolCallDelta {
                        // Gemini assigns no id to a function call; the empty string is the truth
                        // rather than an invention, and the gateway remaps ids before dispatch.
                        id: String::new(),
                        tool,
                        arguments_delta: arguments,
                    });
                }
            }
        }
        if let Some(usage) = value.get("usageMetadata") {
            events.push(StreamEvent::Usage(parse_usage(usage)));
        }
        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
            events.push(StreamEvent::Finish(wire::finish_reason(
                reason,
                state.saw_tool_call,
            )));
        }
    }
    Some(events)
}

/// Reads the chunk's usage, tolerating one that reports none.
fn parse_usage(usage: &Value) -> Usage {
    Usage {
        input_tokens: usage
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok()),
        output_tokens: usage
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok()),
        cached_input_tokens: usage
            .get("cachedContentTokenCount")
            .and_then(Value::as_u64)
            .and_then(|number| u32::try_from(number).ok()),
        estimated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{translate, State};
    use crate::types::{FinishReason, StreamEvent, Usage};

    fn one(payload: &str) -> Vec<StreamEvent> {
        let mut state = State::default();
        translate(payload, &mut state).expect("a parseable payload")
    }

    #[test]
    fn a_text_chunk_becomes_a_delta() {
        assert_eq!(
            one(r#"{"candidates":[{"content":{"parts":[{"text":"Hel"}],"role":"model"}}]}"#),
            vec![StreamEvent::TextDelta("Hel".to_owned())]
        );
    }

    #[test]
    fn usage_and_finish_arrive_together_in_the_last_chunk() {
        let events = one(
            r#"{"candidates":[{"content":{"parts":[],"role":"model"},"finishReason":"STOP"}],
                 "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2}}"#,
        );
        assert_eq!(
            events,
            vec![
                StreamEvent::Usage(Usage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    cached_input_tokens: None,
                    estimated: false,
                }),
                StreamEvent::Finish(FinishReason::Stop),
            ]
        );
    }

    #[test]
    fn a_function_call_is_one_delta_carrying_whole_arguments() {
        let events = one(
            r#"{"candidates":[{"content":{"parts":[
                 {"functionCall":{"name":"aws.ec2.stop","args":{"instance_id":"i-9"}}}],
                 "role":"model"}}]}"#,
        );
        assert_eq!(
            events,
            vec![StreamEvent::ToolCallDelta {
                id: String::new(),
                tool: "aws.ec2.stop".to_owned(),
                arguments_delta: "{\"instance_id\":\"i-9\"}".to_owned(),
            }]
        );
    }

    #[test]
    fn a_stop_after_a_function_call_is_tool_use() {
        let mut state = State::default();
        translate(
            r#"{"candidates":[{"content":{"parts":[
                 {"functionCall":{"name":"stop","args":{}}}],"role":"model"}}]}"#,
            &mut state,
        )
        .expect("the call chunk");
        let events = translate(
            r#"{"candidates":[{"content":{"parts":[],"role":"model"},
                 "finishReason":"STOP"}]}"#,
            &mut state,
        )
        .expect("the finish chunk");
        assert_eq!(events, vec![StreamEvent::Finish(FinishReason::ToolUse)]);
    }

    #[test]
    fn a_chunk_with_no_candidates_contributes_nothing() {
        assert!(one(r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#).is_empty());
    }

    #[test]
    fn a_payload_that_is_not_json_ends_the_stream() {
        let mut state = State::default();
        assert!(translate("not json", &mut state).is_none());
    }
}
