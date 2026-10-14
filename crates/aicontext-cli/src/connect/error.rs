//! The typed failures of `aicontext connect` and `aicontext disconnect`.
//!
//! One enum for both commands: they are one feature — a stored provider reference — and they read and
//! write one file, so a failure in the reader is the same failure whichever command met it
//! (`RULES.md` §4.2). Every variant names its code, its exit code, and its remediation, so neither
//! command improvises any of the three at a call site.
//!
//! A failure that belongs to another crate is delegated rather than re-labelled: a rejected provider
//! id is [`aicontext_providers::ProviderError`]'s `PROVIDER_INVALID_ID`, a missing environment
//! variable is `CREDENTIAL_ENVIRONMENT_MISSING`, and the keychain's own error is
//! `CONNECT-010`. Repeating those codes here would be a second opinion about a question that already
//! has one (`RULES.md` §2).

use aicontext_providers::{CredentialError, KeychainError, ProviderError};

use crate::exit::Exit;
use crate::output::Terminal;
use crate::project::ProjectError;

/// A `connect`/`disconnect` run that could not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    /// The project root could not be used.
    #[error(transparent)]
    Root(#[from] ProjectError),

    /// `.ai/ai.yaml` could not be read.
    #[error("cannot read {path}: {reason}")]
    Read {
        /// The file as the report spells it.
        path: String,
        /// What the filesystem said.
        reason: String,
    },

    /// `.ai/ai.yaml` is not valid YAML.
    #[error("{path} is not valid YAML: {detail}")]
    Malformed {
        /// The file as a report spells it.
        path: String,
        /// The parser's own message, with its line number.
        detail: String,
    },

    /// `.ai/ai.yaml` does not have the shape `schemas/ai-config.schema.json` describes.
    #[error("{path} has an unexpected shape: {detail}")]
    Shape {
        /// The file as a report spells it.
        path: String,
        /// What was wrong with it.
        detail: String,
    },

    /// `.ai/ai.yaml` could not be written.
    #[error("cannot write {path}: {reason}")]
    Write {
        /// The path as a report spells it.
        path: String,
        /// The filesystem's own reason.
        reason: String,
    },

    /// The named adapter has no implementation yet.
    #[error("the `{adapter}` adapter is not implemented yet ({task})")]
    UnsupportedAdapter {
        /// The adapter name that was asked for.
        adapter: String,
        /// The task that will provide it, or a note that none is scheduled.
        task: &'static str,
    },

    /// A flag that requires a value was given an empty one.
    #[error("`{flag}` was given an empty value")]
    BlankValue {
        /// The flag, named without echoing what it held.
        flag: &'static str,
    },

    /// `--keychain-entry` was given with no credential for it to name.
    #[error("`--keychain-entry` names an entry, but no credential was given to store in it")]
    KeychainEntryWithoutCredential,

    /// `connect` cannot verify without a call, and `--offline` forbids every call.
    #[error("`connect` verifies the provider with one minimal call, which `--offline` forbids")]
    Offline,

    /// `disconnect` was asked for a provider the file does not list.
    #[error("no provider `{id}` in {path} (known providers: {known})")]
    UnknownProvider {
        /// The id that was asked for.
        id: String,
        /// The path that was searched.
        path: String,
        /// The ids that *are* present, or `none` when the list is empty.
        known: String,
    },

    /// A provider id was rejected or a call failed, in the provider crate's own vocabulary.
    #[error(transparent)]
    Provider(#[from] ProviderError),

    /// A named credential source could not be resolved.
    #[error(transparent)]
    Credential(#[from] CredentialError),

    /// The OS keychain could not be read, written, or cleared.
    #[error("the OS keychain could not be updated: {source}")]
    Keychain {
        /// The keychain's own, deliberately coarse failure.
        #[from]
        source: KeychainError,
    },
}

impl ConnectError {
    /// The stable code, safe to branch on.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Root(error) => error.code(),
            Self::Provider(error) => error.code(),
            Self::Credential(error) => error.code(),
            Self::Read { .. } => "CONNECT-001",
            Self::Malformed { .. } => "CONNECT-002",
            Self::Shape { .. } => "CONNECT-003",
            Self::Write { .. } => "CONNECT-004",
            Self::UnsupportedAdapter { .. } => "CONNECT-005",
            Self::BlankValue { .. } => "CONNECT-006",
            Self::KeychainEntryWithoutCredential => "CONNECT-007",
            Self::Offline => "CONNECT-008",
            Self::UnknownProvider { .. } => "CONNECT-009",
            Self::Keychain { .. } => "CONNECT-010",
        }
    }

    /// What the developer should do about it (`RULES.md` §4.3).
    pub(crate) fn remediation(&self) -> &'static str {
        match self {
            Self::Root(error) => error.remediation(),
            Self::Provider(error) => error.remediation(),
            Self::Credential(error) => error.remediation(),
            Self::Read { .. } => {
                "check that `.ai/ai.yaml` is readable and is not held by another process, then try again"
            }
            Self::Malformed { .. } => {
                "fix the YAML at the line reported, or move the file aside and connect again"
            }
            Self::Shape { .. } => {
                "make `.ai/ai.yaml` match `.ai/schemas/ai-config.schema.json` — `aicontext doctor` \
                 names every finding"
            }
            Self::Write { .. } => {
                "check that `.ai/` exists and is writable; run `aicontext init` if the project has no \
                 `.ai/` tree yet"
            }
            Self::UnsupportedAdapter { .. } => {
                "use `--adapter openai-compatible`, the only adapter this release ships"
            }
            Self::BlankValue { .. } => "give the flag a non-empty value, or drop it",
            Self::KeychainEntryWithoutCredential => {
                "add `--token <value>` or `--env <VAR>` so there is a credential to name, or drop \
                 `--keychain-entry`"
            }
            Self::Offline => "drop `--offline`: `connect` proves the provider answers before storing it",
            Self::UnknownProvider { .. } => {
                "check the id, or run `aicontext connect` to add the provider first"
            }
            Self::Keychain { .. } => {
                "unlock the OS keychain and try again, or record `--env <VAR>` so the credential can \
                 come from the environment instead"
            }
        }
    }

    /// The exit code `docs/CLI_SPEC.md` §5 assigns to this failure.
    pub(crate) fn exit(&self) -> Exit {
        match self {
            Self::Root(error) => error.exit(),
            Self::Provider(error) if matches!(error, ProviderError::InvalidId { .. }) => Exit::Usage,
            Self::Provider(_) => Exit::Provider,
            Self::Credential(error) => credential_exit(error),
            Self::UnsupportedAdapter { .. }
            | Self::BlankValue { .. }
            | Self::KeychainEntryWithoutCredential
            | Self::Offline
            | Self::UnknownProvider { .. } => Exit::Usage,
            Self::Malformed { .. } | Self::Shape { .. } => Exit::Validation,
            Self::Read { .. } | Self::Write { .. } | Self::Keychain { .. } => Exit::General,
        }
    }

    /// Writes the report to stderr and returns the exit code, once and only once (`RULES.md` §4.5).
    pub(crate) fn report(&self, terminal: &mut Terminal) -> Exit {
        let exit = self.exit();
        terminal.error_report(self.code(), &self.to_string(), self.remediation());
        exit
    }
}

/// Where a credential failure lands: a source the invocation named is a usage error, an unreachable
/// keychain is the environment's problem.
fn credential_exit(error: &CredentialError) -> Exit {
    match error {
        CredentialError::EnvironmentMissing { .. } | CredentialError::KeychainMissing { .. } => {
            Exit::Usage
        }
        _ => Exit::General,
    }
}

#[cfg(test)]
mod tests {
    use aicontext_providers::{CredentialError, ProviderError};

    use super::ConnectError;

    #[test]
    fn a_rejected_provider_id_is_a_usage_error_carrying_the_provider_code() {
        let error = ConnectError::from(ProviderError::InvalidId {
            value: "Not An Id".to_string(),
        });
        assert_eq!(error.code(), "PROVIDER_INVALID_ID");
        assert_eq!(error.exit(), crate::exit::Exit::Usage);
        assert!(error.remediation().contains("lowercase slug"));
    }

    #[test]
    fn a_failed_call_is_a_provider_error() {
        let error = ConnectError::from(ProviderError::Unauthorized {
            provider: aicontext_providers::ProviderId::new("hosted").expect("valid id"),
        });
        assert_eq!(error.code(), "PROVIDER_UNAUTHORIZED");
        assert_eq!(error.exit(), crate::exit::Exit::Provider);
    }

    #[test]
    fn a_missing_variable_is_usage_and_an_unavailable_keychain_is_not() {
        let missing = ConnectError::from(CredentialError::EnvironmentMissing {
            variable: "AICON_AI_HOSTED_KEY".to_string(),
        });
        assert_eq!(missing.code(), "CREDENTIAL_ENVIRONMENT_MISSING");
        assert_eq!(missing.exit(), crate::exit::Exit::Usage);

        let unavailable = ConnectError::from(CredentialError::KeychainUnavailable {
            reference: aicontext_providers::KeychainReference::new("aicontext", "hosted"),
            reason: aicontext_providers::KeychainError::Unavailable,
        });
        assert_eq!(unavailable.code(), "CREDENTIAL_KEYCHAIN_UNAVAILABLE");
        assert_eq!(unavailable.exit(), crate::exit::Exit::General);
    }

    #[test]
    fn the_local_codes_are_stable() {
        assert_eq!(
            ConnectError::Offline.code(),
            "CONNECT-008",
            "a code scripts branch on must not move"
        );
        assert_eq!(
            ConnectError::UnsupportedAdapter {
                adapter: "anthropic".to_string(),
                task: "TASK-054",
            }
            .exit(),
            crate::exit::Exit::Usage
        );
    }
}
