//! Properties of the resolution order in `docs/PLUGIN_SPEC.md` §5.
//!
//! The table tests in `src/resolver.rs` enumerate the matrix. These properties cover the shapes a
//! table cannot: arbitrary combinations of exact and wildcard rules, in either file order, with
//! every default mode and both destructive states. They assert the four guarantees the order
//! exists to provide:
//!
//! - a matching DENY always wins, exact or wildcard;
//! - a destructive capability never resolves to ALLOW or APPROVAL unless a DENY wins first;
//! - an unmatched capability takes the policy's default mode;
//! - rule order and repeated resolution never change the answer.

use std::collections::BTreeMap;

use aicontext_core::PermissionMode;
use aicontext_permissions::{CapabilityRequest, DecisionSource, Policy, Rule};
use proptest::prelude::*;

/// Plugin capabilities with exactly one action segment, so every one can be covered by a
/// `provider.service.*` wildcard.
const PLUGIN_TOOLS: &[&str] = &[
    "aws.ec2.describe",
    "aws.ec2.start",
    "aws.ec2.terminate",
    "github.repo.read",
    "github.issue.create",
    "aws.s3.put",
];

/// The only wildcard shape `Rule::wildcard` accepts.
const WILDCARDS: &[&str] = &["aws.ec2.*", "github.repo.*", "github.issue.*", "aws.s3.*"];

/// Capabilities no generated rule can match: local names have no provider/service segments, and
/// `aws.ec2` lacks the action segment a `aws.ec2.*` wildcard requires.
const UNMATCHED: &[&str] = &[
    "read_file",
    "local.write_file",
    "unknown.tool.action",
    "aws.ec2",
];

/// A tool and the one wildcard that covers it.
const TOOL_WILDCARD_PAIRS: &[(&str, &str)] = &[
    ("aws.ec2.describe", "aws.ec2.*"),
    ("aws.ec2.start", "aws.ec2.*"),
    ("aws.ec2.terminate", "aws.ec2.*"),
    ("github.repo.read", "github.repo.*"),
    ("github.issue.create", "github.issue.*"),
    ("aws.s3.put", "aws.s3.*"),
];

#[derive(Clone, Debug)]
struct RuleSpec {
    pattern: String,
    mode: PermissionMode,
    wildcard: bool,
}

fn mode() -> impl Strategy<Value = PermissionMode> {
    (0u8..4).prop_map(|index| PermissionMode::ALL[usize::from(index)])
}

fn rule_spec() -> impl Strategy<Value = RuleSpec> {
    let exact = (0usize..PLUGIN_TOOLS.len(), mode()).prop_map(|(index, mode)| RuleSpec {
        pattern: PLUGIN_TOOLS[index].to_owned(),
        mode,
        wildcard: false,
    });
    let wildcard = (
        0usize..WILDCARDS.len(),
        prop_oneof![Just(PermissionMode::Deny), Just(PermissionMode::Approval)],
    )
        .prop_map(|(index, mode)| RuleSpec {
            pattern: WILDCARDS[index].to_owned(),
            mode,
            wildcard: true,
        });
    prop_oneof![exact, wildcard]
}

/// One rule per pattern, because `Policy::new` refuses duplicates and the order is what the
/// property varies.
fn unique_specs(specs: Vec<RuleSpec>) -> Vec<RuleSpec> {
    let mut by_pattern = BTreeMap::new();
    for spec in specs {
        by_pattern.entry(spec.pattern.clone()).or_insert(spec);
    }
    by_pattern.into_values().collect()
}

fn build_policy(default_mode: PermissionMode, specs: &[RuleSpec]) -> Policy {
    let rules = specs.iter().map(|spec| {
        let pattern = spec.pattern.clone();
        if spec.wildcard {
            Rule::wildcard(pattern, spec.mode).expect("generated wildcard must be valid")
        } else {
            Rule::exact(pattern, spec.mode).expect("generated exact rule must be valid")
        }
    });
    Policy::new(default_mode, rules).expect("generated policy must be valid")
}

proptest! {
    #[test]
    fn resolution_is_deterministic(
        default_mode in mode(),
        specs in prop::collection::vec(rule_spec(), 0..12),
        tool_index in 0usize..PLUGIN_TOOLS.len(),
        destructive in any::<bool>(),
    ) {
        let policy = build_policy(default_mode, &unique_specs(specs));
        let request = CapabilityRequest::new(PLUGIN_TOOLS[tool_index])
            .with_destructive(destructive);
        let rules_before = policy.rules().to_vec();
        let first = policy.resolve(request);
        let second = policy.resolve(request);
        prop_assert_eq!(first, second);
        prop_assert_eq!(policy.rules(), rules_before.as_slice());
    }

    #[test]
    fn rule_order_never_changes_a_decision(
        default_mode in mode(),
        specs in prop::collection::vec(rule_spec(), 0..12),
        tool_index in 0usize..PLUGIN_TOOLS.len(),
        destructive in any::<bool>(),
    ) {
        let mut forward_specs = unique_specs(specs);
        let forward = build_policy(default_mode, &forward_specs);
        forward_specs.reverse();
        let backward = build_policy(default_mode, &forward_specs);
        let request = CapabilityRequest::new(PLUGIN_TOOLS[tool_index])
            .with_destructive(destructive);
        prop_assert_eq!(forward.resolve(request), backward.resolve(request));
    }

    #[test]
    fn an_exact_deny_dominates_every_other_rule(
        default_mode in mode(),
        specs in prop::collection::vec(rule_spec(), 0..12),
        tool_index in 0usize..PLUGIN_TOOLS.len(),
        destructive in any::<bool>(),
    ) {
        let tool = PLUGIN_TOOLS[tool_index];
        let mut rules: Vec<RuleSpec> = unique_specs(specs)
            .into_iter()
            .filter(|spec| spec.wildcard || spec.pattern != tool)
            .collect();
        rules.push(RuleSpec {
            pattern: tool.to_owned(),
            mode: PermissionMode::Deny,
            wildcard: false,
        });
        let policy = build_policy(default_mode, &rules);
        let request = CapabilityRequest::new(tool).with_destructive(destructive);
        let decision = policy.resolve(request);
        prop_assert_eq!(decision.mode(), PermissionMode::Deny);
        prop_assert_eq!(decision.source(), DecisionSource::ExactDeny);
    }

    #[test]
    fn a_wildcard_deny_dominates_every_other_rule(
        default_mode in mode(),
        specs in prop::collection::vec(rule_spec(), 0..12),
        pair_index in 0usize..TOOL_WILDCARD_PAIRS.len(),
        destructive in any::<bool>(),
    ) {
        let (tool, wildcard) = TOOL_WILDCARD_PAIRS[pair_index];
        let mut rules: Vec<RuleSpec> = unique_specs(specs)
            .into_iter()
            .filter(|spec| {
                if spec.wildcard {
                    spec.pattern != wildcard
                } else {
                    spec.pattern != tool
                }
            })
            .collect();
        rules.push(RuleSpec {
            pattern: wildcard.to_owned(),
            mode: PermissionMode::Deny,
            wildcard: true,
        });
        let policy = build_policy(default_mode, &rules);
        let request = CapabilityRequest::new(tool).with_destructive(destructive);
        let decision = policy.resolve(request);
        prop_assert_eq!(decision.mode(), PermissionMode::Deny);
        prop_assert_eq!(decision.source(), DecisionSource::WildcardDeny);
    }

    #[test]
    fn a_destructive_capability_never_resolves_to_allow_or_approval(
        default_mode in mode(),
        specs in prop::collection::vec(rule_spec(), 0..12),
        tool_index in 0usize..PLUGIN_TOOLS.len(),
    ) {
        let tool = PLUGIN_TOOLS[tool_index];
        let rules: Vec<RuleSpec> = unique_specs(specs)
            .into_iter()
            .filter(|spec| spec.mode != PermissionMode::Deny)
            .collect();
        let policy = build_policy(default_mode, &rules);
        let decision = policy.resolve(CapabilityRequest::new(tool).with_destructive(true));
        let exact_explicit_approval = rules
            .iter()
            .any(|spec| !spec.wildcard && spec.pattern == tool && spec.mode == PermissionMode::ExplicitApproval);
        let expected_source = if exact_explicit_approval {
            DecisionSource::ExactExplicitApproval
        } else {
            DecisionSource::Destructive
        };
        prop_assert_eq!(decision.mode(), PermissionMode::ExplicitApproval);
        prop_assert_eq!(decision.source(), expected_source);
    }

    #[test]
    fn an_unmatched_capability_takes_the_default_mode(
        default_mode in mode(),
        specs in prop::collection::vec(rule_spec(), 0..12),
        tool_index in 0usize..UNMATCHED.len(),
    ) {
        let policy = build_policy(default_mode, &unique_specs(specs));
        let decision = policy.resolve(CapabilityRequest::new(UNMATCHED[tool_index]));
        prop_assert_eq!(decision.mode(), default_mode);
        prop_assert_eq!(decision.source(), DecisionSource::Default);
    }

    #[test]
    fn wildcard_approval_is_checked_before_an_exact_allow(
        default_mode in mode(),
        pair_index in 0usize..TOOL_WILDCARD_PAIRS.len(),
    ) {
        let (tool, wildcard) = TOOL_WILDCARD_PAIRS[pair_index];
        let policy = build_policy(
            default_mode,
            &[
                RuleSpec {
                    pattern: wildcard.to_owned(),
                    mode: PermissionMode::Approval,
                    wildcard: true,
                },
                RuleSpec {
                    pattern: tool.to_owned(),
                    mode: PermissionMode::Allow,
                    wildcard: false,
                },
            ],
        );
        let decision = policy.resolve(CapabilityRequest::new(tool));
        prop_assert_eq!(decision.mode(), PermissionMode::Approval);
        prop_assert_eq!(decision.source(), DecisionSource::WildcardApproval);
    }
}
