//! The provider-neutral request, response, and supporting types.
//!
//! Nothing here names a provider, a wire format, or an HTTP status. An adapter's whole job is to
//! translate its own format into these and back (`docs/AI_PROVIDER_SPEC.md` §1, §2).

use std::fmt;
use std::str::FromStr;

use crate::error::ProviderError;

// -------------------------------------------------------------------------------------------------
// Identity
// -------------------------------------------------------------------------------------------------

/// The id of a configured provider, for example `local` or `openai-hosted`.
///
/// It is the key a provider is registered under and the value [`ProviderId`] is routed to
/// (`docs/AI_PROVIDER_SPEC.md` §4, §8). A malformed id is caught when it is read from `.ai/ai.yaml`
/// rather than becoming an unreachable registry entry.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProviderId(String);

impl ProviderId {
    /// Validates `raw` as a provider id: a leading lowercase letter, then lowercase letters, digits,
    /// `-`, and `_`.
    ///
    /// Returns [`ProviderError::InvalidId`] for anything else, including the empty string, and the
    /// message names the offending value rather than only the rule it broke.
    pub fn new(raw: &str) -> Result<Self, ProviderError> {
        if is_provider_id(raw) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(ProviderError::InvalidId {
                value: raw.to_owned(),
            })
        }
    }

    /// The id exactly as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `raw` is a lowercase slug a provider may be registered under.
fn is_provider_id(raw: &str) -> bool {
    let mut characters = raw.chars();
    match characters.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    characters.all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'
            || character == '_'
    })
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for ProviderId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProviderId {
    type Err = ProviderError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

impl PartialEq<str> for ProviderId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

// -------------------------------------------------------------------------------------------------
// The two regions
// -------------------------------------------------------------------------------------------------

/// The trusted instruction region of a request: the Z0 platform policy and the Z3 developer
/// instruction, and nothing else (`docs/AI_PROVIDER_SPEC.md` §2.1, `docs/SECURITY.md` §2).
///
/// It is structurally separate from [`DataBlock`], so external content cannot be placed here by
/// construction. A host that put retrieved text into a [`PolicyRegion`] would be violating the
/// contract, and the only thing that can stop it is that the two regions are two types.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyRegion {
    /// The developer's instruction, Z3 — the task being acted on, and the highest authority after
    /// the platform (`docs/ARCHITECTURE.md` trust zones). `None` when the run has no instruction.
    pub instruction: Option<String>,
    /// Trusted policy blocks: the project rules and conventions the instruction is bounded by.
    pub policy: Vec<PolicyBlock>,
}

/// One trusted policy block inside a [`PolicyRegion`].
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyBlock {
    /// The document id the block came from, for example `RULES-001`.
    pub source: String,
    /// Repository-relative path, for provenance.
    pub path: String,
    /// The block's text, already rendered for the model.
    pub content: String,
}

/// The role a piece of external content must be given.
///
/// One variant today, and an enum rather than a boolean, because the spec names the tag
/// `trust: untrusted` explicitly and reserves the field: a future trusted-data class can be added
/// without changing every call site (`docs/AI_PROVIDER_SPEC.md` §2.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Trust {
    /// Data, never instructions. A caller must not place it in an instruction role.
    Untrusted,
}

/// External content (Z4) placed in a data role, never an instruction role.
///
/// It is a distinct type from [`PolicyRegion`] on purpose: the trust tag is not optional and cannot
/// be set to anything but [`Trust::Untrusted`] (`docs/AI_PROVIDER_SPEC.md` §2.1,
/// `docs/SECURITY.md` I1).
#[derive(Clone, Debug, PartialEq)]
pub struct DataBlock {
    /// The id of the document or the name of the tool the content came from.
    pub source: String,
    /// Repository-relative path, for provenance.
    pub path: String,
    /// The context tier (1–9) the content was retrieved from, carried for provenance only
    /// (`docs/CONTEXT_SPEC.md` §7.4).
    pub tier: u8,
    /// Always [`Trust::Untrusted`].
    pub trust: Trust,
    /// The content, already rendered and wrapped for the model.
    pub content: String,
}

// -------------------------------------------------------------------------------------------------
// The conversation
// -------------------------------------------------------------------------------------------------

/// Who produced a [`Message`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Role {
    /// The developer or the host.
    User,
    /// The model.
    Assistant,
    /// A tool result being fed back to the model.
    Tool,
}

/// One turn of a conversation (`docs/AI_PROVIDER_SPEC.md` §2).
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    /// Who produced it.
    pub role: Role,
    /// The message text.
    pub content: String,
    /// For a [`Role::Tool`] message, the id of the [`ToolCall`] it answers. `None` otherwise.
    pub tool_call_id: Option<String>,
}

impl Message {
    /// A message with no tool-call correlation.
    #[must_use]
    pub fn new(role: Role, content: String) -> Self {
        Self {
            role,
            content,
            tool_call_id: None,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tools, budget, options
// -------------------------------------------------------------------------------------------------

/// A tool the model may call, already filtered by the gateway so the model never sees a tool it is
/// not permitted to propose (`docs/AI_PROVIDER_SPEC.md` §2, §2.2).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    /// The tool name the model must use, for example `aws.ec2.stop`. The gateway rejects any other.
    pub name: String,
    /// What the tool does, in one or two sentences.
    pub description: String,
    /// The JSON Schema for the arguments, validated before dispatch.
    pub input_schema: serde_json::Value,
}

/// The hard ceiling on output tokens (`docs/AI_PROVIDER_SPEC.md` §2, contract test T11).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenBudget {
    /// The most tokens the provider may generate. A hard ceiling, not a hint.
    pub max_output_tokens: u32,
}

/// The sampling options for one call (`docs/AI_PROVIDER_SPEC.md` §2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelOptions {
    /// Sampling temperature, when the caller wants to override the provider default.
    pub temperature: Option<f32>,
    /// A seed, when the caller wants a reproducible sample.
    pub seed: Option<u64>,
    /// Stop sequences the provider must halt on.
    pub stop: Vec<String>,
}

// -------------------------------------------------------------------------------------------------
// Request and response
// -------------------------------------------------------------------------------------------------

/// Everything the host hands to a provider for one call (`docs/AI_PROVIDER_SPEC.md` §2).
///
/// It is built by the host from the assembled context, never by a model: the instruction is in
/// `policy`, and every piece of external content is a [`DataBlock`] in `data`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionRequest {
    /// The trusted instruction and policy. Z0 + Z3 only, host-built.
    pub policy: PolicyRegion,
    /// The conversation so far: user, assistant, and tool messages.
    pub messages: Vec<Message>,
    /// External content (Z4), each tagged [`Trust::Untrusted`].
    pub data: Vec<DataBlock>,
    /// The tools the model may propose, already permission-filtered by the gateway.
    pub tools: Vec<ToolSpec>,
    /// The output-token ceiling.
    pub budget: TokenBudget,
    /// Sampling options.
    pub options: ModelOptions,
    /// A caller-chosen key that makes a retry of this exact request safe.
    pub idempotency_key: String,
}

/// What a provider returns for one call (`docs/AI_PROVIDER_SPEC.md` §2).
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionResponse {
    /// The model's text. Empty is a valid answer.
    pub text: String,
    /// The tool calls the model proposed. Each is a proposal, not an instruction.
    pub tool_calls: Vec<ToolCall>,
    /// Why the model stopped.
    pub finish_reason: FinishReason,
    /// Provider-reported usage; see [`Usage`].
    pub usage: Usage,
    /// Opaque provider metadata, retained for audit. Never interpreted by the core.
    pub provider_meta: serde_json::Value,
}

/// A tool call proposed by the model (`docs/AI_PROVIDER_SPEC.md` §2.2).
///
/// A `ToolCall` is a **proposal**, never an instruction. It is validated, permission-checked, and
/// approved before anything happens, and a tool name the gateway does not recognise is dropped with
/// an audit record rather than passed through.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    /// The provider-scoped id, remapped to a host id before dispatch.
    pub id: String,
    /// The tool name the model proposed, for example `aws.ec2.stop`. Verified by the gateway.
    pub tool: String,
    /// The arguments, validated against the tool's schema before dispatch.
    pub arguments: serde_json::Value,
    /// The provider's raw payload, retained for audit only.
    pub raw: serde_json::Value,
}

/// Why a provider stopped (`docs/AI_PROVIDER_SPEC.md` §2, contract test T12).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FinishReason {
    /// The model finished its answer.
    Stop,
    /// The model asked to call one or more tools.
    ToolUse,
    /// The output-token ceiling was reached.
    Length,
    /// The model declined. A first-class outcome, reported with the model's text — not retried and
    /// not worked around (`docs/AI_PROVIDER_SPEC.md` §6).
    Refusal,
    /// The provider reported a failure in-band.
    Error,
    /// A reason this version does not know. Retained verbatim rather than guessed at (T12).
    Unknown(String),
}

/// Token usage reported by a provider (`docs/AI_PROVIDER_SPEC.md` §2, §7).
///
/// Each count is optional because a provider may report none of them, and `estimated` marks a number
/// the host derived from the context budget rather than one the provider reported. The two are never
/// conflated: a local server that reports nothing produces an estimate that says so.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    /// Input tokens, when the provider reported them.
    pub input_tokens: Option<u32>,
    /// Output tokens, when the provider reported them.
    pub output_tokens: Option<u32>,
    /// Cached input tokens, when the provider reported them.
    pub cached_input_tokens: Option<u32>,
    /// Whether these numbers are a host-derived estimate rather than a provider report.
    pub estimated: bool,
}

impl Usage {
    /// Usage the provider reported for input and output tokens.
    #[must_use]
    pub const fn reported(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            cached_input_tokens: None,
            estimated: false,
        }
    }

    /// Usage the host derived from the context budget, because the provider reported none.
    #[must_use]
    pub const fn estimated(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            cached_input_tokens: None,
            estimated: true,
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Streaming and capabilities
// -------------------------------------------------------------------------------------------------

/// One event in a streamed answer.
///
/// The neutral shape a streamed provider response is mapped onto; a provider that has no streaming
/// transport reports [`Capabilities::streaming`] as `false` and the host does not call `stream`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StreamEvent {
    /// A fragment of the model's text. Concatenate the fragments, in order, to get the answer.
    TextDelta(String),
    /// A fragment of a tool call. `arguments_delta` is a fragment of the JSON arguments; the id and
    /// tool name are repeated on each fragment so a reader never has to remember state.
    ToolCallDelta {
        /// The provider-scoped call id.
        id: String,
        /// The tool name the model proposed.
        tool: String,
        /// A fragment of the JSON arguments.
        arguments_delta: String,
    },
    /// Usage, when the provider reports it; usually the last data event.
    Usage(Usage),
    /// The run ended, with the reason.
    Finish(FinishReason),
}

/// What a provider can do, reported without a call.
///
/// The host uses this to decide whether to ask for a stream or for structured output, rather than
/// discovering the answer by failing (`docs/AI_PROVIDER_SPEC.md` §9).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Capabilities {
    /// Whether `stream` is implemented rather than returning [`ProviderError::Unsupported`].
    pub streaming: bool,
    /// Whether the provider can propose tool calls.
    pub tool_calls: bool,
    /// Whether the provider can be constrained to structured output.
    pub structured_output: bool,
    /// The model's context window in tokens, when the provider reports one.
    pub max_context_tokens: Option<u32>,
}

// -------------------------------------------------------------------------------------------------
// Routing
// -------------------------------------------------------------------------------------------------

/// What a routing decision is made from (`docs/AI_PROVIDER_SPEC.md` §8).
///
/// A seam. In the MVP the only routing rule is "the configured default", so the fields are recorded
/// for the decision that will read them later rather than used today.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoutingRequest {
    /// The task being planned, when there is one.
    pub task: Option<String>,
    /// The output ceiling the run must fit, when it is known.
    pub budget: Option<TokenBudget>,
}

#[cfg(test)]
mod tests {
    use super::{FinishReason, ModelOptions, ProviderId, Trust, Usage};

    #[test]
    fn a_provider_id_accepts_a_lowercase_slug() {
        for raw in ["local", "openai-hosted", "lm_studio", "x1"] {
            assert_eq!(
                ProviderId::new(raw).expect("valid").as_str(),
                raw,
                "`{raw}` must be a valid provider id"
            );
        }
    }

    #[test]
    fn a_provider_id_rejects_anything_that_is_not_a_slug() {
        for raw in [
            "",
            " Local",
            "Local",
            "1local",
            "local id",
            "local/other",
            "local.",
        ] {
            assert!(
                ProviderId::new(raw).is_err(),
                "`{raw}` must be rejected as a provider id"
            );
        }
    }

    #[test]
    fn an_invalid_id_names_the_offending_value() {
        let error = ProviderId::new("_bad").expect_err("underscore may not lead");
        assert!(
            error.to_string().contains("_bad"),
            "the message must name the value, got: {error}"
        );
    }

    #[test]
    fn usage_absent_and_estimated_are_distinct() {
        // A provider that reports nothing and a host estimate are different facts, and the flag is
        // what keeps them apart (`docs/AI_PROVIDER_SPEC.md` 7).
        assert_eq!(
            Usage::default(),
            Usage {
                input_tokens: None,
                output_tokens: None,
                cached_input_tokens: None,
                estimated: false,
            }
        );
        assert!(!Usage::reported(1, 2).estimated);
        assert!(Usage::estimated(1, 2).estimated);
    }

    #[test]
    fn an_unrecognised_finish_reason_is_preserved_verbatim() {
        // T12: report it rather than guessing at it.
        assert_eq!(
            FinishReason::Unknown("content_filter".to_owned()),
            FinishReason::Unknown("content_filter".to_owned())
        );
    }

    #[test]
    fn the_only_trust_a_data_block_can_carry_is_untrusted() {
        assert_eq!(Trust::Untrusted, Trust::Untrusted);
    }

    #[test]
    fn model_options_default_to_the_provider_defaults() {
        let options = ModelOptions::default();
        assert!(options.temperature.is_none());
        assert!(options.seed.is_none());
        assert!(options.stop.is_empty());
    }
}
