//! Failures of the plugin process supervisor.
//!
//! Every variant carries one of the `PLG` codes in `docs/PLUGIN_SPEC.md` §4. The code is what a
//! caller branches on; the message names the plugin, method, or limit behind the failure so a human
//! can act without reproducing the run.
//!
//! A plugin that answers with a result containing `is_error: true` is not a failure of this type:
//! the call completed and the result is data. These variants are the supervisor's own failures:
//! the process could not be started, spoke outside the protocol, stopped answering, or wrote more
//! than the manifest allows.

use std::io;

use aicontext_core::{AicontextError, ErrorCode};
use aicontext_plugin_sdk::{ApiVersion, PluginError};

/// The supervisor could not start, talk to, or stop a plugin.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SupervisorError {
    /// The manifest is invalid, or a manifest API version this host does not implement.
    ///
    /// The wrapped [`PluginError`] keeps its own code: `PLG-011` for a manifest that breaks a rule
    /// and `PLG-004` for a version the host does not support.
    #[error(transparent)]
    Manifest(#[from] PluginError),

    /// The sandbox was not prepared for this plugin, or could not be applied.
    #[error(transparent)]
    Sandbox(#[from] crate::sandbox::SandboxError),

    /// The executable named by the manifest could not be started.
    #[error("the plugin executable `{program}` could not be started: {source}")]
    Spawn {
        /// The command from `manifest.entry.command`, quoted so an empty value is visible.
        program: String,
        /// The operating system's own diagnosis.
        #[source]
        source: io::Error,
    },

    /// The plugin did not complete the `initialize` handshake.
    #[error("the plugin handshake failed: {detail}")]
    Handshake {
        /// What was wrong, naming the field or frame as concretely as possible.
        detail: String,
    },

    /// The plugin reported an API version this host does not implement.
    #[error(
        "plugin API version {plugin} is not supported by this host, which implements version {host}"
    )]
    ApiVersion {
        /// The version the plugin reported at handshake.
        plugin: ApiVersion,
        /// The version this host implements.
        host: ApiVersion,
    },

    /// A request did not complete before `limits.call_timeout_ms` elapsed.
    #[error("the plugin did not answer `{method}` within {timeout_ms} ms and was killed")]
    Timeout {
        /// The method that was waiting.
        method: &'static str,
        /// The manifest's timeout, in milliseconds.
        timeout_ms: u64,
    },

    /// The plugin wrote a frame, or a result shape, the protocol does not allow.
    #[error("the plugin sent a frame the protocol does not allow: {detail}")]
    Protocol {
        /// The concrete violation.
        detail: String,
    },

    /// The plugin produced more output than `limits.max_output_bytes`.
    #[error("the plugin exceeded its {limit}-byte output limit and was killed: {detail}")]
    OutputLimit {
        /// The cap from the manifest, in bytes.
        limit: usize,
        /// What exceeded the cap: one frame, the notification traffic, or the result's depth.
        detail: String,
    },

    /// The plugin process ended before it answered.
    #[error("the plugin process exited unexpectedly ({status})")]
    Exited {
        /// The exit code, a signal description, or the fact that the process was still running
        /// when its stdout closed.
        status: String,
    },

    /// The process was already stopped, or a previous failure poisoned it.
    #[error("the plugin process is no longer running")]
    NotRunning,

    /// Reading or writing one of the plugin's pipes failed.
    #[error("the plugin process could not be used while {operation}: {source}")]
    Io {
        /// The operation whose pipe failed.
        operation: &'static str,
        /// The operating system's own diagnosis.
        #[source]
        source: io::Error,
    },
}

impl SupervisorError {
    /// The stable code for this failure.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Manifest(error) => error.code(),
            Self::Sandbox(error) => error.code(),
            Self::Spawn { .. } => ErrorCode::PLG_001,
            Self::Handshake { .. } => ErrorCode::PLG_002,
            Self::ApiVersion { .. } => ErrorCode::PLG_004,
            Self::Timeout { .. } => ErrorCode::PLG_003,
            Self::Protocol { .. } => ErrorCode::PLG_005,
            Self::OutputLimit { .. } => ErrorCode::PLG_006,
            Self::Exited { .. } | Self::NotRunning | Self::Io { .. } => ErrorCode::PLG_007,
        }
    }

    /// What the caller should do about it.
    #[must_use]
    pub fn remediation(&self) -> String {
        match self {
            Self::Manifest(error) => error.remediation(),
            Self::Sandbox(error) => error.remediation(),
            Self::Spawn { .. } => {
                "check that the executable exists, is executable, and matches `entry.command` in \
                 the plugin manifest"
                    .to_owned()
            }
            Self::Handshake { .. } => {
                "check that the plugin answers `initialize` with `plugin_api_version` and writes \
                 exactly one JSON-RPC 2.0 response per line on stdout"
                    .to_owned()
            }
            Self::ApiVersion { host, .. } => format!(
                "install a plugin built for plugin API version {host}, or upgrade aicontext to a \
                 version that supports the plugin's version"
            ),
            Self::Timeout { .. } => {
                "raise `limits.call_timeout_ms` only if the operation is legitimately slow; \
                 otherwise fix the plugin so it answers within the limit"
                    .to_owned()
            }
            Self::Protocol { .. } => {
                "write exactly one JSON-RPC 2.0 object per line on stdout, with the params and \
                 result shape the method requires; diagnostics belong in the `log` notification"
                    .to_owned()
            }
            Self::OutputLimit { .. } => {
                "raise `limits.max_output_bytes` only if the result is legitimately large; a \
                 plugin that floods stdout is misbehaving and was killed"
                    .to_owned()
            }
            Self::Exited { .. } | Self::NotRunning | Self::Io { .. } => {
                "start the plugin again and inspect its stderr; a crash is an error, not a silent \
                 no-op"
                    .to_owned()
            }
        }
    }
}

impl From<SupervisorError> for AicontextError {
    fn from(error: SupervisorError) -> Self {
        let code = error.code();
        let remediation = error.remediation();
        Self::new(code, error.to_string(), remediation).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::io;

    use aicontext_core::ErrorCode;
    use aicontext_plugin_sdk::{ApiVersion, PluginError};

    use super::SupervisorError;
    use crate::sandbox::SandboxError;

    fn version(number: u16) -> ApiVersion {
        match ApiVersion::new(number) {
            Some(version) => version,
            None => ApiVersion::CURRENT,
        }
    }

    #[test]
    fn every_variant_reports_the_documented_code() {
        let cases = [
            (
                SupervisorError::Spawn {
                    program: "missing".to_owned(),
                    source: io::Error::other("not found"),
                },
                ErrorCode::PLG_001,
            ),
            (
                SupervisorError::Sandbox(SandboxError::Mismatch {
                    plugin: "fixture".to_owned(),
                    sandbox: "other".to_owned(),
                }),
                ErrorCode::PLG_009,
            ),
            (
                SupervisorError::Handshake {
                    detail: "no response".to_owned(),
                },
                ErrorCode::PLG_002,
            ),
            (
                SupervisorError::Timeout {
                    method: "tools/call",
                    timeout_ms: 150,
                },
                ErrorCode::PLG_003,
            ),
            (
                SupervisorError::ApiVersion {
                    plugin: version(2),
                    host: ApiVersion::CURRENT,
                },
                ErrorCode::PLG_004,
            ),
            (
                SupervisorError::Protocol {
                    detail: "not JSON".to_owned(),
                },
                ErrorCode::PLG_005,
            ),
            (
                SupervisorError::OutputLimit {
                    limit: 1024,
                    detail: "one frame".to_owned(),
                },
                ErrorCode::PLG_006,
            ),
            (
                SupervisorError::Exited {
                    status: "exit code 3".to_owned(),
                },
                ErrorCode::PLG_007,
            ),
            (SupervisorError::NotRunning, ErrorCode::PLG_007),
            (
                SupervisorError::Io {
                    operation: "read stdout",
                    source: io::Error::other("pipe closed"),
                },
                ErrorCode::PLG_007,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.code(), expected, "{error}");
            assert_eq!(error.code().to_string(), expected.to_string());
            assert!(!error.remediation().is_empty());
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn a_manifest_error_keeps_the_sdk_code() {
        let error = SupervisorError::Manifest(PluginError::FrameRule {
            detail: "two objects on one line".to_owned(),
        });
        assert_eq!(error.code(), ErrorCode::PLG_005);

        let error = SupervisorError::Manifest(PluginError::ApiVersionMismatch {
            plugin: version(2),
            host: ApiVersion::CURRENT,
        });
        assert_eq!(error.code(), ErrorCode::PLG_004);
    }

    #[test]
    fn the_conversion_keeps_the_cause_and_the_code() {
        let converted = aicontext_core::AicontextError::from(SupervisorError::OutputLimit {
            limit: 512,
            detail: "one frame".to_owned(),
        });

        assert_eq!(converted.code(), ErrorCode::PLG_006);
        assert!(converted.message().contains("512"));
        assert!(converted.source().is_some());
    }
}
