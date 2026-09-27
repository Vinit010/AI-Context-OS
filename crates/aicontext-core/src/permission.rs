//! Permission modes.
//!
//! The vocabulary of `docs/PLUGIN_SPEC.md` §5, defined once so that the policy file, the resolver,
//! the approval broker, and the audit log cannot drift apart.
//!
//! | Mode | Meaning |
//! |------|---------|
//! | [`Allow`](PermissionMode::Allow) | the action may proceed |
//! | [`Deny`](PermissionMode::Deny) | the action is refused |
//! | [`Approval`](PermissionMode::Approval) | the action may proceed once a human approves this invocation |
//! | [`ExplicitApproval`](PermissionMode::ExplicitApproval) | as `Approval`, and never satisfiable by a flag, a grant, or a session |
//!
//! # Default deny
//!
//! [`PermissionMode::default`] is [`Deny`](PermissionMode::Deny). A tool with no policy entry, an
//! unparseable mode, or an absent policy file is therefore denied without a single `if`. That is
//! invariant `I3` in `docs/SECURITY.md` §3, and it is why `default` exists on this enum at all.

use std::fmt;
use std::str::FromStr;

/// What the policy says about one capability.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum PermissionMode {
    /// The action may proceed. Rendered `allow` in `.ai/permissions/permissions.yaml`.
    Allow,
    /// The action is refused. Rendered `deny`.
    #[default]
    Deny,
    /// The action may proceed once a human approves this invocation. Rendered `approval`.
    Approval,
    /// As `Approval`, and re-confirmed per invocation with no session grant and no flag that
    /// satisfies it. Rendered `explicit_approval`.
    ExplicitApproval,
}

impl PermissionMode {
    /// Every mode, in policy-file order. Used to render help and to test the matrix exhaustively.
    pub const ALL: [Self; 4] = [
        Self::Allow,
        Self::Deny,
        Self::Approval,
        Self::ExplicitApproval,
    ];

    /// The wire form, matching the `mode` key in `.ai/permissions/permissions.yaml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Approval => "approval",
            Self::ExplicitApproval => "explicit_approval",
        }
    }

    /// Whether a human must approve before the action proceeds.
    #[must_use]
    pub const fn requires_approval(self) -> bool {
        matches!(self, Self::Approval | Self::ExplicitApproval)
    }

    /// Whether the approval is re-confirmed per invocation and cannot be granted for a session,
    /// which is what `docs/SECURITY.md` §3 `I4` requires of a destructive capability.
    #[must_use]
    pub const fn requires_explicit_approval(self) -> bool {
        matches!(self, Self::ExplicitApproval)
    }

    /// Whether the action is refused outright, with no approval path.
    #[must_use]
    pub const fn is_denied(self) -> bool {
        matches!(self, Self::Deny)
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for PermissionMode {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for PermissionMode {
    type Err = UnknownPermissionMode;

    /// Parses one of the four wire forms from `docs/PLUGIN_SPEC.md` §5.
    ///
    /// Case, surrounding whitespace, and `-` in place of `_` are normalised, because those are
    /// spelling differences rather than different policies. Nothing else is accepted. An
    /// unrecognised mode is an error and never a fallback to `Deny`: a typo in a security-relevant
    /// file has to be reported, and quietly resolving `explicit-approvl` to the safe answer would
    /// hide a policy the author meant to be strict.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            "approval" => Ok(Self::Approval),
            "explicit_approval" => Ok(Self::ExplicitApproval),
            _ => Err(UnknownPermissionMode::new(raw.to_owned())),
        }
    }
}

/// A mode string that is not one of the four documented modes.
#[derive(Debug, thiserror::Error)]
#[error(
    "`{input}` is not a permission mode; expected one of allow, deny, approval, \
         explicit_approval"
)]
pub struct UnknownPermissionMode {
    input: String,
}

impl UnknownPermissionMode {
    fn new(input: String) -> Self {
        Self { input }
    }

    /// The stable code for this failure, `CTX-003` in `docs/CONTEXT_SPEC.md` §8: the policy file
    /// did not validate against its schema.
    #[must_use]
    pub fn code(&self) -> crate::ErrorCode {
        crate::ErrorCode::CTX_003
    }

    /// The value that was rejected.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> &'static str {
        "use one of allow, deny, approval, explicit_approval; see PLUGIN_SPEC.md 5 for the \
         resolution order and SECURITY.md 3 for what each mode guarantees"
    }
}

impl From<UnknownPermissionMode> for crate::AicontextError {
    fn from(error: UnknownPermissionMode) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use super::PermissionMode;
    use crate::ErrorCode;

    #[test]
    fn an_unmapped_capability_is_denied() {
        // Invariant I3, SECURITY.md 3. The default has to be Deny for this to hold without an
        // explicit branch at every call site.
        assert_eq!(PermissionMode::default(), PermissionMode::Deny);
        assert!(PermissionMode::default().is_denied());
    }

    #[test]
    fn every_mode_round_trips_through_its_wire_form() {
        for mode in PermissionMode::ALL {
            let rendered = mode.as_str();
            assert_eq!(
                rendered.parse::<PermissionMode>().expect("must parse"),
                mode,
                "`{rendered}` must parse back to the same mode"
            );
            assert_eq!(mode.to_string(), rendered);
        }
    }

    #[test]
    fn the_wire_forms_match_the_policy_file() {
        let expected = [
            (PermissionMode::Allow, "allow"),
            (PermissionMode::Deny, "deny"),
            (PermissionMode::Approval, "approval"),
            (PermissionMode::ExplicitApproval, "explicit_approval"),
        ];
        for (mode, wire) in expected {
            assert_eq!(mode.as_str(), wire);
        }
    }

    #[test]
    fn mode_parsing_normalises_case_whitespace_and_dashes() {
        for raw in ["ALLOW", " allow ", "Allow", "\tallow\n"] {
            assert_eq!(
                raw.parse::<PermissionMode>().expect("must parse"),
                PermissionMode::Allow,
                "`{raw}` should read as allow"
            );
        }
        for raw in [
            "EXPLICIT_APPROVAL",
            "explicit-approval",
            " Explicit-Approval ",
        ] {
            assert_eq!(
                raw.parse::<PermissionMode>().expect("must parse"),
                PermissionMode::ExplicitApproval,
                "`{raw}` should read as explicit_approval"
            );
        }
    }

    #[test]
    fn a_typo_is_reported_rather_than_resolved_to_deny() {
        for raw in [
            "",
            "allowd",
            "permitted",
            "true",
            "auto",
            "yes",
            "explicit-approvl",
            // Synonyms that are not a documented wire form are not accepted either. Permission
            // modes are defined exactly once, and a second spelling is drift.
            "allowed",
            "forbidden",
            "ask",
            "explicit",
        ] {
            let error = raw
                .parse::<PermissionMode>()
                .expect_err("an unrecognised mode must fail");
            assert_eq!(error.input(), raw);
            assert!(
                error.to_string().contains("explicit_approval"),
                "the message must list the valid modes: {error}"
            );
        }
    }
    #[test]
    fn only_the_approval_modes_require_a_human() {
        for mode in PermissionMode::ALL {
            assert_eq!(
                mode.requires_approval(),
                mode.requires_explicit_approval() || mode == PermissionMode::Approval,
                "requires_approval disagrees with the spec for {mode}"
            );
        }
        assert!(!PermissionMode::Allow.requires_approval());
        assert!(!PermissionMode::Deny.requires_approval());
        assert!(PermissionMode::Approval.requires_approval());
        assert!(PermissionMode::ExplicitApproval.requires_approval());
    }

    #[test]
    fn explicit_approval_is_narrower_than_approval() {
        // I4: a destructive capability is re-confirmed per invocation, and no session grant
        // satisfies it. Anything that satisfies Approval must not satisfy ExplicitApproval.
        assert!(PermissionMode::ExplicitApproval.requires_explicit_approval());
        assert!(!PermissionMode::Approval.requires_explicit_approval());
        assert!(!PermissionMode::Allow.requires_explicit_approval());
        assert!(!PermissionMode::Deny.requires_explicit_approval());
    }

    #[test]
    fn deny_is_the_only_mode_with_no_approval_path() {
        for mode in PermissionMode::ALL {
            assert_eq!(mode.is_denied(), mode == PermissionMode::Deny);
        }
    }

    #[test]
    fn an_unknown_mode_carries_a_code_and_a_remediation() {
        let mode: Result<PermissionMode, _> = "permitted".parse();
        let error = mode.expect_err("invalid");
        assert_eq!(error.code(), ErrorCode::CTX_003);
        assert!(error.remediation().contains("PLUGIN_SPEC.md"));
    }

    #[test]
    fn an_unknown_mode_converts_into_the_common_error_without_losing_the_cause() {
        use std::error::Error;

        let mode: Result<PermissionMode, _> = "permitted".parse();
        let error = crate::AicontextError::from(mode.expect_err("invalid"));
        assert_eq!(error.code(), ErrorCode::CTX_003);
        assert!(error.message().contains("permitted"));
        assert!(error.source().is_some());
    }

    #[test]
    fn the_mode_list_is_exhaustive() {
        // A fifth mode added later must update this test, which is the point: the matrix is
        // enumerated once and checked, per RULES.md 8.
        assert_eq!(PermissionMode::ALL.len(), 4);
        let rendered: std::collections::BTreeSet<&str> = PermissionMode::ALL
            .iter()
            .map(|mode| mode.as_str())
            .collect();
        assert_eq!(rendered.len(), PermissionMode::ALL.len());
    }
}
