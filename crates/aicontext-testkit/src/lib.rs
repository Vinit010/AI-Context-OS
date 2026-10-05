//! Test support for the AI Context OS workspace.
//!
//! This crate provides two things:
//!
//! 1. **Fixtures.** Temporary projects, temporary Git repositories, and a sample `.ai/` tree, so no
//!    test ever touches the developer's home directory, their global Git configuration, or the
//!    network. Those are [`TempProject`], [`TempRepository`], and [`sample_ai_tree`], added by
//!    `TASK-018`.
//! 2. **Workspace policy checks.** Rules about the shape of the workspace itself, which run as part
//!    of `cargo test` rather than as a shell script nobody runs. Today that is the crate
//!    dependency-direction check (`tests/crate_boundaries.rs`).
//!
//! # Usage
//!
//! This crate is a **dev-dependency only**. It must never appear in any crate's dependencies table:
//! test support leaking into production code is how test-only behaviour becomes shipping
//! behaviour. That rule is itself enforced by the boundary check.
//!
//! # Dependencies
//!
//! One: `tempfile`, for the temporary directories. Everything else is deliberately hand-rolled. A
//! test helper that drags in a dependency tree makes every build slower than the tests it supports
//! (`RULES.md` §6), and `MEM-009` recorded this crate as dependency-free before the fixtures existed.
//! The reasoning for `tempfile` rather than ~40 lines of `std::env::temp_dir()` is in
//! `crates/aicontext-testkit/Cargo.toml`: it is the Windows case where a just-closed handle keeps a
//! directory from being deleted.
//!
//! # What a fixture guarantees
//!
//! Each of these was verified against `git` 2.51 before being written down, and each is a way a test
//! would otherwise pass on one machine and fail on another:
//!
//! | Guarantee | How |
//! |-----------|------|
//! | No global or system Git configuration is read | `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`, and `HOME`/`USERPROFILE`/`XDG_CONFIG_HOME` all point inside the fixture |
//! | No inherited variable can change the answer | the child environment is built with `env_clear()` |
//! | LF stays LF on Windows | `core.autocrlf=false` through the `GIT_CONFIG_COUNT` protocol |
//! | The default branch does not depend on the developer | `init.defaultBranch=main`, pinned the same way |
//! | A commit needs no configured identity | `GIT_AUTHOR_*` and `GIT_COMMITTER_*`, including fixed dates |
//! | Commit hashes are reproducible | the same fixed dates |
//! | A fixture cannot reach the network | `clone`, `fetch`, `push`, `pull`, `remote`, and `submodule` are refused |
//! | No shell is involved | arguments are passed to `Command` as an argv array |

#![forbid(unsafe_code)]

pub mod ai;
pub mod error;
pub mod manifest;
pub mod repo;
pub mod temp;

pub use ai::{SAMPLE_SPEC_PATH, SAMPLE_TASK_ID, sample_ai_tree};
pub use error::FixtureError;
pub use manifest::{Manifest, ManifestError};
pub use repo::{ChangeStatus, TempRepository};
pub use temp::TempProject;
