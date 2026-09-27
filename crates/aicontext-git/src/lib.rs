//! A narrow, safe wrapper over git.
//!
//! The platform is Git-native: knowledge is committed, branched, reviewed, and diffed like code. This
//! crate is the only place allowed to invoke `git`, and it invokes it deliberately.
//!
//! # Safety rules
//!
//! - Arguments are always passed as an argv array. Nothing is ever formatted into a shell string,
//!   so no branch name, path, or commit message can be interpreted as a command.
//! - Read operations only. Committing, pushing, branching, and history rewriting are human actions
//!   and are deliberately not implemented here.
//! - Every failure is a typed error naming the git exit status, because a bare "git failed" costs
//!   the reader an hour.
//!
//! Implemented in `TASK-017`.

#![forbid(unsafe_code)]
