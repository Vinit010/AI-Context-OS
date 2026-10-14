//! The provider abstraction: provider-neutral request and response types, the [`AIProvider`] trait,
//! and [`ProviderRegistry`].
//!
//! `docs/AI_PROVIDER_SPEC.md` is the contract this crate implements, and its first rule is why the
//! crate exists at all: **the context engine must not know that models exist.** `aicontext-context`
//! assembles a `ContextPacket` and stops. Turning that packet into a model call happens here, and no
//! provider's wire format appears in any type that crosses the line (`docs/AI_PROVIDER_SPEC.md` §1).
//!
//! ```text
//! ContextPacket ──▶ AIProvider ──▶ CompletionRequest
//!                         │
//!                         └─◀── CompletionResponse (text + ToolCall list + usage)
//! ```
//!
//! # What is here
//!
//! | Type | Role |
//! |------|------|
//! | [`CompletionRequest`] | What the host sends: trusted policy, messages, untrusted data, tools, budget |
//! | [`CompletionResponse`] | What comes back: text, tool calls, finish reason, usage, opaque metadata |
//! | [`AIProvider`] | The trait an adapter implements; no provider-specific type appears in it |
//! | [`ProviderRegistry`] | Owns the configured adapters and routes a request to one |
//! | [`ProviderError`] | The typed failures of a call, each with a stable code and a remediation |
//!
//! # The two regions are two types
//!
//! [`PolicyRegion`] and [`DataBlock`] are structurally separate, not a convention. Trusted
//! instructions are a [`PolicyRegion`]; external content is a [`DataBlock`], whose trust tag is
//! always [`Trust::Untrusted`]. There is no constructor that moves one into the other
//! (`docs/AI_PROVIDER_SPEC.md` §2.1, `docs/SECURITY.md` §2).
//!
//! # What this crate deliberately does not do
//!
//! - It does not write. No filesystem, no process spawn, no tool execution. A [`ToolCall`] is a
//!   proposal the gateway validates, never an instruction (`docs/AI_PROVIDER_SPEC.md` §2.2).
//! - It holds no credential. Resolving and redacting one is `TASK-052`, per
//!   `decisions/ADR-005-credential-handling.md`.
//! - It ships no adapter yet: `TASK-051` adds the OpenAI-compatible one, and adding it must change
//!   no other crate (`docs/AI_PROVIDER_SPEC.md` §12).
//!
//! # Example
//!
//! ```
//! use aicontext_providers::{DataBlock, ProviderId, ProviderRegistry, RoutingRequest, Trust};
//!
//! // A provider id is a lowercase slug, so a typo in `.ai/ai.yaml` is caught, not carried.
//! let local = ProviderId::new("local").expect("`local` is a valid id");
//!
//! // External content can only ever enter a request as an untrusted data block.
//! let block = DataBlock {
//!     source: "SPEC-context".to_owned(),
//!     path: ".ai/specs/context.md".to_owned(),
//!     tier: 4,
//!     trust: Trust::Untrusted,
//!     content: "…".to_owned(),
//! };
//! assert_eq!(block.trust, Trust::Untrusted);
//!
//! // Routing is a seam: in the MVP it answers with the configured default and nothing else.
//! let registry = ProviderRegistry::new(local.clone());
//! assert_eq!(registry.route(&RoutingRequest::default()), local);
//! ```

#![forbid(unsafe_code)]

mod error;
mod provider;
mod registry;
mod types;

// Re-exported so an adapter implements the trait and reads a stream with only this crate in scope,
// which is what "adding an adapter touches no other crate" has to mean in practice.
pub use async_trait::async_trait;
pub use futures_core::Stream;

pub use error::{ProviderError, TimeoutPhase};
pub use provider::{AIProvider, BoxStream};
pub use registry::ProviderRegistry;
pub use types::{
    Capabilities, CompletionRequest, CompletionResponse, DataBlock, FinishReason, Message,
    ModelOptions, PolicyBlock, PolicyRegion, ProviderId, Role, RoutingRequest, StreamEvent,
    TokenBudget, ToolCall, ToolSpec, Trust, Usage,
};
