//! The request the broker decides, and the audit-safe summary of it.
//!
//! The request owns the parsed arguments and the caller's reason because both are needed to render
//! the prompt, but neither is allowed into an audit record: the summary replaces them with a
//! digest (`docs/SECURITY.md` §6 and §8). The split is a type, not a convention - [`ApprovalEvent`]
//! can only carry an [`ApprovalSummary`], which has no field capable of holding either value.
//!
//! [`ApprovalEvent`]: super::ApprovalEvent

use aicontext_core::PermissionMode;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// One tool call waiting for an approval decision.
///
/// A request is plain data. It does not resolve a policy, load a manifest, or execute anything; the
/// caller has already done the resolution and carries the resulting [`PermissionMode`].
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRequest {
    tool: String,
    plugin: String,
    target: String,
    arguments: Value,
    reason: String,
    mode: PermissionMode,
}

impl ApprovalRequest {
    /// A request for `tool`, provided by `plugin`, acting on `target`.
    ///
    /// `target` is the resource named in the call, for example an instance id or a repository. An
    /// empty target is rendered as an empty string rather than omitted, so a prompt always has the
    /// same shape.
    #[must_use]
    pub fn new(
        tool: impl Into<String>,
        plugin: impl Into<String>,
        target: impl Into<String>,
        arguments: Value,
        mode: PermissionMode,
    ) -> Self {
        Self {
            tool: tool.into(),
            plugin: plugin.into(),
            target: target.into(),
            arguments,
            reason: String::new(),
            mode,
        }
    }

    /// Adds the caller's reason, which is shown to the human and never written to the audit sink.
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = reason.into();
        self
    }

    /// The capability's canonical tool name.
    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The plugin that provides the capability.
    #[must_use]
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The resource the call targets, or an empty string.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The parsed call arguments.
    #[must_use]
    pub const fn arguments(&self) -> &Value {
        &self.arguments
    }

    /// The caller's explanation, or an empty string.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The mode the permission resolver produced.
    #[must_use]
    pub const fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// The audit-safe view of this request.
    ///
    /// The arguments become `sha256:<hex>`; the reason is omitted. Both choices are deliberate:
    /// an argument can carry a credential and a reason is free-form model output, so neither
    /// belongs in a record that outlives the call (`docs/SECURITY.md` §6).
    #[must_use]
    pub fn summary(&self) -> ApprovalSummary {
        ApprovalSummary {
            tool: self.tool.clone(),
            plugin: self.plugin.clone(),
            target: self.target.clone(),
            mode: self.mode,
            arguments_digest: arguments_digest(&self.arguments),
        }
    }
}

/// What an audit record may know about a request.
///
/// The absence of an `arguments` or `reason` field is the control: a sink cannot accidentally store
/// either, because there is nowhere to put them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalSummary {
    tool: String,
    plugin: String,
    target: String,
    mode: PermissionMode,
    arguments_digest: String,
}

impl ApprovalSummary {
    /// The capability's canonical tool name.
    #[must_use]
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The plugin that provides the capability.
    #[must_use]
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The resource the call targets, or an empty string.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The mode the permission resolver produced.
    #[must_use]
    pub const fn mode(&self) -> PermissionMode {
        self.mode
    }

    /// `sha256:<hex>` over the canonical JSON form of the arguments.
    ///
    /// `serde_json::Value` stores object keys in a sorted map, so the digest is stable across two
    /// parses of the same object. It is not stable across a different JSON representation of the
    /// same value (whitespace, for example), because the bytes hashed are the compact form.
    #[must_use]
    pub fn arguments_digest(&self) -> &str {
        &self.arguments_digest
    }
}

impl From<&ApprovalRequest> for ApprovalSummary {
    fn from(request: &ApprovalRequest) -> Self {
        request.summary()
    }
}

/// The digest stored in place of the arguments.
fn arguments_digest(arguments: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(arguments.to_string().as_bytes());
    let digest = hasher.finalize();

    let mut rendered = String::with_capacity("sha256:".len() + digest.len() * 2);
    rendered.push_str("sha256:");
    for byte in digest {
        push_hex_byte(&mut rendered, byte);
    }
    rendered
}

/// Appends two lowercase hex digits, without a `hex` dependency or a fallible `write!`.
fn push_hex_byte(text: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    text.push(char::from(HEX[usize::from(byte >> 4)]));
    text.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ApprovalRequest;
    use aicontext_core::PermissionMode;

    fn request(arguments: serde_json::Value) -> ApprovalRequest {
        ApprovalRequest::new(
            "aws.ec2.stop",
            "aws",
            "i-0abc1234",
            arguments,
            PermissionMode::Approval,
        )
        .with_reason("idle overnight")
    }

    #[test]
    fn a_summary_replaces_the_arguments_with_a_sha256_digest() {
        let summary = request(json!({ "instance_id": "i-0abc1234" })).summary();

        assert_eq!(summary.tool(), "aws.ec2.stop");
        assert_eq!(summary.plugin(), "aws");
        assert_eq!(summary.target(), "i-0abc1234");
        assert_eq!(summary.mode(), PermissionMode::Approval);
        assert!(summary.arguments_digest().starts_with("sha256:"));
        assert_eq!(summary.arguments_digest().len(), "sha256:".len() + 64);
        assert!(
            summary
                .arguments_digest()
                .chars()
                .skip("sha256:".len())
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );
    }

    #[test]
    fn the_digest_does_not_depend_on_object_key_insertion_order() {
        let first = request(json!({ "instance_id": "i-0abc1234", "reason": "idle" })).summary();
        let second = request(json!({ "reason": "idle", "instance_id": "i-0abc1234" })).summary();

        assert_eq!(first.arguments_digest(), second.arguments_digest());
    }

    #[test]
    fn different_arguments_have_different_digests() {
        let first = request(json!({ "instance_id": "i-0abc1234" })).summary();
        let second = request(json!({ "instance_id": "i-0abc1235" })).summary();

        assert_ne!(first.arguments_digest(), second.arguments_digest());
    }

    #[test]
    fn the_summary_has_no_way_to_carry_the_reason_or_the_arguments() {
        // The control is structural: `ApprovalSummary` exposes neither value, so formatting it
        // cannot leak them. This test is the tripwire if a field is ever added back.
        let summary = request(json!({ "instance_id": "i-0abc1234" })).summary();
        let rendered = format!("{summary:?}");

        assert!(!rendered.contains("idle overnight"));
        assert!(!rendered.contains("instance_id"));
        assert!(rendered.contains("sha256:"));
    }

    #[test]
    fn an_empty_reason_is_an_empty_string_not_an_omission() {
        let request =
            ApprovalRequest::new("read_file", "local", "", json!({}), PermissionMode::Allow);
        assert_eq!(request.reason(), "");
        assert_eq!(request.target(), "");
    }
}
