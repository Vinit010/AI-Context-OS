//! The audit log's failure modes and their stable `AUD-` codes.
//!
//! Every failure names what could not be read, written, or verified, and carries the remediation the
//! caller can act on (`RULES.md` §4.3). The codes are defined in `aicontext-core` with the rest of the
//! catalogue, so a script can branch on `AUD-003` without parsing prose.

use std::io;

use aicontext_core::ErrorCode;

/// A failure reading, verifying, or appending to the audit log.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// The log file could not be opened or read.
    #[error("the audit log `{path}` could not be read: {source}")]
    Read {
        /// The path as it was shown to the caller.
        path: String,
        /// The underlying filesystem failure.
        #[source]
        source: io::Error,
    },
    /// The log file could not be created or written.
    #[error("the audit log `{path}` could not be written: {source}")]
    Write {
        /// The path as it was shown to the caller.
        path: String,
        /// The underlying filesystem failure.
        #[source]
        source: io::Error,
    },
    /// The log is larger than the read cap.
    #[error(
        "the audit log `{path}` is {size} bytes, over the {limit}-byte cap; refusing to read it"
    )]
    TooLarge {
        /// The path as it was shown to the caller.
        path: String,
        /// The file's size.
        size: u64,
        /// The cap that was exceeded.
        limit: u64,
    },
    /// A line is not JSON.
    #[error("line {line} of the audit log is not valid JSON: {source}")]
    Malformed {
        /// The 1-based line number.
        line: usize,
        /// The parser's explanation.
        #[source]
        source: serde_json::Error,
    },
    /// A line is JSON but not a record this version accepts.
    #[error("{message}")]
    Invalid {
        /// The 1-based line number, when the failure came from a file.
        line: Option<usize>,
        /// What is wrong, naming the field.
        message: String,
    },
    /// The chain does not link up.
    #[error("the audit chain is broken at line {line}: {message}")]
    Broken {
        /// The 1-based line number of the first record that does not link.
        line: usize,
        /// The concrete mismatch.
        message: String,
    },
    /// The existing log cannot be verified, so nothing was appended.
    #[error("refusing to append to the audit log: {reason}")]
    AppendRefused {
        /// A human explanation, including the underlying failure.
        reason: String,
        /// The failure that refused the append, kept for a caller that walks the source chain.
        #[source]
        cause: Box<Self>,
    },
}

impl AuditError {
    /// A shape failure that did not come from a file.
    pub(crate) fn invalid_record(message: impl Into<String>) -> Self {
        Self::Invalid {
            line: None,
            message: message.into(),
        }
    }

    /// Wraps an existing failure as an append refusal, preserving the cause.
    pub(crate) fn append_refused(cause: Self) -> Self {
        Self::AppendRefused {
            reason: format!("the existing chain is not valid: {cause}"),
            cause: Box::new(cause),
        }
    }

    /// The stable code (`docs/SECURITY.md` §8).
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Read { .. } | Self::Write { .. } => ErrorCode::AUD_001,
            Self::Malformed { .. } | Self::Invalid { .. } => ErrorCode::AUD_002,
            Self::Broken { .. } => ErrorCode::AUD_003,
            Self::TooLarge { .. } => ErrorCode::AUD_004,
            Self::AppendRefused { .. } => ErrorCode::AUD_005,
        }
    }

    /// The 1-based line the failure points at, when it points at one.
    #[must_use]
    pub const fn line(&self) -> Option<usize> {
        match self {
            Self::Malformed { line, .. } | Self::Broken { line, .. } => Some(*line),
            Self::Invalid { line, .. } => *line,
            Self::Read { .. } | Self::Write { .. } | Self::TooLarge { .. } => None,
            // The refusal is about the existing log as a whole; its cause carries the line when
            // there is one.
            Self::AppendRefused { .. } => None,
        }
    }

    /// What the caller should do about it.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::Read { .. } | Self::Write { .. } => {
                "check that `.aicontext/audit.jsonl` is readable and writable, then run it again"
            }
            Self::Malformed { .. } | Self::Invalid { .. } => {
                "restore the damaged record from a known-good copy; the log is append-only and a \
                 damaged line is a corrupted log"
            }
            Self::Broken { .. } => {
                "restore the log from a known-good copy and verify it again; appending cannot \
                 repair a broken chain"
            }
            Self::TooLarge { .. } => {
                "archive the log and start a new one; the read cap bounds every command that reads it"
            }
            Self::AppendRefused { .. } => {
                "repair the chain first; nothing was appended and the existing bytes are unchanged"
            }
        }
    }

    /// The problem as a CLI or library report can carry it.
    #[must_use]
    pub fn problem(&self) -> AuditProblem {
        AuditProblem {
            code: self.code(),
            line: self.line(),
            message: self.to_string(),
            remediation: self.remediation(),
        }
    }
}

/// A verified failure, detached from the filesystem handle that produced it.
///
/// `Verification` returns this rather than an [`AuditError`] because a report has to survive the end
/// of the read: a `serde_json::Error` or an `io::Error` is not `Clone`, and a CLI that wants to print
/// both the problem and the records before it should not have to read the file twice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditProblem {
    code: ErrorCode,
    line: Option<usize>,
    message: String,
    remediation: &'static str,
}

impl AuditProblem {
    /// The stable code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// The 1-based line the problem points at, when it points at one.
    #[must_use]
    pub const fn line(&self) -> Option<usize> {
        self.line
    }

    /// What is wrong, in one sentence.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// What the caller should do about it.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        self.remediation
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::AuditError;

    #[test]
    fn every_failure_maps_to_a_distinct_aud_codes_and_names_its_remediation() {
        let cases = [
            (
                AuditError::Read {
                    path: "audit.jsonl".to_string(),
                    source: std::io::Error::other("nope"),
                },
                "AUD-001",
            ),
            (
                AuditError::Malformed {
                    line: 3,
                    source: serde_json::from_str::<serde_json::Value>("{").expect_err("invalid"),
                },
                "AUD-002",
            ),
            (
                AuditError::Invalid {
                    line: Some(4),
                    message: "seq starts at 1".to_string(),
                },
                "AUD-002",
            ),
            (
                AuditError::Broken {
                    line: 5,
                    message: "prev does not match".to_string(),
                },
                "AUD-003",
            ),
            (
                AuditError::TooLarge {
                    path: "audit.jsonl".to_string(),
                    size: 10,
                    limit: 5,
                },
                "AUD-004",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.code().to_string(), expected);
            assert!(!error.remediation().is_empty());
            assert_eq!(error.problem().code(), error.code());
            assert!(!error.problem().message().is_empty());
        }
    }

    #[test]
    fn the_line_is_kept_only_where_it_means_something() {
        assert_eq!(
            AuditError::Invalid {
                line: Some(4),
                message: "bad".to_string()
            }
            .line(),
            Some(4)
        );
        assert_eq!(
            AuditError::Read {
                path: "p".to_string(),
                source: std::io::Error::other("x")
            }
            .line(),
            None
        );
    }

    #[test]
    fn an_append_refusal_preserves_the_cause_and_uses_its_own_code() {
        let cause = AuditError::Broken {
            line: 2,
            message: "prev does not match".to_string(),
        };
        let refused = AuditError::append_refused(cause);
        assert_eq!(refused.code().to_string(), "AUD-005");
        assert_eq!(refused.code(), ErrorCode::AUD_005);
        assert!(refused.to_string().contains("prev does not match"));
        assert!(std::error::Error::source(&refused).is_some());
        assert!(refused.remediation().contains("nothing was appended"));
    }

    #[test]
    fn a_shape_failure_without_a_file_has_no_line() {
        let error = AuditError::invalid_record("event must not be empty");
        assert_eq!(error.line(), None);
        assert_eq!(error.code().to_string(), "AUD-002");
        assert!(error.to_string().contains("event must not be empty"));
    }

    #[test]
    fn a_too_large_log_names_both_numbers() {
        let error = AuditError::TooLarge {
            path: ".aicontext/audit.jsonl".to_string(),
            size: 100,
            limit: 64,
        };
        let text = error.to_string();
        assert!(text.contains("100"));
        assert!(text.contains("64"));
        assert!(text.contains("audit.jsonl"));
    }
}
