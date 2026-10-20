//! The `OpenAI`-compatible adapter: the chat-completions wire format (`docs/AI_PROVIDER_SPEC.md` §3).
//!
//! Every provider that speaks `POST {base}/chat/completions` — a hosted OpenAI endpoint, Ollama,
//! vLLM, llama.cpp, LM Studio, a corporate proxy in front of any of them — is one config away, so
//! the spec makes this adapter the first one and the CLI's default.
//!
//! # What is in this module, and what is not
//!
//! The request body, the response parser, and the SSE translation are here, because they are what
//! "chat completions" *means*. The conversation with the HTTP server — timeouts, retries,
//! cancellation, the error classes — is not: that is [`crate::transport`], and it is shared with the
//! Anthropic and Gemini adapters, because a 429 means the same thing in every wire format. Adding
//! an adapter therefore adds one module like this one and touches nothing else
//! (`docs/AI_PROVIDER_SPEC.md` §12).
//!
//! # Example
//!
//! ```no_run
//! use aicontext_providers::openai::{OpenAiCompatible, OpenAiCompatibleConfig};
//! use aicontext_providers::ProviderId;
//!
//! let id = ProviderId::new("local")?;
//! let config = OpenAiCompatibleConfig::new(id, "http://localhost:11434/v1", "qwen3-coder");
//! let provider = OpenAiCompatible::new(config)?;
//! # Ok::<(), aicontext_providers::ProviderError>(())
//! ```
//!
//! The example uses port `11434` because that is Ollama's — a real, free, local server, and the
//! reason the adapter exists in the shape it does.

pub(crate) mod stream;
pub(crate) mod wire;

use async_trait::async_trait;

use crate::cancel::CancellationToken;
use crate::error::ProviderError;
use crate::provider::{AIProvider, BoxStream};
use crate::transport::{self, Transport};
use crate::types::{Capabilities, CompletionRequest, CompletionResponse, ProviderId, StreamEvent};

// The credential, the timeouts, and the retry policy are the same three knobs on every adapter, so
// they are one type in `transport` and re-exported here: `openai::Auth` and `transport::Auth` are
// the same path, and code written against the first keeps working.
pub use crate::transport::{Auth, RetryPolicy, Timeouts};

/// Everything the adapter needs to exist: where it is, how it authenticates, and how patient it is.
#[derive(Clone, Debug)]
pub struct OpenAiCompatibleConfig {
    /// The id this provider is registered under.
    pub id: ProviderId,
    /// The server root, for example `http://localhost:11434/v1`. A trailing slash is allowed.
    pub base_url: String,
    /// The model to ask for.
    pub model: String,
    /// How to authenticate.
    pub auth: Auth,
    /// How long to wait.
    pub timeouts: Timeouts,
    /// How failures are repeated.
    pub retry: RetryPolicy,
    /// The token the whole run shares.
    pub cancel: CancellationToken,
}

impl OpenAiCompatibleConfig {
    /// A config for one server and one model: no credential, the default timeouts and retry policy,
    /// and a fresh cancellation token.
    #[must_use]
    pub fn new(id: ProviderId, base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id,
            base_url: base_url.into(),
            model: model.into(),
            auth: Auth::None,
            timeouts: Timeouts::default(),
            retry: RetryPolicy::default(),
            cancel: CancellationToken::new(),
        }
    }

    /// Sets the credential.
    #[must_use]
    pub fn with_auth(mut self, auth: Auth) -> Self {
        self.auth = auth;
        self
    }

    /// Sets the timeouts.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Sets the retry policy.
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Sets the cancellation token.
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// A provider that speaks the `OpenAI` chat-completions shape.
#[derive(Debug)]
pub struct OpenAiCompatible {
    config: OpenAiCompatibleConfig,
    transport: Transport,
}

impl OpenAiCompatible {
    /// Builds the client, which is where the connect timeout is configured.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Transport`] when the client cannot be built.
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, ProviderError> {
        let transport = Transport::new(
            config.id.clone(),
            &config.auth,
            config.timeouts,
            config.retry,
            config.cancel.clone(),
        )?;
        Ok(Self { config, transport })
    }

    /// The configuration this provider was built with.
    #[must_use]
    pub fn config(&self) -> &OpenAiCompatibleConfig {
        &self.config
    }

    /// The absolute URL of the chat-completions endpoint.
    fn endpoint(&self) -> String {
        format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            wire::PATH
        )
    }

    /// The header that carries the credential, or nothing without one.
    ///
    /// A chat-completions server speaks bearer and only bearer: a compatible proxy forwards
    /// `Authorization` and reads nothing else, so both credential variants are presented the same
    /// way here. Anthropic and Gemini map the same [`Auth`] onto their own headers instead.
    fn auth_header(&self) -> Vec<(String, String)> {
        let value = match &self.config.auth {
            Auth::None => return Vec::new(),
            Auth::Bearer(secret) | Auth::ApiKey(secret) => {
                format!("Bearer {}", secret.expose())
            }
        };
        vec![("authorization".to_owned(), value)]
    }

    /// Answers in one shot; the total-timeout wrapper is in `AIProvider::complete`.
    async fn complete_inner(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let body = wire::request_body(&request, &self.config.model, false);
        let seed = transport::seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self
            .transport
            .dispatch(&self.endpoint(), &self.auth_header(), &body, idempotent, seed)
            .await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| self.transport.classify(error))?;
        wire::parse_completion(&self.config.id, &bytes)
    }
}

#[async_trait]
impl AIProvider for OpenAiCompatible {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.transport.within_total(self.complete_inner(request)).await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<StreamEvent>, ProviderError> {
        let body = wire::request_body(&request, &self.config.model, true);
        let seed = transport::seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self
            .transport
            .dispatch(&self.endpoint(), &self.auth_header(), &body, idempotent, seed)
            .await?;
        Ok(stream::open(response, self.config.cancel.clone()))
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            streaming: true,
            tool_calls: true,
            structured_output: false,
            max_context_tokens: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenAiCompatible, OpenAiCompatibleConfig};
    use crate::transport::Auth;
    use crate::{ProviderId, Secret};

    fn config() -> OpenAiCompatibleConfig {
        OpenAiCompatibleConfig::new(
            ProviderId::new("local").expect("valid"),
            "http://localhost:11434/v1",
            "test-model",
        )
    }

    #[test]
    fn the_auth_header_is_bearer_for_either_credential_variant() {
        let secret = Secret::new("sk-live-do-not-print");
        for auth in [
            Auth::Bearer(secret.clone()),
            Auth::ApiKey(secret.clone()),
        ] {
            let provider =
                OpenAiCompatible::new(config().with_auth(auth)).expect("the adapter builds");
            assert_eq!(
                provider.auth_header(),
                vec![(
                    "authorization".to_owned(),
                    format!("Bearer {}", secret.expose())
                )]
            );
        }
        let anonymous = OpenAiCompatible::new(config()).expect("the adapter builds");
        assert!(anonymous.auth_header().is_empty());
    }

    #[test]
    fn the_debug_rendering_redacts_the_credential() {
        let provider = OpenAiCompatible::new(config().with_auth(Auth::Bearer(Secret::new(
            "sk-live-do-not-print",
        ))))
        .expect("the adapter builds");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains("sk-live-do-not-print"), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }

    #[test]
    fn the_endpoint_joins_the_base_url_and_the_path_once() {
        for base in ["http://x/v1", "http://x/v1/"] {
            let mut config = config();
            config.base_url = base.to_owned();
            let provider = OpenAiCompatible::new(config).expect("builds");
            assert_eq!(provider.endpoint(), "http://x/v1/chat/completions");
        }
    }
}
