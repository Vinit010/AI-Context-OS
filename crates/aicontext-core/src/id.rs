//! Entity identifiers.
//!
//! Every document in `.ai/` carries a stable ID, and every reference between documents is an ID
//! rather than a path so that renaming a file cannot break a link (`CONVENTIONS.md` §4).
//!
//! | Entity | Type | Form |
//! |--------|------|------|
//! | Task | [`TaskId`] | `TASK-NNN` |
//! | Decision | [`DecisionId`] | `ADR-NNN` |
//! | Bug | [`BugId`] | `BUG-NNN` |
//! | Change | [`ChangeId`] | `CHG-NNN` |
//! | Any document | [`DocumentId`] | `PREFIX-suffix`, e.g. `SPEC-context`, `TASK-014` |
//!
//! # Why the types are distinct
//!
//! A task ID and a bug ID are both `PREFIX-NNN`, and it is tempting to pass either as a `String`.
//! The compiler cannot then stop `doctor` reporting a bug where it expected a task, which is
//! exactly the class of bug `CTX-004` exists to catch. Each kind gets its own type; [`DocumentId`]
//! is the one deliberate exception, for the places that genuinely handle any document.
//!
//! # Stability
//!
//! IDs are stable and are never reused, even after a document is deleted or cancelled
//! (`CONVENTIONS.md` §4). Nothing in this module allocates on `as_str`, and no method renames or
//! normalises an ID: the string that was validated is the string that is reported.

use std::borrow::Borrow;
use std::fmt;
use std::str::FromStr;

/// Declares an identifier type for a fixed `PREFIX-NNN` entity kind.
///
/// The four kinds below share one grammar, one set of operations, and one set of failure modes, so
/// they share one declaration. An abstraction needs a second use case to be justified, and a fifth
/// copy of this code is not it.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// The prefix every ID of this kind starts with, for example `TASK`.
            pub const PREFIX: &'static str = $prefix;

            /// Validates `raw` and returns the identifier.
            ///
            /// # Errors
            ///
            /// Returns [`InvalidId`] if `raw` is not `PREFIX` followed by `-` and one or more
            /// ASCII digits. Surrounding whitespace, lowercase, and an empty suffix are all
            /// rejected rather than trimmed, because a reference that does not match its document
            /// exactly is a finding, not something to repair silently.
            pub fn new(raw: &str) -> Result<Self, InvalidId> {
                if is_numbered_id(raw, Self::PREFIX) {
                    Ok(Self(raw.to_owned()))
                } else {
                    Err(InvalidId::new(Self::PREFIX, raw))
                }
            }

            /// The identifier exactly as written, including its prefix.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// The number after the prefix, for display and ordering.
            ///
            /// Returns `None` only if the value was built by a future version of this crate that
            /// added a non-numeric suffix kind; that cannot happen for the current types.
            #[must_use]
            pub fn number(&self) -> Option<u32> {
                self.0
                    .strip_prefix(Self::PREFIX)
                    .and_then(|rest| rest.strip_prefix('-'))
                    .and_then(|digits| digits.parse().ok())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                Self::new(raw)
            }
        }
    };
}

define_id! {
    /// A task in `.ai/TASKS.md` or `.ai/tasks/TASK-NNN-<slug>.md`, for example `TASK-014`.
    ///
    /// Numbering is zero-padded to three digits by convention. `TASK-14` and `TASK-014` are both
    /// accepted, because rejecting an unpadded number would reject a human's own convention rather
    /// than a mistake, and `doctor` normalises the display instead.
    TaskId,
    "TASK"
}

define_id! {
    /// An architecture decision record, for example `ADR-001`.
    ///
    /// The prefix is `ADR`, not `DEC`, so that a decision reads the same in prose as it does in its
    /// filename and in Git history.
    DecisionId,
    "ADR"
}

define_id! {
    /// A recorded bug, for example `BUG-002`.
    BugId,
    "BUG"
}

define_id! {
    /// A recorded change, for example `CHG-004`.
    ChangeId,
    "CHG"
}

/// The ID of any document under `.ai/`, whatever its kind.
///
/// This is the union of every ID form in `CONTEXT_SPEC.md` §1: a prefix of two to eight uppercase
/// ASCII letters, a `-`, and then a suffix. What the suffix may be depends on the prefix, because
/// the spec is not uniform — specifications, context notes, workflows, and agent profiles are named
/// (`SPEC-context`, `CTX-release-process`), while tasks, decisions, bugs, and changes are numbered
/// (`TASK-014`). Enforcing that is what stops `TASK-014-extra` from parsing as a task.
///
/// Placeholders in the spec table, such as `ARCH-NNN` and `MEMORY-REG`, are therefore rejected:
/// they are documentation shorthand, not IDs. Real files use `ARCH-001` and `MEMORY-001`.
///
/// Use a [`TaskId`], [`DecisionId`], [`BugId`], or [`ChangeId`] whenever the kind *is* known. The
/// conversions are lossless and free.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DocumentId(String);

impl DocumentId {
    /// Validates `raw` and returns the identifier.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidId`] if `raw` is not an uppercase prefix of 2 to 8 letters, a `-`, and a
    /// non-empty suffix that is either all digits or a lowercase kebab-case slug.
    pub fn new(raw: &str) -> Result<Self, InvalidId> {
        if is_document_id(raw) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(InvalidId::new("<any>", raw))
        }
    }

    /// The identifier exactly as written, including its prefix.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The prefix, for example `TASK` in `TASK-014`.
    #[must_use]
    pub fn prefix(&self) -> &str {
        self.0.split_once('-').map_or("", |(prefix, _)| prefix)
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for DocumentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for DocumentId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for DocumentId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for DocumentId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl FromStr for DocumentId {
    type Err = InvalidId;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::new(raw)
    }
}

macro_rules! impl_from_typed_id {
    ($($name:ident),* $(,)?) => {
        $(
            impl From<$name> for DocumentId {
                fn from(id: $name) -> Self {
                    Self(id.as_str().to_owned())
                }
            }
        )*
    };
}

impl_from_typed_id!(TaskId, DecisionId, BugId, ChangeId);

/// Why an identifier was rejected, and what a valid one looks like.
///
/// A failure to validate is not a bug in the tool and not a lower-level I/O failure, so this type
/// has no `source`: the subject of the failure *is* the input string. It carries the same four
/// things every error in this workspace carries, per `RULES.md` §4.3: a stable code, a message
/// naming the concrete subject, and a remediation hint.
#[derive(Debug, thiserror::Error)]
#[error("`{input}` is not a valid {prefix} identifier: {reason}")]
pub struct InvalidId {
    prefix: &'static str,
    input: String,
    reason: &'static str,
}

impl InvalidId {
    fn new(prefix: &'static str, input: &str) -> Self {
        let reason = if input.is_empty() {
            "the value is empty"
        } else if input.starts_with(char::is_lowercase) {
            "the prefix must be uppercase"
        } else if input.trim() != input {
            "surrounding whitespace is not trimmed"
        } else if prefix == "<any>" {
            "expected PREFIX-suffix, where PREFIX is 2 to 8 uppercase letters and suffix is a \
             number or a lowercase slug"
        } else {
            "expected the prefix, a `-`, and one or more digits"
        };
        Self {
            prefix,
            input: input.to_owned(),
            reason,
        }
    }

    /// The stable code for this failure, `CTX-004` in `docs/CONTEXT_SPEC.md` §8.
    #[must_use]
    pub fn code(&self) -> crate::ErrorCode {
        crate::ErrorCode::CTX_004
    }

    /// The value that was rejected.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> &'static str {
        "use the ID exactly as written in the document's front matter; see CONVENTIONS.md 4 for \
         the form of each entity ID"
    }
}

/// `PREFIX-<digits>`, with the prefix matched case-sensitively.
fn is_numbered_id(value: &str, prefix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// The prefixes whose IDs end in a lowercase slug rather than a number.
///
/// From the `ID` column of `docs/CONTEXT_SPEC.md` §1: specifications, context notes, workflows, and
/// agent profiles are named; every other entity is numbered. This list is the whole of the spec's
/// asymmetry, and it is what stops `TASK-014-extra` from parsing as a task.
const SLUG_PREFIXES: [&str; 4] = ["SPEC", "CTX", "WF", "AGENT"];

/// `PREFIX-suffix`, where the prefix is 2 to 8 uppercase ASCII letters, and the suffix is a number
/// unless [`SLUG_PREFIXES`] says the prefix takes a slug.
fn is_document_id(value: &str) -> bool {
    let Some((prefix, suffix)) = value.split_once('-') else {
        return false;
    };
    let prefix_len = prefix.len();
    if !(2..=8).contains(&prefix_len)
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_uppercase() && b.is_ascii_alphabetic())
    {
        return false;
    }
    if is_digits(suffix) {
        return true;
    }
    SLUG_PREFIXES.contains(&prefix) && is_slug(suffix)
}

fn is_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::{
        BugId, ChangeId, DecisionId, DocumentId, InvalidId, TaskId, is_document_id, is_slug,
    };

    #[test]
    fn accepts_the_documented_id_forms() {
        let cases: [(&str, &str); 4] = [
            ("TASK-014", TaskId::PREFIX),
            ("ADR-001", DecisionId::PREFIX),
            ("BUG-002", BugId::PREFIX),
            ("CHG-004", ChangeId::PREFIX),
        ];
        for (raw, prefix) in cases {
            let (actual_prefix, _) = raw.split_once('-').expect("a prefix");
            assert_eq!(actual_prefix, prefix, "wrong prefix constant for {raw}");
        }

        assert_eq!(TaskId::new("TASK-014").expect("valid").as_str(), "TASK-014");
    }

    #[test]
    fn rejects_anything_that_is_not_prefix_dash_digits() {
        for raw in [
            "TASK",           // no suffix
            "TASK-",          // empty suffix
            "TASK-abc",       // not numeric
            "TASK-014-extra", // trailing junk
            "task-014",       // lowercase prefix
            "TASK-014x",      // trailing junk
            " TASK-014",      // leading whitespace
            "TASK-014 ",      // trailing whitespace
            "",               // empty
            "TASK_014",       // wrong separator
            "BUG-002",        // wrong prefix for TaskId
        ] {
            assert!(
                TaskId::new(raw).is_err(),
                "`{raw}` should not be a valid TaskId"
            );
        }
    }

    #[test]
    fn exposes_the_number_without_reallocating() {
        assert_eq!(TaskId::new("TASK-014").expect("valid").number(), Some(14));
        assert_eq!(BugId::new("BUG-7").expect("valid").number(), Some(7));
    }

    #[test]
    fn compares_against_a_plain_string() {
        let id = TaskId::new("TASK-014").expect("valid");
        assert!(id == "TASK-014");
        assert_eq!(id.to_string(), "TASK-014");
        assert_eq!(id.as_ref(), "TASK-014");
    }

    #[test]
    fn parses_through_the_str_trait() {
        let id: TaskId = "TASK-014".parse().expect("valid");
        assert_eq!(id.as_str(), "TASK-014");
    }

    #[test]
    fn converts_to_a_document_id_losslessly() {
        let id: DocumentId = TaskId::new("TASK-014").expect("valid").into();
        assert_eq!(id.as_str(), "TASK-014");
        assert_eq!(id.prefix(), "TASK");
    }

    #[test]
    fn accepts_every_entity_id_shape_in_the_spec() {
        for raw in [
            "PRD-001",
            "ARCH-001",
            "RULES-001",
            "CONV-001",
            "DESIGN-001",
            "TASKS-001",
            "MEMORY-001",
            "SPEC-context",
            "SPEC-auth-missing",
            "TASK-014",
            "ADR-001",
            "BUG-002",
            "CHG-004",
            "CTX-release-process",
            "WF-release",
            "AGENT-planner",
        ] {
            assert!(is_document_id(raw), "`{raw}` should be a valid DocumentId");
            assert!(DocumentId::new(raw).is_ok(), "`{raw}` should parse");
        }
    }

    #[test]
    fn a_numbered_prefix_refuses_a_slug_suffix() {
        // The spec is not uniform: only SPEC, CTX, WF, and AGENT are named. Letting every prefix
        // take a slug would make `TASK-014-extra` parse, and a task ID is a task ID.
        assert!(!is_document_id("TASK-014-extra"));
        assert!(!is_document_id("ADR-001-final"));
        assert!(!is_document_id("BUG-2-fatal"));
    }

    #[test]
    fn rejects_the_spec_placeholders_that_are_not_real_ids() {
        // The ID column of CONTEXT_SPEC.md 1 uses ARCH-NNN and MEMORY-REG as shorthand. They are
        // not IDs, and accepting them would let a document claim one.
        for raw in ["ARCH-NNN", "MEMORY-REG", "TASKS-NNN", "BUG-NNN"] {
            assert!(
                !is_document_id(raw),
                "`{raw}` is spec shorthand, not an identifier"
            );
        }
    }

    #[test]
    fn rejects_a_document_id_that_is_not_uppercase_and_kebab() {
        for raw in [
            "task-014",        // lowercase prefix
            "TASK",            // no suffix
            "TASK-",           // empty suffix
            "SPEC-",           // empty slug
            "SPEC-Context",    // uppercase slug
            "SPEC-two--words", // empty slug part
            "SPEC-a_b",        // underscore
            "A-1",             // prefix too short
            "ABCDEFGHIJ-1",    // prefix too long
        ] {
            assert!(
                !is_document_id(raw),
                "`{raw}` should not be a valid DocumentId"
            );
        }
    }

    #[test]
    fn a_slug_part_may_contain_digits() {
        assert!(is_slug("v2-release"));
        assert!(!is_slug("V2-release"));
        assert!(!is_slug("-release"));
        assert!(!is_slug("release-"));
    }

    #[test]
    fn an_invalid_id_names_the_input_and_says_what_is_expected() {
        let error = TaskId::new("task-014").expect_err("lowercase prefix is invalid");
        assert_eq!(error.input(), "task-014");
        assert!(
            error.to_string().contains("task-014"),
            "the message must name the rejected value: {error}"
        );
        assert!(
            error.remediation().contains("CONVENTIONS.md"),
            "remediation must point somewhere actionable: {}",
            error.remediation()
        );
    }

    #[test]
    fn an_invalid_id_has_no_lower_level_cause() {
        use std::error::Error;
        let error = TaskId::new("nonsense").expect_err("invalid");
        assert!(
            error.source().is_none(),
            "a malformed string has no underlying cause to chain"
        );
    }

    #[test]
    fn the_four_typed_ids_do_not_accept_each_others_prefixes() {
        assert!(DecisionId::new("TASK-014").is_err());
        assert!(BugId::new("ADR-001").is_err());
        assert!(ChangeId::new("BUG-002").is_err());
    }

    #[test]
    fn an_invalid_id_type_is_constructible_only_through_new() {
        // Compile-time guarantee: the field is private, so an `InvalidId` can only come from a
        // failed validation, which is what makes `code()` and `remediation()` total.
        fn _assert_private(_: &InvalidId) {}
    }
}
