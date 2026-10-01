//! Outcome of one command, mapped onto the exit codes in `docs/CLI_SPEC.md` §5.
//!
//! Exactly one code is returned per run, and it is the only thing a script may branch on. Only the
//! codes this binary can actually produce are defined here; the rest of the table arrives with the
//! command that needs it, so the enum never claims a failure mode that cannot happen (`RULES.md` §2).

/// The outcome of a run, as a stable machine code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Exit {
    /// Success.
    Ok,
    /// An unclassified failure: a filesystem or template problem the caller cannot act on by code.
    General,
    /// Bad arguments, an unknown command, or a command that is not implemented yet.
    Usage,
    /// The command produced something that failed validation.
    Validation,
    /// Approval was required and not granted, including every non-interactive case.
    ApprovalRequired,
    /// A bug. Reserved, and carries a bug reference when one exists.
    Internal,
}

impl Exit {
    /// The numeric exit code, which is the documented contract.
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::General => 1,
            Self::Usage => 2,
            Self::Validation => 3,
            Self::ApprovalRequired => 5,
            Self::Internal => 70,
        }
    }

    /// The name from the §5 table, for the human summary line.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::General => "GENERAL",
            Self::Usage => "USAGE",
            Self::Validation => "VALIDATION",
            Self::ApprovalRequired => "APPROVAL_REQUIRED",
            Self::Internal => "INTERNAL",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Exit;

    #[test]
    fn codes_match_the_published_table() {
        let table = [
            (Exit::Ok, 0u8, "OK"),
            (Exit::General, 1, "GENERAL"),
            (Exit::Usage, 2, "USAGE"),
            (Exit::Validation, 3, "VALIDATION"),
            (Exit::ApprovalRequired, 5, "APPROVAL_REQUIRED"),
            (Exit::Internal, 70, "INTERNAL"),
        ];
        for (exit, code, name) in table {
            assert_eq!(exit.code(), code, "{name} has the wrong code");
            assert_eq!(exit.name(), name);
        }
    }

    #[test]
    fn a_command_produces_exactly_one_code() {
        // The codes are disjoint by construction; this test exists so a future `From`
        // implementation that silently collapsed two outcomes into one would be noticed.
        let all = [
            Exit::Ok,
            Exit::General,
            Exit::Usage,
            Exit::Validation,
            Exit::ApprovalRequired,
            Exit::Internal,
        ];
        for (index, exit) in all.iter().enumerate() {
            for other in &all[index + 1..] {
                assert_ne!(exit.code(), other.code(), "{exit:?} and {other:?} share a code");
            }
        }
    }
}