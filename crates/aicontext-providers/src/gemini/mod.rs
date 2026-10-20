//! The Gemini adapter: the `generateContent` wire format (`docs/AI_PROVIDER_SPEC.md` §3).
//!
//! # What is in this module, and what is not
//!
//! The request body, the response parser, and the SSE translation are here, because they are what
//! "generateContent" *means*: `contents` of parts, a `systemInstruction`, `functionDeclarations`,
//! and a `generationConfig`. The conversation with the HTTP server is not: that is
//! [`crate::transport`], shared with the other two adapters.
//!
//! One thing here is different from the other two adapters, and it is worth knowing before reading
//! the code: **the model is part of the URL**, not the body. `generateContent` is addressed as
//! `models/{model}:generateContent`, so this adapter builds the path per call. A model name is a
//! slug like `gemini-2.5-pro` or `models/gemini-2.5-pro`; a leading `models/` is stripped so both
//! spellings work, and no other rewriting is done — a model name is never URL-encoded here, because
//! the names are slugs and a percent sign in one would be a configuration mistake worth failing on
//! rather than hiding.
//!
//! # Example
//!
//! ```no_run
//! use aicontext_providers::gemini::{Gemini, GeminiConfig};
//! use aicontext_providers::{Auth, ProviderId, Secret};
//!
//! let id = ProviderId::new("gemini")?;
//! let config = GeminiConfig::new(
//!     id,
//!     "https://generativelanguage.googleapis.com/v1beta",
//!     "gemini-2.5-pro",
//! )
//! .with_auth(Auth::ApiKey(Secret::new("AIza…")));
//! let provider = Gemini::new(config)?;
//! # Ok::<(), aicontext_providers::ProviderError>(())
//! ```

pub(crate) mod stream;
pub(crate) mod wire;

use async_trait::async_trait;

use crate::cancel::CancellationToken;
use crate::error::ProviderError;
use crate::provider::{AIProvider, BoxStream};
use crate::transport::{self, Auth, RetryPolicy, Timeouts, Transport};
use crate::types::{Capabilities, CompletionRequest, CompletionResponse, ProviderId, StreamEvent};

/// The header Gemini reads its API key from, whatever else the request carries.
const API_KEY_HEADER: &str = "x-goog-api-key";

/// Everything the adapter needs to exist: where it is, how it authenticates, and how patient it is.
#[derive(Clone, Debug)]
pub struct GeminiConfig {
    /// The id this provider is registered under.
    pub id: ProviderId,
    /// The server root, for example `https://generativelanguage.googleapis.com/v1beta`. A trailing
    /// slash is allowed.
    pub base_url: String,
    /// The model to ask for, with or without a leading `models/`.
    pub model: String,
    /// How to authenticate. [`Auth::ApiKey`] is the usual choice here.
    pub auth: Auth,
    /// How long to wait.
    pub timeouts: Timeouts,
    /// How failures are repeated.
    pub retry: RetryPolicy,
    /// The token the whole run shares.
    pub cancel: CancellationToken,
}

impl GeminiConfig {
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

/// A provider that speaks the Gemini `generateContent` API.
#[derive(Debug)]
pub struct Gemini {
    config: GeminiConfig,
    transport: Transport,
}

impl Gemini {
    /// Builds the client, which is where the connect timeout is configured.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Transport`] when the client cannot be built.
    pub fn new(config: GeminiConfig) -> Result<Self, ProviderError> {
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
    pub fn config(&self) -> &GeminiConfig {
        &self.config
    }

    /// The model with any leading `models/` removed, so both spellings address the same model.
    fn model(&self) -> &str {
        self.config.model.trim_start_matches("models/")
    }

    /// The absolute URL of the endpoint, with the model in the path.
    ///
    /// Streaming changes the method name rather than the body, which is why the flag is a path
    /// decision here and a body field in the other two adapters.
    fn endpoint(&self, stream: bool) -> String {
        let method = if stream {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        format!(
            "{}/models/{}:{}",
            self.config.base_url.trim_end_matches('/'),
            self.model(),
            method
        )
    }

    /// The header that carries the credential, or nothing without one.
    fn auth_headers(&self) -> Vec<(String, String)> {
        self.config
            .auth
            .header(API_KEY_HEADER)
            .into_iter()
            .collect()
    }

    /// Answers in one shot; the total-timeout wrapper is in `AIProvider::complete`.
    async fn complete_inner(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let body = wire::request_body(&request);
        let seed = transport::seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self
            .transport
            .dispatch(
                &self.endpoint(false),
                &self.auth_headers(),
                &body,
                idempotent,
                seed,
            )
            .await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| self.transport.classify(error))?;
        wire::parse_completion(&self.config.id, &bytes)
    }
}

#[async_trait]
impl AIProvider for Gemini {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.transport
            .within_total(self.complete_inner(request))
            .await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<StreamEvent>, ProviderError> {
        let body = wire::request_body(&request);
        let seed = transport::seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self
            .transport
            .dispatch(
                &self.endpoint(true),
                &self.auth_headers(),
                &body,
                idempotent,
                seed,
            )
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
    use super::{API_KEY_HEADER, Gemini, GeminiConfig};
    use crate::transport::Auth;
    use crate::{ProviderId, Secret};

    const SECRET: &str = "AIza-do-not-print";

    fn config() -> GeminiConfig {
        GeminiConfig::new(
            ProviderId::new("gemini").expect("valid"),
            "https://generativelanguage.googleapis.com/v1beta",
            "test-model",
        )
    }

    #[test]
    fn the_model_goes_in_the_path_and_a_models_prefix_is_stripped() {
        let provider = Gemini::new(config()).expect("the adapter builds");
        assert_eq!(
            provider.endpoint(false),
            "https://generativelanguage.googleapis.com/v1beta/models/test-model:generateContent"
        );

        let mut prefixed = config();
        prefixed.model = "models/test-model".to_owned();
        let provider = Gemini::new(prefixed).expect("the adapter builds");
        assert_eq!(
            provider.endpoint(false),
            "https://generativelanguage.googleapis.com/v1beta/models/test-model:generateContent",
            "both spellings must address the same model"
        );
    }

    #[test]
    fn streaming_changes_the_method_name_and_asks_for_sse() {
        let provider = Gemini::new(config()).expect("the adapter builds");
        assert_eq!(
            provider.endpoint(true),
            "https://generativelanguage.googleapis.com/v1beta/models/test-model:\
             streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn an_api_key_goes_in_the_goog_header() {
        let provider = Gemini::new(config().with_auth(Auth::ApiKey(Secret::new(SECRET))))
            .expect("the adapter builds");
        assert_eq!(
            provider.auth_headers(),
            vec![(API_KEY_HEADER.to_owned(), SECRET.to_owned())]
        );
    }

    #[test]
    fn the_debug_rendering_redacts_the_credential() {
        let provider =
            Gemini::new(config().with_auth(Auth::ApiKey(Secret::new(SECRET)))).expect("builds");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains(SECRET), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }
}
