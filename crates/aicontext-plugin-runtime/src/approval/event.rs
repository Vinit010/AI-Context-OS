//! The events the broker hands to the audit sink, and the outcome they describe.
//!
//! Two events cover a prompt: [`ApprovalEvent::Prompted`] is recorded *before* the prompt is shown,
//! so an interrupted prompt still leaves a trace, and [`ApprovalEvent::Decided`] records what
//! happened. A flag grant has no prompt and therefore no `Prompted` event, but it is still decided
//! and recorded.
//!
//! The events carry [`ApprovalSummary`], not the request: arguments are represented by a digest and
//! the free-form reason is absent (`docs/SECURITY.md` §6 and §8).

use std::error::Error;

use super::request::ApprovalSummary;

/// One fact the broker asks the audit sink to record.
///
/// A sink that cannot record must return an error. The broker treats that as a refusal, because an
/// approval that cannot be audited is not an approval (`docs/SECURITY.md` §8).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ApprovalEvent {
    /// The broker is about to show (or is showing) a prompt.
    Prompted {
        /// The call the human is being asked about.
        summary: ApprovalSummary,
    },
    /// The broker reached a decision, including a denial caused by a missing human.
    Decided {
        /// The call the decision is about.
        summary: ApprovalSummary,
        /// What the broker decided and why.
        outcome: ApprovalOutcome,
    },
}

impl ApprovalEvent {
    /// The audit-safe subject shared by both event kinds.
    #[must_use]
    pub const fn summary(&self) -> &ApprovalSummary {
        match self {
            Self::Prompted { summary } | Self::Decided { summary, .. } => summary,
        }
    }

    /// The decision, when this event is [`ApprovalEvent::Decided`].
    #[must_use]
    pub const fn outcome(&self) -> Option<&ApprovalOutcome> {
        match self {
            Self::Prompted { .. } => None,
            Self::Decided { outcome, .. } => Some(outcome),
        }
    }
}

/// The broker's answer for one call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ApprovalOutcome {
    /// The call may proceed.
    Allowed {
        /// How the approval was obtained.
        grant: ApprovalGrant,
    },
    /// The call must not proceed.
    Denied {
        /// Why approval was not obtained.
        reason: DenialReason,
    },
}

impl ApprovalOutcome {
    /// Whether the call may proceed.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// How approval was obtained, or `None` if the call was denied.
    #[must_use]
    pub const fn grant(self) -> Option<ApprovalGrant> {
        match self {
            Self::Allowed { grant } => Some(grant),
            Self::Denied { .. } => None,
        }
    }

    /// Why the call was denied, or `None` if it was allowed.
    #[must_use]
    pub const fn denial(self) -> Option<DenialReason> {
        match self {
            Self::Denied { reason } => Some(reason),
            Self::Allowed { .. } => None,
        }
    }
}

/// How an allowed call was approved.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ApprovalGrant {
    /// The policy resolved to `ALLOW`; no human was involved.
    Policy,
    /// The exact tool was named by `--allow-once`.
    AllowOnce,
    /// A human answered `yes` at the prompt for this invocation.
    Interactive,
}

/// Why a call was denied.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DenialReason {
    /// The policy resolved to `DENY`.
    Policy,
    /// Approval was required and no terminal was available.
    NotInteractive,
    /// A human answered with anything other than the literal word `yes`.
    Declined,
    /// The prompt reached end of input.
    EndOfInput,
    /// The prompt was interrupted.
    Interrupted,
    /// The prompter's own deadline expired before an answer.
    TimedOut,
    /// Reading or writing the prompt failed.
    PromptFailed,
    /// The answer was longer than the prompt accepts.
    AnswerTooLong,
}

impl std::fmt::Display for DenialReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Policy => "the permission policy denies the capability",
            Self::NotInteractive => {
                "approval was required but no interactive terminal was available"
            }
            Self::Declined => "the human declined the call",
            Self::EndOfInput => "the approval prompt reached end of input",
            Self::Interrupted => "the approval prompt was interrupted",
            Self::TimedOut => "the approval prompt timed out",
            Self::PromptFailed => "the approval prompt could not be read or written",
            Self::AnswerTooLong => "the approval answer was too long",
        })
    }
}

/// Where the broker records every prompt and every decision.
///
/// The error type is the sink's own, so `aicontext-audit` can return its typed chain error without
/// this crate depending on it. The broker converts any failure into
/// [`ApprovalError::Audit`](super::ApprovalError) and refuses the call.
pub trait ApprovalAudit {
    /// The sink's failure type.
    type Error: Error + Send + Sync + 'static;

    /// Records one event. Returning an error refuses the call.
    ///
    /// # Errors
    ///
    /// The sink's own failure. The broker does not retry: a second append could duplicate an event.
    fn record(&mut self, event: &ApprovalEvent) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{ApprovalEvent, ApprovalGrant, ApprovalOutcome, DenialReason};
    use crate::approval::ApprovalRequest;
    use aicontext_core::PermissionMode;
    use serde_json::json;

    fn summary() -> crate::approval::ApprovalSummary {
        ApprovalRequest::new(
            "aws.ec2.stop",
            "aws",
            "i-0abc1234",
            json!({ "instance_id": "i-0abc1234" }),
            PermissionMode::Approval,
        )
        .summary()
    }

    #[test]
    fn an_outcome_exposes_exactly_one_of_its_two_answers() {
        let allowed = ApprovalOutcome::Allowed {
            grant: ApprovalGrant::Interactive,
        };
        assert!(allowed.is_allowed());
        assert_eq!(allowed.grant(), Some(ApprovalGrant::Interactive));
        assert_eq!(allowed.denial(), None);

        let denied = ApprovalOutcome::Denied {
            reason: DenialReason::NotInteractive,
        };
        assert!(!denied.is_allowed());
        assert_eq!(denied.grant(), None);
        assert_eq!(denied.denial(), Some(DenialReason::NotInteractive));
    }

    #[test]
    fn only_a_decision_event_exposes_an_outcome() {
        let prompted = ApprovalEvent::Prompted { summary: summary() };
        assert!(prompted.outcome().is_none());
        assert_eq!(prompted.summary().tool(), "aws.ec2.stop");

        let decided = ApprovalEvent::Decided {
            summary: summary(),
            outcome: ApprovalOutcome::Denied {
                reason: DenialReason::Declined,
            },
        };
        assert_eq!(
            decided.outcome().and_then(|outcome| outcome.denial()),
            Some(DenialReason::Declined)
        );
    }
}
