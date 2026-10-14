//! Cooperative cancellation for an in-flight provider call.
//!
//! `docs/AI_PROVIDER_SPEC.md` §6: Ctrl-C cancels the in-flight request, records a partial-usage
//! record, and exits 9. The exit code belongs to the command; what the provider layer owes it is a
//! call that stops promptly and reports [`crate::ProviderError::Cancelled`] rather than hanging or
//! being killed mid-write.
//!
//! The token is shared (`Arc`) because the signal handler and the call are on different tasks. It is
//! a one-way flag: once cancelled, it stays cancelled, which is what makes racing a request against
//! it free of a lost-wakeup race.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// A shared, one-way cancellation signal.
///
/// A clone cancels the same call: the adapter holds one clone and the CLI signal handler holds
/// another.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

/// The shared state behind every clone of a [`CancellationToken`].
#[derive(Debug, Default)]
struct Inner {
    cancelled: AtomicBool,
    changed: Notify,
}

impl CancellationToken {
    /// A token that no one has cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels this token and everyone holding a clone.
    ///
    /// Idempotent: cancelling twice is the same as cancelling once.
    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.changed.notify_waiters();
        }
    }

    /// Whether cancellation has already been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves when [`CancellationToken::cancel`] is called, or immediately if it already has been.
    ///
    /// Safe to race against a request: a cancel between the `is_cancelled` check and this wait still
    /// wakes the waiter, because `Notify` stores one permit.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        // `notify_waiters` only wakes waiters already registered, so register first and re-check
        // afterwards: the check order is what closes the race.
        let notified = self.inner.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::CancellationToken;

    #[test]
    fn a_fresh_token_is_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancelling_every_clone_is_visible_to_every_clone() {
        let token = CancellationToken::new();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }

    #[test]
    fn awaiting_an_already_cancelled_token_returns_at_once() {
        let token = CancellationToken::new();
        token.cancel();
        futures_executor::block_on(token.cancelled());
        assert!(token.is_cancelled());
    }
}
