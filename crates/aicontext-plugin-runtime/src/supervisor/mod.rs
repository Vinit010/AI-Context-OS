//! The plugin process supervisor.
//!
//! `docs/PLUGIN_SPEC.md` §1 puts one JSON-RPC 2.0 pipe between the host and a plugin process, and
//! §7 makes the host responsible for the timeout, the output cap, and the kill. This module is that
//! responsibility:
//!
//! ```text
//! start_plugin(manifest) ── spawn ── initialize ── negotiate ──▶ PluginProcess
//!                                                                    │
//!                                       list_tools / call_tool ──────┤
//!                                                                    │
//!                                       shutdown (or Drop) ─────────┘
//! ```
//!
//! The SDK describes the frames and the version; this module owns the process, the clock, the byte
//! caps, and the decision to stop trusting a stream. It intentionally has no permission logic and
//! no approval prompt: those are `TASK-071`/`TASK-072`, and `TASK-074`'s gateway puts them in front
//! of every call.
//!
//! # What is deliberately not here
//!
//! The sandbox (`TASK-076`) owns the plugin's working directory, its environment, filesystem
//! allowlist, and network policy. This supervisor spawns the executable with the host's environment
//! and expects the sandbox to narrow it before a plugin is started. `TASK-074` owns argument schema
//! validation and the untrusted-result envelope; the supervisor parses results into bounded SDK
//! types but does not decide whether their content is safe to show a model.

mod error;
mod process;
mod transport;

pub use error::SupervisorError;
pub use process::{MAX_RESULT_DEPTH, PluginLog, PluginProcess, start_plugin};
