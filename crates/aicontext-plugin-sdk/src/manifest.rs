//! The plugin manifest: `docs/PLUGIN_SPEC.md` §2 as Rust types.
//!
//! A manifest is a small, committed declaration of what a plugin is, what tools it exposes, and
//! what it may touch. It is validated against `schemas/plugin-manifest.schema.json` before anything
//! is executed, and an invalid manifest is never run.
//!
//! [`PluginManifest::from_json`] applies the rules this crate can check without a JSON Schema
//! engine — the shapes serde can express, the name and tool patterns, and the cross-field rules a
//! schema cannot state (unique tool names, for example). The host performs the full schema
//! validation at install and before execution; the SDK does not carry a runtime schema engine
//! because the installer is the only place that needs one.

use std::collections::BTreeSet;

use aicontext_core::PermissionMode;
use serde::{Deserialize, Serialize};

use crate::error::PluginError;
use crate::version::{ApiVersion, negotiate};

/// The only `manifest_version` this crate reads.
pub const MANIFEST_VERSION: u32 = 1;

/// The largest manifest this crate will parse.
///
/// A manifest is a declaration, not a document. The cap is a hostile-input guard (`RULES.md` §7),
/// not a product limit: a real manifest is a few kilobytes, and a megabyte of JSON is already a
/// denial-of-service attempt rather than a plugin.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// A plugin manifest.
///
/// The field order matches `docs/PLUGIN_SPEC.md` §2 so the documentation and the type can be read
/// side by side. Unknown fields are rejected: a manifest is versioned by `manifest_version`, and a
/// typo must not silently become a default.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Manifest format version. Must be [`MANIFEST_VERSION`].
    pub manifest_version: u32,

    /// The plugin API version, negotiated at handshake (`docs/PLUGIN_SPEC.md` §2 rule 8).
    pub plugin_api_version: String,

    /// The plugin's globally unique name, matching `^[a-z0-9][a-z0-9-]{1,63}$`.
    pub name: String,

    /// The plugin's own semantic version.
    pub version: String,

    /// A one-line description shown by `plugin list` and in approval prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Who publishes the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,

    /// The plugin's license, as an SPDX identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    /// How the host starts the plugin.
    pub entry: EntryPoint,

    /// The `SemVer` range of `aicontext` versions this plugin supports.
    ///
    /// Range resolution belongs to install (`TASK-077`); this crate checks only that the value is
    /// present and non-empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_aicontext_version: Option<String>,

    /// The tools the plugin implements. A request for a tool not listed here is refused.
    pub capabilities: Vec<Capability>,

    /// Where the plugin expects credentials to come from. A manifest never carries a value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Auth>,

    /// The outer filesystem jail. The policy file may narrow it, never widen it.
    pub filesystem: FilesystemScope,

    /// The outer network egress allowlist. An empty list means no network.
    pub network: NetworkScope,

    /// Per-call resource limits.
    pub limits: Limits,
}

impl PluginManifest {
    /// Parse a manifest and apply the checks this crate owns.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::ManifestJson`] for input that is not JSON or does not match the
    /// types, [`PluginError::UnsupportedManifestVersion`] for a future manifest format, and
    /// [`PluginError::ManifestRule`] for a broken name, tool name, `SemVer`, limit, or cross-field
    /// rule. Every failure carries `PLG-011` and a remediation.
    ///
    /// The caller should still validate the text against `schemas/plugin-manifest.schema.json`
    /// before executing the plugin; this function is the author-facing early check, not a
    /// replacement for the host's schema validation.
    pub fn from_json(raw: &str) -> Result<Self, PluginError> {
        if raw.len() > MAX_MANIFEST_BYTES {
            return Err(PluginError::ManifestRule {
                rule: "size",
                detail: format!(
                    "the manifest is {} bytes; the maximum is {MAX_MANIFEST_BYTES}",
                    raw.len()
                ),
            });
        }

        let manifest: Self =
            serde_json::from_str(raw).map_err(|source| PluginError::ManifestJson { source })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Apply the manifest rules that are not expressible as types.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::UnsupportedManifestVersion`] or
    /// [`PluginError::ManifestRule`] naming the first broken rule.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(PluginError::UnsupportedManifestVersion {
                found: self.manifest_version,
                supported: MANIFEST_VERSION,
            });
        }

        if !is_valid_plugin_name(&self.name) {
            return Err(rule(
                "name",
                format!("`{}` must match ^[a-z0-9][a-z0-9-]{{1,63}}$", self.name),
            ));
        }

        if !is_semver(&self.version) {
            return Err(rule(
                "version",
                format!(
                    "`{}` is not a SemVer MAJOR.MINOR.PATCH version",
                    self.version
                ),
            ));
        }

        // Validate the format now, but leave the decision about whether this host supports the
        // version to `negotiate`, because a mismatch is PLG-004 rather than an invalid manifest.
        ApiVersion::parse(&self.plugin_api_version)?;

        if self.entry.command.trim().is_empty() {
            return Err(rule("entry.command", "the command is empty".to_owned()));
        }
        if self.entry.command.contains('\0') {
            return Err(rule(
                "entry.command",
                "the command contains a NUL byte".to_owned(),
            ));
        }

        check_optional_text("description", self.description.as_deref())?;
        check_optional_text("vendor", self.vendor.as_deref())?;
        check_optional_text("license", self.license.as_deref())?;
        check_optional_text(
            "minimum_aicontext_version",
            self.minimum_aicontext_version.as_deref(),
        )?;

        self.validate_capabilities()?;
        self.validate_auth()?;
        self.validate_filesystem()?;
        self.validate_network()?;

        if self.limits.call_timeout_ms == 0 {
            return Err(rule(
                "limits.call_timeout_ms",
                "a per-call timeout of zero would kill every call immediately".to_owned(),
            ));
        }
        if self.limits.max_output_bytes == 0 {
            return Err(rule(
                "limits.max_output_bytes",
                "an output cap of zero would reject every result".to_owned(),
            ));
        }

        Ok(())
    }

    /// The parsed `plugin_api_version`.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidApiVersion`] if the manifest was built beyond this crate's
    /// checks.
    pub fn api_version(&self) -> Result<ApiVersion, PluginError> {
        ApiVersion::parse(&self.plugin_api_version)
    }

    /// Check this plugin against a host's API version.
    ///
    /// # Errors
    ///
    /// Returns [`PluginError::InvalidApiVersion`] for a malformed value or
    /// [`PluginError::ApiVersionMismatch`] (`PLG-004`) for a version the host does not implement.
    pub fn negotiate(&self, host: ApiVersion) -> Result<ApiVersion, PluginError> {
        let plugin = self.api_version()?;
        negotiate(plugin, host)
    }

    fn validate_capabilities(&self) -> Result<(), PluginError> {
        if self.capabilities.is_empty() {
            return Err(rule(
                "capabilities",
                "a plugin must declare at least one tool".to_owned(),
            ));
        }

        let mut seen = BTreeSet::new();
        for capability in &self.capabilities {
            if !is_valid_tool_name(&capability.tool) {
                return Err(rule(
                    "capabilities[].tool",
                    format!(
                        "`{}` is not a three-segment lowercase provider.service.action name",
                        capability.tool
                    ),
                ));
            }
            if !seen.insert(capability.tool.as_str()) {
                return Err(rule(
                    "capabilities[].tool",
                    format!("`{}` is declared more than once", capability.tool),
                ));
            }
            if capability.destructive && capability.effect != Effect::Write {
                return Err(rule(
                    "capabilities[].destructive",
                    format!(
                        "`{}` is destructive but its effect is `{}`",
                        capability.tool,
                        capability.effect.as_str()
                    ),
                ));
            }
        }

        Ok(())
    }

    fn validate_auth(&self) -> Result<(), PluginError> {
        let Some(auth) = &self.auth else {
            return Ok(());
        };
        if auth.references.is_empty() {
            return Err(rule(
                "auth.references",
                "an auth block must name at least one reference".to_owned(),
            ));
        }

        let mut seen = BTreeSet::new();
        for reference in &auth.references {
            if !is_valid_env_name(&reference.name) {
                return Err(rule(
                    "auth.references[].name",
                    format!(
                        "`{}` is not an environment-variable-style name",
                        reference.name
                    ),
                ));
            }
            if !seen.insert(reference.name.as_str()) {
                return Err(rule(
                    "auth.references[].name",
                    format!("`{}` is declared more than once", reference.name),
                ));
            }
        }

        check_optional_text("auth.instructions", auth.instructions.as_deref())
    }

    fn validate_filesystem(&self) -> Result<(), PluginError> {
        for (field, paths) in [
            ("filesystem.read", &self.filesystem.read),
            ("filesystem.write", &self.filesystem.write),
        ] {
            for path in paths {
                if path.trim().is_empty() || path.contains('\0') {
                    return Err(rule(field, format!("`{path}` is not a usable path")));
                }
            }
        }
        Ok(())
    }

    fn validate_network(&self) -> Result<(), PluginError> {
        for host in &self.network.egress {
            if host.trim().is_empty() || host.contains('\0') {
                return Err(rule(
                    "network.egress",
                    format!("`{host}` is not a usable host name"),
                ));
            }
        }
        Ok(())
    }
}

/// How the host starts the plugin process.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EntryPoint {
    /// The executable to run. Resolved by the host, outside the repository's working directory.
    pub command: String,

    /// Arguments passed verbatim, never through a shell.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// One tool a plugin implements.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    /// The tool name, `provider.service.action`.
    pub tool: String,

    /// Whether the tool reads or writes.
    pub effect: Effect,

    /// Whether the tool deletes, terminates, or overwrites something.
    ///
    /// `true` forces at least [`PermissionMode::ExplicitApproval`] regardless of the policy file;
    /// a manifest cannot lower the floor its own flag sets.
    #[serde(default)]
    pub destructive: bool,
}

impl Capability {
    /// The lowest permission this capability requires from the manifest alone.
    ///
    /// `None` means the manifest imposes no floor: the policy file still decides, and the resolver's
    /// default is [`PermissionMode::Deny`]. A destructive capability returns
    /// [`PermissionMode::ExplicitApproval`], which no flag or session grant can satisfy.
    #[must_use]
    pub const fn permission_floor(&self) -> Option<PermissionMode> {
        if self.destructive {
            Some(PermissionMode::ExplicitApproval)
        } else {
            None
        }
    }
}

/// Whether a tool reads or writes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Effect {
    /// The tool observes state and does not mutate it.
    Read,
    /// The tool may mutate state.
    Write,
}

impl Effect {
    /// The wire form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// Where a plugin expects credentials to come from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    /// The auth mechanism. The only mechanism in plugin API version 1 is an environment variable
    /// or a credential command.
    #[serde(rename = "type")]
    pub kind: AuthKind,

    /// The named sources, never the values.
    pub references: Vec<AuthReference>,

    /// Human guidance shown at install and in approval prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// The auth mechanisms plugin API version 1 defines.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthKind {
    /// Read an environment variable, or run a credential command that prints one.
    EnvOrCredentialCommand,
}

/// One named credential source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthReference {
    /// The variable or credential-command name. A name, never a value.
    pub name: String,

    /// Where the named value comes from.
    pub source: AuthSource,
}

/// Where a named credential comes from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthSource {
    /// An environment variable.
    Env,
    /// A command the host runs to obtain the value.
    CredentialCommand,
}

/// The outer filesystem jail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemScope {
    /// Paths the plugin may read.
    pub read: Vec<String>,

    /// Paths the plugin may write.
    pub write: Vec<String>,
}

/// The outer network jail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkScope {
    /// Hosts the plugin may contact. Empty means no network.
    pub egress: Vec<String>,
}

/// Per-call resource limits.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Hard per-call timeout in milliseconds.
    pub call_timeout_ms: u64,

    /// Cap on a single result in bytes.
    pub max_output_bytes: u64,
}

fn rule(rule: &'static str, detail: String) -> PluginError {
    PluginError::ManifestRule { rule, detail }
}

fn check_optional_text(field: &'static str, value: Option<&str>) -> Result<(), PluginError> {
    if let Some(value) = value {
        if value.trim().is_empty() {
            return Err(rule(field, "the value is empty".to_owned()));
        }
        if value.contains('\0') {
            return Err(rule(field, "the value contains a NUL byte".to_owned()));
        }
    }
    Ok(())
}

/// `docs/PLUGIN_SPEC.md` §2 rule 2.
fn is_valid_plugin_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if !(2..=64).contains(&bytes.len()) {
        return false;
    }
    let Some(first) = bytes.first() else {
        return false;
    };
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// `docs/PLUGIN_SPEC.md` §2 rule 3: three lowercase, dot-separated segments.
fn is_valid_tool_name(tool: &str) -> bool {
    let segments: Vec<&str> = tool.split('.').collect();
    if segments.len() != 3 {
        return false;
    }
    segments.iter().all(|segment| {
        let bytes = segment.as_bytes();
        let Some(first) = bytes.first() else {
            return false;
        };
        (first.is_ascii_lowercase() || first.is_ascii_digit())
            && bytes[1..].iter().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_'
            })
    })
}

/// The environment-variable name shape the auth vocabulary accepts.
fn is_valid_env_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    (first.is_ascii_alphabetic() || *first == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// A small `SemVer` 2.0.0 shape check: `MAJOR.MINOR.PATCH[-pre][+build]`.
///
/// Comparison and range resolution are not here. Install (`TASK-077`) resolves
/// `minimum_aicontext_version`; this crate only refuses a `version` that is not `SemVer` at all.
fn is_semver(raw: &str) -> bool {
    let (without_build, build) = match raw.split_once('+') {
        Some((version, build)) => (version, Some(build)),
        None => (raw, None),
    };
    if let Some(build) = build {
        if build.is_empty() || !is_identifier_sequence(build) {
            return false;
        }
    }

    let (core, pre) = match without_build.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (without_build, None),
    };
    if let Some(pre) = pre {
        if pre.is_empty() || !is_identifier_sequence(pre) {
            return false;
        }
    }

    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == &"0" || !part.starts_with('0'))
        })
}

fn is_identifier_sequence(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
}

#[cfg(test)]
mod tests {
    use aicontext_core::{ErrorCode, PermissionMode};

    use super::{MAX_MANIFEST_BYTES, PluginManifest};
    use crate::version::ApiVersion;

    const CANONICAL: &str = include_str!("../tests/fixtures/aws.json");

    fn canonical() -> PluginManifest {
        PluginManifest::from_json(CANONICAL).expect("the canonical manifest is valid")
    }

    #[test]
    fn the_canonical_manifest_parses_with_every_documented_field() {
        let manifest = canonical();
        assert_eq!(manifest.name, "aws");
        assert_eq!(manifest.plugin_api_version, "1");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.entry.command, "aicontext-plugin-aws");
        assert_eq!(manifest.entry.args, ["serve"]);
        assert_eq!(manifest.capabilities.len(), 4);
        assert_eq!(manifest.network.egress, ["api.ec2.us-east-1.amazonaws.com"]);
        assert_eq!(manifest.limits.call_timeout_ms, 15_000);
        assert_eq!(manifest.limits.max_output_bytes, 1_048_576);
    }

    #[test]
    fn a_destructive_capability_requires_explicit_approval() {
        let manifest = canonical();
        let terminate = manifest
            .capabilities
            .iter()
            .find(|capability| capability.tool == "aws.ec2.terminate")
            .expect("the fixture declares terminate");
        assert_eq!(
            terminate.permission_floor(),
            Some(PermissionMode::ExplicitApproval)
        );

        let describe = manifest
            .capabilities
            .iter()
            .find(|capability| capability.tool == "aws.ec2.describe")
            .expect("the fixture declares describe");
        assert_eq!(describe.permission_floor(), None);
    }

    #[test]
    fn the_canonical_manifest_round_trips_through_json() {
        let manifest = canonical();
        let encoded = serde_json::to_string(&manifest).expect("serialisable");
        let decoded = PluginManifest::from_json(&encoded).expect("round trip");
        assert_eq!(decoded, manifest);
    }

    #[test]
    fn a_manifest_for_a_newer_api_version_parses_but_negotiation_is_plg_004() {
        let manifest = canonical();
        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("valid JSON fixture");
        value["plugin_api_version"] = serde_json::json!("2");
        let future = PluginManifest::from_json(&value.to_string()).expect("2 is a valid version");

        let error = future
            .negotiate(ApiVersion::CURRENT)
            .expect_err("the host supports only 1");
        assert_eq!(error.code(), ErrorCode::PLG_004);
        assert_eq!(
            manifest
                .negotiate(ApiVersion::CURRENT)
                .expect("the host version"),
            ApiVersion::CURRENT
        );
    }

    #[test]
    fn an_uppercase_name_is_rejected() {
        let error = mutate(|value| value["name"] = serde_json::json!("AWS"));
        assert_eq!(error.code(), ErrorCode::PLG_011);
        assert!(error.to_string().contains("name"), "{error}");
    }

    #[test]
    fn a_tool_name_with_two_segments_is_rejected() {
        let error = mutate(|value| {
            value["capabilities"][0]["tool"] = serde_json::json!("aws.ec2");
        });
        assert!(error.to_string().contains("three-segment"), "{error}");
    }

    #[test]
    fn a_duplicate_tool_is_rejected() {
        let error = mutate(|value| {
            value["capabilities"][1]["tool"] = serde_json::json!("aws.ec2.describe");
        });
        assert!(error.to_string().contains("more than once"), "{error}");
    }

    #[test]
    fn a_destructive_read_is_rejected() {
        let error = mutate(|value| {
            value["capabilities"][0]["destructive"] = serde_json::json!(true);
        });
        assert!(error.to_string().contains("destructive"), "{error}");
    }

    #[test]
    fn an_empty_capability_list_is_rejected() {
        let error = mutate(|value| value["capabilities"] = serde_json::json!([]));
        assert!(error.to_string().contains("at least one tool"), "{error}");
    }

    #[test]
    fn an_unsupported_manifest_version_is_named() {
        let error = mutate(|value| value["manifest_version"] = serde_json::json!(2));
        assert!(matches!(
            error,
            crate::error::PluginError::UnsupportedManifestVersion {
                found: 2,
                supported: 1
            }
        ));
    }

    #[test]
    fn a_non_semver_version_is_rejected() {
        for version in ["1.0", "v1.0.0", "1.0.0-", "01.0.0", "1.0.0+"] {
            let error = mutate(|value| value["version"] = serde_json::json!(version));
            assert!(
                error.to_string().contains("SemVer"),
                "version = {version:?}: {error}"
            );
        }
    }

    #[test]
    fn a_valid_semver_with_prerelease_and_build_is_accepted() {
        let manifest =
            mutate_ok(|value| value["version"] = serde_json::json!("1.2.3-rc.1+build.5"));
        assert_eq!(manifest.version, "1.2.3-rc.1+build.5");
    }

    #[test]
    fn an_unknown_field_is_rejected() {
        let error = mutate(|value| value["surprise"] = serde_json::json!(true));
        assert!(
            error.to_string().contains("unknown field"),
            "the parser must name the typo: {error}"
        );
    }

    #[test]
    fn a_zero_limit_is_rejected() {
        let error = mutate(|value| value["limits"]["max_output_bytes"] = serde_json::json!(0));
        assert!(error.to_string().contains("max_output_bytes"), "{error}");
    }

    #[test]
    fn an_empty_credential_name_is_rejected() {
        let error = mutate(|value| {
            value["auth"]["references"][0]["name"] = serde_json::json!("BAD NAME");
        });
        assert!(error.to_string().contains("auth.references"), "{error}");
    }

    #[test]
    fn an_oversized_manifest_is_refused_before_parsing() {
        let raw = " ".repeat(MAX_MANIFEST_BYTES + 1);
        let error = PluginManifest::from_json(&raw).expect_err("must be refused");
        assert!(error.to_string().contains("maximum"));
    }

    #[test]
    fn an_empty_manifest_is_a_json_error() {
        let error = PluginManifest::from_json("").expect_err("must be refused");
        assert_eq!(error.code(), ErrorCode::PLG_011);
    }

    fn mutate(change: impl FnOnce(&mut serde_json::Value)) -> crate::error::PluginError {
        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("valid JSON fixture");
        change(&mut value);
        PluginManifest::from_json(&value.to_string()).expect_err("the mutation must be rejected")
    }

    fn mutate_ok(change: impl FnOnce(&mut serde_json::Value)) -> PluginManifest {
        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("valid JSON fixture");
        change(&mut value);
        PluginManifest::from_json(&value.to_string()).expect("the mutation must be accepted")
    }
}
