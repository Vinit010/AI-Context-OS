//! Failures of the plugin sandbox.
//!
//! Every variant is one of the ways the host can refuse to place a plugin in a well-defined
//! environment: the manifest is invalid, a declared path climbs out of the project, a declared host
//! is not a hostname, the sandbox state directory is unusable, or a requested path or host is
//! outside the allowlist. A sandbox failure is `PLG-009` (`docs/PLUGIN_SPEC.md` §4) except for an
//! invalid manifest, which keeps the SDK's own `PLG-011`/`PLG-004`.
//!
//! An error never carries an environment value. The environment is filtered before it is applied,
//! and a refusal that names a variable names the variable, not what it contained.

use std::io;

use aicontext_core::{AicontextError, ErrorCode};
use aicontext_plugin_sdk::PluginError;

/// The host could not establish, or had to refuse, a plugin's sandbox.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SandboxError {
    /// The manifest is invalid, or its API version is not one this host implements.
    #[error(transparent)]
    Manifest(#[from] PluginError),

    /// A declared filesystem path climbs out of the project or resolves outside it through a
    /// symlink.
    #[error(
        "the plugin `{plugin}` declares a {operation} path that escapes its allowed root: {detail}"
    )]
    Escape {
        /// The plugin whose manifest declared the path.
        plugin: String,
        /// `read` or `write`.
        operation: &'static str,
        /// The path and what was wrong with it.
        detail: String,
    },

    /// A declared filesystem path is not a shape the sandbox can enforce.
    #[error("the plugin `{plugin}` declares an invalid {operation} path: {detail}")]
    InvalidPath {
        /// The plugin whose manifest declared the path.
        plugin: String,
        /// `read` or `write`.
        operation: &'static str,
        /// The offending value and why it is not accepted.
        detail: String,
    },

    /// A declared network egress entry is not a bare hostname.
    #[error("the plugin `{plugin}` declares an invalid network host: {detail}")]
    InvalidHost {
        /// The plugin whose manifest declared the host.
        plugin: String,
        /// The offending value and why it is not accepted.
        detail: String,
    },

    /// A declared auth reference would replace a variable the sandbox controls.
    #[error("the plugin `{plugin}` declares an auth variable the sandbox controls: {detail}")]
    Environment {
        /// The plugin whose manifest declared the variable.
        plugin: String,
        /// The variable name and why it cannot be forwarded.
        detail: String,
    },

    /// The sandbox was prepared for a different plugin than the manifest names.
    #[error("the sandbox is for `{sandbox}`, but the manifest is for `{plugin}`")]
    Mismatch {
        /// The plugin named by the manifest.
        plugin: String,
        /// The plugin the sandbox was prepared for.
        sandbox: String,
    },

    /// The state directory, the per-plugin directory, or one of its children is unusable.
    #[error("the sandbox for `{plugin}` is unusable: {detail}")]
    Root {
        /// The plugin whose sandbox could not be prepared.
        plugin: String,
        /// What is wrong, naming the path when there is one.
        detail: String,
    },

    /// A directory or file under the sandbox could not be created or inspected.
    #[error("the sandbox for `{plugin}` could not use `{path}`: {source}")]
    Io {
        /// The plugin whose sandbox could not be prepared.
        plugin: String,
        /// The path the operation was for.
        path: String,
        /// The operating system's own diagnosis.
        #[source]
        source: io::Error,
    },

    /// A path or host outside the manifest's allowlist was requested.
    #[error(
        "the plugin `{plugin}` tried to reach `{target}`, which is outside its {operation} allowlist"
    )]
    Denied {
        /// The plugin that made the request.
        plugin: String,
        /// `read`, `write`, or `network`.
        operation: &'static str,
        /// The requested path or host.
        target: String,
    },
}

impl SandboxError {
    /// The stable code for this failure.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Manifest(error) => error.code(),
            Self::Escape { .. }
            | Self::Mismatch { .. }
            | Self::Root { .. }
            | Self::Io { .. }
            | Self::Denied { .. } => ErrorCode::PLG_009,
            Self::InvalidPath { .. } | Self::InvalidHost { .. } | Self::Environment { .. } => {
                ErrorCode::PLG_011
            }
        }
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Manifest(error) => error.remediation(),
            Self::Escape { operation, .. } => format!(
                "declare a {operation} path inside the project root, or an absolute path, with no \
                 `..` components and no symlink that leaves that root"
            ),
            Self::InvalidHost { .. } => {
                "declare a bare DNS hostname in `network.egress`: no scheme, port, path, wildcard, \
                 whitespace, or trailing dot"
                    .to_owned()
            }
            Self::InvalidPath { .. } => {
                "declare a portable path with no `..` that leaves the project, no Windows reserved \
                 device name, no alternate-data-stream colon, and no trailing dot or space"
                    .to_owned()
            }
            Self::Environment { .. } => {
                "remove the controlled variable from `auth.references`; the sandbox sets `HOME`, \
                 `USERPROFILE`, the temporary-directory variables, `PATH`, and the Windows system \
                 variables itself"
                    .to_owned()
            }
            Self::Mismatch { .. } => {
                "prepare the sandbox from the same manifest that is passed to the gateway or \
                 supervisor; a sandbox is never shared between plugins"
                    .to_owned()
            }
            Self::Root { .. } => {
                "choose a writable state directory outside the committed `.ai/` tree, and remove \
                 any symlink standing where the sandbox needs a directory"
                    .to_owned()
            }
            Self::Io { .. } => {
                "check that the state directory is writable and that no path in the sandbox is a \
                 file where a directory is required"
                    .to_owned()
            }
            Self::Denied { operation, .. } => match *operation {
                "network" => {
                    "add the host to `network.egress` in the manifest and obtain the user's approval \
                     before retrying; an empty egress list means no network"
                        .to_owned()
                }
                _ => format!(
                    "add the path to `filesystem.{operation}` in the manifest and obtain the user's \
                     approval before retrying"
                ),
            },
        }
    }
}

impl From<SandboxError> for AicontextError {
    fn from(error: SandboxError) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::SandboxError;

    #[test]
    fn every_variant_reports_the_documented_code_and_a_remediation() {
        let cases = [
            (
                SandboxError::Escape {
                    plugin: "fixture".to_owned(),
                    operation: "read",
                    detail: "`../outside` leaves the project root".to_owned(),
                },
                ErrorCode::PLG_009,
            ),
            (
                SandboxError::InvalidPath {
                    plugin: "fixture".to_owned(),
                    operation: "write",
                    detail: "`CON` is a reserved device name".to_owned(),
                },
                ErrorCode::PLG_011,
            ),
            (
                SandboxError::InvalidHost {
                    plugin: "fixture".to_owned(),
                    detail: "`https://example.com` is not a bare hostname".to_owned(),
                },
                ErrorCode::PLG_011,
            ),
            (
                SandboxError::Environment {
                    plugin: "fixture".to_owned(),
                    detail: "`HOME` is controlled by the sandbox".to_owned(),
                },
                ErrorCode::PLG_011,
            ),
            (
                SandboxError::Mismatch {
                    plugin: "fixture".to_owned(),
                    sandbox: "other".to_owned(),
                },
                ErrorCode::PLG_009,
            ),
            (
                SandboxError::Root {
                    plugin: "fixture".to_owned(),
                    detail: "the work directory is a file".to_owned(),
                },
                ErrorCode::PLG_009,
            ),
            (
                SandboxError::Io {
                    plugin: "fixture".to_owned(),
                    path: "/tmp/sandbox".to_owned(),
                    source: std::io::Error::other("denied"),
                },
                ErrorCode::PLG_009,
            ),
            (
                SandboxError::Denied {
                    plugin: "fixture".to_owned(),
                    operation: "network",
                    target: "example.com".to_owned(),
                },
                ErrorCode::PLG_009,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.code(), expected, "{error}");
            assert!(!error.remediation().is_empty(), "{error}");
            assert!(!error.to_string().is_empty(), "{error}");
        }
    }

    #[test]
    fn a_manifest_error_keeps_the_sdk_code() {
        let error = SandboxError::Manifest(aicontext_plugin_sdk::PluginError::ManifestRule {
            rule: "name",
            detail: "not a plugin name".to_owned(),
        });
        assert_eq!(error.code(), ErrorCode::PLG_011);
    }

    #[test]
    fn the_conversion_keeps_the_code_and_the_cause() {
        let converted = aicontext_core::AicontextError::from(SandboxError::Denied {
            plugin: "fixture".to_owned(),
            operation: "read",
            target: "/etc/passwd".to_owned(),
        });
        assert_eq!(converted.code(), ErrorCode::PLG_009);
        assert!(converted.source().is_some());
    }
}
