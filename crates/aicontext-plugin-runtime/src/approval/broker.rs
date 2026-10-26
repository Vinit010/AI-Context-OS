//! The decision itself.
//!
//! The broker is the only place that decides whether a call may proceed after the policy has
//! spoken. It is deliberately small and total: every mode has an answer, every unanswered prompt is
//! a denial, and the two flags have exactly the reach `docs/SECURITY.md` §9 gives them.
//!
//! # Order of operations
//!
//! 1. `ALLOW` and `DENY` are answered without a prompt or an event; the tool gateway's own audit
//!    record covers the call.
//! 2. `APPROVAL` checks the exact `--allow-once` set. A match is recorded and allowed.
//! 3. Everything else renders the prompt, records [`ApprovalEvent::Prompted`], asks, and records
//!    [`ApprovalEvent::Decided`]. Recording before asking is what makes an interrupted prompt
//!    visible.
//! 4. A failed audit record stops the call before the next step. The human is never asked a
//!    question whose answer could not be recorded.
//!
//! `EXPLICIT_APPROVAL` deliberately skips step 2: no flag can satisfy it, and it is evaluated again
//! for every invocation (`docs/SECURITY.md` §9, invariant `I4`).

use std::collections::BTreeSet;

use aicontext_core::PermissionMode;

use super::error::ApprovalError;
use super::event::{ApprovalAudit, ApprovalEvent, ApprovalGrant, ApprovalOutcome, DenialReason};
use super::prompt::{PromptError, Prompter, is_approval_answer, render_prompt};
use super::request::ApprovalRequest;

/// Decides one tool call at a time.
///
/// The broker owns its prompter and audit sink because a prompt is a sequential interaction and a
/// sink appends in order. It holds no session grant: `TASK-072` implements the per-invocation
/// decision, and a future session grant must be added here rather than inferred from repetition.
#[derive(Debug)]
pub struct ApprovalBroker<P, A> {
    prompter: P,
    audit: A,
    allow_once: BTreeSet<String>,
}

impl<P: Prompter, A: ApprovalAudit> ApprovalBroker<P, A> {
    /// Builds a broker with the tools `--allow-once` named.
    ///
    /// The set is exact-match and case-sensitive: `--allow-once aws.ec2.stop` does not approve
    /// `aws.ec2.start`, a wildcard, or a differently-cased name. Repetition of the same value is
    /// harmless.
    #[must_use]
    pub fn new(prompter: P, audit: A, allow_once: impl IntoIterator<Item = String>) -> Self {
        Self {
            prompter,
            audit,
            allow_once: allow_once.into_iter().collect(),
        }
    }

    /// Decides whether `request` may proceed.
    ///
    /// A returned [`ApprovalOutcome`] is a decision: allowed, or denied for a reason the caller can
    /// report. An [`ApprovalError`] means the decision could not be made *because it could not be
    /// audited*; the caller must treat it as a refusal, not as an approval.
    ///
    /// # Errors
    ///
    /// [`ApprovalError::Audit`] when the audit sink rejects the prompt or decision event.
    #[must_use = "an approval decision must be acted on or refused"]
    pub fn authorize(
        &mut self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalOutcome, ApprovalError> {
        match request.mode() {
            PermissionMode::Allow => Ok(ApprovalOutcome::Allowed {
                grant: ApprovalGrant::Policy,
            }),
            PermissionMode::Deny => Ok(ApprovalOutcome::Denied {
                reason: DenialReason::Policy,
            }),
            PermissionMode::Approval | PermissionMode::ExplicitApproval => {
                self.authorize_with_human(request)
            }
        }
    }

    /// Consumes the broker, returning the terminal and the audit sink for inspection.
    ///
    /// This exists so a test can assert what the prompter saw and what the sink recorded without
    /// sharing either through an `Rc`. Production code has no reason to call it.
    #[must_use]
    pub fn into_parts(self) -> (P, A) {
        (self.prompter, self.audit)
    }

    /// The prompt path, shared by `APPROVAL` and `EXPLICIT_APPROVAL`.
    fn authorize_with_human(
        &mut self,
        request: &ApprovalRequest,
    ) -> Result<ApprovalOutcome, ApprovalError> {
        let summary = request.summary();

        // Only APPROVAL can be satisfied by the flag. A destructive capability resolves to
        // EXPLICIT_APPROVAL, so this condition is the whole of `I4`.
        if request.mode() == PermissionMode::Approval && self.allow_once.contains(request.tool()) {
            let outcome = ApprovalOutcome::Allowed {
                grant: ApprovalGrant::AllowOnce,
            };
            self.record(ApprovalEvent::Decided { summary, outcome })?;
            return Ok(outcome);
        }

        let prompt = render_prompt(request);
        // Record before showing: a prompt the human saw is a fact even if the answer never arrives.
        self.record(ApprovalEvent::Prompted {
            summary: summary.clone(),
        })?;
        let outcome = match self.prompter.ask(&prompt) {
            Ok(answer) if is_approval_answer(&answer) => ApprovalOutcome::Allowed {
                grant: ApprovalGrant::Interactive,
            },
            Ok(_) => ApprovalOutcome::Denied {
                reason: DenialReason::Declined,
            },
            Err(PromptError::Unavailable) => ApprovalOutcome::Denied {
                reason: DenialReason::NotInteractive,
            },
            Err(PromptError::EndOfInput) => ApprovalOutcome::Denied {
                reason: DenialReason::EndOfInput,
            },
            Err(PromptError::Interrupted) => ApprovalOutcome::Denied {
                reason: DenialReason::Interrupted,
            },
            Err(PromptError::TimedOut) => ApprovalOutcome::Denied {
                reason: DenialReason::TimedOut,
            },
            Err(PromptError::AnswerTooLong) => ApprovalOutcome::Denied {
                reason: DenialReason::AnswerTooLong,
            },
            Err(PromptError::Io { .. }) => ApprovalOutcome::Denied {
                reason: DenialReason::PromptFailed,
            },
        };
        self.record(ApprovalEvent::Decided { summary, outcome })?;
        Ok(outcome)
    }

    /// Records one event or refuses the call.
    fn record(&mut self, event: ApprovalEvent) -> Result<(), ApprovalError> {
        self.audit
            .record(&event)
            .map_err(|error| ApprovalError::Audit {
                message: error.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::io;
    use std::rc::Rc;

    use aicontext_core::PermissionMode;
    use aicontext_permissions::{CapabilityRequest, Policy};
    use serde_json::json;

    use super::ApprovalBroker;
    use crate::approval::{
        ApprovalAudit, ApprovalError, ApprovalEvent, ApprovalGrant, ApprovalOutcome,
        ApprovalRequest, DenialReason, PromptError, Prompter,
    };

    const TOOL: &str = "aws.ec2.stop";

    fn request(mode: PermissionMode) -> ApprovalRequest {
        ApprovalRequest::new(
            TOOL,
            "aws",
            "i-0abc1234",
            json!({ "instance_id": "i-0abc1234" }),
            mode,
        )
        .with_reason("the instance is idle")
    }

    /// A prompter whose answers are written down, so a missing prompt is a test failure.
    #[derive(Debug, Default)]
    struct ScriptedPrompter {
        answers: VecDeque<Result<String, PromptError>>,
        prompts: Vec<String>,
    }

    impl ScriptedPrompter {
        fn answering(answer: &str) -> Self {
            let mut prompter = Self::default();
            prompter.answers.push_back(Ok(answer.to_owned()));
            prompter
        }

        fn failing(error: PromptError) -> Self {
            let mut prompter = Self::default();
            prompter.answers.push_back(Err(error));
            prompter
        }
    }

    impl Prompter for ScriptedPrompter {
        fn ask(&mut self, prompt: &str) -> Result<String, PromptError> {
            self.prompts.push(prompt.to_owned());
            self.answers
                .pop_front()
                .expect("the broker asked for more answers than the test scripted")
        }
    }

    #[derive(Debug)]
    struct AuditFailure;

    impl std::fmt::Display for AuditFailure {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("audit log is closed")
        }
    }

    impl std::error::Error for AuditFailure {}

    /// Records events in order and can fail on the nth one (zero-based).
    #[derive(Debug, Default)]
    struct Recorder {
        events: Vec<ApprovalEvent>,
        fail_on: Option<usize>,
    }

    impl Recorder {
        fn failing_on(index: usize) -> Self {
            Self {
                events: Vec::new(),
                fail_on: Some(index),
            }
        }
    }

    impl ApprovalAudit for Recorder {
        type Error = AuditFailure;

        fn record(&mut self, event: &ApprovalEvent) -> Result<(), Self::Error> {
            if self.fail_on == Some(self.events.len()) {
                return Err(AuditFailure);
            }
            self.events.push(event.clone());
            Ok(())
        }
    }

    fn run(
        mode: PermissionMode,
        prompter: ScriptedPrompter,
        allow_once: &[&str],
    ) -> (ApprovalOutcome, ScriptedPrompter, Recorder) {
        let mut broker = ApprovalBroker::new(
            prompter,
            Recorder::default(),
            allow_once.iter().map(|tool| (*tool).to_owned()),
        );
        let outcome = broker
            .authorize(&request(mode))
            .expect("the recorder is infallible in this test");
        let (prompter, audit) = broker.into_parts();
        (outcome, prompter, audit)
    }

    #[test]
    fn allow_is_granted_without_a_prompt_or_an_event() {
        let (outcome, prompter, audit) =
            run(PermissionMode::Allow, ScriptedPrompter::default(), &[TOOL]);

        assert_eq!(outcome.grant(), Some(ApprovalGrant::Policy));
        assert!(prompter.prompts.is_empty());
        assert!(audit.events.is_empty());
    }

    #[test]
    fn deny_is_refused_without_a_prompt_or_an_event() {
        let (outcome, prompter, audit) =
            run(PermissionMode::Deny, ScriptedPrompter::default(), &[TOOL]);

        assert_eq!(outcome.denial(), Some(DenialReason::Policy));
        assert!(prompter.prompts.is_empty());
        assert!(audit.events.is_empty());
    }

    #[test]
    fn non_interactive_denies_by_default() {
        // `docs/SECURITY.md` §3 invariant I9 and §9: absence of a human is a deny, never consent.
        let (outcome, prompter, audit) = run(
            PermissionMode::Approval,
            ScriptedPrompter::failing(PromptError::Unavailable),
            &[],
        );

        assert_eq!(outcome.denial(), Some(DenialReason::NotInteractive));
        assert_eq!(
            prompter.prompts.len(),
            1,
            "the prompt text is still handed to the terminal so it can be printed"
        );
        assert!(matches!(audit.events[0], ApprovalEvent::Prompted { .. }));
        assert_eq!(
            audit.events[1]
                .outcome()
                .and_then(|outcome| outcome.denial()),
            Some(DenialReason::NotInteractive)
        );
    }

    #[test]
    fn only_the_literal_word_yes_allows_an_interactive_prompt() {
        for answer in ["yes", "yes\n", " yes \n"] {
            let (outcome, prompter, _) = run(
                PermissionMode::Approval,
                ScriptedPrompter::answering(answer),
                &[],
            );
            assert_eq!(
                outcome.grant(),
                Some(ApprovalGrant::Interactive),
                "{answer:?} should approve"
            );
            assert_eq!(prompter.prompts.len(), 1);
        }

        for answer in ["", "\n", "y", "YES", "Yes", "yes please", "no"] {
            let (outcome, _, _) = run(
                PermissionMode::Approval,
                ScriptedPrompter::answering(answer),
                &[],
            );
            assert_eq!(
                outcome.denial(),
                Some(DenialReason::Declined),
                "{answer:?} should deny"
            );
        }
    }

    #[test]
    fn every_prompt_failure_is_a_distinct_denial() {
        let cases = [
            (PromptError::Unavailable, DenialReason::NotInteractive),
            (PromptError::EndOfInput, DenialReason::EndOfInput),
            (PromptError::Interrupted, DenialReason::Interrupted),
            (PromptError::TimedOut, DenialReason::TimedOut),
            (PromptError::AnswerTooLong, DenialReason::AnswerTooLong),
            (
                PromptError::Io {
                    source: io::Error::other("terminal closed"),
                },
                DenialReason::PromptFailed,
            ),
        ];

        for (error, expected) in cases {
            let (outcome, prompter, audit) = run(
                PermissionMode::Approval,
                ScriptedPrompter::failing(error),
                &[],
            );
            assert_eq!(outcome.denial(), Some(expected));
            assert_eq!(prompter.prompts.len(), 1);
            assert_eq!(audit.events.len(), 2);
            assert_eq!(
                audit.events[1]
                    .outcome()
                    .and_then(|outcome| outcome.denial()),
                Some(expected)
            );
        }
    }

    #[test]
    fn an_exact_allow_once_grant_skips_the_prompt_and_is_audited() {
        let (outcome, prompter, audit) = run(
            PermissionMode::Approval,
            ScriptedPrompter::default(),
            &[TOOL],
        );

        assert_eq!(outcome.grant(), Some(ApprovalGrant::AllowOnce));
        assert!(
            prompter.prompts.is_empty(),
            "a flag grant must not ask a question whose answer it would ignore"
        );
        assert_eq!(audit.events.len(), 1, "there is a decision but no prompt");
        assert_eq!(
            audit.events[0]
                .outcome()
                .and_then(|outcome| outcome.grant()),
            Some(ApprovalGrant::AllowOnce)
        );
    }

    #[test]
    fn allow_once_is_exact_and_does_not_match_another_tool() {
        let (outcome, prompter, _) = run(
            PermissionMode::Approval,
            ScriptedPrompter::answering("no"),
            &["aws.ec2.start", "AWS.EC2.STOP"],
        );

        assert_eq!(outcome.denial(), Some(DenialReason::Declined));
        assert_eq!(prompter.prompts.len(), 1);
    }

    #[test]
    fn destructive_requires_explicit_approval_each_call() {
        // I4: the resolver raises a destructive capability to EXPLICIT_APPROVAL, and the broker
        // must ask again on every invocation. `--allow-once` is supplied to prove it is ignored.
        let policy = Policy::default();
        let decision = policy.resolve(CapabilityRequest::new(TOOL).with_destructive(true));
        assert_eq!(decision.mode(), PermissionMode::ExplicitApproval);

        let mut prompter = ScriptedPrompter::default();
        prompter.answers.push_back(Ok("yes\n".to_owned()));
        prompter.answers.push_back(Ok("yes\n".to_owned()));
        let mut broker = ApprovalBroker::new(prompter, Recorder::default(), [TOOL.to_owned()]);

        for _ in 0..2 {
            let outcome = broker
                .authorize(&request(decision.mode()))
                .expect("the recorder is infallible");
            assert_eq!(outcome.grant(), Some(ApprovalGrant::Interactive));
        }

        let (prompter, audit) = broker.into_parts();
        assert_eq!(prompter.prompts.len(), 2, "one prompt per invocation");
        assert!(
            audit
                .events
                .iter()
                .filter_map(ApprovalEvent::outcome)
                .all(|outcome| outcome.grant() == Some(ApprovalGrant::Interactive)),
            "the flag cannot grant an explicit approval"
        );
    }

    #[test]
    fn explicit_approval_denies_in_a_pipe_even_with_allow_once() {
        let (outcome, prompter, _) = run(
            PermissionMode::ExplicitApproval,
            ScriptedPrompter::failing(PromptError::Unavailable),
            &[TOOL],
        );

        assert_eq!(outcome.denial(), Some(DenialReason::NotInteractive));
        assert_eq!(prompter.prompts.len(), 1);
    }

    #[test]
    fn every_prompt_and_answer_is_audited_in_order() {
        // One timeline shared by the audit sink and the prompter proves the ordering: the prompt is
        // recorded before it is shown, and the decision after the answer is read.
        let timeline = Rc::new(RefCell::new(Vec::new()));
        let prompter = TimelinePrompter {
            timeline: Rc::clone(&timeline),
        };
        let audit = TimelineAudit {
            timeline: Rc::clone(&timeline),
        };
        let mut broker = ApprovalBroker::new(prompter, audit, []);
        let outcome = broker
            .authorize(&request(PermissionMode::Approval))
            .expect("the recorder is infallible");

        assert!(outcome.is_allowed());
        assert_eq!(
            *timeline.borrow(),
            ["prompted", "shown", "decided"],
            "an interruption after `shown` must still leave the prompt in the log"
        );
    }

    #[derive(Debug)]
    struct TimelinePrompter {
        timeline: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Prompter for TimelinePrompter {
        fn ask(&mut self, _prompt: &str) -> Result<String, PromptError> {
            self.timeline.borrow_mut().push("shown");
            Ok("yes\n".to_owned())
        }
    }

    #[derive(Debug)]
    struct TimelineAudit {
        timeline: Rc<RefCell<Vec<&'static str>>>,
    }

    impl ApprovalAudit for TimelineAudit {
        type Error = AuditFailure;

        fn record(&mut self, event: &ApprovalEvent) -> Result<(), Self::Error> {
            self.timeline.borrow_mut().push(match event {
                ApprovalEvent::Prompted { .. } => "prompted",
                ApprovalEvent::Decided { .. } => "decided",
            });
            Ok(())
        }
    }

    #[test]
    fn an_audit_failure_before_the_prompt_refuses_without_asking() {
        let mut prompter = ScriptedPrompter::default();
        prompter.answers.push_back(Ok("yes\n".to_owned()));
        let mut broker = ApprovalBroker::new(prompter, Recorder::failing_on(0), []);

        let error = broker
            .authorize(&request(PermissionMode::Approval))
            .expect_err("an unaudited prompt must not be shown");

        assert!(matches!(error, ApprovalError::Audit { .. }));
        let (prompter, audit) = broker.into_parts();
        assert!(
            prompter.prompts.is_empty(),
            "the human must not be asked when the prompt cannot be recorded"
        );
        assert!(audit.events.is_empty());
    }

    #[test]
    fn an_audit_failure_after_the_answer_refuses_the_call() {
        let mut broker = ApprovalBroker::new(
            ScriptedPrompter::answering("yes\n"),
            Recorder::failing_on(1),
            [],
        );

        let error = broker
            .authorize(&request(PermissionMode::Approval))
            .expect_err("an unaudited yes is not an approval");

        assert!(matches!(error, ApprovalError::Audit { .. }));
        let (_, audit) = broker.into_parts();
        assert_eq!(
            audit.events.len(),
            1,
            "the prompt was recorded, the decision was not"
        );
    }

    #[test]
    fn audit_events_do_not_carry_raw_arguments_or_reason() {
        let (_, _, audit) = run(
            PermissionMode::Approval,
            ScriptedPrompter::answering("no"),
            &[],
        );
        let recorded = format!("{:?}", audit.events);

        assert!(
            !recorded.contains("instance_id"),
            "arguments must be a digest, never raw values: {recorded}"
        );
        assert!(
            !recorded.contains("the instance is idle"),
            "the free-form reason is prompt material, not audit material: {recorded}"
        );
        assert!(recorded.contains("sha256:"));
        assert!(recorded.contains(TOOL), "the tool name is the subject");
    }

    #[test]
    fn the_mode_grant_and_answer_matrix_is_total() {
        for mode in PermissionMode::ALL {
            for grant in [false, true] {
                for answer in ["yes", "no", "unavailable"] {
                    let allow_once: &[&str] = if grant { &[TOOL] } else { &[] };
                    let prompter = match answer {
                        "yes" => ScriptedPrompter::answering("yes\n"),
                        "unavailable" => ScriptedPrompter::failing(PromptError::Unavailable),
                        _ => ScriptedPrompter::answering("no\n"),
                    };
                    let (outcome, prompter, audit) = run(mode, prompter, allow_once);

                    match mode {
                        PermissionMode::Allow => {
                            assert_eq!(outcome.grant(), Some(ApprovalGrant::Policy));
                            assert!(prompter.prompts.is_empty());
                            assert!(audit.events.is_empty());
                        }
                        PermissionMode::Deny => {
                            assert_eq!(outcome.denial(), Some(DenialReason::Policy));
                            assert!(prompter.prompts.is_empty());
                            assert!(audit.events.is_empty());
                        }
                        PermissionMode::Approval if grant => {
                            assert_eq!(outcome.grant(), Some(ApprovalGrant::AllowOnce));
                            assert!(prompter.prompts.is_empty());
                            assert_eq!(audit.events.len(), 1);
                        }
                        PermissionMode::Approval => {
                            assert_eq!(prompter.prompts.len(), 1);
                            assert_eq!(audit.events.len(), 2);
                            let expected = match answer {
                                "yes" => Some(ApprovalGrant::Interactive),
                                _ => None,
                            };
                            assert_eq!(outcome.grant(), expected);
                            if expected.is_none() {
                                assert!(outcome.denial().is_some());
                            }
                        }
                        PermissionMode::ExplicitApproval => {
                            assert_eq!(prompter.prompts.len(), 1);
                            assert_eq!(audit.events.len(), 2);
                            assert!(
                                outcome.grant() != Some(ApprovalGrant::AllowOnce),
                                "a flag must not satisfy explicit approval"
                            );
                            if answer == "yes" {
                                assert_eq!(outcome.grant(), Some(ApprovalGrant::Interactive));
                            } else if answer == "unavailable" {
                                assert_eq!(outcome.denial(), Some(DenialReason::NotInteractive));
                            } else {
                                assert_eq!(outcome.denial(), Some(DenialReason::Declined));
                            }
                        }
                    }
                }
            }
        }
    }
}
