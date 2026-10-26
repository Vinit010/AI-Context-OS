//! The permission policy: a default mode and a set of exact and wildcard rules.
//!
//! The policy is plain data with no loader attached. Parsing `.ai/permissions/permissions.yaml` is
//! an I/O concern and belongs above this crate; this module owns only the shape the resolver reads.

use std::collections::BTreeSet;

use aicontext_core::PermissionMode;

use crate::error::PolicyError;
use crate::resolver::{CapabilityRequest, Decision, Resolver};

/// How a rule's pattern matches a tool.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RuleKind {
    /// The pattern equals one tool name exactly.
    Exact,
    /// The pattern is a `provider.service.*` wildcard.
    Wildcard,
}

/// One permission rule.
///
/// A rule is constructed through [`Rule::exact`] or [`Rule::wildcard`], so its pattern and kind
/// cannot disagree. Exact rules may name any non-empty tool, including the local tools in the
/// default policy (`read_file`, for example). A wildcard names a plugin capability's
/// `provider.service` pair and is only valid for [`PermissionMode::Deny`] and
/// [`PermissionMode::Approval`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rule {
    pattern: String,
    mode: PermissionMode,
    kind: RuleKind,
}

impl Rule {
    /// A rule for one exact tool name.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::EmptyPattern`] for an empty or whitespace-only name, and
    /// [`PolicyError::WildcardInExactRule`] if the name contains `*`. A wildcard is not an exact
    /// rule; use [`Rule::wildcard`] so the shape is checked.
    pub fn exact(pattern: impl Into<String>, mode: PermissionMode) -> Result<Self, PolicyError> {
        let pattern = pattern.into();
        if pattern.trim().is_empty() {
            return Err(PolicyError::EmptyPattern);
        }
        if pattern.contains('*') {
            return Err(PolicyError::WildcardInExactRule { pattern });
        }
        Ok(Self {
            pattern,
            mode,
            kind: RuleKind::Exact,
        })
    }

    /// A rule for a tool's `provider.service` wildcard, for example `aws.ec2.*` for
    /// `aws.ec2.describe`.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::EmptyPattern`] for an empty pattern,
    /// [`PolicyError::InvalidWildcard`] unless the pattern is exactly `provider.service.*` with a
    /// non-empty provider and service, and [`PolicyError::UnsupportedWildcardMode`] for a mode the
    /// resolution order never consults. A top-level `*`, a `*.action` pattern, and a wildcard
    /// `allow` or `explicit_approval` are all refused rather than left inert.
    pub fn wildcard(pattern: impl Into<String>, mode: PermissionMode) -> Result<Self, PolicyError> {
        let pattern = pattern.into();
        if pattern.trim().is_empty() {
            return Err(PolicyError::EmptyPattern);
        }
        if !is_provider_service_wildcard(&pattern) {
            return Err(PolicyError::InvalidWildcard { pattern });
        }
        if !matches!(mode, PermissionMode::Deny | PermissionMode::Approval) {
            return Err(PolicyError::UnsupportedWildcardMode { pattern, mode });
        }
        Ok(Self {
            pattern,
            mode,
            kind: RuleKind::Wildcard,
        })
    }

    /// The exact tool name or wildcard pattern.
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The mode this rule grants or denies.
    #[must_use]
    pub const fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// Whether this rule is exact or a wildcard.
    #[must_use]
    pub const fn kind(&self) -> RuleKind {
        self.kind
    }
}

/// A permission policy: the fallback mode plus every rule.
///
/// [`Policy::default()`] is deny-all: the shape a missing or unreadable policy takes before a
/// loader ever runs. A loaded policy carries the file's explicit `defaults.mode`, which the schema
/// publishes as configurable; the shipped template sets it to `deny`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    default_mode: PermissionMode,
    rules: Vec<Rule>,
}

impl Policy {
    /// A policy with an explicit fallback and a set of rules.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::DuplicateRule`] if two rules name the same pattern. Duplicates are
    /// refused rather than resolved by file order, because "a more specific rule always beats a
    /// wildcard, regardless of file order" (`docs/PLUGIN_SPEC.md` §5) and two rules of the same
    /// kind have no defensible tie-break.
    pub fn new(
        default_mode: PermissionMode,
        rules: impl IntoIterator<Item = Rule>,
    ) -> Result<Self, PolicyError> {
        let mut seen = BTreeSet::new();
        let mut accepted = Vec::new();
        for rule in rules {
            if !seen.insert(rule.pattern.clone()) {
                return Err(PolicyError::DuplicateRule {
                    pattern: rule.pattern,
                });
            }
            accepted.push(rule);
        }
        Ok(Self {
            default_mode,
            rules: accepted,
        })
    }

    /// A policy that denies every capability. This is the resolver's zero value.
    #[must_use]
    pub const fn deny_all() -> Self {
        Self {
            default_mode: PermissionMode::Deny,
            rules: Vec::new(),
        }
    }

    /// The mode used when no rule matches.
    #[must_use]
    pub const fn default_mode(&self) -> PermissionMode {
        self.default_mode
    }

    /// The rules in construction order.
    ///
    /// Order is not part of resolution: every step matches at most one rule because duplicate
    /// patterns are refused. The slice is exposed for diagnostics and permission diffs.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Resolve one capability against this policy.
    ///
    /// Equivalent to [`Resolver::new(self).resolve(request)`](Resolver::resolve).
    #[must_use]
    pub fn resolve(&self, request: CapabilityRequest<'_>) -> Decision {
        Resolver::new(self).resolve(request)
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::deny_all()
    }
}

/// The one wildcard form `docs/PLUGIN_SPEC.md` §5 permits: `provider.service.*`.
fn is_provider_service_wildcard(pattern: &str) -> bool {
    let mut parts = pattern.split('.');
    let (Some(provider), Some(service), Some(action)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    action == "*"
        && !provider.is_empty()
        && !service.is_empty()
        && !provider.contains('*')
        && !service.contains('*')
        && !provider.chars().any(char::is_whitespace)
        && !service.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use aicontext_core::PermissionMode;

    use super::{Policy, Rule, RuleKind};
    use crate::error::PolicyError;

    fn exact(pattern: &str, mode: PermissionMode) -> Rule {
        Rule::exact(pattern, mode).expect("a valid exact rule")
    }

    #[test]
    fn an_exact_rule_keeps_its_pattern_mode_and_kind() {
        let rule = exact("read_file", PermissionMode::Allow);
        assert_eq!(rule.pattern(), "read_file");
        assert_eq!(rule.mode(), PermissionMode::Allow);
        assert_eq!(rule.kind(), RuleKind::Exact);
    }

    #[test]
    fn a_wildcard_rule_keeps_its_pattern_mode_and_kind() {
        let rule = Rule::wildcard("aws.ec2.*", PermissionMode::Deny).expect("valid wildcard");
        assert_eq!(rule.pattern(), "aws.ec2.*");
        assert_eq!(rule.mode(), PermissionMode::Deny);
        assert_eq!(rule.kind(), RuleKind::Wildcard);
    }

    #[test]
    fn an_empty_exact_or_wildcard_pattern_is_refused() {
        for pattern in ["", "   ", "\t"] {
            assert!(matches!(
                Rule::exact(pattern, PermissionMode::Allow),
                Err(PolicyError::EmptyPattern)
            ));
            assert!(matches!(
                Rule::wildcard(pattern, PermissionMode::Deny),
                Err(PolicyError::EmptyPattern)
            ));
        }
    }

    #[test]
    fn an_exact_rule_cannot_contain_a_star() {
        for pattern in ["*", "aws.*", "aws.ec2.*", "*.action"] {
            assert!(
                matches!(
                    Rule::exact(pattern, PermissionMode::Allow),
                    Err(PolicyError::WildcardInExactRule { .. })
                ),
                "`{pattern}` must not become an exact rule"
            );
        }
    }

    #[test]
    fn only_provider_service_wildcards_are_valid() {
        for pattern in [
            "*",
            "aws.*",
            "*.action",
            "aws.ec2.describe.*",
            "aws..*",
            ".ec2.*",
            "aws.ec2.*.extra",
            "aws.ec2.start",
            "aws ec2.*",
        ] {
            assert!(
                matches!(
                    Rule::wildcard(pattern, PermissionMode::Deny),
                    Err(PolicyError::InvalidWildcard { .. })
                ),
                "`{pattern}` must be refused as a wildcard"
            );
        }
    }

    #[test]
    fn wildcard_rules_refuse_modes_the_resolution_order_never_consults() {
        for mode in [PermissionMode::Allow, PermissionMode::ExplicitApproval] {
            assert!(matches!(
                Rule::wildcard("aws.ec2.*", mode),
                Err(PolicyError::UnsupportedWildcardMode { .. })
            ));
        }
        for mode in [PermissionMode::Deny, PermissionMode::Approval] {
            assert!(Rule::wildcard("aws.ec2.*", mode).is_ok());
        }
    }

    #[test]
    fn a_policy_refuses_duplicate_patterns_regardless_of_mode() {
        let rules = [
            exact("read_file", PermissionMode::Allow),
            exact("read_file", PermissionMode::Deny),
        ];
        assert!(matches!(
            Policy::new(PermissionMode::Deny, rules),
            Err(PolicyError::DuplicateRule { .. })
        ));

        let rules = [
            Rule::wildcard("aws.ec2.*", PermissionMode::Deny).expect("valid"),
            Rule::wildcard("aws.ec2.*", PermissionMode::Approval).expect("valid"),
        ];
        assert!(matches!(
            Policy::new(PermissionMode::Deny, rules),
            Err(PolicyError::DuplicateRule { .. })
        ));
    }

    #[test]
    fn a_policy_keeps_the_default_mode_and_rules_it_was_given() {
        let policy = Policy::new(
            PermissionMode::Approval,
            [exact("read_file", PermissionMode::Allow)],
        )
        .expect("valid policy");
        assert_eq!(policy.default_mode(), PermissionMode::Approval);
        assert_eq!(policy.rules().len(), 1);
        assert_eq!(policy.rules()[0].pattern(), "read_file");
    }

    #[test]
    fn the_default_policy_is_deny_all() {
        for policy in [Policy::default(), Policy::deny_all()] {
            assert_eq!(policy.default_mode(), PermissionMode::Deny);
            assert!(policy.rules().is_empty());
        }
    }
}
