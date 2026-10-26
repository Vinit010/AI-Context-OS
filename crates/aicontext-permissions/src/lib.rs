//! The permission policy model and resolver for AI Context OS.
//!
//! Permissions answer one question: given the policy a project committed, may this capability
//! proceed, and if so under what conditions? The vocabulary is [`aicontext_core::PermissionMode`],
//! defined once so the policy file, the resolver, the approval broker, and the audit log cannot
//! drift apart.
//!
//! # What this crate is
//!
//! - [`Policy`]: an explicit default mode plus validated exact and `provider.service.*` rules.
//! - [`Resolver`]: the pure eight-step resolution from `docs/PLUGIN_SPEC.md` §5, with a
//!   [`DecisionSource`] recording which step produced the answer.
//! - [`PolicyError`]: every way a policy can fail validation, carrying `CTX-003`.
//!
//! # What this crate is not
//!
//! There is no file loading, no approval prompt, no audit entry, and no tool execution. A policy
//! that could not be read must be represented by [`Policy::default`], which is deny-all; this
//! crate never turns an I/O failure into an allow because it performs no I/O at all.
//!
//! # Example
//!
//! ```
//! use aicontext_core::PermissionMode;
//! use aicontext_permissions::{CapabilityRequest, DecisionSource, Policy, Rule};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let policy = Policy::new(
//!     PermissionMode::Deny,
//!     [Rule::exact("read_file", PermissionMode::Allow)?],
//! )?;
//!
//! let decision = policy.resolve(CapabilityRequest::new("read_file"));
//! assert_eq!(decision.mode(), PermissionMode::Allow);
//! assert_eq!(decision.source(), DecisionSource::ExactAllow);
//!
//! let decision = policy.resolve(CapabilityRequest::new("write_file"));
//! assert_eq!(decision.mode(), PermissionMode::Deny);
//! assert_eq!(decision.source(), DecisionSource::Default);
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod error;
pub mod policy;
pub mod resolver;

pub use error::PolicyError;
pub use policy::{Policy, Rule, RuleKind};
pub use resolver::{CapabilityRequest, Decision, DecisionSource, Resolver};
