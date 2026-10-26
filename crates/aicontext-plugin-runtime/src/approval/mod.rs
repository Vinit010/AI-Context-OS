//! The approval broker and the two seams it needs.
//!
//! `docs/PLUGIN_SPEC.md` §6 makes approval a per-invocation decision, and `docs/SECURITY.md` §9
//! makes every missing answer a denial. The broker is therefore a small state machine:
//!
//! ```text
//! ALLOW               -> allowed, no prompt
//! DENY                -> denied, no prompt
//! APPROVAL + grant    -> allowed by `--allow-once`
//! APPROVAL            -> prompt; `yes` allows, anything else denies
//! EXPLICIT_APPROVAL   -> prompt; the grant is ignored and `yes` allows this call only
//! ```
//!
//! Rendering the prompt, showing it, and writing an audit record are three separate concerns, so
//! the terminal and the audit log can be replaced without changing the decision. The CLI owns the
//! terminal; `aicontext-audit` (`TASK-075`) will own the log.
//!
//! # Example
//!
//! ```
//! use aicontext_core::PermissionMode;
//! use aicontext_plugin_runtime::approval::{
//!     ApprovalAudit, ApprovalBroker, ApprovalEvent, ApprovalRequest, PromptError, Prompter,
//! };
//! use serde_json::json;
//!
//! #[derive(Debug)]
//! struct NoTerminal;
//!
//! impl Prompter for NoTerminal {
//!     fn ask(&mut self, _prompt: &str) -> Result<String, PromptError> {
//!         Err(PromptError::Unavailable)
//!     }
//! }
//!
//! #[derive(Debug)]
//! struct Sink;
//!
//! impl ApprovalAudit for Sink {
//!     type Error = std::convert::Infallible;
//!
//!     fn record(&mut self, _event: &ApprovalEvent) -> Result<(), Self::Error> {
//!         Ok(())
//!     }
//! }
//!
//! let request = ApprovalRequest::new(
//!     "aws.ec2.stop",
//!     "aws",
//!     "i-0abc1234",
//!     json!({ "instance_id": "i-0abc1234" }),
//!     PermissionMode::Approval,
//! )
//! .with_reason("the instance is idle");
//!
//! let mut broker = ApprovalBroker::new(NoTerminal, Sink, []);
//! let outcome = broker.authorize(&request)?;
//! assert!(!outcome.is_allowed());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod broker;
mod error;
mod event;
mod prompt;
mod request;

pub use broker::ApprovalBroker;
pub use error::ApprovalError;
pub use event::{ApprovalAudit, ApprovalEvent, ApprovalGrant, ApprovalOutcome, DenialReason};
pub use prompt::{APPROVAL_ANSWER, PromptError, Prompter, render_prompt};
pub use request::{ApprovalRequest, ApprovalSummary};
