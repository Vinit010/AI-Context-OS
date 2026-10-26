//! The SDK's error type: manifest validation, framing, and version negotiation.
//!
//! A plugin author and the host both need the same answer when something is wrong: a stable code
//! (`PLG-NNN`), a message naming the concrete subject, and a remediation. The codes are defined in
//! [`aicontext_core::ErrorCode`] so the runtime added by `TASK-073` can report them without a second
//! vocabulary, and `docs/PLUGIN_SPEC.md` §4 documents the catalogue.

use aicontext_core::ErrorCode;
use thiserror::Error;

use crate::version::ApiVersion;

/// Something that went wrong while reading a manifest or a protocol frame.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginError {
    /// The manifest is not valid JSON.
    #[error("the plugin manifest is not valid JSON: {source}")]
    ManifestJson {
        /// The parser's own diagnosis, kept as the cause rather than folded into the message.
        #[source]
        source: serde_json::Error,
    },

    /// The manifest's `manifest_version` is not one this SDK understands.
    #[error(
        "the plugin manifest declares manifest_version {found}, but this SDK reads version \
         {supported}"
    )]
    UnsupportedManifestVersion {
        /// The version written in the manifest.
        found: u32,
        /// The version this SDK implements.
        supported: u32,
    },

    /// The manifest is JSON with the right shape but breaks a documented rule.
    #[error("the plugin manifest violates `{rule}`: {detail}")]
    ManifestRule {
        /// The rule that failed, named as the author would find it in `docs/PLUGIN_SPEC.md`.
        rule: &'static str,
        /// What was found, so the message names the concrete value rather than saying "invalid".
        detail: String,
    },

    /// `plugin_api_version` is not a decimal version this host can negotiate.
    #[error(
        "the plugin manifest declares plugin_api_version `{value}`, which is not a single decimal \
         version; this host supports {supported}"
    )]
    InvalidApiVersion {
        /// The rejected value, quoted so an empty string is visible.
        value: String,
        /// The version the host implements.
        supported: ApiVersion,
    },

    /// The plugin's API version is valid but this host does not implement it.
    #[error(
        "plugin API version {plugin} is not supported by this host, which supports version {host}"
    )]
    ApiVersionMismatch {
        /// The version the plugin declared.
        plugin: ApiVersion,
        /// The version the host implements.
        host: ApiVersion,
    },

    /// A protocol line is not valid JSON, or is JSON that does not match the expected shape.
    #[error("the JSON-RPC frame cannot be read: {source}")]
    FrameJson {
        /// The parser's own diagnosis.
        #[source]
        source: serde_json::Error,
    },

    /// A protocol line is JSON but not a frame this protocol allows.
    #[error("the JSON-RPC frame is invalid: {detail}")]
    FrameRule {
        /// The reason, naming the concrete violation.
        detail: String,
    },

    /// A method's params do not have the shape that method requires.
    #[error("the params for `{method}` do not match the protocol: {source}")]
    Params {
        /// The method whose params were rejected.
        method: &'static str,
        /// The deserializer's own diagnosis.
        #[source]
        source: serde_json::Error,
    },

    /// A message could not be serialized into a frame.
    #[error("the JSON-RPC frame cannot be encoded: {source}")]
    EncodeJson {
        /// The serializer's own diagnosis.
        #[source]
        source: serde_json::Error,
    },
}

impl PluginError {
    /// The stable code for this failure.
    ///
    /// Manifest failures are `PLG-011`, a version mismatch is `PLG-004`, and a frame that cannot be
    /// read is `PLG-005` (`docs/PLUGIN_SPEC.md` §4).
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::ManifestJson { .. }
            | Self::UnsupportedManifestVersion { .. }
            | Self::ManifestRule { .. }
            | Self::InvalidApiVersion { .. } => ErrorCode::PLG_011,
            Self::ApiVersionMismatch { .. } => ErrorCode::PLG_004,
            Self::FrameJson { .. }
            | Self::FrameRule { .. }
            | Self::Params { .. }
            | Self::EncodeJson { .. } => ErrorCode::PLG_005,
        }
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::ManifestJson { .. }
            | Self::UnsupportedManifestVersion { .. }
            | Self::ManifestRule { .. } => {
                "fix the plugin manifest and validate it against schemas/plugin-manifest.schema.json"
                    .to_owned()
            }
            Self::InvalidApiVersion { .. } => {
                "write plugin_api_version as a single decimal number, for example \"1\"".to_owned()
            }
            Self::ApiVersionMismatch { host, .. } => format!(
                "install a plugin built for plugin API version {host}, or upgrade aicontext to a \
                 version that supports the plugin's version"
            ),
            Self::FrameJson { .. }
            | Self::FrameRule { .. }
            | Self::Params { .. }
            | Self::EncodeJson { .. } => {
                "write exactly one JSON-RPC 2.0 object per line on stdout, with the params shape \
                 the method requires; diagnostics belong in the `log` notification"
                    .to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::PluginError;
    use crate::version::ApiVersion;

    #[test]
    fn manifest_failures_are_plg_011() {
        let error = PluginError::ManifestRule {
            rule: "name",
            detail: "`AWS` is not lowercase".to_owned(),
        };
        assert_eq!(error.code(), ErrorCode::PLG_011);
        assert!(error.remediation().contains("plugin-manifest.schema.json"));
    }

    #[test]
    fn a_version_mismatch_is_plg_004_and_names_both_versions() {
        let error = PluginError::ApiVersionMismatch {
            plugin: ApiVersion::new(2).expect("2 is a valid version"),
            host: ApiVersion::CURRENT,
        };
        assert_eq!(error.code(), ErrorCode::PLG_004);
        assert!(error.to_string().contains('2'));
        assert!(error.to_string().contains('1'));
    }

    #[test]
    fn frame_failures_are_plg_005() {
        let error = PluginError::FrameRule {
            detail: "two objects on one line".to_owned(),
        };
        assert_eq!(error.code(), ErrorCode::PLG_005);
        assert!(error.remediation().contains("log"));
    }
}
