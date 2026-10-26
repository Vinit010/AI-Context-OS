//! The permission policy's validation failures.
//!
//! A policy that cannot be represented honestly is refused rather than approximated. In particular
//! a wildcard `allow` or `explicit_approval` is an error: the resolution order in
//! `docs/PLUGIN_SPEC.md` §5 gives neither a step, so accepting one would leave a rule that looks
//! load-bearing and can never match.

use aicontext_core::{AicontextError, ErrorCode, PermissionMode};

/// A permission policy that cannot be constructed as written.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum PolicyError {
    /// A rule pattern is empty.
    #[error("a permission rule needs a non-empty pattern")]
    EmptyPattern,

    /// An exact rule contains a `*`.
    ///
    /// Wildcards have their own constructor so that a malformed wildcard cannot become an exact
    /// tool name that silently matches nothing.
    #[error("`{pattern}` contains `*`, but an exact rule must name one tool")]
    WildcardInExactRule {
        /// The rejected pattern.
        pattern: String,
    },

    /// A wildcard is not the one documented shape, `provider.service.*`.
    #[error(
        "`{pattern}` is not a valid wildcard; the only form is `provider.service.*`"
    )]
    InvalidWildcard {
        /// The rejected pattern.
        pattern: String,
    },

    /// A wildcard uses a mode the resolution order never consults.
    #[error(
        "wildcard `{pattern}` uses `{mode}`; wildcard rules may only use `deny` or `approval`"
    )]
    UnsupportedWildcardMode {
        /// The rejected pattern.
        pattern: String,
        /// The mode it asked for.
        mode: PermissionMode,
    },

    /// Two rules name the same pattern.
    #[error("`{pattern}` is declared more than once")]
    DuplicateRule {
        /// The repeated pattern.
        pattern: String,
    },
}

impl PolicyError {
    /// The stable code for this failure, `CTX-003` in `docs/CONTEXT_SPEC.md` §8: the policy file
    /// did not validate against its schema or the rules the resolver can honour.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        ErrorCode::CTX_003
    }

    /// What the caller should do about it.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::EmptyPattern => "give every rule a non-empty tool name",
            Self::WildcardInExactRule { .. } => {
                "use the exact tool name; a wildcard is written only as `provider.service.*` \
                 through `Rule::wildcard`"
            }
            Self::InvalidWildcard { .. } => {
                "write a wildcard as `provider.service.*`, for example `aws.ec2.*`; `*` alone and \
                 `*.action` are not valid"
            }
            Self::UnsupportedWildcardMode { .. } => {
                "use `deny` or `approval` on a wildcard, or name the exact tool for `allow` or \
                 `explicit_approval`"
            }
            Self::DuplicateRule { .. } => {
                "remove the duplicate rule; an exact rule already beats a wildcard regardless of \
                 file order"
            }
        }
    }
}

impl From<PolicyError> for AicontextError {
    fn from(error: PolicyError) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::PolicyError;

    #[test]
    fn every_policy_failure_carries_the_schema_code_and_a_remediation() {
        let errors = [
            PolicyError::EmptyPattern,
            PolicyError::WildcardInExactRule {
                pattern: "aws.*".to_owned(),
            },
            PolicyError::InvalidWildcard {
                pattern: "*.action".to_owned(),
            },
            PolicyError::UnsupportedWildcardMode {
                pattern: "aws.ec2.*".to_owned(),
                mode: aicontext_core::PermissionMode::Allow,
            },
            PolicyError::DuplicateRule {
                pattern: "read_file".to_owned(),
            },
        ];
        for error in errors {
            assert_eq!(error.code(), ErrorCode::CTX_003);
            assert!(!error.remediation().is_empty());
        }
    }

    #[test]
    fn a_policy_error_converts_into_the_common_error_without_losing_its_cause() {
        let error = PolicyError::InvalidWildcard {
            pattern: "aws.*".to_owned(),
        };
        let converted = aicontext_core::AicontextError::from(error.clone());
        assert_eq!(converted.code(), ErrorCode::CTX_003);
        assert!(converted.message().contains("aws.*"));
        assert!(converted.source().is_some());
    }
}
