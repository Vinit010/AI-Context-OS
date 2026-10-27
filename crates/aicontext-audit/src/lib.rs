//! Hash-chained, append-only audit log (`docs/SECURITY.md` §8).
//!
//! Every consequential action the product takes is written here as one JSON object per line. Each
//! record carries `seq`, `prev` (the digest of the record before it), and `self` (the digest of its
//! own canonical bytes without `self`). Reading verifies the whole chain before returning a single
//! record, and appending verifies it before writing, so a damaged log is reported rather than
//! extended.
//!
//! # What the chain does and does not prove
//!
//! A hash chain resists accidental modification and partial alteration. It does **not** resist an
//! attacker with write access to the machine, who can recompute every digest after editing a record.
//! Local-only storage cannot make a stronger claim, and this crate does not make one
//! (`docs/SECURITY.md` §8, risk R-5).
//!
//! The practical mitigation is witnessing: [`Verification::head`] is the digest of the newest
//! verified record, and `aicontext audit verify` prints it so a reviewer can record it somewhere the
//! attacker does not control. A later rewrite then disagrees with the recorded head.
//!
//! # The record
//!
//! ```json
//! {
//!   "seq": 1,
//!   "ts": "2026-09-27T14:53:06.412Z",
//!   "event": "completed",
//!   "actor": { "kind": "agent", "id": "developer", "provider": "openai-compatible", "model": "…" },
//!   "action": { "tool": "aws.ec2.stop", "plugin": "aws", "effect": "write" },
//!   "resource": "i-0abc1234",
//!   "arguments_digest": "sha256:…",
//!   "approval": { "required": "approval", "granted_by": "developer", "mode": "interactive" },
//!   "result": { "status": "success", "duration_ms": 412, "digest": "sha256:…" },
//!   "prev": "sha256:0000…",
//!   "self": "sha256:…"
//! }
//! ```
//!
//! Argument values are never part of the shape: [`AuditEntry`] has an argument *digest* and no
//! argument *value*. That is a type-level guarantee, not a convention a caller has to remember
//! (`docs/SECURITY.md` §6).
//!
//! # Determinism
//!
//! `self` is SHA-256 over the compact JSON of the record with sorted object keys and `self`
//! removed. Key order and whitespace in the file therefore do not change the digest, while any
//! change to a value, the sequence number, or `prev` does.
//!
//! [`Timestamp`] renders UTC with exactly three fractional digits, for example
//! `2026-09-27T14:53:06.412Z`. Wall-clock time is supplied by the caller through [`AuditLog::append`]
//! (or a fixed value through [`AuditLog::append_at`]), so a test can produce the same record twice.

#![deny(missing_docs)]

mod error;
mod hash;
mod log;
mod record;
mod timestamp;

pub use error::{AuditError, AuditProblem};
pub use hash::GENESIS;
pub use log::{AuditLog, Chain, Verification, DEFAULT_DIR, DEFAULT_FILE, MAX_LOG_BYTES};
pub use record::{Action, Actor, Approval, AuditEntry, AuditRecord, ResultSummary};
pub use timestamp::{InvalidTimestamp, Timestamp};
