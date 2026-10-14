//! The retry policy: how a failed attempt is repeated, and in what pattern.
//!
//! `docs/AI_PROVIDER_SPEC.md` §6 says retries are bounded (two by default), only for transport
//! errors and 429/5xx, only when the request is idempotent, and only with exponential backoff and
//! jitter. This module holds those rules; the adapter is the thing that obeys them.

use std::time::Duration;

use crate::error::ProviderError;

/// The retry schedule for one provider: how many, how long, and the ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    /// The most retries after the first attempt. Two means at most three attempts.
    pub max_retries: u32,
    /// The pause before the first retry. Each later retry doubles it.
    pub base_delay: Duration,
    /// The ceiling on a single pause, so a slow provider cannot hold the run.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    /// The pause before retry number `attempt` (zero-based), including jitter.
    ///
    /// Jitter comes from `seed` through a splitmix64 step, so two retries of the same request wait
    /// the same time: the schedule is assertable without a clock, and a fleet of callers does not
    /// synchronise on one delay.
    #[must_use]
    pub fn delay(&self, attempt: u32, seed: u64) -> Duration {
        let backoff = self
            .base_delay
            .saturating_mul(1u32 << attempt.min(16))
            .min(self.max_delay);
        // The cap holds after the jitter too, so `max_delay` is a real ceiling, not a hint.
        backoff
            .saturating_add(jitter(backoff, seed ^ u64::from(attempt)))
            .min(self.max_delay)
    }

    /// Whether the spec says to retry `error` at all: a transport failure or 429/5xx, nothing else.
    ///
    /// A timeout is deliberately not retryable: it already means "this did not finish in the time
    /// you allowed", and silently spending that allowance again is the thing `RULES.md` §6 calls a
    /// silent retry.
    #[must_use]
    pub const fn is_retryable(error: &ProviderError) -> bool {
        matches!(
            error,
            ProviderError::Transport { .. }
                | ProviderError::RateLimited { .. }
                | ProviderError::ServerError { .. }
        )
    }
}

/// Up to a quarter of `base`, spread deterministically from `seed`.
fn jitter(base: Duration, seed: u64) -> Duration {
    let quarter = base / 4;
    if quarter.is_zero() {
        return Duration::ZERO;
    }
    let span = u64::try_from(quarter.as_nanos()).unwrap_or(u64::MAX);
    Duration::from_nanos(mix(seed) % span)
}

/// One splitmix64 step, used to spread retries out; not a security primitive.
const fn mix(seed: u64) -> u64 {
    let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    state ^ (state >> 31)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RetryPolicy;
    use crate::error::ProviderError;
    use crate::{ProviderId, TimeoutPhase};

    fn retryable_errors() -> Vec<ProviderError> {
        let provider = ProviderId::new("local").expect("valid");
        vec![
            ProviderError::Transport {
                provider: provider.clone(),
                source: Box::new(std::io::Error::other("reset")),
            },
            ProviderError::RateLimited {
                provider: provider.clone(),
                retry_after: None,
            },
            ProviderError::ServerError {
                provider: provider.clone(),
                status: 503,
            },
        ]
    }

    #[test]
    fn the_default_is_bounded() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_retries, 2);
        assert!(policy.base_delay < policy.max_delay);
    }

    #[test]
    fn only_transport_and_rate_and_server_errors_are_retryable() {
        for error in retryable_errors() {
            assert!(RetryPolicy::is_retryable(&error), "{error} must retry");
        }
        let provider = ProviderId::new("local").expect("valid");
        for error in [
            ProviderError::Timeout {
                provider: provider.clone(),
                phase: TimeoutPhase::Request,
                limit: Duration::from_millis(1),
            },
            ProviderError::Unauthorized {
                provider: provider.clone(),
            },
            ProviderError::InvalidRequest {
                provider: provider.clone(),
                detail: "bad".to_owned(),
            },
        ] {
            assert!(!RetryPolicy::is_retryable(&error), "{error} must not retry");
        }
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        let policy = RetryPolicy {
            max_retries: 8,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_millis(400),
        };
        let first = policy.delay(0, 1);
        let second = policy.delay(1, 1);
        let third = policy.delay(2, 1);
        assert!(first <= second, "backoff must not shrink");
        assert!(second <= third);
        assert!(third <= policy.max_delay, "the cap must hold");
        assert!(!first.is_zero());
    }

    #[test]
    fn jitter_is_deterministic_for_one_seed_and_spreads_across_seeds() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.delay(1, 7), policy.delay(1, 7));
        assert_ne!(
            policy.delay(1, 1),
            policy.delay(1, 2),
            "different seeds must not collapse to one schedule"
        );
    }
}
