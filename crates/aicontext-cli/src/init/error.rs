//! Failures of the `init` command itself, as opposed to findings about the documents it wrote.
//!
//! Every variant carries a stable code, a message naming the concrete path, and a remediation hint
//! (`RULES.md` §4.3). The codes are `INIT-` rather than `CTX-`: the `CTX-` catalogue in
//! `aicontext_core::ErrorCode` describes document problems, and borrowing one of those codes for a
//! filesystem failure would make a script branch on the wrong thing. The testkit's `WS-` codes set
//! the precedent for a per-binary code space.
//!
//! The process exit code is the second machine-readable handle, and it is the one a script should
//! use; see `docs/CLI_SPEC.md` §5. There is deliberately no conversion into
//! `aicontext_core::AicontextError`: that type carries a `CTX-` document code, and an `INIT-` failure
//! has no honest value to put there.

use std::io;
use std::path::Path;

use crate::exit::Exit;

/// A failure that stopped `init` from completing.
#[derive(Debug, thiserror::Error)]
pub(crate) enum InitError {
    /// The directory the command was pointed at is not usable as a project root.
    #[error("cannot use {path} as the project root: {reason}")]
    UnusableRoot {
        /// The path as the user would recognise it.
        path: String,
        /// Why it cannot be used.
        reason: String,
    },

    /// A template needs a file where a directory already exists.
    #[error("{path} is a directory, but the template needs a file there")]
    PathIsDirectory {
        /// The offending path, relative to the project root.
        path: String,
    },

    /// A path could not be read.
    #[error("cannot read {path}")]
    Read {
        /// The offending path, relative to the project root.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// A path could not be written.
    #[error("cannot write {path}")]
    Write {
        /// The offending path, relative to the project root.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// A directory could not be created.
    #[error("cannot create directory {path}")]
    CreateDirectory {
        /// The offending path, relative to the project root.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// A directory could not be listed during discovery.
    #[error("cannot list {path}")]
    List {
        /// The directory that could not be listed.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// A template still contained a placeholder after substitution, which would have been written
    /// into a document verbatim.
    #[error("template {template} leaves {token} unresolved in {path}")]
    UnresolvedPlaceholder {
        /// The document the placeholder was found in.
        path: String,
        /// The placeholder, without its braces.
        token: String,
        /// The template it came from.
        template: &'static str,
    },

    /// `--force` was asked for where there is no human to confirm with.
    #[error("--force would overwrite {count} edited document(s), and an absent human is a deny")]
    ForceNeedsTerminal {
        /// How many documents would be replaced.
        count: usize,
    },

    /// The `.gitignore` entry did not appear even though it was written.
    #[error("{path} is still not listed in .gitignore after writing it")]
    GitignoreUnchanged {
        /// The entry that should have appeared.
        path: String,
    },
}

impl InitError {
    /// A stable code for this failure, independent of its wording.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => "INIT-001",
            Self::PathIsDirectory { .. } => "INIT-002",
            Self::Read { .. } => "INIT-003",
            Self::Write { .. } => "INIT-004",
            Self::CreateDirectory { .. } => "INIT-005",
            Self::List { .. } => "INIT-006",
            Self::UnresolvedPlaceholder { .. } => "INIT-007",
            Self::ForceNeedsTerminal { .. } => "INIT-008",
            Self::GitignoreUnchanged { .. } => "INIT-009",
        }
    }

    /// What the developer can do about it. Every error answers this (`RULES.md` §4.3).
    pub(crate) const fn remediation(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => {
                "pass --cwd with a writable directory, or run init from inside the project"
            }
            Self::PathIsDirectory { .. } => {
                "move or delete the directory, or use --template blank to start from a smaller skeleton"
            }
            Self::Read { .. } => "check the file's permissions and that nothing else is holding it",
            Self::Write { .. } => {
                "check the directory's permissions and free space, then run init again"
            }
            Self::CreateDirectory { .. } => {
                "check that the path is writable and is not a file, then run init again"
            }
            Self::List { .. } => "check the directory's permissions; discovery is advisory and can be skipped with --no-detect",
            Self::UnresolvedPlaceholder { .. } => {
                "this is a bug in the template: report it with the path and the placeholder"
            }
            Self::ForceNeedsTerminal { .. } => {
                "run init in an interactive terminal to confirm, or leave the document as it is"
            }
            Self::GitignoreUnchanged { .. } => {
                "add .aicontext/ to .gitignore by hand; local state must never be committed"
            }
        }
    }

    /// The exit code this failure produces (`docs/CLI_SPEC.md` §5).
    pub(crate) const fn exit(&self) -> Exit {
        match self {
            Self::UnusableRoot { .. } => Exit::Usage,
            Self::ForceNeedsTerminal { .. } => Exit::ApprovalRequired,
            _ => Exit::General,
        }
    }

    pub(crate) fn read(path: &Path, display: &str, source: io::Error) -> Self {
        Self::Read {
            path: display.to_string(),
            source,
        }
    }

    pub(crate) fn write(path: &str, source: io::Error) -> Self {
        Self::Write {
            path: path.to_string(),
            source,
        }
    }

    pub(crate) fn create_directory(path: &str, source: io::Error) -> Self {
        Self::CreateDirectory {
            path: path.to_string(),
            source,
        }
    }

    pub(crate) fn list(path: &str, source: io::Error) -> Self {
        Self::List {
            path: path.to_string(),
            source,
        }
    }
}