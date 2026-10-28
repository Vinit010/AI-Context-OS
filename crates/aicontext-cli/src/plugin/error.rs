//! The typed failures of `aicontext plugin list|install|remove`.
//!
//! One enum for the three subcommands: they read and write one lockfile and one per-plugin state
//! tree, so a failure in the reader is the same failure whichever subcommand met it (`RULES.md`
//! §4.2). Every variant names its code, its exit code, and its remediation, so no subcommand
//! improvises any of the three at a call site.
//!
//! A manifest failure is delegated rather than re-labelled: the SDK's `PluginError` already carries
//! `PLG-011` (invalid manifest) or `PLG-004` (unsupported plugin API version) and the remediation
//! that points at `schemas/plugin-manifest.schema.json`. Repeating those codes here would be a
//! second opinion about a question that already has one (`RULES.md` §2). The `PLUGIN-` codes below
//! belong to this command: reading a path, recording a grant, and taking one back.

use aicontext_plugin_sdk::PluginError;

use crate::exit::Exit;
use crate::output::Terminal;
use crate::project::ProjectError;

/// A `plugin` run that could not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PluginCommandError {
    /// The project root could not be used.
    #[error(transparent)]
    Root(#[from] ProjectError),

    /// The manifest failed the SDK's own rules or the schema bounds the SDK does not enforce.
    #[error(transparent)]
    Manifest(#[from] PluginError),

    /// The manifest file could not be read, or is not UTF-8.
    #[error("cannot read the plugin manifest {path}: {reason}")]
    ManifestRead {
        /// The path as the report spells it.
        path: String,
        /// What the filesystem or the decoder said.
        reason: String,
    },

    /// The reference names no plugin directory or manifest file this build can find.
    #[error("no plugin manifest at {path}")]
    ManifestNotFound {
        /// The path that was searched, or the reference as typed.
        path: String,
    },

    /// The reference looks like a registry name, and this build has no registry client.
    #[error(
        "`{reference}` is a registry reference, and this build installs local plugin paths only"
    )]
    RegistryUnsupported {
        /// The reference as typed.
        reference: String,
    },

    /// A human did not answer `yes` to the install preview.
    #[error("installation of `{name}` was not confirmed")]
    NotConfirmed {
        /// The plugin the preview described.
        name: String,
    },

    /// The install preview needed a human, and stdin or stderr is not a terminal.
    #[error(
        "installation of `{name}` needs an interactive terminal to confirm in, and this run has none"
    )]
    ConfirmationUnavailable {
        /// The plugin the preview described.
        name: String,
    },

    /// The lockfile exists but cannot be read.
    #[error("cannot read {path}: {reason}")]
    LockfileRead {
        /// The path as the report spells it.
        path: String,
        /// What the filesystem said.
        reason: String,
    },

    /// The lockfile is not the document this build writes.
    #[error("{path} is not a valid plugin lockfile: {detail}")]
    LockfileMalformed {
        /// The path as the report spells it.
        path: String,
        /// What was wrong, naming the concrete field or value.
        detail: String,
    },

    /// The lockfile could not be written.
    #[error("cannot write {path}: {reason}")]
    LockfileWrite {
        /// The path as the report spells it.
        path: String,
        /// What the filesystem said.
        reason: String,
    },

    /// The per-plugin state tree could not be created or written.
    #[error("cannot write the local state for `{name}` at {path}: {reason}")]
    StateWrite {
        /// The plugin whose state was being written.
        name: String,
        /// The path as the report spells it.
        path: String,
        /// What the filesystem said.
        reason: String,
    },

    /// The per-plugin state tree could not be removed.
    #[error("cannot remove the local state for `{name}` at {path}: {reason}")]
    StateRemove {
        /// The plugin whose state was being removed.
        name: String,
        /// The path as the report spells it.
        path: String,
        /// What the filesystem said.
        reason: String,
    },

    /// A state path resolved outside `.aicontext/plugins/`, so deleting it would touch something the
    /// plugin does not own.
    #[error("refusing to remove {path}: it is not inside {plugins}")]
    UnsafeStatePath {
        /// The path that resolved outside the plugin root.
        path: String,
        /// The plugin root it must stay inside.
        plugins: String,
    },

    /// `remove` was asked for a name the lockfile does not record.
    #[error("no plugin named `{name}` is installed (installed: {known})")]
    UnknownPlugin {
        /// The name that was asked for.
        name: String,
        /// The names that are recorded, or `none` when the lockfile is empty.
        known: String,
    },

    /// The JSON report itself could not be encoded or written.
    ///
    /// The command did its work and then failed to say so, which is a defect rather than a user
    /// error: there is no input that makes this happen on purpose.
    #[error("the JSON report could not be written: {detail}")]
    JsonReport {
        /// What the encoder or the output stream said.
        detail: String,
    },
}

impl PluginCommandError {
    /// The stable code, safe to branch on.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Root(error) => error.code(),
            Self::Manifest(error) => error.code().as_str(),
            Self::ManifestRead { .. } => "PLUGIN-001",
            Self::ManifestNotFound { .. } => "PLUGIN-002",
            Self::RegistryUnsupported { .. } => "PLUGIN-003",
            Self::NotConfirmed { .. } | Self::ConfirmationUnavailable { .. } => "PLUGIN-004",
            Self::LockfileRead { .. } => "PLUGIN-005",
            Self::LockfileMalformed { .. } => "PLUGIN-006",
            Self::LockfileWrite { .. } => "PLUGIN-007",
            Self::StateWrite { .. } => "PLUGIN-008",
            Self::StateRemove { .. } | Self::UnsafeStatePath { .. } => "PLUGIN-009",
            Self::UnknownPlugin { .. } => "PLUGIN-010",
            Self::JsonReport { .. } => "PLUGIN-011",
        }
    }

    /// What the developer should do about it (`RULES.md` §4.3).
    pub(crate) fn remediation(&self) -> String {
        match self {
            Self::Root(error) => error.remediation().to_owned(),
            Self::Manifest(error) => error.remediation(),
            Self::ManifestRead { .. } => {
                "check that the manifest is readable and is not held by another process".to_owned()
            }
            Self::ManifestNotFound { .. } => {
                "pass the path to a plugin directory containing `manifest.json`, or to a \
                 `manifest.json` file; this build installs local paths only"
                    .to_owned()
            }
            Self::RegistryUnsupported { .. } => {
                "install from a local path; registry installation arrives with a later task"
                    .to_owned()
            }
            Self::NotConfirmed { .. } => {
                "run the command again and answer `yes`, or pass `--yes` to confirm non-interactively"
                    .to_owned()
            }
            Self::ConfirmationUnavailable { .. } => {
                "run the command in an interactive terminal, or pass `--yes`".to_owned()
            }
            Self::LockfileRead { .. } => {
                "check that `.aicontext/plugins.lock.json` is readable; remove it only if you accept \
                 losing the installed-plugin record"
                    .to_owned()
            }
            Self::LockfileMalformed { .. } => {
                "fix or remove `.aicontext/plugins.lock.json`; `aicontext plugin list` names the \
                 malformed entry"
                    .to_owned()
            }
            Self::LockfileWrite { .. } => {
                "check that `.aicontext/` is writable and is not held by another process".to_owned()
            }
            Self::StateWrite { .. } => {
                "check that `.aicontext/plugins/` is writable and is not held by another process"
                    .to_owned()
            }
            Self::StateRemove { .. } => {
                "check that the plugin directory is writable and is not held by another process"
                    .to_owned()
            }
            Self::UnsafeStatePath { .. } => {
                "remove the directory by hand after checking it is the plugin's own; `remove` never \
                 deletes outside `.aicontext/plugins/`"
                    .to_owned()
            }
            Self::UnknownPlugin { .. } => {
                "check the name with `aicontext plugin list`".to_owned()
            }
            Self::JsonReport { .. } => {
                "this is a defect in aicontext; please report it".to_owned()
            }
        }
    }

    /// The exit code `docs/CLI_SPEC.md` §5 assigns to this failure.
    pub(crate) fn exit(&self) -> Exit {
        match self {
            Self::Root(error) => error.exit(),
            Self::Manifest(_) => Exit::Validation,
            Self::ManifestRead { .. }
            | Self::LockfileRead { .. }
            | Self::LockfileWrite { .. }
            | Self::StateWrite { .. }
            | Self::StateRemove { .. }
            | Self::UnsafeStatePath { .. }
            | Self::JsonReport { .. } => Exit::General,
            Self::ManifestNotFound { .. }
            | Self::RegistryUnsupported { .. }
            | Self::UnknownPlugin { .. } => Exit::Usage,
            Self::LockfileMalformed { .. } => Exit::Validation,
            Self::NotConfirmed { .. } | Self::ConfirmationUnavailable { .. } => {
                Exit::ApprovalRequired
            }
        }
    }

    /// Writes the report to stderr and returns the exit code, once and only once (`RULES.md` §4.5).
    pub(crate) fn report(&self, terminal: &mut Terminal) -> Exit {
        let exit = self.exit();
        terminal.error_report(self.code(), &self.to_string(), &self.remediation());
        exit
    }
}

#[cfg(test)]
mod tests {
    use aicontext_plugin_sdk::PluginError;

    use super::PluginCommandError;

    #[test]
    fn a_manifest_failure_keeps_the_sdk_code_and_remediation() {
        let error = PluginCommandError::from(PluginError::ManifestRule {
            rule: "name",
            detail: "`AWS` is not lowercase".to_owned(),
        });
        assert_eq!(error.code(), "PLG-011");
        assert_eq!(error.exit(), crate::exit::Exit::Validation);
        assert!(error.remediation().contains("plugin-manifest.schema.json"));
    }

    #[test]
    fn an_unsupported_api_version_is_plg_004_and_still_validation() {
        let error = PluginCommandError::from(PluginError::ApiVersionMismatch {
            plugin: aicontext_plugin_sdk::ApiVersion::new(2).expect("2 is a version"),
            host: aicontext_plugin_sdk::ApiVersion::CURRENT,
        });
        assert_eq!(error.code(), "PLG-004");
        assert_eq!(error.exit(), crate::exit::Exit::Validation);
    }

    #[test]
    fn the_local_codes_are_stable_and_distinct() {
        let errors = [
            PluginCommandError::ManifestRead {
                path: "x".to_owned(),
                reason: "r".to_owned(),
            },
            PluginCommandError::ManifestNotFound {
                path: "x".to_owned(),
            },
            PluginCommandError::RegistryUnsupported {
                reference: "x@1".to_owned(),
            },
            PluginCommandError::NotConfirmed {
                name: "x".to_owned(),
            },
            PluginCommandError::ConfirmationUnavailable {
                name: "x".to_owned(),
            },
            PluginCommandError::LockfileRead {
                path: "x".to_owned(),
                reason: "r".to_owned(),
            },
            PluginCommandError::LockfileMalformed {
                path: "x".to_owned(),
                detail: "d".to_owned(),
            },
            PluginCommandError::LockfileWrite {
                path: "x".to_owned(),
                reason: "r".to_owned(),
            },
            PluginCommandError::StateWrite {
                name: "x".to_owned(),
                path: "x".to_owned(),
                reason: "r".to_owned(),
            },
            PluginCommandError::StateRemove {
                name: "x".to_owned(),
                path: "x".to_owned(),
                reason: "r".to_owned(),
            },
            PluginCommandError::UnsafeStatePath {
                path: "x".to_owned(),
                plugins: "p".to_owned(),
            },
            PluginCommandError::UnknownPlugin {
                name: "x".to_owned(),
                known: "none".to_owned(),
            },
            PluginCommandError::JsonReport {
                detail: "d".to_owned(),
            },
        ];
        let mut codes: Vec<&str> = errors.iter().map(PluginCommandError::code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            11,
            "each local failure has one stable PLUGIN- code"
        );
    }

    #[test]
    fn an_unconfirmed_install_is_an_approval_failure() {
        for error in [
            PluginCommandError::NotConfirmed {
                name: "x".to_owned(),
            },
            PluginCommandError::ConfirmationUnavailable {
                name: "x".to_owned(),
            },
        ] {
            assert_eq!(error.code(), "PLUGIN-004");
            assert_eq!(error.exit().code(), 5);
        }
    }
}
