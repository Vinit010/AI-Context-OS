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
//! The parser (`TASK-016`), discovery (`TASK-030`), the content index (`TASK-031`), the retrieval
//! plan (`TASK-032`), and the lexical retriever (`TASK-033`) have arrived; the graph, filter, and
//! glob retrievers are `TASK-034`.

#![forbid(unsafe_code)]

mod body;
mod codec;
pub mod discovery;
pub mod doctor;
mod error;
mod frontmatter;
pub mod index;
pub mod register;
pub mod retrieval;
mod value;

pub use discovery::{Confidence, ProjectProfile, Signal, discover};
pub use doctor::{Checked, Explanation, Finding, Inputs, Report};
pub use error::ContextError;
pub use frontmatter::{DOCUMENT_MAX_BYTES, Document, FRONT_MATTER_MAX_BYTES, FrontMatter};
pub use index::{
    ContextIndex, DocumentKind, Heading, INDEX_PATH, IndexEntry, IndexError, ScanReport,
};
pub use register::{Register, TaskEntry};
pub use retrieval::{
    ContextBudget, DEFAULT_MAX_TOKENS, FrontMatterPredicate, LexicalRetriever,
    MANDATORY_ENTRY_POINT, MANDATORY_RULES, PlanOptions, PredicateOp, RetrievalPlan,
    RetrievalTrace, Retriever, ScoredDocument, Tier,
};
pub use value::Value;
