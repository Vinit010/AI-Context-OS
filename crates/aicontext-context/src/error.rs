//! The parser's own failure domain: one enum, per `RULES.md` §4.2.
//!
//! Every variant answers three questions, because a parser that says only "invalid front matter"
//! costs the author a search: **which line**, **what is wrong**, and **what to do**. The line is
//! always 1-based and always relative to the file, not to the extracted block, because the number a
//! person sees in their editor is the number they will search for.
//!
//! This enum converts into [`AicontextError`] so that a caller in `aicontext-cli` can handle every
//! failure in the workspace without enumerating this type, and the cause chain survives the trip
//! (`aicontext-core` documents the funnel).

use std::fmt::Write as _;

use aicontext_core::{AicontextError, ErrorCode, InvalidId, Severity};
use thiserror::Error;

use crate::codec::YamlError;

/// Everything that can go wrong while reading a document's front matter.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ContextError {
    /// The block opened and never closed.
    #[error("front matter opens on line {line} but no closing `---` follows it")]
    FrontMatterNotClosed {
        /// The line the opening `---` is on.
        line: usize,
    },

    /// The block could not be read: broken YAML, or a shape with no fields to read.
    ///
    /// Also the case where the block is valid YAML but its top level is not a mapping - a list or a
    /// bare scalar. The distinction between "broken YAML" and "YAML that has no fields" is not
    /// worth a second variant: both are `CTX-002`, both name a line, and the underlying reason says
    /// which of the two happened.
    #[error("line {line}: front matter could not be read: {reason}")]
    Yaml {
        /// The line the YAML layer reported, mapped back to the file.
        line: usize,
        /// The underlying explanation, with the offending text quoted.
        reason: String,
    },

    /// The same key appears twice.
    #[error("line {line}: `{key}` appears twice in front matter, so one value would be lost")]
    DuplicateKey {
        /// The line of the second occurrence.
        line: usize,
        /// The repeated key.
        key: String,
    },

    /// A key is present but holds the wrong kind of value.
    #[error("line {line}: `{key}` must be {expected}, but it is {actual}")]
    WrongType {
        /// The line the key is on.
        line: usize,
        /// The key whose value has the wrong shape.
        key: String,
        /// The shape the contract requires, in words.
        expected: String,
        /// The shape actually found, in words.
        actual: String,
    },

    /// A date is not ISO-8601 `YYYY-MM-DD`, or is not a real calendar date.
    #[error(
        "line {line}: `{key}` must be an ISO-8601 date such as 2026-09-27, but it is `{value}`"
    )]
    InvalidDate {
        /// The line the date is on.
        line: usize,
        /// The key holding the date.
        key: String,
        /// The value as written.
        value: String,
    },

    /// An `id` that is not a well-formed entity identifier.
    #[error("line {line}: `{key}` is not a valid entity ID: {source}")]
    InvalidId {
        /// The line the identifier is on.
        line: usize,
        /// The key holding the identifier.
        key: String,
        /// The identifier's own rejection, which states the expected shape.
        #[source]
        source: InvalidId,
    },

    /// The front-matter block is over its size limit.
    #[error("front matter is {bytes} bytes, over the {limit}-byte maximum")]
    FrontMatterTooLarge {
        /// The size that was rejected.
        bytes: usize,
        /// The limit that was exceeded.
        limit: usize,
    },

    /// The whole document is over its size limit.
    #[error("document is {bytes} bytes, over the {limit}-byte maximum")]
    DocumentTooLarge {
        /// The size that was rejected.
        bytes: usize,
        /// The limit that was exceeded.
        limit: usize,
    },

    /// A value in memory could not be rendered back to YAML.
    ///
    /// Reported as `CTX-002` rather than given a code of its own. The `CTX` catalogue in
    /// `docs/CONTEXT_SPEC.md` §8 enumerates *findings about a user's documents*, and this is the
    /// dual of an unparseable block rather than a new kind of finding. Minting a number here would
    /// create a code the catalogue does not define, which `aicontext-core` explicitly forbids. If a
    /// distinct code is wanted, add it to §8 in the task that owns the catalogue and widen this
    /// variant.
    #[error("front matter cannot be written: {reason}")]
    NotWritable {
        /// Why the value could not be rendered.
        reason: String,
    },
}

impl ContextError {
    /// The stable code this failure reports, for a caller that wants to branch without converting.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::FrontMatterNotClosed { .. }
            | Self::Yaml { .. }
            | Self::DuplicateKey { .. }
            | Self::WrongType { .. }
            | Self::InvalidDate { .. }
            | Self::NotWritable { .. } => ErrorCode::CTX_002,
            Self::InvalidId { source, .. } => source.code(),
            Self::FrontMatterTooLarge { .. } | Self::DocumentTooLarge { .. } => ErrorCode::CTX_018,
        }
    }

    /// How serious this failure is, and what a caller should do about it.
    ///
    /// An oversized document is a *warning*-shaped problem in the catalogue, but a parser that
    /// returned it as a warning would have nowhere to put it: it has no document to attach it to.
    /// The size limits exist to stop an unbounded read (`RULES.md` §11), so the refusal is an
    /// error, and the severity reported to the user is the one that stopped the read.
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            Self::FrontMatterTooLarge { .. } | Self::DocumentTooLarge { .. } => Severity::Warning,
            _ => Severity::Error,
        }
    }

    /// The line in the file this failure is about, when it is about a line.
    ///
    /// Size failures are about a file as a whole and have no line, which is why this returns an
    /// `Option` rather than a sentinel.
    #[must_use]
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::FrontMatterNotClosed { line }
            | Self::Yaml { line, .. }
            | Self::DuplicateKey { line, .. }
            | Self::WrongType { line, .. }
            | Self::InvalidDate { line, .. }
            | Self::InvalidId { line, .. } => Some(*line),
            Self::FrontMatterTooLarge { .. }
            | Self::DocumentTooLarge { .. }
            | Self::NotWritable { .. } => None,
        }
    }

    /// Turns a codec failure into a domain failure, moving its line number onto the file's numbering.
    ///
    /// `block_start_line` is the line the block's first line occupies, so a YAML error reported on
    /// the block's second line becomes the file's third. Getting this wrong is not cosmetic: the
    /// number is the first thing an author uses, and a line number pointing four lines off is worse
    /// than none.
    ///
    /// The YAML layer numbers lines from the start of the block it was handed and writes positions
    /// into its messages, sometimes several of them - an unclosed quote names both where the scan
    /// gave up and where the quote was opened. Those are shifted onto the file's numbering as well,
    /// because a message holding two different numberings gives a reader no way to tell which line
    /// to jump to, and the prose is the part that carries the position only it knows.
    pub(crate) fn from_yaml(error: &YamlError, block_start_line: usize) -> Self {
        let reason = restate_positions(&error.message, block_start_line - 1);
        match error.line {
            Some(relative) => Self::Yaml {
                line: block_start_line + relative - 1,
                reason,
            },
            None => Self::Yaml {
                line: block_start_line,
                reason,
            },
        }
    }
}

/// Moves every `at line <n> column <m>` a message mentions onto the file's numbering.
///
/// `shift` is `block_start_line - 1`, so a position only ever moves forwards, by exactly the amount
/// the reported line moves. Anything that does not parse as a position is left as it was found and
/// the rest of the message is copied through, so an unexpected message shape costs a diagnostic,
/// never an error.
fn restate_positions(message: &str, shift: usize) -> String {
    let mut restated = String::with_capacity(message.len());
    let mut rest = message;

    while let Some(index) = rest.find(" at line ") {
        let (head, tail) = rest.split_at(index);
        restated.push_str(head);
        let after_line = tail.strip_prefix(" at line ").unwrap_or_default();
        let (line, after_line) = take_digits(after_line);
        let Some(after_column) = after_line.strip_prefix(" column ") else {
            restated.push_str(tail);
            return restated;
        };
        let (column, remainder) = take_digits(after_column);

        let moved = line
            .parse::<usize>()
            .ok()
            .and_then(|line| line.checked_add(shift));
        if column.is_empty() || moved.is_none() {
            restated.push_str(tail);
            return restated;
        }

        let _ = write!(
            restated,
            " at line {} column {column}",
            moved.unwrap_or_default()
        );
        rest = remainder;
    }
    restated.push_str(rest);
    restated
}

/// Splits `text` into its leading run of digits and whatever follows.
fn take_digits(text: &str) -> (&str, &str) {
    let end = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    text.split_at(end)
}

impl From<ContextError> for AicontextError {
    fn from(error: ContextError) -> Self {
        let code = error.code();
        let message = error.to_string();
        let remediation = error.remediation().to_string();
        Self::new(code, message, remediation).with_source(error)
    }
}

impl ContextError {
    /// What the author should do about this failure.
    fn remediation(&self) -> &'static str {
        match self {
            Self::FrontMatterNotClosed { .. } => {
                "add a `---` line after the last front-matter key; an unclosed block makes the \
                 whole file unreadable as a document"
            }
            Self::Yaml { .. } => {
                "fix the YAML on the reported line; front matter must be \
                 `key: value` lines, so a bare list or scalar has no fields to read"
            }
            Self::DuplicateKey { .. } => {
                "remove one of the two occurrences; only the first value would survive"
            }
            Self::WrongType { .. } => {
                "quote the value if it is text, or list the items if it is a sequence"
            }
            Self::InvalidDate { .. } => {
                "write the date as YYYY-MM-DD, unquoted, for example 2026-09-27"
            }
            Self::InvalidId { source, .. } => source.remediation(),
            Self::FrontMatterTooLarge { .. } => {
                "move the long prose into the body; front matter is for fields a tool reads, and it \
                 is capped at 64 KiB"
            }
            Self::DocumentTooLarge { .. } => {
                "split the document. The body is a human view and a machine does not have to read \
                 the whole of it"
            }
            Self::NotWritable { .. } => {
                "this is a defect in the parser, not in the document; please report it"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::DocumentId;
    use aicontext_core::{AicontextError, ErrorCode, Severity};

    use super::{ContextError, restate_positions};

    #[test]
    fn every_parse_failure_is_ctx_002_except_ids_and_size() {
        let cases = [
            (
                ContextError::FrontMatterNotClosed { line: 1 },
                ErrorCode::CTX_002,
            ),
            (
                ContextError::Yaml {
                    line: 1,
                    reason: "bad".into(),
                },
                ErrorCode::CTX_002,
            ),
            (
                ContextError::Yaml {
                    line: 1,
                    reason: "found a list".into(),
                },
                ErrorCode::CTX_002,
            ),
            (
                ContextError::DuplicateKey {
                    line: 2,
                    key: "id".into(),
                },
                ErrorCode::CTX_002,
            ),
            (
                ContextError::WrongType {
                    line: 2,
                    key: "tags".into(),
                    expected: "a list".into(),
                    actual: "a string".into(),
                },
                ErrorCode::CTX_002,
            ),
            (
                ContextError::InvalidDate {
                    line: 2,
                    key: "created".into(),
                    value: "27-09-2026".into(),
                },
                ErrorCode::CTX_002,
            ),
            (
                ContextError::FrontMatterTooLarge {
                    bytes: 70_000,
                    limit: 65_536,
                },
                ErrorCode::CTX_018,
            ),
            (
                ContextError::DocumentTooLarge {
                    bytes: 2_000_000,
                    limit: 1_048_576,
                },
                ErrorCode::CTX_018,
            ),
            (
                ContextError::NotWritable {
                    reason: "tag".into(),
                },
                ErrorCode::CTX_002,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.code(), expected, "for {error}");
        }
    }

    #[test]
    fn an_invalid_id_keeps_the_identifiers_own_code() {
        let error = ContextError::InvalidId {
            line: 2,
            key: "id".into(),
            source: DocumentId::new("task-014").expect_err("lowercase prefix"),
        };

        assert_eq!(error.code(), ErrorCode::CTX_004);
    }

    #[test]
    fn a_size_failure_reports_itself_as_a_warning() {
        // CONTEXT_SPEC.md 8 rates CTX-018 a warning, and this is the one variant where that
        // rating is meaningful to a caller rather than a formality.
        assert_eq!(
            ContextError::DocumentTooLarge { bytes: 2, limit: 1 }.severity(),
            Severity::Warning
        );
        assert_eq!(
            ContextError::FrontMatterNotClosed { line: 1 }.severity(),
            Severity::Error
        );
    }

    #[test]
    fn the_line_is_reported_when_there_is_one_and_absent_when_there_is_not() {
        assert_eq!(
            ContextError::DuplicateKey {
                line: 7,
                key: "id".into(),
            }
            .line(),
            Some(7)
        );
        assert_eq!(
            ContextError::DocumentTooLarge { bytes: 2, limit: 1 }.line(),
            None,
            "a file that is too large has no offending line"
        );
    }

    #[test]
    fn a_codec_line_is_moved_onto_the_files_numbering() {
        // The block starts on line 1, so block line 3 is file line 3.
        let error = ContextError::from_yaml(
            &crate::codec::YamlError {
                message: "bad".into(),
                line: Some(3),
            },
            1,
        );
        assert_eq!(error.line(), Some(3));

        // An inline block inside a body does not start at line 1, and getting this wrong is the
        // failure mode this test exists to prevent.
        let error = ContextError::from_yaml(
            &crate::codec::YamlError {
                message: "bad".into(),
                line: Some(2),
            },
            40,
        );
        assert_eq!(error.line(), Some(41));

        // A codec failure with no location lands on the block's first line rather than line 1 of
        // the file, so the number is never wrong by more than the block is long.
        let error = ContextError::from_yaml(&crate::codec::YamlError::new("no location"), 40);
        assert_eq!(error.line(), Some(40));
    }

    #[test]
    fn positions_inside_the_reason_are_moved_onto_the_files_numbering_too() {
        // The YAML layer numbers from the start of the block and writes positions into its message.
        // An unclosed quote names two of them: where the scan gave up, and where the quote opened -
        // the second is the line the author has to edit, and it is in the prose alone.
        let error = ContextError::from_yaml(
            &crate::codec::YamlError {
                message: "found unexpected end of stream at line 3 column 1, while scanning a \
                          quoted scalar at line 2 column 8"
                    .into(),
                line: Some(3),
            },
            2,
        );

        let ContextError::Yaml { line, reason } = error else {
            panic!("a codec failure becomes a Yaml failure");
        };
        assert_eq!(
            line, 4,
            "the block's first line is file line 2, so the block's line 3 is the file's line 4"
        );
        assert_eq!(
            reason,
            "found unexpected end of stream at line 4 column 1, while scanning a quoted scalar at \
             line 3 column 8",
            "both positions must be in the file's numbering, got: {reason}"
        );
    }

    #[test]
    fn a_message_with_no_position_in_it_is_left_alone() {
        // Most failures come from converting a value tree rather than from scanning, and carry no
        // position at all. The rewrite has to leave those exactly as they were.
        let error = ContextError::from_yaml(
            &crate::codec::YamlError::new("`weight` must be a number, not a list of them"),
            2,
        );

        assert_eq!(
            error.to_string(),
            "line 2: front matter could not be read: `weight` must be a number, not a list of them"
        );
    }

    #[test]
    fn a_message_with_something_position_shaped_in_it_is_not_mangled() {
        // The rewrite is a courtesy, not a requirement, so anything it cannot read is copied
        // through verbatim. A message that says "at line of" must not lose its tail.
        let restated = restate_positions("bad at line of the file at column one", 1);
        assert_eq!(restated, "bad at line of the file at column one");

        // And a position that would move to line 0, or that overflows, is left as it was.
        assert_eq!(
            restate_positions("at line 0 column 3", 1),
            "at line 0 column 3"
        );
        assert_eq!(
            restate_positions("at line 99999999999999999999 column 1", 1),
            "at line 99999999999999999999 column 1"
        );
    }

    #[test]
    fn a_yaml_failure_does_not_claim_to_be_a_shape_problem() {
        // This variant covers broken YAML *and* valid YAML with no fields in it, so its own text has
        // to be neutral; the reason underneath says which happened. A message hard-coded to "not a
        // mapping" told an author with a stray tab that their block had the wrong shape.
        let error = ContextError::Yaml {
            line: 3,
            reason: "mapping values are not allowed in this context".into(),
        };

        assert_eq!(
            error.to_string(),
            "line 3: front matter could not be read: mapping values are not allowed in this context"
        );
    }

    #[test]
    fn converting_keeps_the_code_the_message_and_the_cause() {
        let error = ContextError::DuplicateKey {
            line: 4,
            key: "id".into(),
        };
        let converted: AicontextError = error.into();

        assert_eq!(converted.code(), ErrorCode::CTX_002);
        assert!(
            converted.message().contains("line 4"),
            "got {}",
            converted.message()
        );
        assert!(
            converted.remediation().contains("remove one"),
            "a caller must be told what to do, got {}",
            converted.remediation()
        );
        assert!(
            std::error::Error::source(&converted).is_some(),
            "the domain error must survive as the cause"
        );
        assert!(converted.to_string().starts_with("CTX-002: "));
    }

    #[test]
    fn every_variant_states_what_to_do_about_it() {
        // A parser error without a remediation hint sends the author to the source code instead.
        let cases = [
            ContextError::FrontMatterNotClosed { line: 1 },
            ContextError::Yaml {
                line: 1,
                reason: "bad".into(),
            },
            ContextError::DuplicateKey {
                line: 1,
                key: "id".into(),
            },
            ContextError::WrongType {
                line: 1,
                key: "tags".into(),
                expected: "a list".into(),
                actual: "a string".into(),
            },
            ContextError::InvalidDate {
                line: 1,
                key: "created".into(),
                value: "nope".into(),
            },
            ContextError::InvalidId {
                line: 1,
                key: "id".into(),
                source: DocumentId::new("nope").expect_err("not an ID"),
            },
            ContextError::FrontMatterTooLarge { bytes: 2, limit: 1 },
            ContextError::DocumentTooLarge { bytes: 2, limit: 1 },
            ContextError::NotWritable {
                reason: "tag".into(),
            },
        ];

        for case in cases {
            assert!(
                !case.remediation().is_empty(),
                "{case} has no remediation, so an author would be left guessing"
            );
        }
    }
}
