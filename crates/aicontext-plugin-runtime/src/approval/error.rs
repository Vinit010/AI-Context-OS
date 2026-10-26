//! Failures that stop the broker before it can reach a decision.
//!
//! A human saying `no`, a missing terminal, and an interrupted prompt are outcomes, not errors:
//! they are expected answers the caller reports. An audit sink that cannot record is different -
//! the broker cannot know that a prompt or a decision was preserved, so the call must not proceed.
//! That failure carries `PLG-010`, "approval required but unavailable" (`docs/PLUGIN_SPEC.md` §4).

use aicontext_core::{AicontextError, ErrorCode};

/// The broker could not produce a decision that it could audit.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApprovalError {
    /// The audit sink rejected a prompt or decision event.
    #[error("the approval decision could not be audited: {message}")]
    Audit {
        /// The sink's own message, preserved rather than replaced by a generic one.
        message: String,
    },
}

impl ApprovalError {
    /// The stable code for an approval that could not be made available.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        ErrorCode::PLG_010
    }

    /// What the caller should do about it.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::Audit { .. } => {
                "restore write access to the audit log and retry; the tool call was refused"
            }
        }
    }
}

impl From<ApprovalError> for AicontextError {
    fn from(error: ApprovalError) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use aicontext_core::ErrorCode;

    use super::ApprovalError;

    #[test]
    fn an_audit_failure_carries_the_unavailable_approval_code_and_a_remediation() {
        let error = ApprovalError::Audit {
            message: "audit log is closed".to_owned(),
        };

        assert_eq!(error.code(), ErrorCode::PLG_010);
        assert_eq!(error.code().to_string(), "PLG-010");
        assert!(!error.remediation().is_empty());
        assert!(error.to_string().contains("audit log is closed"));
    }

    #[test]
    fn the_conversion_keeps_the_cause_and_the_code() {
        let converted = aicontext_core::AicontextError::from(ApprovalError::Audit {
            message: "disk full".to_owned(),
        });

        assert_eq!(converted.code(), ErrorCode::PLG_010);
        assert!(converted.message().contains("disk full"));
        assert!(converted.source().is_some());
    }
}
