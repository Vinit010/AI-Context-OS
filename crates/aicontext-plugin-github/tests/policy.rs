//! The permission matrix the plugin relies on.
//!
//! The plugin does not decide whether a call may run. The host resolves every capability through
//! `aicontext-permissions` before the plugin is contacted (`docs/PLUGIN_SPEC.md` §5), and the base
//! template grants no `github.*` tool. These tests pin the two halves of that contract: the default
//! is DENY, and an ALLOW exists only when the developer writes the exact rule.

use aicontext_core::PermissionMode;
use aicontext_permissions::{CapabilityRequest, DecisionSource, Policy, Rule};
use aicontext_plugin_github::tools;

fn exact(tool: &str, mode: PermissionMode) -> Rule {
    Rule::exact(tool, mode).expect("the test tool name is a valid exact rule")
}

fn policy(rules: Vec<Rule>) -> Policy {
    Policy::new(PermissionMode::Deny, rules).expect("the test rule set is valid")
}

#[test]
fn every_github_tool_is_denied_by_the_default_policy() {
    let policy = Policy::deny_all();
    for tool in tools::TOOL_NAMES {
        let decision = policy.resolve(CapabilityRequest::new(tool));
        assert_eq!(
            decision.mode(),
            PermissionMode::Deny,
            "{tool} must not be allowed by the default policy"
        );
        assert_eq!(decision.source(), DecisionSource::Default);
    }
}

#[test]
fn an_exact_grant_allows_only_the_named_tool() {
    let policy = policy(vec![exact(tools::ISSUE_READ, PermissionMode::Allow)]);

    let granted = policy.resolve(CapabilityRequest::new(tools::ISSUE_READ));
    assert_eq!(granted.mode(), PermissionMode::Allow);
    assert_eq!(granted.source(), DecisionSource::ExactAllow);

    for tool in tools::TOOL_NAMES {
        if tool == tools::ISSUE_READ {
            continue;
        }
        assert_eq!(
            policy.resolve(CapabilityRequest::new(tool)).mode(),
            PermissionMode::Deny,
            "granting {} must not grant {tool}",
            tools::ISSUE_READ
        );
    }
}

#[test]
fn all_seven_tools_need_seven_explicit_grants() {
    let rules = tools::TOOL_NAMES
        .iter()
        .map(|tool| exact(tool, PermissionMode::Allow))
        .collect();
    let policy = policy(rules);

    for tool in tools::TOOL_NAMES {
        assert_eq!(
            policy.resolve(CapabilityRequest::new(tool)).mode(),
            PermissionMode::Allow,
            "{tool} has an explicit grant"
        );
    }
}

#[test]
fn an_issue_wildcard_approval_does_not_cover_repo_reads() {
    let policy = policy(vec![
        Rule::wildcard("github.issue.*", PermissionMode::Approval)
            .expect("github.issue.* is a provider.service wildcard"),
    ]);

    for tool in [tools::ISSUE_LIST, tools::ISSUE_READ] {
        let decision = policy.resolve(CapabilityRequest::new(tool));
        assert_eq!(decision.mode(), PermissionMode::Approval);
        assert_eq!(decision.source(), DecisionSource::WildcardApproval);
    }
    assert_eq!(
        policy
            .resolve(CapabilityRequest::new(tools::REPO_READ))
            .mode(),
        PermissionMode::Deny
    );
}

#[test]
fn a_wildcard_deny_beats_an_exact_allow() {
    let policy = policy(vec![
        Rule::wildcard("github.issue.*", PermissionMode::Deny)
            .expect("github.issue.* is a provider.service wildcard"),
        exact(tools::ISSUE_READ, PermissionMode::Allow),
    ]);

    let decision = policy.resolve(CapabilityRequest::new(tools::ISSUE_READ));
    assert_eq!(decision.mode(), PermissionMode::Deny);
    assert_eq!(decision.source(), DecisionSource::WildcardDeny);
}

#[test]
fn an_allow_rule_cannot_lower_the_destructive_floor() {
    let policy = policy(vec![exact(tools::REPO_READ, PermissionMode::Allow)]);
    let decision = policy.resolve(CapabilityRequest::new(tools::REPO_READ).with_destructive(true));

    assert_eq!(decision.mode(), PermissionMode::ExplicitApproval);
    assert_eq!(decision.source(), DecisionSource::Destructive);
}
