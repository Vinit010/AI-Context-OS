//! The typed failures of calling an AI provider.
//!
//! `RULES.md` §4.2 asks for one enum per failure domain, and the domain here is a single thing:
//! a call to a model provider did not return a usable answer. A caller that must decide whether to
//! retry, re-authenticate, or give up cannot make that decision from a message string, which is the
//! reason the variants are separate types rather than one `Failed { message }`.
//!
//! ```text
//! PROVIDER_TRANSPORT           the provider could not be reached
//! PROVIDER_TIMEOUT             a configured connect, request, or total timeout elapsed
//! PROVIDER_UNAUTHORIZED        the credential is missing, expired, or wrong (401)
//! PROVIDER_FORBIDDEN           the credential may not call this model (403)
//! PROVIDER_NOT_FOUND           the model or endpoint does not exist (404)
//! PROVIDER_INVALID_REQUEST     the provider rejected the request shape (400)
//! PROVIDER_RATE_LIMITED        the provider asked the caller to slow down (429)
//! PROVIDER_SERVER_ERROR        the provider failed on its own side (5xx)
//! PROVIDER_MALFORMED_RESPONSE  a body this adapter could not read
//! PROVIDER_UNSUPPORTED         a capability this provider does not offer
//! PROVIDER_CANCELLED           the run was cancelled rather than failing
//! PROVIDER_INVALID_ID          a provider id that is not a lowercase slug
//! ```
//!
//! # Why the codes are names, not `PREFIX-NNN`
//!
//! The spec names the timeout failure `PROVIDER_TIMEOUT` (`docs/AI_PROVIDER_SPEC.md` §6), and there
//! is no external catalogue here to number the way `CTX-NNN` numbers the `doctor` checks. The code a
//! script branches on is therefore the string, and the mapping onto a process exit code (`6`,
//! `PROVIDER_ERROR`, in `docs/CLI_SPEC.md` §5) belongs to the command that runs a provider rather
//! than to this type.
//!
//! # What is deliberately not here
//!
//! There is no variant for a *refusal*. `finish_reason: Refusal` is a successful answer with a
//! [`crate::FinishReason`], reported with the model's text, not a failure to retry around
//! (`docs/AI_PROVIDER_SPEC.md` §6).

use std::error::Error;
use std::fmt;
use std::time::Duration;

use thiserror::Error;

use crate::types::ProviderId;

/// The typed failure of a provider call, with a stable code and a remediation hint.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProviderError {
    /// The provider could not be reached: DNS, connection, TLS, or a broken read.
    #[error("`{provider}` could not be reached: {source}")]
    Transport {
        /// The provider that could not be reached.
        provider: ProviderId,
        /// The transport's own error, kept rather than flattened into the message
        /// (`RULES.md` §4.4).
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A configured timeout elapsed (`docs/AI_PROVIDER_SPEC.md` §6).
    #[error("`{provider}` timed out during the {phase} phase (limit {limit:?})")]
    Timeout {
        /// The provider that did not answer in time.
        provider: ProviderId,
        /// Which timeout elapsed.
        phase: TimeoutPhase,
        /// The limit that was configured.
        limit: Duration,
    },

    /// The credential is missing, expired, or wrong (HTTP 401).
    #[error("`{provider}` rejected the credential")]
    Unauthorized {
        /// The provider that rejected the credential.
        provider: ProviderId,
    },

    /// The credential is valid but may not call this model (HTTP 403).
    #[error("`{provider}` refused this credential")]
    Forbidden {
        /// The provider that refused.
        provider: ProviderId,
    },

    /// The model or endpoint does not exist (HTTP 404).
    #[error("`{provider}` does not know the requested model or endpoint: {detail}")]
    NotFound {
        /// The provider that was asked.
        provider: ProviderId,
        /// Which model or endpoint was not found.
        detail: String,
    },

    /// The provider rejected the request shape (HTTP 400).
    #[error("`{provider}` rejected the request: {detail}")]
    InvalidRequest {
        /// The provider that rejected the request.
        provider: ProviderId,
        /// What the provider said was wrong.
        detail: String,
    },

    /// The provider asked the caller to slow down (HTTP 429).
    #[error("`{provider}` rate limited the request")]
    RateLimited {
        /// The provider that rate limited the request.
        provider: ProviderId,
        /// The provider's retry-after hint, when it gave one.
        retry_after: Option<Duration>,
    },

    /// The provider failed on its own side (HTTP 5xx).
    #[error("`{provider}` returned server error {status}")]
    ServerError {
        /// The provider that failed.
        provider: ProviderId,
        /// The status the provider returned.
        status: u16,
    },

    /// The provider returned a body this adapter could not read.
    #[error("`{provider}` returned a response aicontext does not understand: {detail}")]
    MalformedResponse {
        /// The provider that returned the body.
        provider: ProviderId,
        /// What was wrong with it.
        detail: String,
    },

    /// The provider does not offer something the request requires.
    #[error("`{provider}` does not support {capability}")]
    Unsupported {
        /// The provider that lacks the capability.
        provider: ProviderId,
        /// The capability, named as the host asked for it.
        capability: &'static str,
    },

    /// The run was cancelled by the user or a signal, rather than failing
    /// (`docs/AI_PROVIDER_SPEC.md` §6, `docs/CLI_SPEC.md` §5 exit 9).
    #[error("`{provider}` was cancelled before it finished")]
    Cancelled {
        /// The provider whose call was interrupted.
        provider: ProviderId,
    },

    /// A provider id that is not a lowercase slug.
    #[error(
        "`{value}` is not a provider id: an id starts with a lowercase letter and holds only \
         lowercase letters, digits, `-`, and `_`"
    )]
    InvalidId {
        /// The value that was offered as an id.
        value: String,
    },
}

impl ProviderError {
    /// The stable code for this failure, safe to branch on.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Transport { .. } => "PROVIDER_TRANSPORT",
            Self::Timeout { .. } => "PROVIDER_TIMEOUT",
            Self::Unauthorized { .. } => "PROVIDER_UNAUTHORIZED",
            Self::Forbidden { .. } => "PROVIDER_FORBIDDEN",
            Self::NotFound { .. } => "PROVIDER_NOT_FOUND",
            Self::InvalidRequest { .. } => "PROVIDER_INVALID_REQUEST",
            Self::RateLimited { .. } => "PROVIDER_RATE_LIMITED",
            Self::ServerError { .. } => "PROVIDER_SERVER_ERROR",
            Self::MalformedResponse { .. } => "PROVIDER_MALFORMED_RESPONSE",
            Self::Unsupported { .. } => "PROVIDER_UNSUPPORTED",
            Self::Cancelled { .. } => "PROVIDER_CANCELLED",
            Self::InvalidId { .. } => "PROVIDER_INVALID_ID",
        }
    }

    /// What the caller should do about it (`RULES.md` §4.3).
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::Transport { .. } => {
                "check the provider's `base_url` and that the server is reachable, then try again"
            }
            Self::Timeout { .. } => {
                "raise the configured timeout for that phase, or send a smaller request"
            }
            Self::Unauthorized { .. } => {
                "set the provider's credential with `aicontext connect`, and check it has not expired"
            }
            Self::Forbidden { .. } => {
                "use a credential that may call this model, or choose another provider"
            }
            Self::NotFound { .. } => {
                "check the model name and `base_url`; the provider does not know this model"
            }
            Self::InvalidRequest { .. } => {
                "this is a defect in the request aicontext built, not in your configuration; please \
                 report it"
            }
            Self::RateLimited { .. } => {
                "wait for the provider's retry-after window, or use a provider with more headroom"
            }
            Self::ServerError { .. } => "the provider failed on its own side; try again later",
            Self::MalformedResponse { .. } => {
                "the provider returned a shape aicontext does not understand; check the server's \
                 version and report it if it is current"
            }
            Self::Unsupported { .. } => {
                "choose a provider that supports this capability, or turn the feature off"
            }
            Self::Cancelled { .. } => "re-run when you are ready; the call was cancelled, not failed",
            Self::InvalidId { .. } => {
                "name the provider with a lowercase slug, for example `local` or `openai-hosted`"
            }
        }
    }
}

/// Which of the three configured timeouts elapsed (`docs/AI_PROVIDER_SPEC.md` §6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TimeoutPhase {
    /// Establishing the connection.
    Connect,
    /// Waiting for the provider's answer to one request.
    Request,
    /// The whole call, including retries.
    Total,
}

impl fmt::Display for TimeoutPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connect => "connect",
            Self::Request => "request",
            Self::Total => "total",
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use super::{ProviderError, TimeoutPhase};
    use crate::ProviderId;

    fn provider() -> ProviderId {
        ProviderId::new("fake").expect("`fake` is a valid id")
    }

    fn every_variant() -> Vec<ProviderError> {
        vec![
            ProviderError::Transport {
                provider: provider(),
                source: Box::new(std::io::Error::other("connection refused")),
            },
            ProviderError::Timeout {
                provider: provider(),
                phase: TimeoutPhase::Request,
                limit: Duration::from_secs(30),
            },
            ProviderError::Unauthorized {
                provider: provider(),
            },
            ProviderError::Forbidden {
                provider: provider(),
            },
            ProviderError::NotFound {
                provider: provider(),
                detail: "model `qwen2.5-coder`".to_owned(),
            },
            ProviderError::InvalidRequest {
                provider: provider(),
                detail: "`messages` was empty".to_owned(),
            },
            ProviderError::RateLimited {
                provider: provider(),
                retry_after: Some(Duration::from_secs(5)),
            },
            ProviderError::ServerError {
                provider: provider(),
                status: 503,
            },
            ProviderError::MalformedResponse {
                provider: provider(),
                detail: "`choices` missing".to_owned(),
            },
            ProviderError::Unsupported {
                provider: provider(),
                capability: "streaming",
            },
            ProviderError::Cancelled {
                provider: provider(),
            },
            ProviderError::InvalidId {
                value: "Bad Id".to_owned(),
            },
        ]
    }

    #[test]
    fn every_variant_has_a_stable_code_and_a_remediation() {
        for error in every_variant() {
            assert!(
                error.code().starts_with("PROVIDER_"),
                "{error} has an off-family code: {}",
                error.code()
            );
            assert!(
                !error.remediation().is_empty(),
                "{error} has no remediation, so a caller would be left guessing"
            );
            assert!(!error.to_string().is_empty(), "a variant renders as nothing");
        }
    }

    #[test]
    fn the_codes_are_distinct() {
        let codes: BTreeSet<&'static str> =
            every_variant().iter().map(ProviderError::code).collect();
        assert_eq!(
            codes.len(),
            every_variant().len(),
            "each failure mode needs its own code, or a script cannot tell them apart"
        );
    }

    #[test]
    fn a_timeout_names_the_phase_that_elapsed() {
        for (phase, text) in [
            (TimeoutPhase::Connect, "connect"),
            (TimeoutPhase::Request, "request"),
            (TimeoutPhase::Total, "total"),
        ] {
            assert_eq!(phase.to_string(), text);
            let error = ProviderError::Timeout {
                provider: provider(),
                phase,
                limit: Duration::from_millis(1),
            };
            assert!(
                error.to_string().contains(text),
                "the phase must appear in the message, got: {error}"
            );
        }
    }

    #[test]
    fn a_transport_failure_keeps_its_source() {
        // RULES.md 4.4: the source chain is the difference between a five-minute and a two-hour fix.
        let error = ProviderError::Transport {
            provider: provider(),
            source: Box::new(std::io::Error::other("connection refused")),
        };
        let source = std::error::Error::source(&error).expect("the transport error is the source");
        assert!(source.to_string().contains("connection refused"));
    }
}
