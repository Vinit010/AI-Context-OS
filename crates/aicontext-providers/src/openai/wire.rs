//! Translation between the neutral types and the `OpenAI` chat-completions wire shape.
//!
//! Everything provider-shaped lives here, and one rule governs the whole file: the two regions are
//! mapped to two roles. `PolicyRegion` becomes `system` messages; `DataBlock`s become a `user`
//! message wrapped in an envelope. A `DataBlock` can therefore never reach a `system` message
//! (`docs/AI_PROVIDER_SPEC.md` §2.1, §11 T9).

use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::error::ProviderError;
use crate::types::{
    CompletionRequest, CompletionResponse, DataBlock, FinishReason, Message, ProviderId, Role,
    ToolCall, ToolSpec, Usage,
};

/// The path every OpenAI-compatible server exposes, relative to `base_url`.
pub(super) const PATH: &str = "/chat/completions";

/// Builds the JSON body for a chat-completions call.
pub(super) fn request_body(request: &CompletionRequest, model: &str, stream: bool) -> Value {
    let mut messages = Vec::new();
    if let Some(instruction) = &request.policy.instruction {
        messages.push(system(instruction));
    }
    for block in &request.policy.policy {
        messages.push(system(&block.content));
    }
    for message in &request.messages {
        messages.push(message_json(message));
    }
    if !request.data.is_empty() {
        messages.push(serde_json::json!({
            "role": "user",
            "content": render_data(&request.data),
        }));
    }

    let mut body = Map::new();
    body.insert("model".to_owned(), Value::from(model));
    body.insert("messages".to_owned(), Value::Array(messages));
    // T11: the ceiling is `max_tokens`, and it is always sent.
    body.insert(
        "max_tokens".to_owned(),
        Value::from(request.budget.max_output_tokens),
    );
    if let Some(temperature) = request.options.temperature {
        if let Some(number) = serde_json::Number::from_f64(f64::from(temperature)) {
            body.insert("temperature".to_owned(), Value::Number(number));
        }
    }
    if let Some(seed) = request.options.seed {
        body.insert("seed".to_owned(), Value::from(seed));
    }
    if !request.options.stop.is_empty() {
        body.insert("stop".to_owned(), serde_json::json!(request.options.stop));
    }
    if !request.tools.is_empty() {
        body.insert(
            "tools".to_owned(),
            Value::Array(request.tools.iter().map(tool_json).collect()),
        );
    }
    if stream {
        body.insert("stream".to_owned(), Value::Bool(true));
    }
    Value::Object(body)
}

/// A `system` message, the only shape trusted content can take.
fn system(content: &str) -> Value {
    serde_json::json!({ "role": "system", "content": content })
}

/// One conversation message.
fn message_json(message: &Message) -> Value {
    let mut object = Map::new();
    object.insert("role".to_owned(), Value::from(role_name(message.role)));
    object.insert("content".to_owned(), Value::from(message.content.clone()));
    if let Some(id) = &message.tool_call_id {
        object.insert("tool_call_id".to_owned(), Value::from(id.clone()));
    }
    Value::Object(object)
}

/// The wire name of a role.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// One tool declaration, in the `function` shape.
fn tool_json(tool: &ToolSpec) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        }
    })
}

/// Renders the untrusted `DataBlock`s into one envelope, never into a forwardable instruction.
fn render_data(blocks: &[DataBlock]) -> String {
    let mut rendered = String::new();
    for (position, block) in blocks.iter().enumerate() {
        if position > 0 {
            rendered.push_str("\n\n");
        }
        let _ = write!(
            rendered,
            "<untrusted-data source=\"{}\" path=\"{}\" tier=\"{}\" trust=\"untrusted\">\n{}\n\
             </untrusted-data>",
            escape(&block.source),
            escape(&block.path),
            block.tier,
            block.content,
        );
    }
    rendered
}

/// Escapes the four characters that can end the envelope's attribute or open one.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Parses a non-streaming chat-completions body into the neutral response.
pub(super) fn parse_completion(
    provider: &ProviderId,
    bytes: &[u8],
) -> Result<CompletionResponse, ProviderError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| malformed(provider, &error.to_string()))?;
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| malformed(provider, "the response carried no choices"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| malformed(provider, "the first choice carried no message"))?;
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tool_calls = parse_tool_calls(provider, message)?;
    let finish_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map_or(FinishReason::Stop, finish_reason);
    let usage = parse_usage(value.get("usage"));
    Ok(CompletionResponse {
        text,
        tool_calls,
        finish_reason,
        usage,
        provider_meta: value,
    })
}

/// Parses the tool calls of a message, if any.
fn parse_tool_calls(
    provider: &ProviderId,
    message: &Value,
) -> Result<Vec<ToolCall>, ProviderError> {
    let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    calls
        .iter()
        .map(|call| parse_tool_call(provider, call))
        .collect()
}

/// Parses one tool call, keeping its raw payload for the audit trail.
fn parse_tool_call(provider: &ProviderId, call: &Value) -> Result<ToolCall, ProviderError> {
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let function = call
        .get("function")
        .ok_or_else(|| malformed(provider, "a tool call carried no function"))?;
    let tool = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(provider, "a tool call carried no function name"))?
        .to_owned();
    let arguments = match function.get("arguments") {
        Some(Value::String(text)) => parse_arguments(provider, text)?,
        Some(other) => other.clone(),
        None => Value::Null,
    };
    Ok(ToolCall {
        id,
        tool,
        arguments,
        raw: call.clone(),
    })
}

/// Parses the JSON-encoded arguments of a tool call, which newer models stream as text.
fn parse_arguments(provider: &ProviderId, text: &str) -> Result<Value, ProviderError> {
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(text).map_err(|error| malformed(provider, &error.to_string()))
}

/// The neutral meaning of a wire finish reason, preserving one this version does not know (T12).
pub(super) fn finish_reason(raw: &str) -> FinishReason {
    match raw {
        "stop" => FinishReason::Stop,
        "tool_calls" | "function_call" => FinishReason::ToolUse,
        "length" => FinishReason::Length,
        "content_filter" | "refusal" => FinishReason::Refusal,
        "error" => FinishReason::Error,
        other => FinishReason::Unknown(other.to_owned()),
    }
}

/// Reads provider usage, tolerating a server that reports none (T4).
pub(super) fn parse_usage(usage: Option<&Value>) -> Usage {
    let Some(usage) = usage else {
        return Usage::default();
    };
    Usage {
        input_tokens: count(usage, "prompt_tokens"),
        output_tokens: count(usage, "completion_tokens"),
        cached_input_tokens: usage
            .get("prompt_tokens_details")
            .and_then(|details| count(details, "cached_tokens"))
            .or_else(|| count(usage, "prompt_cache_hit_tokens")),
        estimated: false,
    }
}

/// A `u32` field, or `None` when it is absent or out of range.
fn count(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
}

/// The provider's own message for a failed request, capped so a body cannot flood an error.
pub(super) fn error_detail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            return truncate(message);
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return truncate(message);
        }
    }
    truncate(text.trim())
}

/// Caps `text` at a few hundred characters.
fn truncate(text: &str) -> String {
    const LIMIT: usize = 400;
    if text.chars().count() <= LIMIT {
        return text.to_owned();
    }
    let mut capped: String = text.chars().take(LIMIT).collect();
    capped.push('…');
    capped
}

/// A `MalformedResponse` naming the provider.
fn malformed(provider: &ProviderId, detail: &str) -> ProviderError {
    ProviderError::MalformedResponse {
        provider: provider.clone(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{finish_reason, parse_completion, parse_usage, request_body};
    use crate::types::{
        DataBlock, FinishReason, Message, ModelOptions, PolicyBlock, PolicyRegion, ProviderId,
        Role, TokenBudget, Trust,
    };

    fn provider() -> ProviderId {
        ProviderId::new("local").expect("valid")
    }

    fn request() -> crate::types::CompletionRequest {
        crate::types::CompletionRequest {
            policy: PolicyRegion {
                instruction: Some("do the task".to_owned()),
                policy: vec![PolicyBlock {
                    source: "RULES-001".to_owned(),
                    path: ".ai/RULES.md".to_owned(),
                    content: "be careful".to_owned(),
                }],
            },
            messages: vec![Message::new(Role::User, "hello".to_owned())],
            data: Vec::new(),
            tools: Vec::new(),
            budget: TokenBudget {
                max_output_tokens: 512,
            },
            options: ModelOptions::default(),
            idempotency_key: "key".to_owned(),
        }
    }

    #[test]
    fn a_data_block_is_never_a_system_message() {
        let mut request = request();
        request.data.push(DataBlock {
            source: "SPEC".to_owned(),
            path: ".ai/spec.md".to_owned(),
            tier: 4,
            trust: Trust::Untrusted,
            content: "ATTACK".to_owned(),
        });
        let body = request_body(&request, "model", false);
        let messages = body["messages"].as_array().expect("messages");
        for message in messages {
            if message["role"] == "system" {
                assert!(
                    !message["content"]
                        .as_str()
                        .unwrap_or_default()
                        .contains("ATTACK"),
                    "untrusted content reached the system role"
                );
            }
        }
        let rendered = messages
            .iter()
            .filter(|message| message["role"] == "user")
            .filter_map(|message| message["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("ATTACK"),
            "the content must still be sent"
        );
        assert!(rendered.contains("<untrusted-data"), "it must be enveloped");
    }

    #[test]
    fn the_ceiling_is_always_sent() {
        let body = request_body(&request(), "model", false);
        assert_eq!(body["max_tokens"], 512);
    }

    #[test]
    fn a_non_finite_temperature_is_dropped_rather_than_written() {
        let mut request = request();
        request.options.temperature = Some(f32::NAN);
        let body = request_body(&request, "model", false);
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn an_unknown_finish_reason_is_preserved() {
        assert_eq!(
            finish_reason("something_new"),
            FinishReason::Unknown("something_new".to_owned())
        );
    }

    #[test]
    fn an_absent_usage_is_not_an_estimate() {
        assert_eq!(parse_usage(None), crate::types::Usage::default());
    }

    #[test]
    fn a_malformed_body_is_a_typed_error() {
        let error = parse_completion(&provider(), b"not json").expect_err("must fail");
        assert_eq!(error.code(), "PROVIDER_MALFORMED_RESPONSE");
    }

    #[test]
    fn usage_reads_the_openai_keys() {
        let usage = parse_usage(Some(&serde_json::json!({
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "prompt_tokens_details": { "cached_tokens": 2 },
        })));
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(4));
        assert_eq!(usage.cached_input_tokens, Some(2));
        assert_eq!(finish_reason("length"), FinishReason::Length);
        let empty = parse_usage(Some(&serde_json::json!({})));
        assert_eq!(empty, crate::types::Usage::default());
        assert_eq!(
            parse_usage(None),
            crate::types::Usage::default(),
            "an unknown value is the default"
        );
    }
}
