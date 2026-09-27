//! Core domain types for AI Context OS.
//!
//! This crate is the dependency floor of the workspace. It depends on no other internal crate, and
//! it depends on no async runtime, no CLI framework, and no AI provider SDK. That constraint is
//! what lets the rest of the system stay replaceable, and it is enforced by the boundary test in
//! `aicontext-testkit`.
//!
//! # Responsibility
//!
//! Stable identifiers, the small value types shared across every layer (statuses, severities,
//! permission modes), and the common error type that other crates extend.
//!
//! # Deliberately absent
//!
//! - Filesystem access. I/O belongs in the layer that owns the concern.
//! - Time and randomness. Functions here are pure so they can be exhaustively tested.
//! - Any notion of an AI model. The context engine must not know that models exist
//!   (`docs/AI_PROVIDER_SPEC.md` §1).
//!
//! Domain types arrive in `TASK-011`.

#![forbid(unsafe_code)]
