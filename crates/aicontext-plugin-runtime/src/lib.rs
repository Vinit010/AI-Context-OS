//! The plugin runtime for AI Context OS.
//!
//! The runtime is the layer between a tool call the model proposed and a plugin process that may
//! act on the world. `TASK-072` builds the part that decides whether a human approved the call;
//! `TASK-073` adds the process supervisor and `TASK-074` the single gateway that uses both.
//!
//! # What this crate is
//!
//! - [`ApprovalRequest`]: one call, with the tool, plugin, target, parsed arguments, caller's
//!   reason, and the permission mode the resolver produced.
//! - [`ApprovalBroker`]: the decision. It honours an exact `--allow-once` grant for `APPROVAL`,
//!   prompts for `APPROVAL` and `EXPLICIT_APPROVAL`, denies every non-interactive or unanswered
//!   prompt, and never lets a flag satisfy `EXPLICIT_APPROVAL`.
//! - [`Prompter`]: the seam that shows a rendered prompt and returns one line. The CLI implements
//!   it with a terminal; tests implement it with a script.
//! - [`ApprovalAudit`]: the sink that records every prompt and every decision. The durable
//!   hash-chained log is `TASK-075`; this crate only depends on the seam.
//! - [`start_plugin`]: spawns one plugin, completes the `initialize` handshake, and negotiates the
//!   plugin API version.
//! - [`PluginProcess`]: the one live pipe to that plugin. `list_tools` and `call_tool` are bounded
//!   by the manifest's timeout and output cap; any failure kills the process and poisons the pipe.
//!
//! # What this crate is not
//!
//! The approval broker has no terminal I/O, file access, or clock: the prompt is rendered as a
//! `String`, but writing it belongs to the CLI, and the audit event is described, but storing it
//! belongs to `aicontext-audit`. Those boundaries are what let the approval matrix be tested with
//! no terminal and no temporary file. The supervisor owns the process and the deadline, but not
//! the sandbox (`TASK-076`), argument schema validation, or the untrusted-result envelope
//! (`TASK-074`).
//!
//! # Default deny
//!
//! Every path that cannot reach an explicit `yes` is a denial: a prompt that cannot be shown, an
//! end of input, an interrupt, a timeout reported by the prompter, an over-long answer, and an
//! audit sink that cannot record. An approval broker that guessed would be worse than no prompt at
//! all (`docs/SECURITY.md` §9).

#![forbid(unsafe_code)]

pub mod approval;
pub mod supervisor;

pub use approval::{
    APPROVAL_ANSWER, ApprovalAudit, ApprovalBroker, ApprovalError, ApprovalEvent, ApprovalGrant,
    ApprovalOutcome, ApprovalRequest, ApprovalSummary, DenialReason, PromptError, Prompter,
    render_prompt,
};
pub use supervisor::{MAX_RESULT_DEPTH, PluginLog, PluginProcess, SupervisorError, start_plugin};
