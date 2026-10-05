//! The error model: stable codes, severities, and the common error type.
//!
//! Every failure this workspace reports carries the same four things, per `RULES.md` §4.3: a
//! stable machine code, a human message naming the concrete subject, a cause, and a remediation
//! hint. A caller can therefore always answer "what now?" without reading the source.
//!
//! ```text
//! AicontextError { code, message, remediation, cause }
//!        |         |        |            |         |
//!        |         |        |            |         +-- the lower-level error, never dropped
//!        |         |        |            +------------ what the caller should do
//!        |         |        +------------------------- the failure in one line, naming the subject
//!        |         +---------------------------------- stable and safe to branch on, e.g. CTX-007
//!        +------------------------------------------------ the type every crate funnels into
//! ```
//!
//! # Why one error type at the boundary, and enums below it
//!
//! `RULES.md` §4.2 asks for one enum per failure domain, and that is what every crate below this
//! one does: `ContextError`, `IndexError`, and so on. A caller in `aicontext-cli` that must handle
//! every failure from every crate cannot enumerate them, and `Box<dyn Error>` in a public
//! signature is forbidden. [`AicontextError`] is the funnel: a concrete struct, not a trait object,
//! so a `Result<T, AicontextError>` stays `Debug` and `Display` without a blanket impl. Each
//! domain enum converts into it with `From`, which is what preserves the cause chain.

use std::error::Error;
use std::fmt;

use crate::id::InvalidId;

/// The family a code belongs to, which is also its prefix.
///
/// A family is added by the task that introduces the failure domain it describes, so this crate
/// never carries codes for crates that do not exist yet. The `#[non_exhaustive]` attribute makes
/// adding one a non-breaking change for callers that match on it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ErrorFamily {
    /// A `CTX-NNN` finding from the `doctor` check catalogue, `docs/CONTEXT_SPEC.md` §8.
    Context,
    /// A `GIT-NNN` failure from invoking git, introduced by `TASK-017` in `aicontext-git`.
    ///
    /// Its own family rather than reuse of `Context`, because `CTX-NNN` is transcribed from the
    /// `doctor` check catalogue and a git failure is not a check on a document. Folding them together
    /// would mean a caller branching on `CTX-003` cannot tell whether a schema failed to validate or
    /// git refused a command, and `docs/CLI_SPEC.md` §6 promises `findings[].code` is stable.
    Git,
}

impl ErrorFamily {
    /// The prefix rendered in front of the number, for example `CTX` in `CTX-007`.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Context => "CTX",
            Self::Git => "GIT",
        }
    }
}

impl fmt::Display for ErrorFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.prefix())
    }
}

/// A stable, machine-readable code such as `CTX-007`.
///
/// The number is safe to branch on and the string is safe to match on; neither is ever reworded.
/// `docs/CLI_SPEC.md` §6 states that `findings[].code` is stable, and this type is what makes that
/// promise enforceable rather than aspirational.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ErrorCode {
    family: ErrorFamily,
    number: u16,
}

impl ErrorCode {
    /// A code in the `CTX` family, the `doctor` check catalogue in `docs/CONTEXT_SPEC.md` §8.
    ///
    /// Only the checks in that catalogue are codes. A failure with no entry there is a new
    /// failure mode, and adding it to the spec is the point: an unlisted code is a finding the
    /// tool cannot yet explain.
    #[must_use]
    pub const fn context(number: u16) -> Self {
        debug_assert!(
            number >= 1 && number <= 999,
            "a check code is CTX-001 to CTX-999; add it to CONTEXT_SPEC.md 8 if it is new"
        );
        Self {
            family: ErrorFamily::Context,
            number,
        }
    }

    /// The family this code belongs to.
    #[must_use]
    pub const fn family(self) -> ErrorFamily {
        self.family
    }

    /// The number within the family, for example `7` in `CTX-007`.
    #[must_use]
    pub const fn number(self) -> u16 {
        self.number
    }

    // The catalogue, transcribed from docs/CONTEXT_SPEC.md 8. One constant per documented check, so
    // that a typo is a compile error rather than a code nothing matches.
    /// `AI.md` is missing.
    pub const CTX_001: Self = Self::context(1);
    /// Front matter is absent or unparseable.
    pub const CTX_002: Self = Self::context(2);
    /// Schema validation failed.
    pub const CTX_003: Self = Self::context(3);
    /// An ID does not match its location, or duplicates another document.
    pub const CTX_004: Self = Self::context(4);
    /// `type` does not match the schema for that location.
    pub const CTX_005: Self = Self::context(5);
    /// An unknown front-matter key.
    pub const CTX_006: Self = Self::context(6);
    /// A reference to a document that does not exist.
    pub const CTX_007: Self = Self::context(7);
    /// A reference to a deprecated or superseded document.
    pub const CTX_008: Self = Self::context(8);
    /// An `IN_PROGRESS` task with no acceptance criteria and no waiver.
    pub const CTX_009: Self = Self::context(9);
    /// A dependency cycle in tasks or specs.
    pub const CTX_010: Self = Self::context(10);
    /// An illegal task status transition in history.
    pub const CTX_011: Self = Self::context(11);
    /// The schema copy in `.ai/schemas` differs from the source.
    pub const CTX_012: Self = Self::context(12);
    /// `ARCHITECTURE.md` contradicts the discovered project profile.
    pub const CTX_013: Self = Self::context(13);
    /// A deprecated decision has no successor.
    pub const CTX_014: Self = Self::context(14);
    /// A memory entry is older than the file it describes.
    pub const CTX_015: Self = Self::context(15);
    /// Credential-shaped content in `.ai/`.
    pub const CTX_016: Self = Self::context(16);
    /// A generated file was hand-edited.
    pub const CTX_017: Self = Self::context(17);
    /// A document is larger than the configured maximum.
    pub const CTX_018: Self = Self::context(18);
    /// The index cache is stale or missing.
    pub const CTX_019: Self = Self::context(19);
    /// A document recommends the inline-to-split switch.
    pub const CTX_020: Self = Self::context(20);

    // The GIT family, introduced by TASK-017. Unlike CTX there is no external catalogue to
    // transcribe: these are the failure modes of spawning a read-only `git`, and the set is closed
    // because the wrapper offers no operation that could fail any other way. Adding a code here means
    // adding an operation, or a failure mode that operation can hit.
    /// A code in the `GIT` family, the read-only git wrapper's failure modes.
    ///
    /// `number` is bounded to 1-999 like every other family so that a typo is a compile-time panic
    /// rather than a code that silently matches nothing at runtime.
    #[must_use]
    pub const fn git(number: u16) -> Self {
        debug_assert!(
            number >= 1 && number <= 999,
            "a git code is GIT-001 to GIT-999; add it to this impl if it is new"
        );
        Self {
            family: ErrorFamily::Git,
            number,
        }
    }

    /// `git` is not installed, or the process could not be started.
    pub const GIT_001: Self = Self::git(1);
    /// The directory is not inside a Git working tree.
    pub const GIT_002: Self = Self::git(2);
    /// A git command exited with a non-zero status.
    pub const GIT_003: Self = Self::git(3);
    /// The repository has no commits, so `HEAD` names nothing.
    pub const GIT_004: Self = Self::git(4);
    /// The git process was terminated by a signal, or killed before it could exit.
    ///
    /// This was originally "the index is held by another process", which turned out to be
    /// unreachable: with `.git/index.lock` present, `status --porcelain=v1`,
    /// `status --porcelain=v2`, `diff --name-only HEAD`, `log`, and `branch --show-current` were all
    /// measured exiting 0 on git 2.51. Every command this wrapper offers is read-only and none of
    /// them acquires the index lock, so the code could never have fired. A code with no reachable path
    /// is worse than no code, because the contract says it exists.
    pub const GIT_005: Self = Self::git(5);
    /// Git produced output this crate could not read.
    ///
    /// Two causes share this code because both mean the same thing to a caller — the wrapper cannot
    /// honestly report what the repository contains: the bytes are not UTF-8 (a filename on Unix may
    /// be any byte sequence), or the output does not have the documented porcelain v2 shape (a future
    /// git changed it). Reporting a clean working tree on unrecognised output would be a lie, which
    /// `RULES.md` §4.8 rules out, so this is an error rather than an empty result.
    pub const GIT_006: Self = Self::git(6);
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}-{:03}", self.family.prefix(), self.number)
    }
}

/// How serious a finding is.
///
/// Ordered from least to most serious, because `docs/CLI_SPEC.md` §5 requires a command that both
/// validates and acts to exit with the most severe applicable code. The `#[default]` is `Error`, not
/// `Info`: a finding that arrives without a stated severity must fail closed, or a forgotten
/// annotation would quietly downgrade a validation failure to a note.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Severity {
    /// Informational. A cache is stale, a switch to split files is suggested.
    Info,
    /// Something is wrong but the command can still complete.
    Warning,
    /// The command cannot claim success.
    #[default]
    Error,
}

impl Severity {
    /// The wire form, matching the `severity` field of the JSON envelope
    /// (`docs/CLI_SPEC.md` §6).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// The severity this finding has under `--strict`, which promotes every warning to an error
    /// (`docs/CONTEXT_SPEC.md` §8). Info is promoted to warning rather than straight to error,
    /// because `--strict` is about tolerating no warnings, not about silencing advice.
    #[must_use]
    pub const fn under_strict(self) -> Self {
        match self {
            Self::Info => Self::Warning,
            Self::Warning | Self::Error => Self::Error,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The error every crate in this workspace funnels into.
///
/// Constructed with a code, a message, and a remediation hint, because those three are the minimum
/// a caller needs to act. The cause is optional, since a validated string has no lower-level
/// failure behind it; it is never dropped once attached.
///
/// # Example
///
/// ```
/// use aicontext_core::{AicontextError, ErrorCode};
///
/// let error = AicontextError::new(
///     ErrorCode::CTX_007,
///     "TASK-014 references SPEC-auth-missing, which does not exist",
///     "create .ai/specs/auth-missing.md or fix the reference",
/// )
/// .with_source(std::io::Error::other("reference scan"));
///
/// assert_eq!(error.code(), ErrorCode::CTX_007);
/// assert!(std::error::Error::source(&error).is_some());
/// assert!(error.to_string().starts_with("CTX-007: "));
/// ```
#[derive(Debug)]
pub struct AicontextError {
    code: ErrorCode,
    message: Box<str>,
    remediation: Box<str>,
    cause: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl AicontextError {
    /// An error with everything a caller needs to act, and no cause yet.
    ///
    /// `message` must name the concrete subject — the path, the tool, the document, the identifier
    /// — because "invalid input" costs the reader a search (`RULES.md` §4.3).
    #[must_use]
    pub fn new(
        code: ErrorCode,
        message: impl Into<Box<str>>,
        remediation: impl Into<Box<str>>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            remediation: remediation.into(),
            cause: None,
        }
    }

    /// Attaches the lower-level failure, keeping the chain intact.
    ///
    /// An error with a cause is worth more than one without, so this is chainable.
    #[must_use]
    pub fn with_source(mut self, cause: impl Error + Send + Sync + 'static) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }

    /// The stable code, safe to branch on.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The human message, without the code. May be reworded between versions.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> &str {
        &self.remediation
    }

    /// The lower-level cause, if one was attached.
    #[must_use]
    pub fn cause(&self) -> Option<&(dyn Error + 'static)> {
        self.cause
            .as_ref()
            .map(|boxed| boxed.as_ref() as &(dyn Error + 'static))
    }
}

impl fmt::Display for AicontextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for AicontextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause()
    }
}

impl From<InvalidId> for AicontextError {
    fn from(error: InvalidId) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{AicontextError, ErrorCode, ErrorFamily, Severity};

    #[test]
    fn a_new_error_carries_its_code_message_and_remediation() {
        let error = AicontextError::new(
            ErrorCode::CTX_007,
            "TASK-014 references SPEC-auth-missing, which does not exist",
            "create .ai/specs/auth-missing.md or fix the reference",
        );

        assert_eq!(error.code(), ErrorCode::CTX_007);
        assert!(error.message().contains("SPEC-auth-missing"));
        assert!(error.remediation().contains(".ai/specs/auth-missing.md"));
    }

    #[test]
    fn display_leads_with_the_stable_code() {
        let error = AicontextError::new(ErrorCode::CTX_001, "AI.md missing", "run aicontext init");
        assert_eq!(error.to_string(), "CTX-001: AI.md missing");
    }

    #[test]
    fn with_source_keeps_the_whole_chain() {
        let root = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let error =
            AicontextError::new(ErrorCode::CTX_002, "cannot read TASKS.md", "check the path")
                .with_source(root);

        let source = error.source().expect("the cause must survive");
        assert!(source.to_string().contains("no such file"));

        // The chain continues below the cause, rather than stopping at the first layer.
        let below = source.source();
        assert!(below.is_none(), "io::Error has no further cause here");
    }

    #[test]
    fn an_error_without_a_cause_reports_none() {
        let error = AicontextError::new(ErrorCode::CTX_003, "schema failed", "fix the field");
        assert!(error.source().is_none());
        assert!(error.cause().is_none());
    }

    #[test]
    fn codes_render_in_the_documented_format() {
        for (code, expected) in [
            (ErrorCode::CTX_001, "CTX-001"),
            (ErrorCode::CTX_009, "CTX-009"),
            (ErrorCode::CTX_010, "CTX-010"),
            (ErrorCode::CTX_020, "CTX-020"),
        ] {
            assert_eq!(code.to_string(), expected);
        }
    }

    #[test]
    fn a_code_exposes_its_family_and_number() {
        assert_eq!(ErrorCode::CTX_007.family(), ErrorFamily::Context);
        assert_eq!(ErrorCode::CTX_007.number(), 7);
        assert_eq!(ErrorCode::CTX_007.family().prefix(), "CTX");
    }

    #[test]
    fn the_git_codes_render_in_the_documented_format_and_carry_their_own_family() {
        // TASK-017 added the GIT family. Its codes must be distinguishable from CTX at a glance,
        // because CLI_SPEC.md 6 makes `code` a stable contract that scripts branch on: a caller
        // matching on the family must be able to tell "git failed" from "a document failed".
        for (code, expected) in [
            (ErrorCode::GIT_001, "GIT-001"),
            (ErrorCode::GIT_002, "GIT-002"),
            (ErrorCode::GIT_003, "GIT-003"),
            (ErrorCode::GIT_004, "GIT-004"),
            (ErrorCode::GIT_005, "GIT-005"),
            (ErrorCode::GIT_006, "GIT-006"),
        ] {
            assert_eq!(code.to_string(), expected);
            assert_eq!(code.family(), ErrorFamily::Git);
            assert_eq!(code.family().prefix(), "GIT");
        }
    }

    #[test]
    fn the_git_family_does_not_reuse_a_context_number() {
        // The two families may each start at 001. What must not happen is GIT-007 rendering as
        // "CTX-007", which is what a shared prefix would produce. `git(7)` is used rather than a
        // constant because GIT-007 is deliberately not one: the catalogue has six entries, and a
        // seventh must be a decision rather than a stray literal.
        assert_eq!(ErrorCode::git(7).number(), ErrorCode::CTX_007.number());
        assert_ne!(
            ErrorCode::git(7).to_string(),
            ErrorCode::CTX_007.to_string()
        );
    }

    #[test]
    fn the_documented_catalogue_has_twenty_six_distinct_codes() {
        let catalogue = [
            ErrorCode::CTX_001,
            ErrorCode::CTX_002,
            ErrorCode::CTX_003,
            ErrorCode::CTX_004,
            ErrorCode::CTX_005,
            ErrorCode::CTX_006,
            ErrorCode::CTX_007,
            ErrorCode::CTX_008,
            ErrorCode::CTX_009,
            ErrorCode::CTX_010,
            ErrorCode::CTX_011,
            ErrorCode::CTX_012,
            ErrorCode::CTX_013,
            ErrorCode::CTX_014,
            ErrorCode::CTX_015,
            ErrorCode::CTX_016,
            ErrorCode::CTX_017,
            ErrorCode::CTX_018,
            ErrorCode::CTX_019,
            ErrorCode::CTX_020,
            ErrorCode::GIT_001,
            ErrorCode::GIT_002,
            ErrorCode::GIT_003,
            ErrorCode::GIT_004,
            ErrorCode::GIT_005,
            ErrorCode::GIT_006,
        ];
        let unique: std::collections::BTreeSet<String> =
            catalogue.iter().map(ErrorCode::to_string).collect();
        assert_eq!(unique.len(), catalogue.len(), "codes must be distinct");
    }

    #[test]
    fn severity_orders_from_info_through_error() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert_eq!(Severity::Warning.max(Severity::Error), Severity::Error);
    }

    #[test]
    fn a_finding_with_no_stated_severity_fails_closed() {
        assert_eq!(Severity::default(), Severity::Error);
    }

    #[test]
    fn strict_promotes_every_warning_to_an_error() {
        // CONTEXT_SPEC.md 8: --strict promotes every warning to an error.
        assert_eq!(Severity::Warning.under_strict(), Severity::Error);
        assert_eq!(Severity::Error.under_strict(), Severity::Error);
        assert_eq!(Severity::Info.under_strict(), Severity::Warning);
    }

    #[test]
    fn severity_renders_the_wire_form() {
        assert_eq!(Severity::Info.as_str(), "info");
        assert_eq!(Severity::Warning.as_str(), "warning");
        assert_eq!(Severity::Error.as_str(), "error");
        assert_eq!(Severity::Warning.to_string(), "warning");
    }

    #[test]
    fn an_invalid_id_becomes_an_aicontext_error_that_keeps_its_cause() {
        use crate::TaskId;

        let error = AicontextError::from(TaskId::new("task-014").expect_err("lowercase prefix"));

        assert_eq!(error.code(), ErrorCode::CTX_004);
        assert!(error.message().contains("task-014"));
        assert!(error.source().is_some(), "the InvalidId must be the cause");
    }
}
