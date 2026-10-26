//! Pure resolution of one capability against a [`Policy`].
//!
//! [`Resolver::resolve`] implements `docs/PLUGIN_SPEC.md` §5 exactly: the first matching step
//! wins, and an unmatched capability takes the policy's default mode. Every decision records the
//! step that produced it as a [`DecisionSource`], so an audit entry can explain *why* a tool was
//! allowed, denied, or sent for approval without re-running the resolver against a policy that may
//! since have changed.
//!
//! # Resolution order
//!
//! ```text
//! 1  exact DENY                               -> DENY
//! 2  provider.service wildcard DENY           -> DENY
//! 3  exact EXPLICIT_APPROVAL                  -> EXPLICIT_APPROVAL
//! 4  the capability is destructive            -> EXPLICIT_APPROVAL
//! 5  exact APPROVAL                           -> APPROVAL
//! 6  provider.service wildcard APPROVAL       -> APPROVAL
//! 7  exact ALLOW                              -> ALLOW
//! 8  no match                                 -> the policy's default mode
//! ```
//!
//! The destructive step is a floor, not a preference: a destructive capability never resolves to
//! [`PermissionMode::Allow`] or [`PermissionMode::Approval`], even when the policy names it, because
//! `docs/SECURITY.md` §3 invariant `I4` requires explicit approval for every destructive
//! invocation. An explicit or wildcard DENY still wins over the floor.
//!
//! # Purity
//!
//! This module reads no file, clock, environment, or random source. It takes a policy and a
//! capability, and returns a value. A caller that loads a policy is responsible for turning I/O
//! failures into a policy rather than into an accidental allow; [`Policy::default`] is deny-all
//! for exactly that case.

use aicontext_core::PermissionMode;

use crate::policy::{Policy, Rule, RuleKind};

/// One capability presented to the resolver, plus the fact the resolver cannot derive itself.
///
/// The tool name is the capability's canonical dotted name. For a plugin tool that is
/// `provider.service.action` (`aws.ec2.describe`); for a local tool it is the name in the template
/// (`read_file`). Whether the capability is destructive comes from the tool descriptor, not from
/// the name: a tool named `read_file` may be destructive if it is declared that way, and guessing
/// from the name would be wrong in both directions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRequest<'a> {
    tool: &'a str,
    destructive: bool,
}

impl<'a> CapabilityRequest<'a> {
    /// A non-destructive request for `tool`.
    #[must_use]
    pub const fn new(tool: &'a str) -> Self {
        Self {
            tool,
            destructive: false,
        }
    }

    /// Mark this request destructive, or clear the mark.
    ///
    /// A destructive request is resolved to [`PermissionMode::ExplicitApproval`] unless a rule
    /// denies it first, per `docs/PLUGIN_SPEC.md` §5 and `docs/SECURITY.md` §3 `I4`.
    #[must_use]
    pub const fn with_destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    /// The capability's canonical tool name.
    #[must_use]
    pub const fn tool(&self) -> &'a str {
        self.tool
    }

    /// Whether the tool descriptor declares this capability destructive.
    #[must_use]
    pub const fn is_destructive(&self) -> bool {
        self.destructive
    }
}

/// Why a [`Decision`] has the mode it has.
///
/// This is audit provenance, not policy: it names the first resolution step that matched. A
/// caller must not treat the source as permission to proceed — [`Decision::mode`] is the answer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum DecisionSource {
    /// Step 1: an exact rule denied the tool.
    ExactDeny,
    /// Step 2: a `provider.service.*` rule denied the tool.
    WildcardDeny,
    /// Step 3: an exact rule required explicit approval.
    ExactExplicitApproval,
    /// Step 4: the capability is destructive and no exact explicit-approval rule or deny matched
    /// first, so the floor raised it to explicit approval.
    Destructive,
    /// Step 5: an exact rule required approval.
    ExactApproval,
    /// Step 6: a `provider.service.*` rule required approval.
    WildcardApproval,
    /// Step 7: an exact rule allowed the tool.
    ExactAllow,
    /// Step 8: no rule matched, so the policy's default mode applies.
    Default,
}

/// The resolved outcome for one capability.
///
/// A decision is a plain value: it can be logged, compared, and passed to the approval broker
/// without carrying a borrow of the policy. It is not an approval itself; a mode that requires
/// approval still needs the broker's per-invocation decision (`docs/PLUGIN_SPEC.md` §6).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Decision {
    mode: PermissionMode,
    source: DecisionSource,
}

impl Decision {
    const fn new(mode: PermissionMode, source: DecisionSource) -> Self {
        Self { mode, source }
    }

    /// The mode the policy resolves to.
    #[must_use]
    pub const fn mode(self) -> PermissionMode {
        self.mode
    }

    /// The resolution step that produced [`Decision::mode`].
    #[must_use]
    pub const fn source(self) -> DecisionSource {
        self.source
    }
}

/// A policy ready to resolve capabilities.
///
/// The resolver borrows the policy and holds no other state, so constructing one is free and
/// resolving the same capability twice always produces the same decision.
#[derive(Clone, Copy, Debug)]
pub struct Resolver<'a> {
    policy: &'a Policy,
}

impl<'a> Resolver<'a> {
    /// Resolve against `policy`.
    #[must_use]
    pub const fn new(policy: &'a Policy) -> Self {
        Self { policy }
    }

    /// Resolve one capability, first match wins.
    #[must_use]
    pub fn resolve(self, request: CapabilityRequest<'_>) -> Decision {
        let tool = request.tool();

        if self.matches(tool, RuleKind::Exact, PermissionMode::Deny) {
            return Decision::new(PermissionMode::Deny, DecisionSource::ExactDeny);
        }
        if self.matches(tool, RuleKind::Wildcard, PermissionMode::Deny) {
            return Decision::new(PermissionMode::Deny, DecisionSource::WildcardDeny);
        }
        if self.matches(tool, RuleKind::Exact, PermissionMode::ExplicitApproval) {
            return Decision::new(
                PermissionMode::ExplicitApproval,
                DecisionSource::ExactExplicitApproval,
            );
        }
        if request.is_destructive() {
            return Decision::new(
                PermissionMode::ExplicitApproval,
                DecisionSource::Destructive,
            );
        }
        if self.matches(tool, RuleKind::Exact, PermissionMode::Approval) {
            return Decision::new(PermissionMode::Approval, DecisionSource::ExactApproval);
        }
        if self.matches(tool, RuleKind::Wildcard, PermissionMode::Approval) {
            return Decision::new(PermissionMode::Approval, DecisionSource::WildcardApproval);
        }
        if self.matches(tool, RuleKind::Exact, PermissionMode::Allow) {
            return Decision::new(PermissionMode::Allow, DecisionSource::ExactAllow);
        }

        Decision::new(self.policy.default_mode(), DecisionSource::Default)
    }

    /// Whether a rule of `kind` and `mode` matches `tool`.
    fn matches(self, tool: &str, kind: RuleKind, mode: PermissionMode) -> bool {
        self.policy
            .rules()
            .iter()
            .any(|rule| rule.mode() == mode && rule.kind() == kind && rule_matches(rule, tool))
    }
}

/// Whether one validated rule names `tool`.
fn rule_matches(rule: &Rule, tool: &str) -> bool {
    match rule.kind() {
        RuleKind::Exact => rule.pattern() == tool,
        RuleKind::Wildcard => wildcard_matches(rule.pattern(), tool),
    }
}

/// Whether a `provider.service.*` wildcard covers exactly one action segment of `tool`.
///
/// Wildcards are valid only at the `provider.service` level (`docs/PLUGIN_SPEC.md` §5), so
/// `aws.ec2.*` covers `aws.ec2.describe` but not `aws.ec2` (no action), `aws.ec2.describe.extra`
/// (two action segments), or `aws.s3.describe` (another service). The pattern is validated before
/// a [`Rule`] exists, but this function is total for any `&str` so a caller cannot construct a
/// match by bypassing validation.
fn wildcard_matches(pattern: &str, tool: &str) -> bool {
    let Some(prefix) = pattern.strip_suffix(".*") else {
        return false;
    };
    let Some(rest) = tool.strip_prefix(prefix) else {
        return false;
    };
    let Some(action) = rest.strip_prefix('.') else {
        return false;
    };
    !action.is_empty() && !action.contains('.')
}

#[cfg(test)]
mod tests {
    use aicontext_core::PermissionMode;

    use super::{CapabilityRequest, Decision, DecisionSource, Resolver, wildcard_matches};
    use crate::policy::{Policy, Rule};

    fn exact(pattern: &str, mode: PermissionMode) -> Rule {
        Rule::exact(pattern, mode).expect("a valid exact rule")
    }

    fn wildcard(pattern: &str, mode: PermissionMode) -> Rule {
        Rule::wildcard(pattern, mode).expect("a valid wildcard rule")
    }

    fn policy(default_mode: PermissionMode, rules: Vec<Rule>) -> Policy {
        Policy::new(default_mode, rules).expect("a valid policy")
    }

    fn resolve(policy: &Policy, tool: &str) -> Decision {
        policy.resolve(CapabilityRequest::new(tool))
    }

    fn resolve_destructive(policy: &Policy, tool: &str) -> Decision {
        policy.resolve(CapabilityRequest::new(tool).with_destructive(true))
    }

    #[test]
    fn an_exact_rule_resolves_to_its_own_mode_and_source() {
        let expected = [
            (PermissionMode::Allow, DecisionSource::ExactAllow),
            (PermissionMode::Deny, DecisionSource::ExactDeny),
            (PermissionMode::Approval, DecisionSource::ExactApproval),
            (
                PermissionMode::ExplicitApproval,
                DecisionSource::ExactExplicitApproval,
            ),
        ];
        for (mode, source) in expected {
            let policy = policy(PermissionMode::Deny, vec![exact("read_file", mode)]);
            let decision = resolve(&policy, "read_file");
            assert_eq!(decision.mode(), mode, "exact {mode} must resolve to itself");
            assert_eq!(decision.source(), source);
        }
    }

    #[test]
    fn a_wildcard_resolves_only_the_two_modes_the_order_consults() {
        let expected = [
            (PermissionMode::Deny, DecisionSource::WildcardDeny),
            (PermissionMode::Approval, DecisionSource::WildcardApproval),
        ];
        for (mode, source) in expected {
            let policy = policy(PermissionMode::Deny, vec![wildcard("aws.ec2.*", mode)]);
            let decision = resolve(&policy, "aws.ec2.describe");
            assert_eq!(decision.mode(), mode);
            assert_eq!(decision.source(), source);
        }
    }

    #[test]
    fn a_wildcard_deny_beats_every_non_deny_rule_and_the_destructive_floor() {
        let policy = policy(
            PermissionMode::Allow,
            vec![
                wildcard("aws.ec2.*", PermissionMode::Deny),
                exact("aws.ec2.describe", PermissionMode::Allow),
                exact("aws.ec2.start", PermissionMode::ExplicitApproval),
                exact("aws.ec2.stop", PermissionMode::Approval),
            ],
        );
        for tool in [
            "aws.ec2.describe",
            "aws.ec2.start",
            "aws.ec2.stop",
            "aws.ec2.terminate",
        ] {
            let decision = resolve_destructive(&policy, tool);
            assert_eq!(decision.mode(), PermissionMode::Deny, "{tool}");
            assert_eq!(decision.source(), DecisionSource::WildcardDeny, "{tool}");
        }
    }

    #[test]
    fn an_exact_deny_beats_a_wildcard_approval_and_the_destructive_floor() {
        let policy = policy(
            PermissionMode::Allow,
            vec![
                exact("aws.ec2.describe", PermissionMode::Deny),
                wildcard("aws.ec2.*", PermissionMode::Approval),
            ],
        );
        let decision = resolve_destructive(&policy, "aws.ec2.describe");
        assert_eq!(decision.mode(), PermissionMode::Deny);
        assert_eq!(decision.source(), DecisionSource::ExactDeny);
    }

    #[test]
    fn a_destructive_capability_is_explicit_approval_before_approval_allow_and_default() {
        let policy = policy(
            PermissionMode::Allow,
            vec![
                exact("aws.ec2.start", PermissionMode::Approval),
                exact("aws.ec2.stop", PermissionMode::Allow),
                wildcard("aws.ec2.*", PermissionMode::Approval),
            ],
        );
        for tool in ["aws.ec2.start", "aws.ec2.stop", "aws.ec2.describe"] {
            let decision = resolve_destructive(&policy, tool);
            assert_eq!(decision.mode(), PermissionMode::ExplicitApproval, "{tool}");
            assert_eq!(decision.source(), DecisionSource::Destructive, "{tool}");
        }
        assert_eq!(
            resolve_destructive(&policy, "github.repo.read").source(),
            DecisionSource::Destructive,
            "the floor is ahead of the default"
        );
    }

    #[test]
    fn an_exact_explicit_approval_is_reported_ahead_of_the_destructive_floor() {
        let policy = policy(
            PermissionMode::Deny,
            vec![exact("aws.ec2.terminate", PermissionMode::ExplicitApproval)],
        );
        let decision = resolve_destructive(&policy, "aws.ec2.terminate");
        assert_eq!(decision.mode(), PermissionMode::ExplicitApproval);
        assert_eq!(decision.source(), DecisionSource::ExactExplicitApproval);
    }

    #[test]
    fn a_wildcard_approval_is_checked_before_an_exact_allow() {
        let policy = policy(
            PermissionMode::Deny,
            vec![
                wildcard("aws.ec2.*", PermissionMode::Approval),
                exact("aws.ec2.describe", PermissionMode::Allow),
            ],
        );
        let decision = resolve(&policy, "aws.ec2.describe");
        assert_eq!(decision.mode(), PermissionMode::Approval);
        assert_eq!(decision.source(), DecisionSource::WildcardApproval);
    }

    #[test]
    fn an_exact_allow_beats_a_default_that_asks_for_approval() {
        let policy = policy(
            PermissionMode::Approval,
            vec![exact("read_file", PermissionMode::Allow)],
        );
        let decision = resolve(&policy, "read_file");
        assert_eq!(decision.mode(), PermissionMode::Allow);
        assert_eq!(decision.source(), DecisionSource::ExactAllow);
    }

    #[test]
    fn every_default_mode_is_used_for_an_unmatched_capability() {
        for mode in PermissionMode::ALL {
            let policy = policy(mode, Vec::new());
            let decision = resolve(&policy, "mystery.tool.action");
            assert_eq!(decision.mode(), mode);
            assert_eq!(decision.source(), DecisionSource::Default);
        }
    }

    #[test]
    fn the_default_policy_denies_an_unspecified_capability() {
        for tool in ["read_file", "aws.ec2.describe", "mystery.tool.action"] {
            let decision = Policy::default().resolve(CapabilityRequest::new(tool));
            assert_eq!(decision.mode(), PermissionMode::Deny, "{tool}");
            assert_eq!(decision.source(), DecisionSource::Default, "{tool}");
        }
    }

    #[test]
    fn a_destructive_capability_under_deny_all_still_needs_explicit_approval() {
        let decision = Policy::default()
            .resolve(CapabilityRequest::new("aws.ec2.terminate").with_destructive(true));
        assert_eq!(decision.mode(), PermissionMode::ExplicitApproval);
        assert_eq!(decision.source(), DecisionSource::Destructive);
    }

    #[test]
    fn a_wildcard_covers_exactly_one_action_segment() {
        let policy = policy(
            PermissionMode::Deny,
            vec![wildcard("aws.ec2.*", PermissionMode::Approval)],
        );
        let matching = ["aws.ec2.describe", "aws.ec2.start", "aws.ec2.terminate"];
        for tool in matching {
            assert_eq!(
                resolve(&policy, tool).source(),
                DecisionSource::WildcardApproval,
                "{tool} must match aws.ec2.*"
            );
        }
        let non_matching = [
            "aws.ec2",
            "aws",
            "aws.ec2.describe.extra",
            "aws.ec2x.describe",
            "aws.s3.describe",
            "aws.ec2.",
            "read_file",
        ];
        for tool in non_matching {
            assert_eq!(
                resolve(&policy, tool).source(),
                DecisionSource::Default,
                "{tool} must not match aws.ec2.*"
            );
        }
    }

    #[test]
    fn wildcard_matching_is_total_for_malformed_patterns() {
        assert!(wildcard_matches("aws.ec2.*", "aws.ec2.describe"));
        assert!(!wildcard_matches("aws.ec2", "aws.ec2.describe"));
        assert!(!wildcard_matches("aws.ec2.*", "aws.s3.describe"));
        assert!(!wildcard_matches("aws.ec2.*", "aws.ec2"));
        assert!(!wildcard_matches("aws.ec2.*", "aws.ec2."));
        assert!(!wildcard_matches("aws.ec2.*", "aws.ec2.describe.extra"));
    }

    #[test]
    fn resolving_the_same_request_twice_is_deterministic() {
        let policy = policy(
            PermissionMode::Deny,
            vec![
                wildcard("aws.ec2.*", PermissionMode::Approval),
                exact("read_file", PermissionMode::Allow),
            ],
        );
        for tool in ["aws.ec2.describe", "read_file", "github.repo.read"] {
            assert_eq!(resolve(&policy, tool), resolve(&policy, tool), "{tool}");
            assert_eq!(
                resolve_destructive(&policy, tool),
                resolve_destructive(&policy, tool),
                "{tool}"
            );
        }
    }

    #[test]
    fn rule_order_does_not_change_a_decision() {
        let rules = vec![
            exact("read_file", PermissionMode::Allow),
            exact("aws.ec2.describe", PermissionMode::Allow),
            exact("aws.ec2.start", PermissionMode::Approval),
            wildcard("aws.ec2.*", PermissionMode::Deny),
            wildcard("github.repo.*", PermissionMode::Approval),
        ];
        let forward = policy(PermissionMode::Allow, rules.clone());
        let mut reversed_rules = rules;
        reversed_rules.reverse();
        let reversed = policy(PermissionMode::Allow, reversed_rules);
        for tool in [
            "read_file",
            "aws.ec2.describe",
            "aws.ec2.start",
            "github.repo.read",
            "github.issue.create",
            "mystery.tool.action",
        ] {
            assert_eq!(resolve(&forward, tool), resolve(&reversed, tool), "{tool}");
            assert_eq!(
                resolve_destructive(&forward, tool),
                resolve_destructive(&reversed, tool),
                "{tool}"
            );
        }
    }

    #[test]
    fn every_decision_source_is_reachable() {
        let cases = [
            (
                policy(
                    PermissionMode::Deny,
                    vec![exact("deny_me", PermissionMode::Deny)],
                ),
                "deny_me",
                false,
                DecisionSource::ExactDeny,
            ),
            (
                policy(
                    PermissionMode::Deny,
                    vec![wildcard("aws.ec2.*", PermissionMode::Deny)],
                ),
                "aws.ec2.describe",
                false,
                DecisionSource::WildcardDeny,
            ),
            (
                policy(
                    PermissionMode::Deny,
                    vec![exact("explicit_me", PermissionMode::ExplicitApproval)],
                ),
                "explicit_me",
                false,
                DecisionSource::ExactExplicitApproval,
            ),
            (
                policy(PermissionMode::Deny, Vec::new()),
                "destructive",
                true,
                DecisionSource::Destructive,
            ),
            (
                policy(
                    PermissionMode::Deny,
                    vec![exact("approve_me", PermissionMode::Approval)],
                ),
                "approve_me",
                false,
                DecisionSource::ExactApproval,
            ),
            (
                policy(
                    PermissionMode::Deny,
                    vec![wildcard("aws.ec2.*", PermissionMode::Approval)],
                ),
                "aws.ec2.describe",
                false,
                DecisionSource::WildcardApproval,
            ),
            (
                policy(
                    PermissionMode::Deny,
                    vec![exact("allow_me", PermissionMode::Allow)],
                ),
                "allow_me",
                false,
                DecisionSource::ExactAllow,
            ),
            (
                policy(PermissionMode::Deny, Vec::new()),
                "unknown",
                false,
                DecisionSource::Default,
            ),
        ];
        for (policy, tool, destructive, source) in cases {
            let request = CapabilityRequest::new(tool).with_destructive(destructive);
            assert_eq!(Resolver::new(&policy).resolve(request).source(), source);
        }
    }

    #[test]
    fn the_full_mode_matrix_resolves_as_documented() {
        let exact_source = |mode| match mode {
            PermissionMode::Allow => DecisionSource::ExactAllow,
            PermissionMode::Deny => DecisionSource::ExactDeny,
            PermissionMode::Approval => DecisionSource::ExactApproval,
            PermissionMode::ExplicitApproval => DecisionSource::ExactExplicitApproval,
        };

        for default_mode in PermissionMode::ALL {
            for rule_mode in PermissionMode::ALL {
                let exact_policy =
                    policy(default_mode, vec![exact("matrix.tool.action", rule_mode)]);

                let ordinary = resolve(&exact_policy, "matrix.tool.action");
                assert_eq!(
                    ordinary,
                    Decision::new(rule_mode, exact_source(rule_mode)),
                    "exact {rule_mode} under default {default_mode}"
                );

                let destructive = resolve_destructive(&exact_policy, "matrix.tool.action");
                let expected_destructive = if rule_mode == PermissionMode::Deny {
                    Decision::new(PermissionMode::Deny, DecisionSource::ExactDeny)
                } else if rule_mode == PermissionMode::ExplicitApproval {
                    Decision::new(
                        PermissionMode::ExplicitApproval,
                        DecisionSource::ExactExplicitApproval,
                    )
                } else {
                    Decision::new(
                        PermissionMode::ExplicitApproval,
                        DecisionSource::Destructive,
                    )
                };
                assert_eq!(
                    destructive, expected_destructive,
                    "destructive exact {rule_mode} under default {default_mode}"
                );

                let unmatched = resolve(&exact_policy, "matrix.tool.other");
                assert_eq!(
                    unmatched,
                    Decision::new(default_mode, DecisionSource::Default),
                    "unmatched capability under default {default_mode}"
                );
            }

            for wildcard_mode in [PermissionMode::Deny, PermissionMode::Approval] {
                let wildcard_policy =
                    policy(default_mode, vec![wildcard("matrix.tool.*", wildcard_mode)]);
                let ordinary = resolve(&wildcard_policy, "matrix.tool.action");
                let expected_source = if wildcard_mode == PermissionMode::Deny {
                    DecisionSource::WildcardDeny
                } else {
                    DecisionSource::WildcardApproval
                };
                assert_eq!(
                    ordinary,
                    Decision::new(wildcard_mode, expected_source),
                    "wildcard {wildcard_mode} under default {default_mode}"
                );

                let destructive = resolve_destructive(&wildcard_policy, "matrix.tool.action");
                let expected_destructive = if wildcard_mode == PermissionMode::Deny {
                    Decision::new(PermissionMode::Deny, DecisionSource::WildcardDeny)
                } else {
                    Decision::new(
                        PermissionMode::ExplicitApproval,
                        DecisionSource::Destructive,
                    )
                };
                assert_eq!(
                    destructive, expected_destructive,
                    "destructive wildcard {wildcard_mode} under default {default_mode}"
                );

                let unmatched = resolve(&wildcard_policy, "matrix.tool");
                assert_eq!(
                    unmatched,
                    Decision::new(default_mode, DecisionSource::Default),
                    "wildcard must not match a two-segment capability"
                );
            }
        }
    }
}
