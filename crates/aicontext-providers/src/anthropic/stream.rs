//! Turns a Messages API SSE stream into `StreamEvent`s.
//!
//! The pump is [`crate::transport::stream`], shared by all three adapters. What is here is what the
//! Messages API's stream means: `message_start` carries the input-token count, `content_block_delta`
//! carries a text fragment or a JSON fragment of a tool call's arguments, and `message_delta`
//! carries the stop reason and the output-token count — so usage arrives split across the first and
//! last events, and is reported once, at the end, with both halves.
//!
//! Anthropic also labels each event twice, in the `event:` field and in the payload's `type`. The
//! pump reads only `data:`, so the payload's own `type` is what this module dispatches on and
//! nothing is lost.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::provider::BoxStream;
use crate::transport::stream;
use crate::types::StreamEvent;

use super::wire;

/// Opens the pump over a Messages API response, carrying the per-stream block memory.
pub(super) fn open(
    response: reqwest::Response,
    cancel: crate::cancel::CancellationToken,
) -> BoxStream<StreamEvent> {
    let mut blocks = Blocks::new();
    stream::open(response, cancel, move |payload| {
        translate(payload, &mut blocks)
    })
}

/// What one content block has revealed so far: a tool-use block is named when it starts and its
/// arguments then arrive as fragments.
#[derive(Default)]
struct Blocks {
    /// Index to `(id, name)`, for the tool-use blocks seen so far.
    tools: BTreeMap<u64, (String, String)>,
    /// The input-token count from `message_start`, held until `message_delta` reports both halves.
    input_tokens: Option<u32>,
}

impl Blocks {
    fn new() -> Self {
        Self::default()
    }
}

/// Parses one `data:` payload; an unknown event contributes nothing and ends nothing.
fn translate(payload: &str, blocks: &mut Blocks) -> Option<Vec<StreamEvent>> {
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let mut events = Vec::new();
    match value.get("type").and_then(Value::as_str) {
        Some("message_start") => {
            blocks.input_tokens = value
                .pointer("/message/usage/input_tokens")
                .and_then(Value::as_u64)
                .and_then(|number| u32::try_from(number).ok());
        }
        Some("content_block_start") => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
            if value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") {
                let id = value
                    .pointer("/content_block/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let name = value
                    .pointer("/content_block/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                blocks.tools.insert(index, (id, name));
            }
        }
        Some("content_block_delta") => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
            let delta = value.get("delta")?;
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            events.push(StreamEvent::TextDelta(text.to_owned()));
                        }
                    }
                }
                Some("input_json_delta") => {
                    let (id, tool) = blocks
                        .tools
                        .get(&index)
                        .cloned()
                        .unwrap_or_default();
                    let arguments = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    events.push(StreamEvent::ToolCallDelta {
                        id,
                        tool,
                        arguments_delta: arguments.to_owned(),
                    });
                }
                // `thinking_delta` and any future delta type contribute no neutral event; the
                // payload stays in the provider's own stream, which this adapter does not reinterpret.
                _ => {}
            }
        }
        Some("message_delta") => {
            // Usage is split: input tokens arrive with `message_start`, output tokens here.
            let output = value
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .and_then(|number| u32::try_from(number).ok());
            if blocks.input_tokens.is_some() || output.is_some() {
                events.push(StreamEvent::Usage(crate::types::Usage {
                    input_tokens: blocks.input_tokens,
                    output_tokens: output,
                    cached_input_tokens: None,
                    estimated: false,
                }));
            }
            if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                events.push(StreamEvent::Finish(wire::finish_reason(reason)));
            }
        }
        // `message_stop` marks the end; the reader ends when the bytes do.
        _ => {}
    }
    Some(events)
}

#[cfg(test)]
mod tests {
    use super::{translate, Blocks};
    use crate::types::{FinishReason, StreamEvent, Usage};

    fn one(payload: &str) -> Vec<StreamEvent> {
        let mut blocks = Blocks::new();
        translate(payload, &mut blocks).expect("a parseable payload")
    }

    #[test]
    fn text_fragments_become_deltas() {
        assert_eq!(
            one(r#"{"type":"content_block_delta","index":0,
                 "delta":{"type":"text_delta","text":"Hel"}}"#),
            vec![StreamEvent::TextDelta("Hel".to_owned())]
        );
    }

    #[test]
    fn usage_is_reported_once_with_both_halves() {
        let mut blocks = Blocks::new();
        assert!(translate(
            r#"{"type":"message_start","message":{"usage":{"input_tokens":3}}}"#,
            &mut blocks
        )
        .expect("message_start")
        .is_empty());
        let events = translate(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},
                 "usage":{"output_tokens":5}}"#,
            &mut blocks,
        )
        .expect("message_delta");
        assert_eq!(
            events,
            vec![
                StreamEvent::Usage(Usage {
                    input_tokens: Some(3),
                    output_tokens: Some(5),
                    cached_input_tokens: None,
                    estimated: false,
                }),
                StreamEvent::Finish(FinishReason::Stop),
            ]
        );
    }

    #[test]
    fn a_tool_call_is_named_when_its_block_starts_and_repeated_on_each_fragment() {
        let mut blocks = Blocks::new();
        assert!(translate(
            r#"{"type":"content_block_start","index":0,
                 "content_block":{"type":"tool_use","id":"toolu_1","name":"aws.ec2.stop"}}"#,
            &mut blocks
        )
        .expect("block start")
        .is_empty());
        let first = translate(
            r#"{"type":"content_block_delta","index":0,
                 "delta":{"type":"input_json_delta","partial_json":"{\"instance"}}"#,
            &mut blocks,
        )
        .expect("first fragment");
        let second = translate(
            r#"{"type":"content_block_delta","index":0,
                 "delta":{"type":"input_json_delta","partial_json":"_id\":\"i-9\"}"}}"#,
            &mut blocks,
        )
        .expect("second fragment");
        assert_eq!(
            first,
            vec![StreamEvent::ToolCallDelta {
                id: "toolu_1".to_owned(),
                tool: "aws.ec2.stop".to_owned(),
                arguments_delta: "{\"instance".to_owned(),
            }]
        );
        assert_eq!(
            second,
            vec![StreamEvent::ToolCallDelta {
                id: "toolu_1".to_owned(),
                tool: "aws.ec2.stop".to_owned(),
                arguments_delta: "_id\":\"i-9\"}".to_owned(),
            }],
            "the id and name from block_start must survive into every fragment"
        );
    }

    #[test]
    fn an_unknown_event_contributes_nothing_and_ends_nothing() {
        assert!(one(r#"{"type":"ping"}"#).is_empty());
        assert!(one(r#"{"type":"content_block_delta","index":0,
            "delta":{"type":"thinking_delta","thinking":"hmm"}}"#)
        .is_empty());
    }

    #[test]
    fn a_payload_that_is_not_json_ends_the_stream() {
        let mut blocks = Blocks::new();
        assert!(translate("not json", &mut blocks).is_none());
    }
}
