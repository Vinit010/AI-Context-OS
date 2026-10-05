//! The error type every fixture helper returns.
//!
//! A test fixture can fail for exactly one reason — the test environment is not what it needed — and
//! that is a bug in the test rather than a condition a caller recovers from. So this error exists to
//! *say what happened*, not to be handled: a test unwraps it with a message naming the fixture call,
//! and `RULES.md` §4's rule that errors carry a code, a message, and a remediation still applies
//! because the message is what a failing test prints.
//!
//! One enum rather than one per module, because the variants are the failure modes of *building a
//! fixture* and a test that matched on them separately would learn nothing. It is `#[non_exhaustive]`
//! so a new helper can add a variant without breaking a caller that matches on it.
//!
//! `Display` and `Error` are written by hand rather than derived through `thiserror`, which is what
//! every other error enum in this workspace does. This crate is test support that is compiled only
//! for `cargo test`, and pulling the workspace's error dependency into it so five variants can
//! spell themselves would make every build pay for a production crate to serve a test-only one. The
//! manual impl below is the same shape `thiserror` generates; nothing is lost but the brevity.
//!
//! ```text
//! FIX-001  the project name cannot be used
//! FIX-002  a filesystem operation failed
//! FIX-003  git is not on PATH
//! FIX-004  a git command failed
//! FIX-005  a git command that would reach the network was refused
//! ```

use std::error::Error;
use std::fmt;
use std::io;

/// Something that went wrong while building a fixture.
#[derive(Debug)]
#[non_exhaustive]
pub enum FixtureError {
    /// The project name cannot be used, so the fixture would not be the project it claims to be.
    UnusableName {
        /// The name as the test supplied it.
        name: String,
        /// Why it cannot be used.
        reason: String,
    },
    /// A filesystem operation on the fixture tree failed.
    Io {
        /// What was being done, in the infinitive, for example `write a fixture file`.
        action: &'static str,
        /// The underlying failure. Also what [`Error::source`] returns.
        source: io::Error,
    },
    /// `git` is not on `PATH`, so no repository fixture can be built.
    GitMissing {
        /// The failure that made git unusable. Also what [`Error::source`] returns.
        source: io::Error,
    },
    /// A git command returned a non-zero exit status.
    GitFailed {
        /// The command as an argv array, so the reader sees what was actually run.
        argv: Vec<String>,
        /// The exit status, or `None` when the process was killed by a signal.
        status: Option<i32>,
        /// What git wrote to stderr, trailing whitespace removed.
        stderr: String,
    },
    /// A git command that would reach the network was refused.
    ///
    /// Refused rather than merely discouraged, because a fixture that clones leaves behind a test
    /// that fails on an offline machine and passes on a connected one. `RULES.md` §8 requires tests
    /// to be deterministic and forbids the network.
    NetworkRefused {
        /// The subcommand that was refused, for example `clone`.
        subcommand: String,
    },
}

impl FixtureError {
    /// A stable machine code, safe to assert on in a test.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnusableName { .. } => "FIX-001",
            Self::Io { .. } => "FIX-002",
            Self::GitMissing { .. } => "FIX-003",
            Self::GitFailed { .. } => "FIX-004",
            Self::NetworkRefused { .. } => "FIX-005",
        }
    }

    /// What to do about it (`RULES.md` §4.3).
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        match self {
            Self::UnusableName { .. } => {
                "pass a single directory name, such as `payments-ledger`, with no path separator"
            }
            Self::Io { .. } => {
                "check that the temporary directory is writable and not removed underneath the test"
            }
            Self::GitMissing { .. } => {
                "install git and make sure it is on PATH; every repository fixture needs it"
            }
            Self::GitFailed { .. } => {
                "the fixture asked git for something the repository cannot answer yet; check that \
                 the repository has the commit or branch the test is asserting on"
            }
            Self::NetworkRefused { .. } => {
                "build the repository locally with TempRepository; a fixture must not reach the \
                 network (RULES.md 8)"
            }
        }
    }
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.code())?;
        match self {
            Self::UnusableName { name, reason } => {
                write!(formatter, "project name `{name}` cannot be used: {reason}")
            }
            Self::Io { action, source } => {
                write!(formatter, "cannot {action}: {source}")
            }
            Self::GitMissing { source } => {
                write!(formatter, "cannot run git: {source}")
            }
            Self::GitFailed {
                argv,
                status,
                stderr,
            } => {
                write!(formatter, "git {}", argv.join(" "))?;
                match status {
                    Some(code) => write!(formatter, " exited {code}")?,
                    None => write!(formatter, " was killed by a signal")?,
                }
                if !stderr.is_empty() {
                    write!(formatter, ": {stderr}")?;
                }
                Ok(())
            }
            Self::NetworkRefused { subcommand } => {
                write!(
                    formatter,
                    "`git {subcommand}` is refused in a fixture because it would reach the network"
                )
            }
        }
    }
}

impl Error for FixtureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnusableName { .. } | Self::GitFailed { .. } | Self::NetworkRefused { .. } => {
                None
            }
            Self::Io { source, .. } | Self::GitMissing { source } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::FixtureError;

    fn codes() -> Vec<(&'static str, &'static str)> {
        let missing = FixtureError::GitMissing {
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "git"),
        };
        vec![
            (
                FixtureError::UnusableName {
                    name: "a/b".to_owned(),
                    reason: "separator".to_owned(),
                }
                .code(),
                "FIX-001",
            ),
            (
                FixtureError::Io {
                    action: "write a fixture file",
                    source: std::io::Error::other("disk full"),
                }
                .code(),
                "FIX-002",
            ),
            (missing.code(), "FIX-003"),
            (
                FixtureError::GitFailed {
                    argv: vec!["log".to_owned()],
                    status: Some(128),
                    stderr: "no commits yet".to_owned(),
                }
                .code(),
                "FIX-004",
            ),
            (
                FixtureError::NetworkRefused {
                    subcommand: "clone".to_owned(),
                }
                .code(),
                "FIX-005",
            ),
        ]
    }

    #[test]
    fn the_codes_are_distinct_and_match_the_module_doc() {
        let codes = codes();
        for (error_code, expected) in &codes {
            assert_eq!(error_code, expected);
        }
        let unique: std::collections::BTreeSet<&str> =
            codes.iter().map(|(code, _)| *code).collect();
        assert_eq!(unique.len(), codes.len(), "codes must be distinct");
    }

    #[test]
    fn every_variant_answers_what_now() {
        let errors = [
            FixtureError::UnusableName {
                name: String::new(),
                reason: "empty".to_owned(),
            },
            FixtureError::Io {
                action: "write a fixture file",
                source: std::io::Error::other("disk full"),
            },
            FixtureError::GitMissing {
                source: std::io::Error::other("not found"),
            },
            FixtureError::GitFailed {
                argv: vec!["log".to_owned(), "--oneline".to_owned()],
                status: Some(128),
                stderr: "does not have any commits yet".to_owned(),
            },
            FixtureError::NetworkRefused {
                subcommand: "fetch".to_owned(),
            },
        ];
        for error in &errors {
            assert!(
                !error.remediation().is_empty(),
                "{error} has no remediation"
            );
            assert!(error.to_string().starts_with(error.code()), "{error}");
        }
    }

    #[test]
    fn a_network_refusal_names_the_subcommand_and_the_rule() {
        let error = FixtureError::NetworkRefused {
            subcommand: "clone".to_owned(),
        };
        let text = error.to_string();
        assert!(text.contains("git clone"), "{text}");
        assert!(text.contains("network"), "{text}");
        assert!(error.remediation().contains("TempRepository"));
    }

    #[test]
    fn a_failure_names_the_command_the_status_and_what_git_said() {
        let error = FixtureError::GitFailed {
            argv: vec!["log".to_owned(), "--oneline".to_owned()],
            status: Some(128),
            stderr: "fatal: your current branch 'main' does not have any commits yet".to_owned(),
        };
        let text = error.to_string();
        assert!(text.contains("git log --oneline"), "{text}");
        assert!(text.contains("exited 128"), "{text}");
        assert!(text.contains("does not have any commits yet"), "{text}");
        assert!(error.source().is_none(), "there is no cause below this");
    }

    #[test]
    fn a_filesystem_failure_keeps_its_cause() {
        let error = FixtureError::Io {
            action: "write a fixture file",
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert!(error.to_string().contains("cannot write a fixture file"));
        let cause = error.source().expect("the cause must survive");
        assert!(cause.to_string().contains("denied"));
    }
}
