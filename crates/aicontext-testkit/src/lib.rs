//! Test support for the AI Context OS workspace.
//!
//! This crate provides two things:
//!
//! 1. **Fixtures.** Temporary projects, temporary Git repositories, and sample `.ai/` trees, so no
//!    test ever touches the developer's home directory, their global Git configuration, or the
//!    network. It arrives with `TASK-018`.
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
//! None. Everything here is deliberately hand-rolled, because a test helper that drags in a
//! dependency tree makes every build slower than the tests it supports
//! (`RULES.md` §6).

#![forbid(unsafe_code)]

pub mod manifest;

pub use manifest::{Manifest, ManifestError};
