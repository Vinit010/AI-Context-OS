//! Project knowledge: discovery, parsing, indexing, retrieval, and validation.
//!
//! This crate turns the `.ai/` directory into a typed, queryable, validated body of context, and
//! assembles a `ContextPacket` describing exactly what an agent should be told and why.
//!
//! # Storage contract
//!
//! Every document may begin with a YAML front-matter block, which is the only machine-read source
//! of its metadata. The Markdown body is the human view and is **never** parsed as structure
//! (`decisions/ADR-003-knowledge-storage-contract.md`).
//!
//! # Determinism
//!
//! Retrieval is lexical and rule-based, with a fixed priority ladder and a fully ordered tie-break.
//! No embeddings, no network, no model call. The same repository state and the same task must
//! produce the same packet (`decisions/ADR-004-deterministic-retrieval.md`).
//!
//! # Boundaries
//!
//! Depends on `aicontext-core` only. Must not depend on `aicontext-providers`: this crate assembles
//! context, it does not send it to a model.
//!
//! Modules arrive in `TASK-016` (parser), `TASK-030` (discovery), and `TASK-031` (index).

#![forbid(unsafe_code)]

mod codec;
pub mod doctor;
mod error;
mod frontmatter;
mod value;

pub use doctor::{Checked, Explanation, Finding, Inputs, Report};
pub use error::ContextError;
pub use frontmatter::{DOCUMENT_MAX_BYTES, Document, FRONT_MATTER_MAX_BYTES, FrontMatter};
pub use value::Value;
