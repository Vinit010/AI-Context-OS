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
//! # Layout
//!
//! Three private modules, one re-exported surface:
//!
//! - **identifiers** — [`TaskId`], [`DecisionId`], [`BugId`], [`ChangeId`], [`DocumentId`],
//!   [`InvalidId`]
//! - **errors** — [`ErrorCode`], [`ErrorFamily`], [`Severity`], [`AicontextError`]
//! - **permissions** — [`PermissionMode`], [`UnknownPermissionMode`]
//!
//! # Example
//!
//! ```
//! use aicontext_core::{AicontextError, ErrorCode, PermissionMode, TaskId};
//!
//! // Identifiers are validated once, at the boundary, and are then total.
//! let id = TaskId::new("TASK-014").expect("a task ID from the register");
//! assert_eq!(id.as_str(), "TASK-014");
//!
//! // An unmapped capability is denied without an explicit branch anywhere.
//! assert_eq!(PermissionMode::default(), PermissionMode::Deny);
//!
//! // Every failure a caller can act on arrives the same way.
//! let error = AicontextError::new(
//!     ErrorCode::CTX_007,
//!     format!("{id} references SPEC-auth-missing, which does not exist"),
//!     "create .ai/specs/auth-missing.md or fix the reference",
//! );
//! assert_eq!(error.to_string(), "CTX-007: TASK-014 references SPEC-auth-missing, which does not exist");
//! ```

#![forbid(unsafe_code)]

mod error;
mod id;
mod permission;

pub use error::{AicontextError, ErrorCode, ErrorFamily, Severity};
pub use id::{BugId, ChangeId, DecisionId, DocumentId, InvalidId, TaskId};
pub use permission::{PermissionMode, UnknownPermissionMode};
