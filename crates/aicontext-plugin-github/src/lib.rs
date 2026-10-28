//! The read-only GitHub plugin for AI Context OS.
//!
//! This crate is both a library and the `aicontext-plugin-github` executable. The host starts the
//! executable, negotiates plugin API version 1, lists the seven tools in [`tools`], and calls them
//! through the one gateway path described by `docs/PLUGIN_SPEC.md`.
//!
//! # Read-only by construction
//!
//! Every capability in `manifest.json` has `effect: "read"` and `destructive: false`, and the HTTP
//! client in [`client`] exposes only `GET`. There is no shell, no filesystem write, no token
//! logging, and no second path to GitHub. The write tools in `TASK-081` will add a separate,
//! reviewable path behind approval.
//!
//! # Untrusted results
//!
//! Issue bodies, pull request bodies, commit messages, branch names, and file contents are data
//! from GitHub. Each tool result carries a top-level `trust: "untrusted"` marker and a notice, and
//! the host wraps the result in its own untrusted envelope. Nothing here treats a remote string as
//! an instruction.
//!
//! # Installing
//!
//! Build and install the binary, then register the manifest:
//!
//! ```text
//! cargo install --path crates/aicontext-plugin-github
//! aicontext plugin install crates/aicontext-plugin-github
//! ```
//!
//! The manifest requests only read capabilities. `aicontext plugin install` records the grant, but
//! every call is still resolved through `.ai/permissions/permissions.yaml`; the base template does
//! not pre-grant any `github.*` tool, so the first use asks the developer to add an explicit rule.

#![forbid(unsafe_code)]

pub mod client;
pub mod error;
pub mod server;
pub mod tools;

pub use client::{GitHubClient, PageInfo};
pub use error::GitHubError;
pub use tools::TOOL_NAMES;

/// The plugin's globally unique name.
pub const PLUGIN_NAME: &str = "github";

/// The plugin's own version, from `Cargo.toml`.
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The plugin API version this executable implements.
pub const PLUGIN_API_VERSION: &str = aicontext_plugin_sdk::PLUGIN_API_VERSION;

/// The committed manifest, embedded so the executable and its declaration cannot drift on disk.
pub const MANIFEST_JSON: &str = include_str!("../manifest.json");

/// Parse and validate the embedded manifest with the SDK's cross-field checks.
///
/// # Errors
///
/// Returns [`aicontext_plugin_sdk::PluginError`] if the embedded JSON is malformed, names a tool
/// twice, or violates a manifest rule. The executable calls this before serving, so a binary whose
/// manifest is invalid never speaks the protocol.
pub fn manifest() -> Result<aicontext_plugin_sdk::PluginManifest, aicontext_plugin_sdk::PluginError>
{
    aicontext_plugin_sdk::PluginManifest::from_json(MANIFEST_JSON)
}
