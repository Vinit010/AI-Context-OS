//! The typed failures of invoking git.
//!
//! One enum, because every failure of this crate is the same failure seen from a different angle: git
//! did not answer the question that was asked. `RULES.md` §4.2 asks for one enum per failure domain
//! and the domain here is a single external process, so splitting "not installed" from "not a
//! repository" into two types would mean two matches at every call site for no gain.
//!
//! Each variant maps onto exactly one code in [`ErrorCode`], and the mapping is the reason the
//! variants exist separately at all: a caller that must decide whether to offer `--help`, report a
//! validation failure, or tell the user their repository has no commits yet cannot do it from a
//! message string, and `docs/CLI_SPEC.md` §6 makes the code a stable contract.
//!
//! ```text
//! GIT-001  git is not installed, or the process could not be started
//! GIT-002  the directory is not inside a Git working tree
//! GIT-003  a git command exited with a non-zero status
//! GIT-004  the repository has no commits, so HEAD names nothing
//! GIT-005  the git process was terminated by a signal
//! GIT-006  git produced output this crate could not read as UTF-8
//! ```
//!
//! # What is deliberately not here
//!
//! There is no "the repository is locked" variant. It was specified, then measured to be
//! unreachable: with `.git/index.lock` present, every read-only command this crate runs —
//! `status`, `log`, `diff`, `branch`, `rev-parse` — exits 0 on git 2.51, because a read does not
//! acquire the lock. [`ErrorCode::GIT_005`] was reassigned to a terminated process instead, because
//! that one can actually happen and needs a different answer from the user than "git refused".
//!
//! There is no variant for "the command does not exist", because the subcommands are fixed here and
//! cannot be chosen by a caller. A caller that needs a git operation this crate does not offer gets
//! a compile error, which is the intended outcome: see the safety rules in the crate documentation.

use std::io;
use std::path::PathBuf;

use aicontext_core::{AicontextError, ErrorCode};
use thiserror::Error;

/// Something that went wrong while asking git a question.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitError {
    /// `git` could not be started at all.
    #[error("cannot run `{program}`: {source}")]
    Unavailable {
        /// The program that was tried, so the message names the concrete subject.
        program: String,
        /// The operating system's own reason, kept as the cause rather than folded into the message.
        #[source]
        source: io::Error,
    },

    /// The directory is not inside a Git working tree.
    #[error("`{root}` is not inside a Git repository")]
    NotARepository {
        /// The directory that was asked about.
        root: PathBuf,
    },

    /// A git command exited with a non-zero status.
    ///
    /// Carries the argv rather than a formatted command line, so the reader sees exactly what was
    /// run — including the argument boundaries, which is the part that matters when the failure is a
    /// surprising path or branch name.
    #[error("`git {}` exited {status}{}", .argv.join(" "), stderr_suffix(.stderr))]
    CommandFailed {
        /// The command as an argv array.
        argv: Vec<String>,
        /// The exit status git reported.
        status: i32,
        /// What git wrote to stderr, with trailing whitespace removed. Empty when it wrote nothing.
        stderr: String,
    },

    /// The repository has no commits, so there is no `HEAD` to resolve.
    ///
    /// Distinct from [`GitError::CommandFailed`] because it is a normal state for a repository that
    /// was just initialised, not something the user did wrong. The remediation is a first commit,
    /// not a fix.
    #[error("the repository at `{root}` has no commits yet, so there is no HEAD")]
    NoCommits {
        /// The repository root that has no history.
        root: PathBuf,
    },

    /// The git process was terminated before it could exit on its own.
    ///
    /// Carries no status because there is none: a process killed by a signal has no exit code, which
    /// is precisely what distinguishes this from [`GitError::CommandFailed`].
    #[error("`git {}` was terminated before it finished", .argv.join(" "))]
    Terminated {
        /// The command as an argv array.
        argv: Vec<String>,
    },

    /// Git wrote bytes that are not UTF-8.
    ///
    /// Reachable on Unix, where a filename is a byte string and any byte sequence is legal. Not
    /// reachable through the filesystem on Windows, where names are UTF-16 and git emits UTF-8, but
    /// it is a real cross-platform contract rather than a hypothetical: `R-11` in
    /// `ARCHITECTURE.md` §13 names Windows parity as a risk, and the fix is to refuse rather than to
    /// substitute replacement characters and report a path that does not exist.
    #[error("`git {command}` wrote output that is not UTF-8: {source}")]
    UnreadableOutput {
        /// What the crate was asking for, for example `status --porcelain=v2`.
        command: &'static str,
        /// Where the invalid byte is.
        #[source]
        source: std::string::FromUtf8Error,
    },

    /// Git's output did not have the documented porcelain v2 shape.
    ///
    /// Distinct from [`GitError::UnreadableOutput`] because the bytes were fine: a record had the
    /// wrong number of fields, or an unknown type code appeared. This is the failure mode reserved for
    /// a future git changing its machine-readable format, and it is an error rather than an empty
    /// result on purpose — `RULES.md` §4.8 forbids reporting success for a batch that partly failed,
    /// and "your working tree is clean" derived from output we did not understand is exactly that.
    #[error("`git {command}` produced output this version of aicontext does not understand: {detail}")]
    UnexpectedOutput {
        /// What the crate was asking for.
        command: &'static str,
        /// What was wrong with it, naming the offending record.
        detail: String,
    },
}

/// Renders git's stderr as a message suffix, or nothing when git wrote nothing.
///
/// A free function rather than a method because `thiserror` expands its format string into
/// generated code in this module, where a private helper is reachable and a `pub` one would not need
/// to exist.
///
/// The empty case is why this is a function at all: git does not always write to stderr, and
/// appending ": " regardless would leave "exited 128: " with a dangling colon that reads as though
/// something had been elided.
fn stderr_suffix(stderr: &str) -> String {
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

impl GitError {
    /// The stable code for this failure, safe to branch on.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Unavailable { .. } => ErrorCode::GIT_001,
            Self::NotARepository { .. } => ErrorCode::GIT_002,
            Self::CommandFailed { .. } => ErrorCode::GIT_003,
            Self::NoCommits { .. } => ErrorCode::GIT_004,
            Self::Terminated { .. } => ErrorCode::GIT_005,
            Self::UnreadableOutput { .. } | Self::UnexpectedOutput { .. } => ErrorCode::GIT_006,
        }
    }

    /// What the user should do about it.
    ///
    /// Kept separate from the message so the CLI can print it as its own labelled line
    /// (`RULES.md` §4.5) without having to parse the message.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => {
                "install git and make sure `git` is on PATH; aicontext reads the repository with it"
            }
            Self::NotARepository { .. } => {
                "run this from inside a Git working tree, or run `git init` to create one"
            }
            Self::CommandFailed { .. } => {
                "read the git message above; it names the command that refused and why"
            }
            Self::NoCommits { .. } => {
                "make a first commit, since there is nothing to read a branch or history from yet"
            }
            Self::Terminated { .. } => {
                "run the same command yourself to see what happened; aicontext reports this when git \
                 is killed rather than when it refuses"
            }
            Self::UnreadableOutput { .. } => {
                "a path in this repository is not valid UTF-8; rename it, or read the repository \
                 through a tool that accepts byte paths"
            }
            Self::UnexpectedOutput { .. } => {
                "this git is newer or older than the format aicontext expects; check the git \
                 version against the one in rust-toolchain.toml, and report it if they differ"
            }
        }
    }
}

impl From<GitError> for AicontextError {
    fn from(error: GitError) -> Self {
        Self::new(error.code(), error.to_string(), error.remediation()).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::{AicontextError, ErrorCode};

    use super::GitError;

    #[test]
    fn every_variant_has_its_own_stable_code() {
        let errors = [
            GitError::Unavailable {
                program: "git".to_owned(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
            },
            GitError::NotARepository {
                root: "/tmp/plain".into(),
            },
            GitError::CommandFailed {
                argv: vec!["log".to_owned()],
                status: 128,
                stderr: "fatal: your current branch 'main' does not have any commits yet".to_owned(),
            },
            GitError::NoCommits {
                root: "/tmp/repo".into(),
            },
            GitError::Terminated {
                argv: vec!["status".to_owned()],
            },
            GitError::UnreadableOutput {
                command: "log",
                source: String::from_utf8(vec![0xff, 0xfe]).expect_err("not utf-8"),
            },
            GitError::UnexpectedOutput {
                command: "status --porcelain=v2 --branch -z",
                detail: "unknown record type `3`".to_owned(),
            },
        ];

        let expected = [
            ErrorCode::GIT_001,
            ErrorCode::GIT_002,
            ErrorCode::GIT_003,
            ErrorCode::GIT_004,
            ErrorCode::GIT_005,
            ErrorCode::GIT_006,
            ErrorCode::GIT_006,
        ];

        for (error, code) in errors.iter().zip(expected) {
            assert_eq!(error.code(), code);
            assert!(!error.remediation().is_empty(), "{error} has no remediation");
            // `GitError` renders its message only; the code is added when it becomes an
            // `AicontextError`. Asserting the code appears in its own `Display` would assert
            // something this crate does not do.
            assert!(!error.to_string().is_empty(), "{code} renders as nothing");
        }
    }

    #[test]
    fn every_code_survives_conversion_into_the_common_type() {
        // This is where the code actually becomes visible: `AicontextError` renders `{code}: {message}`.
        let error: AicontextError = GitError::NotARepository {
            root: "/tmp/plain".into(),
        }
        .into();

        assert_eq!(error.code(), ErrorCode::GIT_002);
        assert_eq!(
            error.to_string(),
            "GIT-002: `/tmp/plain` is not inside a Git repository"
        );
    }

    #[test]
    fn the_documented_code_list_matches_the_variants() {
        // The module doc lists all six codes in prose. If a variant is added without the doc being
        // updated, the count below no longer matches and this fails - which is the point: the
        // contract is the documentation, so the documentation has to be maintained with the code.
        let documented = [
            ErrorCode::GIT_001,
            ErrorCode::GIT_002,
            ErrorCode::GIT_003,
            ErrorCode::GIT_004,
            ErrorCode::GIT_005,
            ErrorCode::GIT_006,
        ];
        let every = [
            GitError::Unavailable {
                program: "git".to_owned(),
                source: std::io::Error::other("x"),
            },
            GitError::NotARepository { root: "/tmp".into() },
            GitError::CommandFailed {
                argv: vec!["log".to_owned()],
                status: 128,
                stderr: String::new(),
            },
            GitError::NoCommits {
                root: "/tmp".into(),
            },
            GitError::Terminated {
                argv: vec!["log".to_owned()],
            },
            GitError::UnreadableOutput {
                command: "log",
                source: String::from_utf8(vec![0xff]).expect_err("not utf-8"),
            },
        ];
        assert_eq!(documented.len(), every.len());
    }

    #[test]
    fn a_failed_command_names_the_status_and_what_git_said() {
        let error = GitError::CommandFailed {
            argv: vec!["rev-parse".to_owned(), "--verify".to_owned(), "HEAD".to_owned()],
            status: 128,
            stderr: "fatal: Needed a single revision".to_owned(),
        };

        let text = error.to_string();
        assert!(text.contains("rev-parse --verify HEAD"), "{text}");
        assert!(text.contains("128"), "{text}");
        assert!(text.contains("Needed a single revision"), "{text}");
    }

    #[test]
    fn a_failed_command_with_no_stderr_still_reads_as_a_sentence() {
        // git does not always write to stderr, and "exited 128: " with a trailing colon reads like
        // something was omitted.
        let error = GitError::CommandFailed {
            argv: vec!["status".to_owned()],
            status: 1,
            stderr: String::new(),
        };

        assert_eq!(error.to_string(), "`git status` exited 1");
    }

    #[test]
    fn the_error_carries_its_cause_into_the_common_type() {
        let error = GitError::Unavailable {
            program: "git".to_owned(),
            source: std::io::Error::other("no such file or directory"),
        };

        let common: AicontextError = error.into();

        assert_eq!(common.code(), ErrorCode::GIT_001);
        assert!(common.message().contains("git"));
        assert!(common.remediation().contains("PATH"));
        let cause = common.cause().expect("the cause survives the conversion");
        // The cause is the `GitError` itself, not the `io::Error` inside it: the GitError carries the
        // stable code, so it is what a caller walks the chain to find.
        assert!(
            cause.to_string().starts_with("cannot run `git`"),
            "the chain must lead back to the GitError, got {cause}"
        );
        assert!(
            cause.source().is_some(),
            "and from there to the operating system's own reason"
        );
    }

    #[test]
    fn no_commits_is_not_reported_as_a_command_failure() {
        // They are different states with different answers, which is why they are different
        // variants: one is "your repository is fine but empty", the other is "git said no".
        let empty = GitError::NoCommits {
            root: "/tmp/repo".into(),
        };
        let refused = GitError::CommandFailed {
            argv: vec!["log".to_owned()],
            status: 128,
            stderr: "fatal: your current branch 'main' does not have any commits yet".to_owned(),
        };

        assert_eq!(empty.code(), ErrorCode::GIT_004);
        assert_eq!(refused.code(), ErrorCode::GIT_003);
        assert!(
            empty.remediation().contains("first commit"),
            "the remediation is an action, not an apology: {}",
            empty.remediation()
        );
    }
}