//! The Anthropic adapter: the Messages API wire format (`docs/AI_PROVIDER_SPEC.md` §3).
//!
//! # What is in this module, and what is not
//!
//! The request body, the response parser, and the SSE translation are here, because they are what
//! "the Messages API" *means*: a top-level `system` string, a required `max_tokens`, `tool_use` and
//! `tool_result` content blocks. The conversation with the HTTP server is not: that is
//! [`crate::transport`], shared with the other two adapters, because a 429 means the same thing in
//! every wire format.
//!
//! # Example
//!
//! ```no_run
//! use aicontext_providers::anthropic::{Anthropic, AnthropicConfig};
//! use aicontext_providers::{Auth, ProviderId, Secret};
//!
//! let id = ProviderId::new("claude")?;
//! let config = AnthropicConfig::new(id, "https://api.anthropic.com/v1", "claude-sonnet-4-5")
//!     .with_auth(Auth::ApiKey(Secret::new("sk-ant-…")));
//! let provider = Anthropic::new(config)?;
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

/// The header Anthropic reads its API key from, whatever else the request carries.
const API_KEY_HEADER: &str = "x-api-key";

/// The API version every Messages API call states, so a server can serve an old contract.
const VERSION_HEADER: &str = "anthropic-version";

/// The version string this adapter sends.
const VERSION: &str = "2023-06-01";

/// Everything the adapter needs to exist: where it is, how it authenticates, and how patient it is.
///
/// The same three knobs as [`crate::openai::OpenAiCompatibleConfig`], because a host configures a
/// provider the same way whichever wire format it speaks; only the credential's *header* differs.
#[derive(Clone, Debug)]
pub struct AnthropicConfig {
    /// The id this provider is registered under.
    pub id: ProviderId,
    /// The server root, for example `https://api.anthropic.com/v1`. A trailing slash is allowed.
    pub base_url: String,
    /// The model to ask for.
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

impl AnthropicConfig {
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

/// A provider that speaks the Anthropic Messages API.
#[derive(Debug)]
pub struct Anthropic {
    config: AnthropicConfig,
    transport: Transport,
}

impl Anthropic {
    /// Builds the client, which is where the connect timeout is configured.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Transport`] when the client cannot be built.
    pub fn new(config: AnthropicConfig) -> Result<Self, ProviderError> {
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
    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }

    /// The absolute URL of the messages endpoint.
    fn endpoint(&self) -> String {
        format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            wire::PATH
        )
    }

    /// The headers that carry the credential and the API version.
    ///
    /// An API key is the raw value in `x-api-key`, which is the header this server reads; a bearer
    /// token is still `Authorization`, because that is what the scheme means and Anthropic accepts
    /// it for OAuth-issued tokens.
    fn auth_headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![(VERSION_HEADER.to_owned(), VERSION.to_owned())];
        if let Some(header) = self.config.auth.header(API_KEY_HEADER) {
            headers.push(header);
        }
        headers
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
            .dispatch(
                &self.endpoint(),
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
impl AIProvider for Anthropic {
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
        let body = wire::request_body(&request, &self.config.model, true);
        let seed = transport::seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self
            .transport
            .dispatch(
                &self.endpoint(),
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
    use super::{API_KEY_HEADER, Anthropic, AnthropicConfig, VERSION, VERSION_HEADER};
    use crate::transport::Auth;
    use crate::{ProviderId, Secret};

    const SECRET: &str = "sk-ant-do-not-print";

    fn config() -> AnthropicConfig {
        AnthropicConfig::new(
            ProviderId::new("claude").expect("valid"),
            "https://api.anthropic.com/v1",
            "test-model",
        )
    }

    #[test]
    fn an_api_key_goes_in_its_own_header_and_the_version_is_always_sent() {
        let provider = Anthropic::new(config().with_auth(Auth::ApiKey(Secret::new(SECRET))))
            .expect("the adapter builds");
        let headers = provider.auth_headers();
        assert!(headers.contains(&(VERSION_HEADER.to_owned(), VERSION.to_owned())));
        assert!(headers.contains(&(API_KEY_HEADER.to_owned(), SECRET.to_owned())));
    }

    #[test]
    fn a_bearer_token_stays_authorization() {
        let provider = Anthropic::new(config().with_auth(Auth::Bearer(Secret::new(SECRET))))
            .expect("the adapter builds");
        let headers = provider.auth_headers();
        assert!(headers.contains(&("authorization".to_owned(), format!("Bearer {SECRET}"))));
        assert!(!headers.iter().any(|(name, _)| name == API_KEY_HEADER));
    }

    #[test]
    fn the_debug_rendering_redacts_the_credential() {
        let provider =
            Anthropic::new(config().with_auth(Auth::ApiKey(Secret::new(SECRET)))).expect("builds");
        let rendered = format!("{provider:?}");
        assert!(!rendered.contains(SECRET), "{rendered}");
        assert!(rendered.contains("redacted"), "{rendered}");
    }

    #[test]
    fn the_endpoint_joins_the_base_url_and_the_path_once() {
        for base in ["https://x/v1", "https://x/v1/"] {
            let mut config = config();
            config.base_url = base.to_owned();
            let provider = Anthropic::new(config).expect("builds");
            assert_eq!(provider.endpoint(), "https://x/v1/messages");
        }
    }
}
