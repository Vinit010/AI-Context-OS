//! The typed failures of `aicontext agent run` (`TASK-055`).
//!
//! One rule decides every variant: a failure that already has a vocabulary is *delegated* to it
//! rather than re-labelled. A malformed `.ai/ai.yaml` is the connect reader's `CONNECT-002`, an
//! assembly that cannot fit is the context crate's `CTX-030`, a rejected credential is
//! `CREDENTIAL_ENVIRONMENT_MISSING`, and an unreachable server is `PROVIDER_TRANSPORT` — repeating
//! any of those codes here would be a second opinion about a question that already has one
//! (`RULES.md` §2). What this module owns is what only `agent run` can fail at: no provider to run
//! against, a task the register does not declare, a model answer that could not be trusted as a
//! proposal, and a plan file that could not be written.
//!
//! Every variant names its code, its exit code, and its remediation here, so no call site
//! improvises any of the three.

use aicontext_providers::{CredentialError, ProviderError};

use crate::connect::ConnectError;
use crate::exit::Exit;
use crate::output::Terminal;
use crate::project::ProjectError;

/// An `agent run` that could not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AgentError {
    /// The project root could not be used.
    #[error(transparent)]
    Root(#[from] ProjectError),

    /// `.ai/ai.yaml` could not be read, or does not have the shape the schema describes. The
    /// reader's own failure, reported in the reader's vocabulary.
    #[error(transparent)]
    Config(#[from] ConnectError),

    /// Context assembly failed because the never-dropped tiers do not fit the budget
    /// (`CTX-030`). The context crate's own code, severity, and remediation.
    #[error(transparent)]
    Budget(#[from] aicontext_context::BudgetError),

    /// A credential the profile names could not be resolved.
    #[error(transparent)]
    Credential(#[from] CredentialError),

    /// The provider failed, timed out, or was unreachable.
    #[error(transparent)]
    Provider(#[from] ProviderError),

    /// `.ai/ai.yaml` names no default provider, so there is nothing to run against.
    #[error("{path} names no `default_provider` (known providers: {known})")]
    NoDefaultProvider {
        /// The file as a report spells it.
        path: String,
        /// The ids the file does list, or `none`.
        known: String,
    },

    /// The default provider points at an id the file does not list — a dangling reference.
    #[error("the default provider `{id}` is not listed in {path} (known providers: {known})")]
    DefaultNotListed {
        /// The id `default_provider` names.
        id: String,
        /// The file that was searched.
        path: String,
        /// The ids the file does list, or `none`.
        known: String,
    },

    /// `--task` names a task the register does not declare.
    #[error("no task `{id}` in .ai/TASKS.md (known tasks: {known})")]
    UnknownTask {
        /// The id that was asked for.
        id: String,
        /// Every id the register declares, as a comma-separated list.
        known: String,
    },

    /// The profile records an adapter this build does not implement.
    #[error("unknown adapter `{adapter}` (known adapters: {known})")]
    UnsupportedAdapter {
        /// The adapter name the file recorded.
        adapter: String,
        /// Every adapter name this build accepts, as a comma-separated list.
        known: String,
    },

    /// The model answered with something that claimed to be an `action_proposal` and could not
    /// prove it (`docs/AI_PROVIDER_SPEC.md` §9). Rejected whole, and recorded.
    #[error("the model's answer is not a valid action proposal:\n  {}", .problems.join("\n  "))]
    MalformedProposal {
        /// Every problem the validation found, one per line.
        problems: Vec<String>,
    },

    /// The model declined to produce a plan, which is a first-class outcome reported rather than
    /// worked around (`docs/AI_PROVIDER_SPEC.md` §6).
    #[error("the model declined to produce a plan: {}", clip(text))]
    Declined {
        /// The model's own words, kept so the refusal can be read.
        text: String,
    },

    /// The provider answered with something plan mode cannot interpret, such as a tool call when
    /// no tools were offered.
    #[error("the provider returned a response `agent run` cannot interpret: {detail}")]
    UnexpectedResponse {
        /// What was wrong with it, in the adapter's terms.
        detail: String,
    },

    /// The plan document could not be written under `.aicontext/plans/`.
    #[error("cannot write the plan to {path}: {reason}")]
    PlanWrite {
        /// Where the run tried to write.
        path: String,
        /// What the filesystem said.
        reason: String,
    },

    /// `agent run` calls a provider, and `--offline` forbids every call.
    #[error("`agent run` asks the provider for the plan, which `--offline` forbids")]
    Offline,
}

impl AgentError {
    /// The stable code, safe to branch on.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Root(error) => error.code(),
            Self::Config(error) => error.code(),
            // `BudgetError::code()` is the `const` `CTX-030` for every variant it has, and
            // `ErrorCode` offers only `Display`. The literal keeps this return `&'static str`;
            // the drift-net test below pins it to the context crate's own `code()`.
            Self::Budget(_) => "CTX-030",
            Self::Credential(error) => error.code(),
            Self::Provider(error) => error.code(),
            Self::NoDefaultProvider { .. } | Self::DefaultNotListed { .. } => "AGENT-001",
            Self::UnknownTask { .. } => "AGENT-002",
            Self::Offline => "AGENT-003",
            Self::UnsupportedAdapter { .. } => "AGENT-004",
            Self::MalformedProposal { .. } => "AGENT-005",
            Self::Declined { .. } => "AGENT-007",
            Self::UnexpectedResponse { .. } => "AGENT-008",
            Self::PlanWrite { .. } => "AGENT-009",
        }
    }

    /// What the developer should do about it (`RULES.md` §4.3).
    pub(crate) fn remediation(&self) -> &'static str {
        match self {
            Self::Root(error) => error.remediation(),
            Self::Config(error) => error.remediation(),
            Self::Budget(error) => error.remediation(),
            Self::Credential(error) => error.remediation(),
            Self::Provider(error) => error.remediation(),
            Self::NoDefaultProvider { .. } | Self::DefaultNotListed { .. } => {
                "run `aicontext connect --provider <id> --base-url <url> --model <name>` to \
                 configure one, or fix `default_provider` in `.ai/ai.yaml`"
            }
            Self::UnknownTask { .. } => {
                "check the task id against `.ai/TASKS.md` — the register is the list of tasks that \
                 exist"
            }
            Self::Offline => "drop `--offline`, or use `--dry-run`, which makes no call",
            Self::UnsupportedAdapter { .. } => {
                "check the `adapter` in `.ai/ai.yaml` against the list printed above — \
                 `docs/AI_PROVIDER_SPEC.md` §3 describes each adapter's wire format"
            }
            Self::MalformedProposal { .. } => {
                "the problems above say what the proposal was missing; re-run once — a model that \
                 was told the contract once may still answer with prose, which is accepted"
            }
            Self::Declined { .. } => {
                "the model's answer is printed above; try a wider task scope, or a different model"
            }
            Self::UnexpectedResponse { .. } => {
                "check the provider's own status page or logs; the response could not be \
                 interpreted as an answer to this request"
            }
            Self::PlanWrite { .. } => {
                "check that `.aicontext/` is writable and has free space, then run it again"
            }
        }
    }

    /// The exit code `docs/CLI_SPEC.md` §5 assigns to this failure.
    pub(crate) fn exit(&self) -> Exit {
        match self {
            Self::Root(error) => error.exit(),
            Self::Config(error) => error.exit(),
            // A credential the profile names but the machine cannot produce is a local setup
            // failure, like a plan file that cannot be written — not the provider's fault.
            Self::Credential(_) | Self::PlanWrite { .. } => Exit::General,
            // An answer the provider layer could not read is the provider's failure, exactly as a
            // transport error is.
            Self::Provider(_) | Self::UnexpectedResponse { .. } => Exit::Provider,
            // The packet or the model's answer did not hold up: a validation failure.
            Self::NoDefaultProvider { .. }
            | Self::DefaultNotListed { .. }
            | Self::UnsupportedAdapter { .. }
            | Self::MalformedProposal { .. }
            | Self::Declined { .. }
            // Assembly runs on an already-built index, so its failures are the packet's shape —
            // today always `CTX-030`, a budget that cannot hold the never-dropped tiers — and the
            // `context` command maps the same failure to the same exit code for the same reason.
            | Self::Budget(_) => Exit::Validation,
            Self::UnknownTask { .. } | Self::Offline => Exit::Usage,
        }
    }

    /// Writes the report to stderr and returns the exit code, once and only once (`RULES.md` §4.5).
    pub(crate) fn report(&self, terminal: &mut Terminal) -> Exit {
        let exit = self.exit();
        terminal.error_report(self.code(), &self.to_string(), self.remediation());
        exit
    }
}

/// A model's refusal, long enough to read and short enough to fit a report line.
fn clip(text: &str) -> String {
    const MAX: usize = 240;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let clipped: String = trimmed.chars().take(MAX).collect();
    format!("{clipped}…")
}

#[cfg(test)]
mod tests {
    use aicontext_providers::{CredentialError, ProviderError};

    use super::AgentError;
    use crate::exit::Exit;

    #[test]
    fn the_agent_codes_are_stable() {
        // Codes a script branches on must not move, so each one is asserted where it is defined.
        let no_provider = AgentError::NoDefaultProvider {
            path: ".ai/ai.yaml".to_string(),
            known: "none".to_string(),
        };
        assert_eq!(no_provider.code(), "AGENT-001");
        assert_eq!(no_provider.exit(), Exit::Validation);

        let unknown = AgentError::UnknownTask {
            id: "TASK-999".to_string(),
            known: "TASK-001, TASK-002".to_string(),
        };
        assert_eq!(unknown.code(), "AGENT-002");
        assert_eq!(unknown.exit(), Exit::Usage);
        assert!(unknown.to_string().contains("TASK-001"));

        assert_eq!(AgentError::Offline.code(), "AGENT-003");
        assert_eq!(AgentError::Offline.exit(), Exit::Usage);
    }

    #[test]
    fn a_failure_with_another_vocabulary_is_delegated_not_relabelled() {
        let provider = AgentError::from(ProviderError::Unauthorized {
            provider: aicontext_providers::ProviderId::new("hosted").expect("valid id"),
        });
        assert_eq!(provider.code(), "PROVIDER_UNAUTHORIZED");
        assert_eq!(provider.exit(), Exit::Provider);

        let credential = AgentError::from(CredentialError::EnvironmentMissing {
            variable: "AICON_AI_HOSTED_KEY".to_string(),
        });
        assert_eq!(credential.code(), "CREDENTIAL_ENVIRONMENT_MISSING");
        assert_eq!(credential.exit(), Exit::General);
    }

    #[test]
    fn the_budget_code_is_pinned_to_the_context_crate() {
        // `code()` returns the literal "CTX-030" because `ErrorCode` offers only `Display`.
        // This pins the literal to the crate that owns the code, so a renumbering in
        // `CONTEXT_SPEC.md` §8 fails here instead of shipping a stale label.
        let budget = aicontext_context::BudgetError::TooSmall {
            needed: 900,
            max_tokens: 100,
        };
        let wrapped = AgentError::from(budget);
        assert_eq!(wrapped.code(), "CTX-030");
        assert_eq!(
            wrapped.exit(),
            Exit::Validation,
            "the context command's code too"
        );
        let own = aicontext_context::BudgetError::TooSmall {
            needed: 900,
            max_tokens: 100,
        };
        assert_eq!(format!("{}", own.code()), wrapped.code());
    }

    #[test]
    fn a_refusal_keeps_the_models_words_and_a_bounded_length() {
        let short = AgentError::Declined {
            text: "  I will not plan that.  ".to_string(),
        };
        assert!(short.to_string().contains("I will not plan that."));

        let long = AgentError::Declined {
            text: "x".repeat(500),
        };
        assert!(
            long.to_string().chars().count() < 300,
            "the report line is bounded"
        );
    }

    #[test]
    fn a_malformed_proposal_reports_every_problem() {
        let error = AgentError::MalformedProposal {
            problems: vec![
                "`version` must be 1, found 2".to_string(),
                "actions[0]: unknown action type `dance`".to_string(),
            ],
        };
        assert_eq!(error.code(), "AGENT-005");
        assert_eq!(error.exit(), Exit::Validation);
        let message = error.to_string();
        assert!(message.contains("`version` must be 1"));
        assert!(message.contains("unknown action type `dance`"));
    }
}
