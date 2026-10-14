//! The provider abstraction from outside the crate.
//!
//! These tests use only what a real adapter would: the crate's public exports. They are the small,
//! local version of `docs/AI_PROVIDER_SPEC.md` §11 — a provider can be implemented, registered,
//! called, and streamed without naming any type this crate does not export.

use aicontext_providers::{
    AIProvider, BoxStream, Capabilities, CompletionRequest, CompletionResponse, DataBlock,
    FinishReason, Message, ModelOptions, PolicyBlock, PolicyRegion, ProviderError, ProviderId,
    ProviderRegistry, Role, Stream, StreamEvent, TokenBudget, ToolCall, ToolSpec, Trust, Usage,
    async_trait,
};

/// A provider that echoes the last message, so the trait can be exercised without a transport.
///
/// It uses only this crate's re-exports — `async_trait` and `Stream` — which is what "adding an
/// adapter touches no other crate" has to mean in practice.
struct EchoProvider {
    id: ProviderId,
}

impl EchoProvider {
    fn new(id: &str) -> Self {
        Self {
            id: ProviderId::new(id).expect("a test provider id is a valid slug"),
        }
    }
}

#[async_trait]
impl AIProvider for EchoProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let text = request
            .messages
            .last()
            .map_or_else(String::new, |message| message.content.clone());
        Ok(CompletionResponse {
            text,
            tool_calls: vec![ToolCall {
                id: "call-1".to_owned(),
                tool: "aws.ec2.stop".to_owned(),
                arguments: serde_json::json!({ "instance_id": "i-0abc1234" }),
                raw: serde_json::json!({}),
            }],
            finish_reason: FinishReason::ToolUse,
            usage: Usage::reported(12, 4),
            provider_meta: serde_json::Value::Null,
        })
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<BoxStream<StreamEvent>, ProviderError> {
        let events = vec![
            StreamEvent::TextDelta("hel".to_owned()),
            StreamEvent::TextDelta("lo".to_owned()),
            StreamEvent::Usage(Usage::reported(1, 2)),
            StreamEvent::Finish(FinishReason::Stop),
        ];
        Ok(Box::pin(Events::new(events)))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            streaming: true,
            tool_calls: true,
            ..Capabilities::default()
        }
    }
}

/// A stream that yields a fixed list of events and then ends.
struct Events {
    remaining: std::collections::VecDeque<StreamEvent>,
}

impl Events {
    fn new(events: Vec<StreamEvent>) -> Self {
        Self {
            remaining: events.into(),
        }
    }
}

impl Stream for Events {
    type Item = StreamEvent;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.remaining.pop_front())
    }
}

/// Drains a stream to completion on the current thread, without an async runtime.
fn drain(mut stream: BoxStream<StreamEvent>) -> Vec<StreamEvent> {
    let mut collected = Vec::new();
    futures_executor::block_on(std::future::poll_fn(|context| {
        loop {
            match stream.as_mut().poll_next(context) {
                std::task::Poll::Ready(Some(event)) => collected.push(event),
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(()),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }));
    collected
}

fn request() -> CompletionRequest {
    CompletionRequest {
        policy: PolicyRegion {
            instruction: Some("Summarise the module.".to_owned()),
            policy: vec![PolicyBlock {
                source: "SPEC-context".to_owned(),
                path: ".ai/specs/context.md".to_owned(),
                content: "…".to_owned(),
            }],
        },
        messages: vec![Message::new(Role::User, "hello".to_owned())],
        data: vec![DataBlock {
            source: "src/lib.rs".to_owned(),
            path: "src/lib.rs".to_owned(),
            tier: 2,
            trust: Trust::Untrusted,
            content: "pub fn main() {}".to_owned(),
        }],
        tools: vec![ToolSpec {
            name: "aws.ec2.stop".to_owned(),
            description: "Stop an instance.".to_owned(),
            input_schema: serde_json::json!({ "type": "object" }),
        }],
        budget: TokenBudget {
            max_output_tokens: 256,
        },
        options: ModelOptions {
            temperature: Some(0.0),
            seed: Some(7),
            stop: vec!["\n\n".to_owned()],
        },
        idempotency_key: "req-1".to_owned(),
    }
}

#[test]
fn an_adapter_is_reachable_through_the_trait_object_alone() {
    let id = ProviderId::new("local").expect("valid");
    let mut registry = ProviderRegistry::new(id.clone());
    registry.register(Box::new(EchoProvider::new("local")));

    // The caller names only `dyn AIProvider`; the concrete type never appears outside this test.
    let provider: &dyn AIProvider = registry.get(&id).expect("registered");
    assert_eq!(provider.id(), &id);
    assert!(provider.capabilities().streaming);
    assert!(provider.capabilities().tool_calls);
}

#[test]
fn a_completion_maps_to_the_neutral_types() {
    let provider = EchoProvider::new("local");
    let response = futures_executor::block_on(provider.complete(request()))
        .expect("the echo provider succeeds");
    assert_eq!(response.text, "hello");
    assert_eq!(response.finish_reason, FinishReason::ToolUse);
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].tool, "aws.ec2.stop");
    assert_eq!(response.usage, Usage::reported(12, 4));
    assert!(!response.usage.estimated);
}

#[test]
fn a_stream_yields_its_events_in_order_and_then_ends() {
    let provider = EchoProvider::new("local");
    let stream =
        futures_executor::block_on(provider.stream(request())).expect("the echo provider streams");
    let events = drain(stream);
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("hel".to_owned()),
            StreamEvent::TextDelta("lo".to_owned()),
            StreamEvent::Usage(Usage::reported(1, 2)),
            StreamEvent::Finish(FinishReason::Stop),
        ]
    );
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::TextDelta(delta) => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "hello");
}

/// A provider with no capabilities at all: it must say so rather than fail late.
struct NoStream {
    id: ProviderId,
}

#[async_trait]
impl AIProvider for NoStream {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        Err(ProviderError::Unsupported {
            provider: self.id.clone(),
            capability: "completion",
        })
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<BoxStream<StreamEvent>, ProviderError> {
        Err(ProviderError::Unsupported {
            provider: self.id.clone(),
            capability: "streaming",
        })
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
}

#[test]
fn a_provider_that_cannot_stream_says_so_rather_than_failing_late() {
    // Capabilities are what let the host choose, instead of discovering the answer by failing
    // (`docs/AI_PROVIDER_SPEC.md` 9).
    let provider = NoStream {
        id: ProviderId::new("bare").expect("valid"),
    };
    assert!(!provider.capabilities().streaming);
    // `BoxStream` is not `Debug`, so the error is taken by matching rather than `expect_err`.
    let Err(error) = futures_executor::block_on(provider.stream(request())) else {
        panic!("streaming must not be offered");
    };
    assert_eq!(error.code(), "PROVIDER_UNSUPPORTED");
    assert!(error.remediation().contains("provider"));
}

#[test]
fn a_provider_id_is_a_lowercase_slug_or_an_error() {
    assert_eq!(
        ProviderId::new("openai-hosted").expect("valid").as_str(),
        "openai-hosted"
    );
    let error = ProviderId::new("OpenAI Hosted").expect_err("not a slug");
    assert_eq!(error.code(), "PROVIDER_INVALID_ID");
    assert!(error.to_string().contains("OpenAI Hosted"));
}

#[test]
fn a_refusal_is_an_outcome_not_an_error() {
    // The trait never returns a refusal as `Err`; it is a `CompletionResponse` with its own reason
    // (`docs/AI_PROVIDER_SPEC.md` 6). This asserts the encoding the adapter must use.
    let response = CompletionResponse {
        text: "I cannot help with that.".to_owned(),
        tool_calls: Vec::new(),
        finish_reason: FinishReason::Refusal,
        usage: Usage::estimated(5, 6),
        provider_meta: serde_json::Value::Null,
    };
    assert_eq!(response.finish_reason, FinishReason::Refusal);
    assert!(
        response.usage.estimated,
        "an estimate must be labelled as one"
    );
    assert!(response.tool_calls.is_empty());
}
