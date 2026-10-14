//! The OpenAI-compatible adapter: one implementation that reaches OpenAI and any server that
//! speaks the same `/chat/completions` shape.
//!
//! `docs/AI_PROVIDER_SPEC.md` §3 names this adapter and explains why it is first: the hosted model
//! and the local model (Ollama, llama.cpp, vLLM, LM Studio, OpenRouter, Azure) are one
//! implementation over a configurable `base_url`, which is the cheapest possible proof that the
//! abstraction is real.
//!
//! ```no_run
//! use aicontext_providers::openai::{OpenAiCompatible, OpenAiCompatibleConfig};
//! use aicontext_providers::ProviderId;
//!
//! # async fn run() -> Result<(), aicontext_providers::ProviderError> {
//! let config = OpenAiCompatibleConfig::new(
//!     ProviderId::new("local")?,
//!     "http://localhost:11434/v1",
//!     "qwen2.5-coder",
//! );
//! let provider = OpenAiCompatible::new(config)?;
//! let request = aicontext_providers::CompletionRequest {
//!     policy: aicontext_providers::PolicyRegion::default(),
//!     messages: vec![aicontext_providers::Message::new(
//!         aicontext_providers::Role::User,
//!         "hello".to_owned(),
//!     )],
//!     data: Vec::new(),
//!     tools: Vec::new(),
//!     budget: aicontext_providers::TokenBudget { max_output_tokens: 256 },
//!     options: aicontext_providers::ModelOptions::default(),
//!     idempotency_key: "run-1".to_owned(),
//! };
//! let answer = provider.complete(request).await?;
//! # let _ = answer;
//! # Ok(())
//! # }
//! ```
//!
//! # What this adapter does not do
//!
//! It never resolves a credential (that is `TASK-052`): [`Auth::Bearer`] carries a value that was
//! handed in, and the value is redacted everywhere it could be printed. It never switches provider
//! or model on a failure (`docs/AI_PROVIDER_SPEC.md` §6): a failure is a failure.

mod retry;
mod sse;
mod stream;
mod wire;

pub use retry::RetryPolicy;

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER};
use reqwest::StatusCode;
use serde_json::Value;

use crate::cancel::CancellationToken;
use crate::error::{ProviderError, TimeoutPhase};
use crate::provider::{AIProvider, BoxStream};
use crate::secret::Secret;
use crate::types::{
    Capabilities, CompletionRequest, CompletionResponse, FinishReason, ProviderId, StreamEvent,
};

/// How the adapter proves who it is.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Auth {
    /// No credential. A typical local server does not want one.
    None,
    /// A bearer token, sent as `Authorization: Bearer …`. The value is a [`Secret`], so it cannot be
    /// printed by accident (`docs/AI_PROVIDER_SPEC.md` §11 T10).
    Bearer(Secret),
}

impl Default for Auth {
    fn default() -> Self {
        Self::None
    }
}

/// The three timeouts the spec requires, each configurable
/// (`docs/AI_PROVIDER_SPEC.md` §6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timeouts {
    /// The limit on establishing a connection.
    pub connect: Duration,
    /// The limit on one request and its response.
    pub request: Duration,
    /// The limit on the whole call, retries included.
    pub total: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(60),
            total: Duration::from_secs(120),
        }
    }
}

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
    /// The three timeouts.
    pub timeouts: Timeouts,
    /// The retry schedule.
    pub retry: RetryPolicy,
    /// The run's cancellation token.
    pub cancel: CancellationToken,
}

impl OpenAiCompatibleConfig {
    /// A configuration for `base_url` and `model`, with no auth, the default timeouts, the default
    /// retry schedule, and a token nobody has cancelled.
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

    /// Sets the authentication mode.
    #[must_use]
    pub fn with_auth(mut self, auth: Auth) -> Self {
        self.auth = auth;
        self
    }

    /// Sets the three timeouts.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Sets the retry schedule.
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Sets the cancellation token the run shares.
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// A provider that speaks the OpenAI chat-completions shape.
#[derive(Debug)]
pub struct OpenAiCompatible {
    config: OpenAiCompatibleConfig,
    client: reqwest::Client,
}

impl OpenAiCompatible {
    /// Builds the client, which is where the connect timeout is configured.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Transport`] if the client cannot be built at all.
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .connect_timeout(config.timeouts.connect)
            .build()
            .map_err(|source| ProviderError::Transport {
                provider: config.id.clone(),
                source: Box::new(source),
            })?;
        Ok(Self { config, client })
    }

    /// The configuration this adapter was built with.
    #[must_use]
    pub fn config(&self) -> &OpenAiCompatibleConfig {
        &self.config
    }

    /// The absolute URL of a completion call.
    fn endpoint(&self) -> String {
        format!("{}{}", self.config.base_url.trim_end_matches('/'), wire::PATH)
    }

    /// Turns an HTTP failure into the typed error the spec names, reading the provider's message.
    async fn failure(&self, response: reqwest::Response) -> ProviderError {
        let provider = self.config.id.clone();
        let status = response.status();
        let retry_after = retry_after(response.headers());
        let detail = match response.bytes().await {
            Ok(bytes) => wire::error_detail(&bytes),
            Err(error) => return self.classify(error),
        };
        match status.as_u16() {
            400 => ProviderError::InvalidRequest { provider, detail },
            401 => ProviderError::Unauthorized { provider },
            403 => ProviderError::Forbidden { provider },
            404 => ProviderError::NotFound { provider, detail },
            429 => ProviderError::RateLimited {
                provider,
                retry_after,
            },
            code if (500..600).contains(&code) => ProviderError::ServerError {
                provider,
                status: code,
            },
            code if (400..500).contains(&code) => ProviderError::InvalidRequest { provider, detail },
            code => ProviderError::MalformedResponse {
                provider,
                detail: format!("unexpected HTTP status {code}"),
            },
        }
    }

    /// Sends one attempt, honouring the request timeout and racing cancellation.
    async fn attempt(&self, body: &Value) -> Result<reqwest::Response, ProviderError> {
        let provider = self.config.id.clone();
        let mut request = self
            .client
            .post(self.endpoint())
            .timeout(self.config.timeouts.request)
            .header(CONTENT_TYPE, "application/json");
        if let Auth::Bearer(secret) = &self.config.auth {
            // Constructing the header by hand is what keeps an invalid credential a typed error
            // rather than a panic inside reqwest, and none of it prints the value.
            let value = HeaderValue::from_str(&format!("Bearer {}", secret.expose())).map_err(|_| {
                ProviderError::InvalidRequest {
                    provider: provider.clone(),
                    detail: "the configured credential is not a valid HTTP header value".to_owned(),
                }
            })?;
            request = request.header(AUTHORIZATION, value);
        }
        let encoded = serde_json::to_vec(body).map_err(|error| ProviderError::InvalidRequest {
            provider: provider.clone(),
            detail: error.to_string(),
        })?;
        let cancel = self.config.cancel.clone();
        tokio::select! {
            () = cancel.cancelled() => Err(ProviderError::Cancelled { provider }),
            result = request.body(encoded).send() => {
                result.map_err(|error| self.classify(error))
            }
        }
    }

    /// Sends the body with bounded retries, returning a successful response.
    async fn dispatch(
        &self,
        body: &Value,
        idempotent: bool,
        seed: u64,
    ) -> Result<reqwest::Response, ProviderError> {
        let mut attempt = 0u32;
        loop {
            match self.attempt(body).await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let error = self.failure(response).await;
                    if !self.may_retry(&error, idempotent, attempt) {
                        return Err(error);
                    }
                    self.pause(attempt, seed, &error).await;
                }
                Err(error) => {
                    if !self.may_retry(&error, idempotent, attempt) {
                        return Err(error);
                    }
                    self.pause(attempt, seed, &error).await;
                }
            }
            attempt += 1;
        }
    }

    /// Whether the loop may make another attempt.
    fn may_retry(&self, error: &ProviderError, idempotent: bool, attempt: u32) -> bool {
        idempotent && attempt < self.config.retry.max_retries && RetryPolicy::is_retryable(error)
    }

    /// Waits out the backoff, logging the retry and watching for cancellation.
    async fn pause(&self, attempt: u32, seed: u64, error: &ProviderError) {
        let delay = self.delay(attempt, seed, error);
        tracing::warn!(
            provider = %self.config.id,
            code = error.code(),
            attempt = attempt + 1,
            delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            "provider call failed; retrying"
        );
        let cancel = self.config.cancel.clone();
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = cancel.cancelled() => {}
        }
    }

    /// The pause before `attempt`, preferring the provider's own retry-after hint.
    fn pause_for(&self, error: &ProviderError) -> Option<Duration> {
        match error {
            ProviderError::RateLimited {
                retry_after: Some(hint),
                ..
            } => Some((*hint).min(self.config.retry.max_delay)),
            _ => None,
        }
    }

    /// The pause before `attempt`.
    async fn _unused(&self) {}

    /// The classification of a transport-level failure.
    fn classify(&self, error: reqwest::Error) -> ProviderError {
        let provider = self.config.id.clone();
        if error.is_timeout() {
            let (phase, limit) = if error.is_connect() {
                (TimeoutPhase::Connect, self.config.timeouts.connect)
            } else {
                (TimeoutPhase::Request, self.config.timeouts.request)
            };
            return ProviderError::Timeout {
                provider,
                phase,
                limit,
            };
        }
        ProviderError::Transport {
            provider,
            source: Box::new(error),
        }
    }

    /// The pause before another attempt, either the policy's or the provider's hint.
    fn delay_for(&self, attempt: u32, seed: u64, error: &ProviderError) -> Duration {
        self.pause_for(error)
            .unwrap_or_else(|| self.config.retry.delay(attempt, seed))
    }

    /// The prediction used by `pause`.
    fn retry_ready(&self) -> bool {
        true
    }
}

/// Sends the body and, on failure, either retries or returns the typed error.
#[async_trait]
impl AIProvider for OpenAiCompatible {
    fn id(&self) -> &ProviderId {
        &self.config.id
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let provider = self.config.id.clone();
        let limit = self.config.timeouts.total;
        match tokio::time::timeout(limit, self.complete_inner(request)).await {
            Ok(result) => result,
            Err(_) => Err(ProviderError::Timeout {
                provider,
                phase: TimeoutPhase::Total,
                limit,
            }),
        }
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<BoxStream<StreamEvent>, ProviderError> {
        let body = wire::request_body(&request, &self.config.model, true);
        let seed = seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self.dispatch(&body, idempotent, seed).await?;
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

impl OpenAiCompatible {
    /// Answers in one shot, without the total-timeout wrapper.
    async fn complete_inner(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let body = wire::request_body(&request, &self.config.model, false);
        let seed = seed_of(&request.idempotency_key);
        let idempotent = !request.idempotency_key.is_empty();
        let response = self.dispatch(&body, idempotent, seed).await?;
        let bytes = response
            .bytes()
            .await
            .map_err(|error| self.classify(error))?;
        wire::parse_completion(&self.config.id, &bytes)
    }
}

/// The `Retry-After` header as whole seconds; a date form is left to a later task.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// A deterministic seed for the retry jitter.
fn seed_of(idempotency_key: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in idempotency_key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
