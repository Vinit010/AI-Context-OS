//! The gateway's audit seam and the facts it records.
//!
//! `docs/SECURITY.md` §8 requires one record per event, and §9 requires a call whose approval
//! cannot be recorded to be refused. The gateway therefore treats a sink failure as a refusal at
//! every stage, not as a warning.
//!
//! There are four event kinds:
//!
//! - [`GatewayEvent::SandboxRefused`] is written before a plugin process exists, when the manifest
//!   names a path, host, or environment variable the sandbox cannot accept.
//! - [`GatewayEvent::Approval`] forwards the broker's own prompt and decision events through the
//!   same sink, so an operator sees `Prompted`, `Decided`, `Executing`, and `Completed` in call
//!   order rather than in two files.
//! - [`GatewayEvent::Executing`] is written after every check has passed and immediately before the
//!   plugin call. A call that has an `Executing` record and no `Completed` record was interrupted
//!   while the plugin was running.
//! - [`GatewayEvent::Completed`] is the terminal record for every call that reached the gateway,
//!   including validation refusals and policy denials. A refusal before execution has no
//!   `Executing` record.
//!
//! Summaries deliberately contain a digest of the arguments, not the arguments, and never the
//! caller's reason. The reasoning is the same as the approval broker's: a credential can hide in an
//! argument and a reason is free-form model output (`docs/SECURITY.md` §6).

use std::error::Error;

use aicontext_core::{ErrorCode, PermissionMode};
use aicontext_plugin_sdk::Effect;

use crate::approval::{ApprovalAudit, ApprovalEvent};

/// One call's audit-safe identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallSummary {
    tool: String,
    plugin: String,
    target: String,
    arguments_digest: String,
    mode: Option<PermissionMode>,
    effect: Option<Effect>,
    destructive: bool,
}

impl CallSummary {
    pub(crate) fn new(
        tool: impl Into<String>,
        plugin: impl Into<String>,
        target: impl Into<String>,
        arguments_digest: impl Into<String>,
        mode: Option<PermissionMode>,
        effect: Option<Effect>,
        destructive: bool,
    ) -> Self {
        Self {
            tool: tool.into(),
            plugin: plugin.into(),
            target: target.into(),
            arguments_digest: arguments_digest.into(),
            mode,
            effect,
            destructive,
        }
    }

    pub(crate) fn with_mode(mut self, mode: PermissionMode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// The capability's canonical tool name.
    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The plugin that provides the capability.
    #[must_use]
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The resource the call targets, or an empty string.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// `sha256:<hex>` over the compact JSON argument set.
    #[must_use]
    pub fn arguments_digest(&self) -> &str {
        &self.arguments_digest
    }

    /// The resolved permission mode, or `None` when validation stopped the call first.
    #[must_use]
    pub const fn mode(&self) -> Option<PermissionMode> {
        self.mode
    }

    /// The manifest's effect for the capability, when it is declared.
    #[must_use]
    pub const fn effect(&self) -> Option<Effect> {
        self.effect
    }

    /// Whether the manifest marks the capability destructive.
    #[must_use]
    pub const fn destructive(&self) -> bool {
        self.destructive
    }
}

/// The last fact recorded about one call.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResultSummary {
    /// The plugin answered. `is_error` carries the plugin's own refusal as data.
    Succeeded {
        /// The plugin's `is_error` flag.
        is_error: bool,
        /// How long the plugin call took.
        duration_ms: u64,
        /// The serialised envelope's size.
        bytes: usize,
        /// `sha256:<hex>` of the serialised envelope.
        digest: String,
    },
    /// The call was refused before a result existed, or the supervisor failed.
    Refused {
        /// The first stage that stopped the call.
        stage: RefusalStage,
        /// The stable code, when the refusal is a `PLG` failure rather than a denial.
        code: Option<ErrorCode>,
        /// A human-readable explanation. Never contains raw arguments.
        detail: String,
    },
}

impl ResultSummary {
    /// Whether the plugin answered.
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded { .. })
    }

    /// The stage that refused the call, when it did not execute.
    #[must_use]
    pub const fn refusal_stage(&self) -> Option<RefusalStage> {
        match self {
            Self::Succeeded { .. } => None,
            Self::Refused { stage, .. } => Some(*stage),
        }
    }
}

/// The first gate that stopped a call.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RefusalStage {
    /// The tool is not declared, the arguments do not match the schema, or the schema is invalid.
    Validation,
    /// The manifest does not declare the capability, or the policy resolved to `DENY`.
    Permission,
    /// The human, or the absence of a human, denied the call.
    Approval,
    /// The plugin process failed while executing.
    Execution,
    /// The result could not be normalised within the envelope limits.
    Envelope,
}

impl RefusalStage {
    /// The stable wire form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Permission => "permission",
            Self::Approval => "approval",
            Self::Execution => "execution",
            Self::Envelope => "envelope",
        }
    }
}

/// One fact the gateway asks the audit sink to record.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GatewayEvent {
    /// The sandbox refused to prepare the plugin, before any process existed.
    ///
    /// `detail` is the sandbox error's message: it names the plugin, the control, and the refused
    /// value, and never contains a credential. The event is written before the error is returned,
    /// so an escape attempt is auditable even though no call happened.
    SandboxRefused {
        /// The plugin the manifest names.
        plugin: String,
        /// The stable `PLG` code for the refusal.
        code: ErrorCode,
        /// What was refused, without an environment value or raw arguments.
        detail: String,
    },
    /// A broker prompt or decision, forwarded unchanged so one sink sees one sequence.
    Approval(ApprovalEvent),
    /// Every check passed; the plugin call is about to start.
    Executing {
        /// The call being dispatched.
        call: CallSummary,
    },
    /// The call reached a terminal state.
    Completed {
        /// The call that ended.
        call: CallSummary,
        /// Whether it produced a result or was refused.
        result: ResultSummary,
    },
}

impl GatewayEvent {
    /// The call this event is about, when it is a gateway event.
    #[must_use]
    pub const fn call(&self) -> Option<&CallSummary> {
        match self {
            Self::Executing { call } | Self::Completed { call, .. } => Some(call),
            Self::SandboxRefused { .. } | Self::Approval(_) => None,
        }
    }

    /// The broker event, when this is an approval event.
    #[must_use]
    pub const fn approval(&self) -> Option<&ApprovalEvent> {
        match self {
            Self::Approval(event) => Some(event),
            Self::SandboxRefused { .. } | Self::Executing { .. } | Self::Completed { .. } => None,
        }
    }
}

/// Where the gateway records every approval, dispatch, and completion.
///
/// The error type is the sink's own, so the durable hash-chained log (`TASK-075`) can return its
/// typed chain error without this crate depending on it. A sink failure refuses the call with
/// `PLG-010`: a record that cannot be written is not a record.
pub trait GatewayAudit {
    /// The sink's failure type.
    type Error: Error + Send + Sync + 'static;

    /// Records one event. Returning an error refuses the call.
    ///
    /// # Errors
    ///
    /// The sink's own failure. The gateway does not retry: a second append could duplicate an
    /// event.
    fn record(&mut self, event: &GatewayEvent) -> Result<(), Self::Error>;
}

/// Adapts the gateway sink to the broker's narrower [`ApprovalAudit`] trait.
#[derive(Debug)]
pub(crate) struct ApprovalBridge<'a, A> {
    sink: &'a mut A,
}

impl<'a, A> ApprovalBridge<'a, A> {
    pub(crate) const fn new(sink: &'a mut A) -> Self {
        Self { sink }
    }
}

impl<A: GatewayAudit> ApprovalAudit for ApprovalBridge<'_, A> {
    type Error = A::Error;

    fn record(&mut self, event: &ApprovalEvent) -> Result<(), Self::Error> {
        self.sink.record(&GatewayEvent::Approval(event.clone()))
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::PermissionMode;
    use aicontext_plugin_sdk::Effect;

    use super::{
        ApprovalBridge, CallSummary, GatewayAudit, GatewayEvent, RefusalStage, ResultSummary,
    };
    use crate::approval::{ApprovalAudit, ApprovalEvent, ApprovalOutcome, DenialReason};

    fn summary() -> CallSummary {
        CallSummary::new(
            "fixture.echo.run",
            "fixture",
            "target",
            "sha256:abc",
            Some(PermissionMode::Allow),
            Some(Effect::Write),
            false,
        )
    }

    #[derive(Debug, Default)]
    struct Recorder {
        events: Vec<GatewayEvent>,
    }

    impl GatewayAudit for Recorder {
        type Error = std::convert::Infallible;

        fn record(&mut self, event: &GatewayEvent) -> Result<(), Self::Error> {
            self.events.push(event.clone());
            Ok(())
        }
    }

    #[test]
    fn a_result_summary_is_either_a_success_or_the_stage_that_refused() {
        let success = ResultSummary::Succeeded {
            is_error: true,
            duration_ms: 5,
            bytes: 100,
            digest: "sha256:abc".to_owned(),
        };
        assert!(success.is_success());
        assert_eq!(success.refusal_stage(), None);

        let refused = ResultSummary::Refused {
            stage: RefusalStage::Approval,
            code: None,
            detail: DenialReason::Declined.to_string(),
        };
        assert!(!refused.is_success());
        assert_eq!(refused.refusal_stage(), Some(RefusalStage::Approval));
    }

    #[test]
    fn the_broker_events_are_forwarded_without_a_second_sink() {
        let mut recorder = Recorder::default();
        let mut bridge = ApprovalBridge::new(&mut recorder);
        let event = ApprovalEvent::Decided {
            summary: crate::approval::ApprovalRequest::new(
                "fixture.echo.run",
                "fixture",
                "target",
                serde_json::json!({"text": "x"}),
                PermissionMode::Allow,
            )
            .summary(),
            outcome: ApprovalOutcome::Denied {
                reason: DenialReason::NotInteractive,
            },
        };

        bridge.record(&event).expect("the recorder is infallible");

        assert_eq!(recorder.events.len(), 1);
        assert_eq!(recorder.events[0].approval(), Some(&event));
        assert!(recorder.events[0].call().is_none());
    }

    #[test]
    fn only_gateway_events_carry_a_call_summary() {
        let executing = GatewayEvent::Executing { call: summary() };
        assert_eq!(
            executing.call().map(CallSummary::tool),
            Some("fixture.echo.run")
        );
        assert!(executing.approval().is_none());

        let completed = GatewayEvent::Completed {
            call: summary(),
            result: ResultSummary::Refused {
                stage: RefusalStage::Validation,
                code: Some(aicontext_core::ErrorCode::PLG_012),
                detail: "bad arguments".to_owned(),
            },
        };
        assert_eq!(
            completed.call().map(CallSummary::mode),
            Some(Some(PermissionMode::Allow))
        );
    }
}
