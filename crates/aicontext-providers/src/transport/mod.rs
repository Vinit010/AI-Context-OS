//! What every adapter shares: credentials, timeouts, retries, error parsing, and the stream pump.
//!
//! `docs/AI_PROVIDER_SPEC.md` §3 gives three adapters and §6 makes deadlines, bounded retries,
//! cancellation, and error classification part of the contract every one of them obeys. The three
//! differ only in their wire format; the conversation with the HTTP server — one client, three
//! timeouts, one retry policy, one set of failure classes, one SSE grammar — is identical, so it is
//! written once here and each adapter's module holds only its own mapping onto the wire
//! (`docs/AI_PROVIDER_SPEC.md` §12: an adapter lives in one module).
//!
//! This module was lifted out of the OpenAI-compatible adapter when the second and third arrived,
//! and the rules it implements are unchanged; only the URL, the headers, and the meaning of a
//! response moved out to the adapter.

pub(crate) mod envelope;
pub(crate) mod retry;
pub(crate) mod sse;
pub(crate) mod stream;

use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER};
use serde_json::Value;

use crate::cancel::CancellationToken;
use crate::credential::Redactor;
use crate::error::{ProviderError, TimeoutPhase};
use crate::secret::Secret;
use crate::types::ProviderId;

pub use retry::RetryPolicy;

/// How an adapter proves who it is, whatever header it proves it with.
///
/// [`Secret`] is the credential itself, so it cannot be printed by accident
/// (`docs/AI_PROVIDER_SPEC.md` §11 T10). Each adapter maps a variant onto its own header with
/// [`Auth::header`], which is the only place a credential becomes an HTTP value.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub enum Auth {
    /// No credential. A typical local server does not want one.
    #[default]
    None,
    /// A bearer token, sent as `Authorization: Bearer …`.
    Bearer(Secret),
    /// An API key, sent as the raw value in the header the server reads it from: `x-api-key` for
    /// Anthropic, `x-goog-api-key` for Gemini.
    ApiKey(Secret),
}

impl Auth {
    /// The credential as a `(header name, header value)` pair, or `None` without one.
    ///
    /// A bearer token is always `Authorization`, because that is what the scheme means; an API key
    /// goes in `key_header`, which the adapter names because only it knows which header its server
    /// reads.
    pub(crate) fn header(&self, key_header: &'static str) -> Option<(String, String)> {
        match self {
            Auth::None => None,
            Auth::Bearer(secret) => Some((
                "authorization".to_owned(),
                format!("Bearer {}", secret.expose()),
            )),
            Auth::ApiKey(secret) => Some((key_header.to_owned(), secret.expose().to_owned())),
        }
    }

    /// The redactor for this credential, for the one string in a call that the provider built.
    ///
    /// A provider's error body is where a credential we sent could be echoed back, so redaction
    /// happens at this one boundary (`docs/SECURITY.md` §6) rather than at each call site.
    pub(crate) fn redactor(&self) -> Redactor {
        match self {
            Auth::None => Redactor::new(),
            Auth::Bearer(secret) | Auth::ApiKey(secret) => {
                Redactor::new().with_secret(secret.clone())
            }
        }
    }
}

/// The three timeouts the spec requires, each configurable (`docs/AI_PROVIDER_SPEC.md` §6).
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

/// One adapter's side of the HTTP conversation, shared by all three adapters.
///
/// It owns the client, the retry policy, and the cancellation token every adapter is configured
/// with, so an adapter keeps its own config and builds one of these beside it.
#[derive(Debug)]
pub(crate) struct Transport {
    provider: ProviderId,
    client: reqwest::Client,
    timeouts: Timeouts,
    retry: RetryPolicy,
    cancel: CancellationToken,
    redactor: Redactor,
}

impl Transport {
    /// Builds the client, which is where the connect timeout is configured.
    ///
    /// # Errors
    ///
    /// Returns [`ProviderError::Transport`] when the client cannot be built.
    pub(crate) fn new(
        provider: ProviderId,
        auth: &Auth,
        timeouts: Timeouts,
        retry: RetryPolicy,
        cancel: CancellationToken,
    ) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .build()
            .map_err(|source| ProviderError::Transport {
                provider: provider.clone(),
                source: Box::new(source),
            })?;
        Ok(Self {
            redactor: auth.redactor(),
            provider,
            client,
            timeouts,
            retry,
            cancel,
        })
    }

    /// Wraps one call in the timeout that covers retries too (`docs/AI_PROVIDER_SPEC.md` §6).
    pub(crate) async fn within_total<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, ProviderError>>,
    ) -> Result<T, ProviderError> {
        let provider = self.provider.clone();
        let limit = self.timeouts.total;
        match tokio::time::timeout(limit, future).await {
            Ok(result) => result,
            Err(_) => Err(ProviderError::Timeout {
                provider,
                phase: TimeoutPhase::Total,
                limit,
            }),
        }
    }

    /// Sends `body` to `url`, retrying a retryable failure when `idempotent` allows it.
    ///
    /// `headers` carries the adapter's own header set — its credential, and anything its wire
    /// format requires. The request timeout, the cancellation race, and every failure class are
    /// the ones the spec fixes, so all three adapters behave identically here.
    pub(crate) async fn dispatch(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
        idempotent: bool,
        seed: u64,
    ) -> Result<reqwest::Response, ProviderError> {
        let mut attempt = 0u32;
        loop {
            let error = match self.attempt(url, headers, body).await {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => self.failure(response).await,
                Err(error) => error,
            };
            if !self.may_retry(&error, idempotent, attempt) {
                return Err(error);
            }
            self.pause(attempt, seed, &error).await;
            attempt += 1;
        }
    }

    /// Sends one attempt, honouring the request timeout and racing cancellation.
    async fn attempt(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<reqwest::Response, ProviderError> {
        let provider = self.provider.clone();
        let mut request = self
            .client
            .post(url)
            .timeout(self.timeouts.request)
            .header(CONTENT_TYPE, "application/json");
        for (name, value) in headers {
            // Constructing the headers by hand keeps an invalid credential a typed error rather
            // than a panic inside reqwest, and none of this prints the value.
            let header =
                HeaderValue::from_str(value).map_err(|_| ProviderError::InvalidRequest {
                    provider: provider.clone(),
                    detail: "the configured credential is not a valid HTTP header value".to_owned(),
                })?;
            request = request.header(name, header);
        }
        let encoded = serde_json::to_vec(body).map_err(|error| ProviderError::InvalidRequest {
            provider: provider.clone(),
            detail: error.to_string(),
        })?;
        let cancel = self.cancel.clone();
        tokio::select! {
            () = cancel.cancelled() => Err(ProviderError::Cancelled { provider }),
            result = request.body(encoded).send() => result.map_err(|error| self.classify(error)),
        }
    }

    /// Whether the loop may make another attempt.
    fn may_retry(&self, error: &ProviderError, idempotent: bool, attempt: u32) -> bool {
        idempotent && attempt < self.retry.max_retries && RetryPolicy::is_retryable(error)
    }

    /// Waits out the backoff, logging the retry and watching for cancellation.
    async fn pause(&self, attempt: u32, seed: u64, error: &ProviderError) {
        let delay = self.delay(attempt, seed, error);
        tracing::warn!(
            provider = %self.provider,
            code = error.code(),
            retry = attempt + 1,
            delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            "provider call failed; retrying",
        );
        let cancel = self.cancel.clone();
        tokio::select! {
            biased;
            () = cancel.cancelled() => {}
            () = tokio::time::sleep(delay) => {}
        }
    }

    /// The pause before the next attempt: the provider's `Retry-After` when it gave one, the
    /// policy's backoff otherwise.
    fn delay(&self, attempt: u32, seed: u64, error: &ProviderError) -> Duration {
        if let ProviderError::RateLimited {
            retry_after: Some(after),
            ..
        } = error
        {
            return (*after).min(self.retry.max_delay);
        }
        self.retry.delay(attempt, seed)
    }

    /// The typed error for an HTTP failure, read from the provider's own body.
    ///
    /// The body is scrubbed with this provider's credential before it becomes the error's text,
    /// because a provider is the one party in a call that can echo a key back at us.
    async fn failure(&self, response: reqwest::Response) -> ProviderError {
        let provider = self.provider.clone();
        let status = response.status();
        let retry_after = retry_after(response.headers());
        let detail = match response.bytes().await {
            Ok(bytes) => self.redactor.redact(&error_detail(&bytes)).into_owned(),
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
            code if (400..500).contains(&code) => {
                ProviderError::InvalidRequest { provider, detail }
            }
            code => ProviderError::MalformedResponse {
                provider,
                detail: format!("unexpected HTTP status {code}"),
            },
        }
    }

    /// The typed error for a failure below HTTP.
    pub(crate) fn classify(&self, error: reqwest::Error) -> ProviderError {
        let provider = self.provider.clone();
        if error.is_timeout() {
            let (phase, limit) = if error.is_connect() {
                (TimeoutPhase::Connect, self.timeouts.connect)
            } else {
                (TimeoutPhase::Request, self.timeouts.request)
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
}

/// The provider's own words for what went wrong, capped so a body cannot flood an error.
///
/// The shape is the same across the three adapters — an `error` object carrying a `message`, with
/// a bare `message` tolerated — so one reader serves all of them.
pub(crate) fn error_detail(bytes: &[u8]) -> String {
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

/// The `Retry-After` header as whole seconds; the date form is left to a later task.
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// A deterministic seed for the retry jitter: FNV-1a over the idempotency key.
pub(crate) fn seed_of(idempotency_key: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in idempotency_key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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

#[cfg(test)]
mod tests {
    use super::{Auth, Timeouts, error_detail, seed_of};
    use crate::secret::Secret;

    const SECRET: &str = "sk-live-abcdefghijklmnop";

    #[test]
    fn a_bearer_token_is_authorization_and_an_api_key_is_the_named_header() {
        let bearer = Auth::Bearer(Secret::new(SECRET));
        assert_eq!(
            bearer.header("x-api-key"),
            Some(("authorization".to_owned(), format!("Bearer {SECRET}")))
        );
        let key = Auth::ApiKey(Secret::new(SECRET));
        assert_eq!(
            key.header("x-goog-api-key"),
            Some(("x-goog-api-key".to_owned(), SECRET.to_owned()))
        );
        assert_eq!(Auth::None.header("x-api-key"), None);
    }

    #[test]
    fn the_default_timeouts_cover_each_phase() {
        let timeouts = Timeouts::default();
        assert!(timeouts.connect < timeouts.request);
        assert!(timeouts.request < timeouts.total);
    }

    #[test]
    fn an_error_detail_reads_the_nested_message_and_caps_it() {
        assert_eq!(
            error_detail(br#"{"error":{"message":"quota exhausted"}}"#),
            "quota exhausted"
        );
        assert_eq!(error_detail(br#"{"message":"try later"}"#), "try later");
        let long = error_detail(br#"{"error":{"message":"x"}}"#);
        assert!(long.len() <= 410);
    }

    #[test]
    fn the_jitter_seed_is_stable_for_one_key() {
        assert_eq!(seed_of("abc"), seed_of("abc"));
        assert_ne!(seed_of("abc"), seed_of("abd"));
    }
}
