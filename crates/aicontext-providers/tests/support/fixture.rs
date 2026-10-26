//! One fixture per adapter, so `docs/AI_PROVIDER_SPEC.md` §11's contract suite runs against all
//! three wire formats without being written three times.
//!
//! The contract is provider-neutral, so the tests that assert it must be too: each test in
//! `tests/contract.rs` loops over these fixtures, and everything that differs between the adapters
//! — the endpoint path, the wire spelling of a reply, a finish reason, or a usage count, the header
//! a credential goes in, where the policy and the data land in the body, the shape of a stream — is
//! a method here rather than a branch in a test. What stays in the test is the assertion, which is
//! the part that is genuinely the same for every adapter.

use std::time::Duration;

use aicontext_providers::anthropic::{Anthropic, AnthropicConfig};
use aicontext_providers::gemini::{Gemini, GeminiConfig};
use aicontext_providers::openai::{OpenAiCompatible, OpenAiCompatibleConfig};
use aicontext_providers::{
    AIProvider, Auth, CancellationToken, CompletionRequest, FinishReason, Message, ModelOptions,
    PolicyRegion, ProviderId, RetryPolicy, Role, Secret, StreamEvent, Timeouts, TokenBudget,
};
use serde_json::Value;

use super::Canned;

/// The knobs a contract test varies. Everything else about a provider is the adapter's own.
#[derive(Clone)]
pub(crate) struct Tweak {
    /// How long to wait.
    pub timeouts: Timeouts,
    /// How failures are repeated.
    pub retry: RetryPolicy,
    /// The token the whole run shares.
    pub cancel: CancellationToken,
    /// How to authenticate.
    pub auth: Auth,
}

impl Default for Tweak {
    fn default() -> Self {
        Self {
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            cancel: CancellationToken::new(),
            auth: Auth::None,
        }
    }
}

/// A retry policy whose backoff is negligible, so a retrying test stays fast and still asserts the
/// number of attempts.
pub(crate) fn tiny_retry(max_retries: u32) -> RetryPolicy {
    RetryPolicy {
        max_retries,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    }
}

/// One adapter, in every respect the contract suite needs to tell it apart from the other two.
pub(crate) trait Fixture {
    /// The adapter's name, so an assertion message says which of the three failed.
    fn name(&self) -> &'static str;

    /// The endpoint path a call lands on, for T1.
    fn path(&self) -> &'static str;

    /// Builds this adapter against `base_url`, with the knobs the test varies applied.
    fn build(&self, base_url: &str, tweak: &Tweak) -> Box<dyn AIProvider>;

    /// A body carrying `text` and the wire spelling of a finish reason, with no usage.
    fn reply(&self, text: &str, finish: &str) -> Value;

    /// The key this wire nests its usage under.
    fn usage_key(&self) -> &'static str;

    /// A usage object reporting 10 input, 4 output, and 2 cached tokens.
    fn usage(&self) -> Value;

    /// A body whose only tool call is `aws.ec2.stop` with `{"instance_id":"i-9"}`.
    fn tool_call(&self) -> Value;

    /// The finish reasons this wire spells, in a text-only context.
    fn finish_reasons(&self) -> &'static [(&'static str, FinishReason)];

    /// The credential variant this adapter is usually configured with.
    fn auth(&self, secret: Secret) -> Auth;

    /// The header the credential lands in on the wire.
    fn credential_header(&self) -> &'static str;

    /// The trusted region of a recorded body — the policy, which must never hold data (T9).
    fn trusted_text(&self, body: &Value) -> String;

    /// The untrusted conversation of a recorded body, where the data envelope must appear (T9).
    fn conversation_text(&self, body: &Value) -> String;

    /// The output ceiling as this wire spells it (T11).
    fn ceiling(&self, body: &Value) -> Value;

    /// The id this wire assigns to the one tool call in [`Fixture::tool_call`] (T2).
    fn tool_call_id(&self) -> &'static str;

    /// A canned SSE reply, and the events it must become: text, then a finish.
    fn text_stream(&self) -> (Canned, Vec<StreamEvent>);

    /// A canned SSE reply, and the events it must become: one tool call's fragments.
    fn tool_stream(&self) -> (Canned, Vec<StreamEvent>);

    /// The `Debug` rendering of this adapter's provider, for the redaction probe (T10).
    fn provider_debug(&self, base_url: &str, auth: Auth) -> String;

    /// A reply carrying this wire's spelling of the shared usage object.
    fn reply_with_usage(&self, text: &str, finish: &str) -> Value {
        let mut body = self.reply(text, finish);
        body[self.usage_key()] = self.usage();
        body
    }
}

/// The provider id every fixture registers under; the name is not what these tests assert on.
pub(crate) fn id() -> ProviderId {
    ProviderId::new("local").expect("a valid provider id")
}

/// The request every test starts from: one user message, an instruction, and an idempotency key
/// that makes it retryable.
pub(crate) fn request() -> CompletionRequest {
    CompletionRequest {
        policy: PolicyRegion {
            instruction: Some("answer the question".to_owned()),
            policy: Vec::new(),
        },
        messages: vec![Message::new(Role::User, "hello".to_owned())],
        data: Vec::new(),
        tools: Vec::new(),
        budget: TokenBudget {
            max_output_tokens: 256,
        },
        options: ModelOptions::default(),
        idempotency_key: "run-1".to_owned(),
    }
}

/// A request that must not be retried, for the tests that assert one attempt.
pub(crate) fn once() -> CompletionRequest {
    CompletionRequest {
        idempotency_key: String::new(),
        ..request()
    }
}

/// The chat-completions fixture: `OpenAI`, Ollama, vLLM, and every compatible server.
pub(crate) struct OpenAi;

/// The Anthropic Messages API fixture.
pub(crate) struct AnthropicFixture;

/// The Gemini `generateContent` fixture.
pub(crate) struct GeminiFixture;

/// All three fixtures, in the order `docs/AI_PROVIDER_SPEC.md` §3 lists the adapters.
pub(crate) fn all() -> Vec<Box<dyn Fixture>> {
    vec![
        Box::new(OpenAi),
        Box::new(AnthropicFixture),
        Box::new(GeminiFixture),
    ]
}

// -------------------------------------------------------------------------------------------------
// OpenAI-compatible
// -------------------------------------------------------------------------------------------------

impl Fixture for OpenAi {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }

    fn path(&self) -> &'static str {
        "/chat/completions"
    }

    fn build(&self, base_url: &str, tweak: &Tweak) -> Box<dyn AIProvider> {
        let config = OpenAiCompatibleConfig::new(id(), base_url, "test-model")
            .with_auth(tweak.auth.clone())
            .with_timeouts(tweak.timeouts)
            .with_retry(tweak.retry)
            .with_cancel(tweak.cancel.clone());
        Box::new(OpenAiCompatible::new(config).expect("the adapter builds"))
    }

    fn reply(&self, text: &str, finish: &str) -> Value {
        serde_json::json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": { "role": "assistant", "content": text },
                    "finish_reason": finish,
                }
            ],
        })
    }

    fn usage_key(&self) -> &'static str {
        "usage"
    }

    fn usage(&self) -> Value {
        serde_json::json!({
            "prompt_tokens": 10,
            "completion_tokens": 4,
            "prompt_tokens_details": { "cached_tokens": 2 },
        })
    }

    fn tool_call(&self) -> Value {
        serde_json::json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": serde_json::Value::Null,
                        "tool_calls": [
                            {
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "aws.ec2.stop",
                                    "arguments": "{\"instance_id\":\"i-1\"}",
                                },
                            }
                        ],
                    },
                    "finish_reason": "tool_calls",
                }
            ],
        })
    }

    fn finish_reasons(&self) -> &'static [(&'static str, FinishReason)] {
        &[
            ("stop", FinishReason::Stop),
            ("tool_calls", FinishReason::ToolUse),
            ("length", FinishReason::Length),
            ("content_filter", FinishReason::Refusal),
            ("error", FinishReason::Error),
        ]
    }

    fn auth(&self, secret: Secret) -> Auth {
        Auth::Bearer(secret)
    }

    fn credential_header(&self) -> &'static str {
        "authorization"
    }

    fn trusted_text(&self, body: &Value) -> String {
        body["messages"]
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .filter(|message| message["role"] == "system")
                    .filter_map(|message| message["content"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn conversation_text(&self, body: &Value) -> String {
        body["messages"]
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .filter(|message| message["role"] == "user")
                    .filter_map(|message| message["content"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn ceiling(&self, body: &Value) -> Value {
        body["max_tokens"].clone()
    }

    fn tool_call_id(&self) -> &'static str {
        "call_1"
    }

    fn text_stream(&self) -> (Canned, Vec<StreamEvent>) {
        let events = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        (
            Canned::new(200, events).with_header("content-type", "text/event-stream"),
            vec![
                StreamEvent::TextDelta("Hel".to_owned()),
                StreamEvent::TextDelta("lo".to_owned()),
                StreamEvent::Finish(FinishReason::Stop),
            ],
        )
    }

    fn tool_stream(&self) -> (Canned, Vec<StreamEvent>) {
        let events = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"function\":{\"name\":\"aws.ec2.stop\",\"arguments\":\"{\\\"instance\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"_id\\\":\\\"i-9\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
        );
        (
            Canned::new(200, events).with_header("content-type", "text/event-stream"),
            vec![
                StreamEvent::ToolCallDelta {
                    id: "call_9".to_owned(),
                    tool: "aws.ec2.stop".to_owned(),
                    arguments_delta: "{\"instance".to_owned(),
                },
                StreamEvent::ToolCallDelta {
                    id: "call_9".to_owned(),
                    tool: "aws.ec2.stop".to_owned(),
                    arguments_delta: "_id\":\"i-9\"}".to_owned(),
                },
            ],
        )
    }

    fn provider_debug(&self, base_url: &str, auth: Auth) -> String {
        let config = OpenAiCompatibleConfig::new(id(), base_url, "test-model").with_auth(auth);
        let provider = OpenAiCompatible::new(config).expect("the adapter builds");
        format!("{provider:?}")
    }
}

// -------------------------------------------------------------------------------------------------
// Anthropic
// -------------------------------------------------------------------------------------------------

impl Fixture for AnthropicFixture {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn path(&self) -> &'static str {
        "/messages"
    }

    fn build(&self, base_url: &str, tweak: &Tweak) -> Box<dyn AIProvider> {
        let config = AnthropicConfig::new(id(), base_url, "test-model")
            .with_auth(tweak.auth.clone())
            .with_timeouts(tweak.timeouts)
            .with_retry(tweak.retry)
            .with_cancel(tweak.cancel.clone());
        Box::new(Anthropic::new(config).expect("the adapter builds"))
    }

    fn reply(&self, text: &str, finish: &str) -> Value {
        serde_json::json!({
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": "test-model",
            "content": [{ "type": "text", "text": text }],
            "stop_reason": finish,
        })
    }

    fn usage_key(&self) -> &'static str {
        "usage"
    }

    fn usage(&self) -> Value {
        serde_json::json!({
            // The Messages API reports cache reads beside `input_tokens`; 8 + 2 is the shared
            // contract's 10 input tokens, of which 2 are cached.
            "input_tokens": 8,
            "output_tokens": 4,
            "cache_read_input_tokens": 2,
        })
    }

    fn tool_call(&self) -> Value {
        serde_json::json!({
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_1",
                    "name": "aws.ec2.stop",
                    "input": { "instance_id": "i-1" },
                }
            ],
            "stop_reason": "tool_use",
        })
    }

    fn finish_reasons(&self) -> &'static [(&'static str, FinishReason)] {
        &[
            ("end_turn", FinishReason::Stop),
            ("stop_sequence", FinishReason::Stop),
            ("tool_use", FinishReason::ToolUse),
            ("max_tokens", FinishReason::Length),
            ("refusal", FinishReason::Refusal),
        ]
    }

    fn auth(&self, secret: Secret) -> Auth {
        Auth::ApiKey(secret)
    }

    fn credential_header(&self) -> &'static str {
        "x-api-key"
    }

    fn trusted_text(&self, body: &Value) -> String {
        body["system"].as_str().unwrap_or_default().to_owned()
    }

    fn conversation_text(&self, body: &Value) -> String {
        body["messages"]
            .as_array()
            .map(|messages| {
                messages
                    .iter()
                    .filter(|message| message["role"] == "user")
                    .filter_map(|message| message["content"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn ceiling(&self, body: &Value) -> Value {
        body["max_tokens"].clone()
    }

    fn tool_call_id(&self) -> &'static str {
        "toolu_1"
    }

    fn text_stream(&self) -> (Canned, Vec<StreamEvent>) {
        let events = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        (
            Canned::new(200, events).with_header("content-type", "text/event-stream"),
            vec![
                StreamEvent::TextDelta("Hel".to_owned()),
                StreamEvent::TextDelta("lo".to_owned()),
                StreamEvent::Usage(aicontext_providers::Usage {
                    input_tokens: Some(3),
                    output_tokens: Some(5),
                    cached_input_tokens: None,
                    estimated: false,
                }),
                StreamEvent::Finish(FinishReason::Stop),
            ],
        )
    }

    fn tool_stream(&self) -> (Canned, Vec<StreamEvent>) {
        let events = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"aws.ec2.stop\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"instance\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"_id\\\":\\\"i-9\\\"}\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":7}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        (
            Canned::new(200, events).with_header("content-type", "text/event-stream"),
            vec![
                StreamEvent::ToolCallDelta {
                    id: "toolu_1".to_owned(),
                    tool: "aws.ec2.stop".to_owned(),
                    arguments_delta: "{\"instance".to_owned(),
                },
                StreamEvent::ToolCallDelta {
                    id: "toolu_1".to_owned(),
                    tool: "aws.ec2.stop".to_owned(),
                    arguments_delta: "_id\":\"i-9\"}".to_owned(),
                },
                // The Messages API splits usage across the first and last events; the adapter
                // reports it once, with both halves.
                StreamEvent::Usage(aicontext_providers::Usage {
                    input_tokens: Some(3),
                    output_tokens: Some(7),
                    cached_input_tokens: None,
                    estimated: false,
                }),
                StreamEvent::Finish(FinishReason::ToolUse),
            ],
        )
    }

    fn provider_debug(&self, base_url: &str, auth: Auth) -> String {
        let config = AnthropicConfig::new(id(), base_url, "test-model").with_auth(auth);
        let provider = Anthropic::new(config).expect("the adapter builds");
        format!("{provider:?}")
    }
}

// -------------------------------------------------------------------------------------------------
// Gemini
// -------------------------------------------------------------------------------------------------

impl Fixture for GeminiFixture {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn path(&self) -> &'static str {
        "/models/test-model:generateContent"
    }

    fn build(&self, base_url: &str, tweak: &Tweak) -> Box<dyn AIProvider> {
        let config = GeminiConfig::new(id(), base_url, "test-model")
            .with_auth(tweak.auth.clone())
            .with_timeouts(tweak.timeouts)
            .with_retry(tweak.retry)
            .with_cancel(tweak.cancel.clone());
        Box::new(Gemini::new(config).expect("the adapter builds"))
    }

    fn reply(&self, text: &str, finish: &str) -> Value {
        serde_json::json!({
            "candidates": [
                {
                    "content": { "parts": [{ "text": text }], "role": "model" },
                    "finishReason": finish,
                }
            ],
        })
    }

    fn usage_key(&self) -> &'static str {
        "usageMetadata"
    }

    fn usage(&self) -> Value {
        serde_json::json!({
            "promptTokenCount": 10,
            "candidatesTokenCount": 4,
            "cachedContentTokenCount": 2,
        })
    }

    fn tool_call(&self) -> Value {
        serde_json::json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            {
                                "functionCall": {
                                    "name": "aws.ec2.stop",
                                    "args": { "instance_id": "i-1" },
                                }
                            }
                        ],
                        "role": "model",
                    },
                    // Gemini spells a turn that proposed a function `STOP`; the adapter's rule is
                    // that a `STOP` with a call in it is tool use.
                    "finishReason": "STOP",
                }
            ],
        })
    }

    fn finish_reasons(&self) -> &'static [(&'static str, FinishReason)] {
        // No `ToolUse` entry: in a text-only context Gemini's `STOP` is a plain stop, and tool use
        // needs a `functionCall` part — which T2 supplies. That rule is the adapter's own.
        &[
            ("STOP", FinishReason::Stop),
            ("MAX_TOKENS", FinishReason::Length),
            ("SAFETY", FinishReason::Refusal),
            ("RECITATION", FinishReason::Refusal),
            ("MALFORMED_FUNCTION_CALL", FinishReason::Error),
        ]
    }

    fn auth(&self, secret: Secret) -> Auth {
        Auth::ApiKey(secret)
    }

    fn credential_header(&self) -> &'static str {
        "x-goog-api-key"
    }

    fn trusted_text(&self, body: &Value) -> String {
        body.pointer("/systemInstruction/parts/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    fn conversation_text(&self, body: &Value) -> String {
        body["contents"]
            .as_array()
            .map(|contents| {
                contents
                    .iter()
                    .filter(|entry| entry["role"] == "user")
                    .filter_map(|entry| entry.pointer("/parts/0/text"))
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn ceiling(&self, body: &Value) -> Value {
        body.pointer("/generationConfig/maxOutputTokens")
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn tool_call_id(&self) -> &'static str {
        // Gemini assigns no id to a function call; the neutral field is empty rather than invented.
        ""
    }

    fn text_stream(&self) -> (Canned, Vec<StreamEvent>) {
        let events = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hel\"}],\"role\":\"model\"}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"lo\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":3,\"candidatesTokenCount\":2}}\n\n",
        );
        (
            Canned::new(200, events).with_header("content-type", "text/event-stream"),
            vec![
                StreamEvent::TextDelta("Hel".to_owned()),
                StreamEvent::TextDelta("lo".to_owned()),
                StreamEvent::Usage(aicontext_providers::Usage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    cached_input_tokens: None,
                    estimated: false,
                }),
                StreamEvent::Finish(FinishReason::Stop),
            ],
        )
    }

    fn tool_stream(&self) -> (Canned, Vec<StreamEvent>) {
        let events = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"aws.ec2.stop\",\"args\":{\"instance_id\":\"i-9\"}}}],\"role\":\"model\"}}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[],\"role\":\"model\"},\"finishReason\":\"STOP\"}]}\n\n",
        );
        (
            Canned::new(200, events).with_header("content-type", "text/event-stream"),
            vec![
                StreamEvent::ToolCallDelta {
                    // The arguments arrive whole, so one delta carries all of them.
                    id: String::new(),
                    tool: "aws.ec2.stop".to_owned(),
                    arguments_delta: "{\"instance_id\":\"i-9\"}".to_owned(),
                },
                StreamEvent::Finish(FinishReason::ToolUse),
            ],
        )
    }

    fn provider_debug(&self, base_url: &str, auth: Auth) -> String {
        let config = GeminiConfig::new(id(), base_url, "test-model").with_auth(auth);
        let provider = Gemini::new(config).expect("the adapter builds");
        format!("{provider:?}")
    }
}
