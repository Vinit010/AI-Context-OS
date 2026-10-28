//! The plugin runtime for AI Context OS.
//!
//! The runtime is the layer between a tool call the model proposed and a plugin process that may
//! act on the world. `TASK-072` builds the part that decides whether a human approved the call;
//! `TASK-073` adds the process supervisor, `TASK-074` the single gateway that uses both, and
//! `TASK-076` the sandbox that decides where the process runs and what it may reach.
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
//! - [`ToolGateway`]: the one path from a proposed call to a plugin. It validates the arguments,
//!   resolves the policy, asks the broker, dispatches once, normalises the result, and audits every
//!   stage.
//! - [`Sandbox`]: the per-plugin working tree, environment, filesystem allowlist, and network
//!   policy. It is prepared from the manifest and the host's state/project roots, and every
//!   refusal is a typed `PLG-009` (or `PLG-011` for an invalid manifest declaration).
//! - [`UntrustedToolResult`]: the only form of a plugin result the gateway returns: sanitised data
//!   marked `trust: "untrusted"`, with host-recorded metadata and plugin metadata dropped.
//!
//! # What this crate is not
//!
//! The approval broker has no terminal I/O, file access, or clock: the prompt is rendered as a
//! `String`, but writing it belongs to the CLI, and the audit event is described, but storing it
//! belongs to `aicontext-audit`. Those boundaries are what let the approval matrix be tested with
//! no terminal and no temporary file. The supervisor owns the process and the deadline; the
//! gateway owns argument validation and the untrusted-result envelope; the sandbox owns the
//! working directory, environment, and allowlists. The sandbox is not a kernel boundary: a plugin's
//! own syscalls are constrained only by the operating system, and that strength differs across
//! platforms (`docs/SECURITY.md` §12, risk R-3).
//!
//! # Default deny
//!
//! Every path that cannot reach an explicit `yes` is a denial: a prompt that cannot be shown, an
//! end of input, an interrupt, a timeout reported by the prompter, an over-long answer, and an
//! audit sink that cannot record. An approval broker that guessed would be worse than no prompt at
//! all (`docs/SECURITY.md` §9).

#![forbid(unsafe_code)]

pub mod approval;
pub mod gateway;
pub mod sandbox;
pub mod supervisor;

mod hash;
mod json;

pub use approval::{
    APPROVAL_ANSWER, ApprovalAudit, ApprovalBroker, ApprovalError, ApprovalEvent, ApprovalGrant,
    ApprovalOutcome, ApprovalRequest, ApprovalSummary, DenialReason, PromptError, Prompter,
    render_prompt,
};
pub use gateway::{
    CallSummary, EnvelopeError, GatewayAudit, GatewayError, GatewayEvent, GatewayOutcome, HostMeta,
    Refusal, RefusalStage, ResultSummary, TRUST_UNTRUSTED, ToolCall, ToolGateway,
    UntrustedToolResult,
};
pub use sandbox::{
    Environment, FilesystemPolicy, NetworkPolicy, Sandbox, SandboxError, SandboxRoots,
};
pub use supervisor::{MAX_RESULT_DEPTH, PluginLog, PluginProcess, SupervisorError, start_plugin};
