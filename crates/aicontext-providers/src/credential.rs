//! Credential resolution and redaction.
//!
//! `docs/SECURITY.md` §6 and `ADR-005` fix the rules this module exists to enforce:
//!
//! * a credential is resolved in one order — an explicit flag, then an environment variable, then
//!   the OS keychain — and is never read from `.ai/`, which records only the *source*;
//! * a value never reaches `.ai/`, stdout, a log, an audit record, or an error, and redaction
//!   happens at one boundary rather than at each call site.
//!
//! [`Secret`] is the type-level half: a value that prints as `[redacted]` and can only be read back
//! by asking. This module is the other half. [`CredentialResolver`] decides *which* secret a provider
//! gets, and [`Redactor`] scrubs text that is about to be logged or turned into an error — most
//! importantly a provider's error body, which is the one string in a call that the provider, not we,
//! built.
//!
//! The two ports, [`Environment`] and [`Keychain`], are traits rather than direct calls. A test must
//! not read the developer's environment or keychain (`RULES.md` §8), and `std::env::set_var` is
//! `unsafe` in edition 2024, which `RULES.md` §3 forbids; injecting both keeps the tests hermetic and
//! safe. [`ProcessEnvironment`] and [`KeyringKeychain`] are the real implementations a command uses.

use std::borrow::Cow;
use std::fmt;

use thiserror::Error;

use crate::secret::Secret;

/// What replaces a credential in output a human or a log will read.
const REDACTED: &str = "[redacted]";

/// Where a configured provider's credential can come from.
///
/// This is the *source*, never the value: it is the shape `.ai/ai.yaml` records under `auth`
/// (`docs/AI_PROVIDER_SPEC.md` §4), and the whole point is that it can be checked into a repository.
/// The value is found by [`CredentialResolver`] at call time and lives only in a [`Secret`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum CredentialSource {
    /// The provider needs no credential, which a local server usually does not.
    #[default]
    None,
    /// A process environment variable, named by the config and read at call time.
    Environment {
        /// The variable's name — never its value.
        variable: String,
    },
    /// An entry in the OS keychain.
    Keychain {
        /// Which entry to read.
        reference: KeychainReference,
    },
    /// The environment first, then the keychain: the `env_or_keychain` type in
    /// `docs/AI_PROVIDER_SPEC.md` §4, for a machine that may or may not have the keychain unlocked.
    EnvironmentOrKeychain {
        /// The variable's name.
        variable: String,
        /// The keychain entry to fall back to.
        reference: KeychainReference,
    },
}

/// The name of an entry in the OS keychain: a service and an account.
///
/// Keychains are keyed by a service/account pair; naming both here means the same string is used to
/// store an entry (`connect`, `TASK-053`) and to read it back, which is the difference between a
/// credential that resolves and one that mysteriously does not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeychainReference {
    service: String,
    account: String,
}

impl KeychainReference {
    /// Names the entry `service` / `account`.
    #[must_use]
    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    /// The service half of the key.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The account half of the key.
    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }
}

impl fmt::Display for KeychainReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "service `{}` account `{}`",
            self.service, self.account
        )
    }
}

/// The OS keychain, as the resolver needs it.
///
/// A trait, not a call, for two reasons: a test must not touch the developer's real keychain
/// (`RULES.md` §8), so a fake is injected; and `connect` (`TASK-053`) writes through the same seam
/// that resolution reads through. [`KeyringKeychain`] is the implementation a command uses.
pub trait Keychain: fmt::Debug {
    /// Reads the entry, or `None` when the keychain holds nothing under that reference.
    ///
    /// # Errors
    ///
    /// Returns a [`KeychainError`] when the keychain itself could not be reached, not when it simply
    /// has no such entry.
    fn get(&self, reference: &KeychainReference) -> Result<Option<Secret>, KeychainError>;

    /// Stores `secret` under `reference`, replacing any existing value.
    ///
    /// # Errors
    ///
    /// Returns a [`KeychainError`] when the entry could not be written.
    fn set(&self, reference: &KeychainReference, secret: &Secret) -> Result<(), KeychainError>;

    /// Removes the entry, answering whether one was there.
    ///
    /// # Errors
    ///
    /// Returns a [`KeychainError`] when the delete could not be attempted.
    fn delete(&self, reference: &KeychainReference) -> Result<bool, KeychainError>;
}

/// A failure reported by the OS keychain itself.
///
/// Deliberately coarse, and deliberately without a `#[source]`: a platform error's text can echo the
/// entry it was looking up, and `RULES.md` §4.11 makes "no secret in any output" absolute. A short
/// variant that cannot replay provider text is worth more than a long one that might.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[non_exhaustive]
pub enum KeychainError {
    /// The keychain refused: access was denied, or it is locked.
    #[error("the OS keychain denied access")]
    Denied,
    /// No keychain is available.
    #[error("the OS keychain is unavailable")]
    Unavailable,
    /// The keychain reported a failure we do not have a better name for.
    #[error("the OS keychain reported an unexpected failure")]
    Failed,
}

/// The environment, as the resolver needs it.
///
/// A trait, not `std::env::var`, because a test must not depend on the developer's environment, and
/// because `std::env::set_var` is `unsafe` in edition 2024 (`RULES.md` §3), so a test cannot fake a
/// variable in place. [`ProcessEnvironment`] is the real implementation.
pub trait Environment: fmt::Debug {
    /// The value of `name`, or `None` when the variable is unset *or empty*.
    ///
    /// An empty variable is treated as absent: `export AICON_AI_HOSTED_KEY=` is a mistake far more
    /// often than it is a credential.
    fn variable(&self, name: &str) -> Option<Secret>;
}

/// The real process environment, which is what a command actually reads.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn variable(&self, name: &str) -> Option<Secret> {
        std::env::var(name)
            .ok()
            .filter(|value| !value.is_empty())
            .map(Secret::new)
    }
}

/// Resolves the one credential a provider call needs, in the order `docs/SECURITY.md` §6 fixes:
/// explicit flag, then environment, then the OS keychain.
///
/// The resolver borrows the two ports rather than owning them, because it does no work beyond a
/// call: it is built at the call site, used once, and dropped.
#[derive(Debug)]
pub struct CredentialResolver<'a> {
    environment: &'a dyn Environment,
    keychain: &'a dyn Keychain,
}

impl<'a> CredentialResolver<'a> {
    /// Binds the two ports.
    #[must_use]
    pub fn new(environment: &'a dyn Environment, keychain: &'a dyn Keychain) -> Self {
        Self {
            environment,
            keychain,
        }
    }

    /// Resolves `source`, letting an explicit value (a CLI flag) win.
    ///
    /// `Ok(None)` means the provider needs no credential; an `Err` means one was named and could not
    /// be found, which the caller must treat as a failure rather than as "no credential"
    /// (`RULES.md` §4.10 — no silent fallbacks).
    ///
    /// # Errors
    ///
    /// [`CredentialError::EnvironmentMissing`] when a named variable is unset or empty;
    /// [`CredentialError::KeychainMissing`] when the keychain holds no such entry; and
    /// [`CredentialError::KeychainUnavailable`] when the keychain could not be read at all.
    pub fn resolve(
        &self,
        source: &CredentialSource,
        explicit: Option<&Secret>,
    ) -> Result<Option<Secret>, CredentialError> {
        if let Some(explicit) = explicit.filter(|secret| !secret.is_empty()) {
            return Ok(Some(explicit.clone()));
        }
        match source {
            CredentialSource::None => Ok(None),
            CredentialSource::Environment { variable } => {
                Ok(Some(self.read_environment(variable)?))
            }
            CredentialSource::Keychain { reference } => Ok(Some(self.read_keychain(reference)?)),
            CredentialSource::EnvironmentOrKeychain {
                variable,
                reference,
            } => match self.environment.variable(variable) {
                Some(secret) => Ok(Some(secret)),
                None => Ok(Some(self.read_keychain(reference)?)),
            },
        }
    }

    /// Reads a named variable, or reports it by name.
    fn read_environment(&self, variable: &str) -> Result<Secret, CredentialError> {
        self.environment
            .variable(variable)
            .ok_or_else(|| CredentialError::EnvironmentMissing {
                variable: variable.to_owned(),
            })
    }

    /// Reads a keychain entry, or reports it by reference.
    fn read_keychain(&self, reference: &KeychainReference) -> Result<Secret, CredentialError> {
        self.keychain
            .get(reference)
            .map_err(|reason| CredentialError::KeychainUnavailable {
                reference: reference.clone(),
                reason,
            })?
            .ok_or_else(|| CredentialError::KeychainMissing {
                reference: reference.clone(),
            })
    }
}

/// A credential that was named but could not be resolved.
///
/// Every variant names the *source* — a variable or a keychain entry — and no variant can carry a
/// value, so this error is safe to print (`docs/SECURITY.md` §3, I5).
#[derive(Debug, Eq, PartialEq, Error)]
#[non_exhaustive]
pub enum CredentialError {
    /// A named environment variable is unset or empty.
    #[error("the environment variable `{variable}` is unset or empty")]
    EnvironmentMissing {
        /// The variable the config named.
        variable: String,
    },
    /// The keychain holds no entry under the named reference.
    #[error("the OS keychain has no entry for {reference}")]
    KeychainMissing {
        /// The entry the config named.
        reference: KeychainReference,
    },
    /// The keychain could not be read.
    #[error("the OS keychain could not be read for {reference}: {reason}")]
    KeychainUnavailable {
        /// The entry the config named.
        reference: KeychainReference,
        /// What the keychain reported, in its coarse form.
        reason: KeychainError,
    },
}

impl CredentialError {
    /// The stable machine code, in the shape `RULES.md` §4.3 requires.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::EnvironmentMissing { .. } => "CREDENTIAL_ENVIRONMENT_MISSING",
            Self::KeychainMissing { .. } => "CREDENTIAL_KEYCHAIN_MISSING",
            Self::KeychainUnavailable { .. } => "CREDENTIAL_KEYCHAIN_UNAVAILABLE",
        }
    }

    /// What the user can do about it.
    #[must_use]
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::EnvironmentMissing { .. } => {
                "set the variable, or run `aicontext connect` to store the credential in the OS \
                 keychain"
            }
            Self::KeychainMissing { .. } => {
                "run `aicontext connect` to store the credential in the OS keychain"
            }
            Self::KeychainUnavailable { .. } => {
                "unlock the OS keychain (or make it available), then retry"
            }
        }
    }
}

/// Redacts credentials from text on its way to a log, a diagnostic, or an error message.
///
/// `docs/SECURITY.md` §6 asks for redaction "at the logging boundary, not ad hoc", so there is one
/// type that does it and a caller either holds one or does not log. It works two ways, by key name
/// and by value shape, because the two catch different mistakes: [`Redactor::redact_field`] drops a
/// value whose *field* is credential-shaped (a structured log), and [`Redactor::redact`] scrubs a
/// string that merely *contains* a credential (a provider's error body).
///
/// Values it was told about are matched exactly. Values it was not are caught by shape, for the
/// well-known credential prefixes, as defense in depth.
#[derive(Clone, Debug, Default)]
pub struct Redactor {
    values: Vec<Secret>,
}

impl Redactor {
    /// A redactor that knows no values yet; it still masks credential-shaped text.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a value to redact exactly wherever it appears.
    #[must_use]
    pub fn with_secret(mut self, secret: Secret) -> Self {
        self.values.push(secret);
        self
    }

    /// Scrubs every known value and every credential-shaped token from `text`.
    ///
    /// Returns a borrow when there was nothing to redact, so the common case allocates nothing.
    #[must_use]
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if let Some(masked) = self.mask_known(text) {
            return Cow::Owned(masked);
        }
        match mask_patterns(text) {
            Some(masked) => Cow::Owned(masked),
            None => Cow::Borrowed(text),
        }
    }

    /// Scrubs a value read under `key`, dropping it outright when the key is credential-shaped.
    ///
    /// Returns a borrow when the value needed no change.
    #[must_use]
    pub fn redact_field<'a>(&self, key: &str, value: &'a str) -> Cow<'a, str> {
        if is_credential_key(key) {
            Cow::Borrowed(REDACTED)
        } else {
            self.redact(value)
        }
    }

    /// Replaces each known value wherever it appears.
    fn mask_known(&self, text: &str) -> Option<String> {
        let mut masked: Option<String> = None;
        for secret in &self.values {
            let value = secret.expose();
            if value.is_empty() {
                continue;
            }
            let haystack = masked.as_deref().unwrap_or(text);
            if haystack.contains(value) {
                masked = Some(haystack.replace(value, REDACTED));
            }
        }
        masked
    }
}

/// Replaces every credential-shaped token in `text`.
fn mask_patterns(text: &str) -> Option<String> {
    let mut output = String::with_capacity(text.len());
    let mut changed = false;
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(length) = prefixed_token(rest) {
            output.push_str(REDACTED);
            rest = &rest[length..];
            changed = true;
        } else if let Some(length) = bearer_token(rest) {
            output.push_str("Bearer ");
            output.push_str(REDACTED);
            rest = &rest[length..];
            changed = true;
        } else {
            let character = rest.chars().next().expect("`rest` is not empty");
            output.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    changed.then_some(output)
}

/// The byte length of a credential with a well-known prefix at the start of `text`, if any.
fn prefixed_token(text: &str) -> Option<usize> {
    /// Prefixes that open a credential. Every entry is ASCII, so a byte match is a character match.
    const PREFIXES: &[&str] = &[
        "sk-ant-",
        "sk-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxr-",
        "xoxs-",
        "AKIA",
        "ASIA",
    ];
    /// The shortest body that is a credential rather than prose that happens to start with a prefix.
    const MIN_BODY: usize = 8;
    PREFIXES.iter().find_map(|prefix| {
        let tail = text.strip_prefix(prefix)?;
        let body = tail.bytes().take_while(|byte| is_token_byte(*byte)).count();
        (body >= MIN_BODY).then_some(prefix.len() + body)
    })
}

/// The byte length of a `Bearer <token>` sequence at the start of `text`, if any.
fn bearer_token(text: &str) -> Option<usize> {
    /// The scheme an `Authorization` header uses.
    const PREFIX: &str = "Bearer ";
    /// The shortest token worth masking.
    const MIN_BODY: usize = 8;
    let tail = text.strip_prefix(PREFIX)?;
    let body = tail
        .bytes()
        .take_while(|byte| is_token_byte(*byte) || *byte == b'.')
        .count();
    (body >= MIN_BODY).then_some(PREFIX.len() + body)
}

/// Whether `byte` can appear in a credential.
fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// Whether a field name is credential-shaped, so its value must not be logged.
fn is_credential_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "authorization"
            | "api_key"
            | "apikey"
            | "api-key"
            | "password"
            | "passwd"
            | "secret"
            | "credential"
    ) || [
        "_token",
        "_secret",
        "_password",
        "_passwd",
        "_credential",
        "_api_key",
        "_access_key",
        "_private_key",
        "_secret_key",
    ]
    .iter()
    .any(|suffix| key.ends_with(suffix))
}

/// The OS keychain, reached through the `keyring` crate.
///
/// Each platform's native credential store is the backend — the Windows Credential Manager, the macOS
/// Keychain, or the Linux kernel keyring — so no secret is ever written to a file of our own design.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringKeychain;

impl Keychain for KeyringKeychain {
    fn get(&self, reference: &KeychainReference) -> Result<Option<Secret>, KeychainError> {
        match entry_for(reference)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(error) if is_no_entry(&error) => Ok(None),
            Err(error) => Err(classify_error(&error)),
        }
    }

    fn set(&self, reference: &KeychainReference, secret: &Secret) -> Result<(), KeychainError> {
        entry_for(reference)?
            .set_password(secret.expose())
            .map_err(|error| classify_error(&error))
    }

    fn delete(&self, reference: &KeychainReference) -> Result<bool, KeychainError> {
        match entry_for(reference)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(error) if is_no_entry(&error) => Ok(false),
            Err(error) => Err(classify_error(&error)),
        }
    }
}

/// Opens the entry, mapping a construction failure like any other keychain failure.
fn entry_for(reference: &KeychainReference) -> Result<keyring::Entry, KeychainError> {
    keyring::Entry::new(reference.service(), reference.account())
        .map_err(|error| classify_error(&error))
}

/// Whether a `keyring` error means "there is no such entry" rather than "something went wrong".
fn is_no_entry(error: &keyring::Error) -> bool {
    matches!(error, keyring::Error::NoEntry)
}

/// Maps a `keyring` error onto our coarse one, dropping its text on purpose: the platform message
/// can name the entry it was reading, and I5 makes "no secret in any output" absolute.
fn classify_error(error: &keyring::Error) -> KeychainError {
    match error {
        keyring::Error::NoStorageAccess(_) => KeychainError::Denied,
        _ => KeychainError::Failed,
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::{
        CredentialError, CredentialResolver, CredentialSource, Environment, Keychain,
        KeychainError, KeychainReference, Redactor,
    };
    use crate::secret::Secret;

    const SECRET: &str = "sk-live-abcdefghijklmnop";

    /// An environment that is exactly what the test says it is.
    #[derive(Debug, Default)]
    struct FakeEnvironment {
        variables: BTreeMap<String, String>,
    }

    impl FakeEnvironment {
        fn with(pairs: &[(&str, &str)]) -> Self {
            Self {
                variables: pairs
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
            }
        }
    }

    impl Environment for FakeEnvironment {
        fn variable(&self, name: &str) -> Option<Secret> {
            self.variables
                .get(name)
                .filter(|value| !value.is_empty())
                .cloned()
                .map(Secret::new)
        }
    }

    /// A keychain whose contents the test controls.
    #[derive(Debug, Default)]
    struct FakeKeychain {
        entries: RefCell<BTreeMap<(String, String), String>>,
        broken: bool,
    }

    impl FakeKeychain {
        fn with(pairs: &[(&str, &str, &str)]) -> Self {
            let keychain = Self::default();
            for (service, account, value) in pairs {
                keychain.entries.borrow_mut().insert(
                    ((*service).to_owned(), (*account).to_owned()),
                    (*value).to_owned(),
                );
            }
            keychain
        }

        fn broken() -> Self {
            Self {
                entries: RefCell::new(BTreeMap::new()),
                broken: true,
            }
        }

        fn key(reference: &KeychainReference) -> (String, String) {
            (
                reference.service().to_owned(),
                reference.account().to_owned(),
            )
        }
    }

    impl Keychain for FakeKeychain {
        fn get(&self, reference: &KeychainReference) -> Result<Option<Secret>, KeychainError> {
            if self.broken {
                return Err(KeychainError::Unavailable);
            }
            Ok(self
                .entries
                .borrow()
                .get(&Self::key(reference))
                .cloned()
                .map(Secret::new))
        }

        fn set(&self, reference: &KeychainReference, secret: &Secret) -> Result<(), KeychainError> {
            if self.broken {
                return Err(KeychainError::Unavailable);
            }
            self.entries
                .borrow_mut()
                .insert(Self::key(reference), secret.expose().to_owned());
            Ok(())
        }

        fn delete(&self, reference: &KeychainReference) -> Result<bool, KeychainError> {
            if self.broken {
                return Err(KeychainError::Unavailable);
            }
            Ok(self
                .entries
                .borrow_mut()
                .remove(&Self::key(reference))
                .is_some())
        }
    }

    /// Resolves through the two fakes, so no test reads the real environment or keychain.
    fn resolve(
        environment: &FakeEnvironment,
        keychain: &FakeKeychain,
        source: &CredentialSource,
        explicit: Option<&Secret>,
    ) -> Result<Option<Secret>, CredentialError> {
        CredentialResolver::new(environment, keychain).resolve(source, explicit)
    }

    fn keychain_source() -> CredentialSource {
        CredentialSource::Keychain {
            reference: KeychainReference::new("aicontext", "hosted"),
        }
    }

    fn environment_or_keychain_source() -> CredentialSource {
        CredentialSource::EnvironmentOrKeychain {
            variable: "AICON_AI_HOSTED_KEY".to_owned(),
            reference: KeychainReference::new("aicontext", "hosted"),
        }
    }

    #[test]
    fn an_explicit_flag_wins_over_the_environment_and_the_keychain() {
        let environment = FakeEnvironment::with(&[("AICON_AI_HOSTED_KEY", "from-the-environment")]);
        let keychain = FakeKeychain::with(&[("aicontext", "hosted", "from-the-keychain")]);
        let explicit = Secret::new(SECRET);

        let resolved = resolve(
            &environment,
            &keychain,
            &environment_or_keychain_source(),
            Some(&explicit),
        )
        .expect("an explicit flag resolves")
        .expect("a credential");

        assert_eq!(resolved.expose(), SECRET);
    }

    #[test]
    fn the_environment_precedes_the_keychain() {
        let environment = FakeEnvironment::with(&[("AICON_AI_HOSTED_KEY", "from-the-environment")]);
        let keychain = FakeKeychain::with(&[("aicontext", "hosted", "from-the-keychain")]);

        let resolved = resolve(
            &environment,
            &keychain,
            &environment_or_keychain_source(),
            None,
        )
        .expect("the environment resolves")
        .expect("a credential");

        assert_eq!(resolved.expose(), "from-the-environment");
    }

    #[test]
    fn the_keychain_is_the_last_resort() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::with(&[("aicontext", "hosted", "from-the-keychain")]);

        let resolved = resolve(
            &environment,
            &keychain,
            &environment_or_keychain_source(),
            None,
        )
        .expect("the keychain resolves")
        .expect("a credential");

        assert_eq!(resolved.expose(), "from-the-keychain");
    }

    #[test]
    fn an_explicit_keychain_source_reads_the_keychain() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::with(&[("aicontext", "hosted", "from-the-keychain")]);

        let resolved = resolve(&environment, &keychain, &keychain_source(), None)
            .expect("the keychain resolves")
            .expect("a credential");

        assert_eq!(resolved.expose(), "from-the-keychain");
    }

    #[test]
    fn an_empty_explicit_flag_falls_through() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::with(&[("aicontext", "hosted", "from-the-keychain")]);
        let explicit = Secret::from("");

        let resolved = resolve(
            &environment,
            &keychain,
            &environment_or_keychain_source(),
            Some(&explicit),
        )
        .expect("the fallback resolves")
        .expect("a credential");

        assert_eq!(resolved.expose(), "from-the-keychain");
    }

    #[test]
    fn an_empty_environment_variable_is_absent() {
        let environment = FakeEnvironment::with(&[("AICON_AI_HOSTED_KEY", "")]);
        let keychain = FakeKeychain::with(&[("aicontext", "hosted", "from-the-keychain")]);

        let resolved = resolve(
            &environment,
            &keychain,
            &environment_or_keychain_source(),
            None,
        )
        .expect("the fallback resolves")
        .expect("a credential");

        assert_eq!(resolved.expose(), "from-the-keychain");
    }

    #[test]
    fn a_none_source_needs_no_credential() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();

        let resolved = resolve(&environment, &keychain, &CredentialSource::None, None)
            .expect("no credential is not a failure");

        assert!(resolved.is_none());
    }

    #[test]
    fn a_missing_environment_variable_names_the_variable() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();
        let source = CredentialSource::Environment {
            variable: "AICON_AI_HOSTED_KEY".to_owned(),
        };

        let error = resolve(&environment, &keychain, &source, None).expect_err("nothing to read");

        assert_eq!(error.code(), "CREDENTIAL_ENVIRONMENT_MISSING");
        assert!(error.to_string().contains("AICON_AI_HOSTED_KEY"));
        assert!(!error.remediation().is_empty());
    }

    #[test]
    fn a_missing_keychain_entry_names_the_reference() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::default();

        let error = resolve(&environment, &keychain, &keychain_source(), None)
            .expect_err("nothing to read");

        assert_eq!(error.code(), "CREDENTIAL_KEYCHAIN_MISSING");
        let rendered = error.to_string();
        assert!(rendered.contains("aicontext"));
        assert!(rendered.contains("hosted"));
    }

    #[test]
    fn an_unavailable_keychain_is_reported() {
        let environment = FakeEnvironment::default();
        let keychain = FakeKeychain::broken();

        let error = resolve(&environment, &keychain, &keychain_source(), None)
            .expect_err("nothing to read");

        assert_eq!(error.code(), "CREDENTIAL_KEYCHAIN_UNAVAILABLE");
    }

    #[test]
    fn a_keychain_reference_reports_both_halves() {
        let reference = KeychainReference::new("aicontext", "hosted");
        assert_eq!(reference.service(), "aicontext");
        assert_eq!(reference.account(), "hosted");
        assert_eq!(
            reference.to_string(),
            "service `aicontext` account `hosted`"
        );
    }

    #[test]
    fn a_known_value_is_redacted_wherever_it_appears() {
        let redactor = Redactor::new().with_secret(Secret::new(SECRET));
        let leaked = format!("the server said: {SECRET} because the key was {SECRET}");

        assert_eq!(
            redactor.redact(&leaked),
            "the server said: [redacted] because the key was [redacted]"
        );
    }

    #[test]
    fn a_credential_shaped_token_is_redacted_without_being_known() {
        let redactor = Redactor::new();
        let leaked =
            "ignore this: sk-abcdefghijklmnop and ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

        let scrubbed = redactor.redact(leaked);

        assert!(!scrubbed.contains("sk-abcdefghijklmnop"), "{scrubbed}");
        assert!(!scrubbed.contains("ghp_"), "{scrubbed}");
        assert!(
            scrubbed.contains("ignore this: [redacted] and [redacted]"),
            "{scrubbed}"
        );
    }

    #[test]
    fn a_bearer_token_is_redacted() {
        let redactor = Redactor::new();
        let leaked = "authorization: Bearer abcdefghijklmnop";

        let scrubbed = redactor.redact(leaked);

        assert_eq!(scrubbed, "authorization: Bearer [redacted]");
    }

    #[test]
    fn a_credential_field_is_redacted_by_name() {
        let redactor = Redactor::new();

        assert_eq!(
            redactor.redact_field("authorization", "Bearer abc"),
            "[redacted]"
        );
        assert_eq!(redactor.redact_field("api_key", "whatever"), "[redacted]");
        assert_eq!(
            redactor.redact_field("access_token", "opaque"),
            "[redacted]"
        );
    }

    #[test]
    fn a_field_that_is_not_a_credential_passes_through() {
        let redactor = Redactor::new();

        assert_eq!(redactor.redact_field("max_tokens", "137"), "137");
        assert_eq!(
            redactor.redact_field("model", "qwen2.5-coder"),
            "qwen2.5-coder"
        );
        assert_eq!(redactor.redact_field("idempotency_key", "abc"), "abc");
    }

    #[test]
    fn text_without_a_credential_is_borrowed_unchanged() {
        let redactor = Redactor::new();
        assert!(matches!(
            redactor.redact("nothing to see"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(
            redactor.redact_field("model", "qwen2.5-coder"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn no_credential_in_any_output() {
        let reference = KeychainReference::new("aicontext", "hosted");
        let errors = [
            CredentialError::EnvironmentMissing {
                variable: "AICON_AI_HOSTED_KEY".to_owned(),
            },
            CredentialError::KeychainMissing {
                reference: reference.clone(),
            },
            CredentialError::KeychainUnavailable {
                reference,
                reason: KeychainError::Denied,
            },
        ];
        for error in &errors {
            for rendered in [
                format!("{error}"),
                format!("{error:?}"),
                error.code().to_owned(),
                error.remediation().to_owned(),
            ] {
                assert!(
                    !rendered.contains(SECRET),
                    "a credential leaked: {rendered}"
                );
            }
        }
        for error in [
            KeychainError::Denied,
            KeychainError::Unavailable,
            KeychainError::Failed,
        ] {
            for rendered in [format!("{error}"), format!("{error:?}")] {
                assert!(
                    !rendered.contains(SECRET),
                    "a credential leaked: {rendered}"
                );
            }
        }
        let redactor = Redactor::new().with_secret(Secret::new(SECRET));
        let rendered = format!("{redactor:?}");
        assert!(
            !rendered.contains(SECRET),
            "a credential leaked: {rendered}"
        );
    }
}
