//! The installed-plugin lockfile and the permission vocabulary it records.
//!
//! `docs/PLUGIN_SPEC.md` §8 requires a local record of installed plugins and their granted
//! permissions. The record is `.aicontext/plugins.lock.json` — git-ignored local state, not a
//! committed document — because the grant belongs to this checkout and this developer, while
//! `.ai/permissions/permissions.yaml` remains the committed policy a tool call is still resolved
//! against.
//!
//! The file is deliberately plain JSON with a version field, a plugin list sorted by name, and a
//! permission list sorted by kind and identity. Two runs that install the same plugins in a
//! different order therefore produce the same bytes, which is what makes the round-trip tests
//! meaningful. A permission is a tagged value (`tool`, `filesystem`, `network`, `auth`), so an
//! unknown kind is a parse failure rather than a permission this build silently ignores.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use aicontext_plugin_sdk::{AuthSource, Effect, PluginProtocol};
use serde::{Deserialize, Serialize};

use crate::plugin::error::PluginCommandError;
use crate::project;

/// The lockfile format version this build reads and writes.
pub(crate) const LOCKFILE_VERSION: u32 = 1;

/// The file name under `.aicontext/`.
pub(crate) const LOCKFILE_NAME: &str = "plugins.lock.json";

/// The lockfile path as a human and a JSON report spells it.
pub(crate) const LOCKFILE_DISPLAY: &str = ".aicontext/plugins.lock.json";

/// The largest lockfile this build will read.
///
/// A lockfile is a record, not a document. The cap is a hostile-input guard (`RULES.md` §11): a
/// real file is a few kilobytes even with many plugins, and a multi-megabyte one is an attempt to
/// make `list` an unbounded read rather than a plugin record.
pub(crate) const MAX_LOCKFILE_BYTES: u64 = 4 * 1024 * 1024;

/// How an installed plugin was signed.
///
/// The MVP has no registry client and no signature verifier, so the only status this build can
/// honestly record is [`Unsigned`](Self::Unsigned). The type exists rather than a bare string so a
/// later verifier adds a variant without changing the lockfile's shape.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SignatureStatus {
    /// No signature was verified. Displayed as unsigned, never as trusted.
    Unsigned,
}

impl SignatureStatus {
    /// The wire form, which is also the human form.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned => "unsigned",
        }
    }
}

impl fmt::Display for SignatureStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether a filesystem permission allows reading or writing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FilesystemAccess {
    /// The plugin may read at or below the path.
    Read,
    /// The plugin may write at or below the path.
    Write,
}

impl FilesystemAccess {
    /// The wire form, which is also the human form.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// One permission granted to an installed plugin.
///
/// The variants are the four things a manifest can request: a tool, a filesystem path, a network
/// host, and a credential reference. `kind` is the tag, so an unknown value is a malformed
/// lockfile (`PLUGIN-006`) rather than an unenforced permission.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Permission {
    /// One declared capability and the effect the manifest gave it.
    Tool {
        /// The tool name, `provider.service.action`.
        tool: String,
        /// Whether the tool reads or writes.
        effect: Effect,
        /// Whether the manifest marked the tool destructive.
        destructive: bool,
    },
    /// One filesystem read or write root.
    Filesystem {
        /// Read or write.
        access: FilesystemAccess,
        /// The path as the manifest declared it.
        path: String,
    },
    /// One network egress host.
    Network {
        /// The bare host name.
        host: String,
    },
    /// One named credential source. A name, never a value.
    Auth {
        /// The environment variable or credential-command name.
        name: String,
        /// Where the value comes from.
        source: AuthSource,
    },
}

impl Permission {
    /// The stable identity of a permission, used for sorting, de-duplication, and the diff.
    ///
    /// The kind is the first component so tools sort before filesystem paths, then network hosts,
    /// then auth references. The third component distinguishes read from write for the same path;
    /// the other kinds use an empty third component. A tool's effect and destructive flag are not
    /// part of the identity because two declarations of one tool are contradictory, not two
    /// permissions.
    fn identity_key(&self) -> (u8, &str, &str) {
        match self {
            Self::Tool { tool, .. } => (0, tool, ""),
            Self::Filesystem { access, path } => (1, path, access.as_str()),
            Self::Network { host } => (2, host, ""),
            Self::Auth { name, .. } => (3, name, ""),
        }
    }

    /// Sort by kind and identity, so every writer emits the same order.
    fn sort_key(&self) -> (u8, &str, &str) {
        self.identity_key()
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tool {
                tool,
                effect,
                destructive,
            } => {
                write!(formatter, "tool `{tool}` ({})", effect.as_str())?;
                if *destructive {
                    formatter.write_str(", destructive")?;
                }
                Ok(())
            }
            Self::Filesystem { access, path } => {
                write!(formatter, "filesystem {} `{path}`", access.as_str())
            }
            Self::Network { host } => write!(formatter, "network egress `{host}`"),
            Self::Auth { name, source } => {
                write!(formatter, "auth `{name}` ({})", auth_source_name(*source))
            }
        }
    }
}

/// The wire name of an auth source, with a fallback for a variant a future SDK adds.
pub(crate) fn auth_source_name(source: AuthSource) -> &'static str {
    match source {
        AuthSource::Env => "env",
        AuthSource::CredentialCommand => "credential_command",
        _ => "unknown",
    }
}

/// One plugin recorded in the lockfile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InstalledPlugin {
    /// The plugin's globally unique name.
    pub(crate) name: String,
    /// The plugin's own `SemVer` version.
    pub(crate) version: String,
    /// The plugin API version the manifest declared.
    pub(crate) plugin_api_version: String,
    /// The wire protocol the manifest declared.
    ///
    /// Omitted in records written before MCP registration existed, where the only protocol was
    /// `aiplugin`; the default keeps those records readable rather than calling them malformed.
    #[serde(default, skip_serializing_if = "PluginProtocol::is_default")]
    pub(crate) protocol: PluginProtocol,
    /// The manifest's one-line description, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
    /// The manifest's `minimum_aicontext_version`, recorded but not resolved by this build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) minimum_aicontext_version: Option<String>,
    /// Lowercase hex SHA-256 of the manifest bytes that were validated.
    pub(crate) manifest_digest: String,
    /// The local manifest path the plugin was installed from.
    pub(crate) source: String,
    /// The signature status shown at install. Always `unsigned` in this build.
    pub(crate) signature: SignatureStatus,
    /// Every permission the developer confirmed, sorted by [`Permission::sort_key`].
    pub(crate) permissions: Vec<Permission>,
}

/// The local record of installed plugins (`docs/PLUGIN_SPEC.md` §8).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lockfile {
    /// The lockfile format version; must be [`LOCKFILE_VERSION`].
    pub(crate) lockfile_version: u32,
    /// The installed plugins, sorted by name on save.
    #[serde(default)]
    pub(crate) plugins: Vec<InstalledPlugin>,
}

impl Lockfile {
    /// An empty record, which is what a project with no lockfile has.
    #[must_use]
    pub(crate) fn empty() -> Self {
        Self {
            lockfile_version: LOCKFILE_VERSION,
            plugins: Vec::new(),
        }
    }

    /// The lockfile's absolute path under `root`.
    #[must_use]
    pub(crate) fn path(root: &Path) -> PathBuf {
        root.join(".aicontext").join(LOCKFILE_NAME)
    }

    /// Load the lockfile, treating an absent file as an empty record.
    ///
    /// An absent lockfile is not damage: a project that has installed nothing is in the state the
    /// file would describe. A file that exists but cannot be read or parsed is an error rather than
    /// an empty record, because reporting "nothing is installed" for a corrupt record would hide
    /// the plugins whose permissions are still on disk.
    pub(crate) fn load(root: &Path) -> Result<Self, PluginCommandError> {
        let path = Self::path(root);
        let spelled = project::display(&path);
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::empty());
            }
            Err(error) => {
                return Err(PluginCommandError::LockfileRead {
                    path: spelled,
                    reason: error.to_string(),
                });
            }
        };
        if metadata.len() > MAX_LOCKFILE_BYTES {
            return Err(PluginCommandError::LockfileMalformed {
                path: spelled,
                detail: format!(
                    "it is {} bytes; the maximum is {MAX_LOCKFILE_BYTES}",
                    metadata.len()
                ),
            });
        }
        let bytes = fs::read(&path).map_err(|error| PluginCommandError::LockfileRead {
            path: spelled.clone(),
            reason: error.to_string(),
        })?;
        let lockfile: Self = serde_json::from_slice(&bytes).map_err(|error| {
            PluginCommandError::LockfileMalformed {
                path: spelled.clone(),
                detail: error.to_string(),
            }
        })?;
        lockfile.validate(&spelled)?;
        Ok(lockfile)
    }

    /// Check the parts of the document serde cannot: the version, portable unique names, digests,
    /// and unique permission identities.
    fn validate(&self, path: &str) -> Result<(), PluginCommandError> {
        if self.lockfile_version != LOCKFILE_VERSION {
            return Err(PluginCommandError::LockfileMalformed {
                path: path.to_owned(),
                detail: format!(
                    "lockfile_version is {}; this build reads {LOCKFILE_VERSION}",
                    self.lockfile_version
                ),
            });
        }

        let mut names = BTreeSet::new();
        for plugin in &self.plugins {
            if !is_portable_plugin_name(&plugin.name) {
                return Err(PluginCommandError::LockfileMalformed {
                    path: path.to_owned(),
                    detail: format!(
                        "`{}` is not a portable plugin name (two to sixty-four lowercase letters, \
                         digits, or hyphens, and not a Windows reserved device name)",
                        plugin.name
                    ),
                });
            }
            if !names.insert(plugin.name.as_str()) {
                return Err(PluginCommandError::LockfileMalformed {
                    path: path.to_owned(),
                    detail: format!("`{}` is recorded more than once", plugin.name),
                });
            }
            if plugin.version.trim().is_empty() {
                return Err(PluginCommandError::LockfileMalformed {
                    path: path.to_owned(),
                    detail: format!("`{}` has no version", plugin.name),
                });
            }
            if plugin.plugin_api_version.trim().is_empty() {
                return Err(PluginCommandError::LockfileMalformed {
                    path: path.to_owned(),
                    detail: format!("`{}` has no plugin_api_version", plugin.name),
                });
            }
            if plugin.source.trim().is_empty() {
                return Err(PluginCommandError::LockfileMalformed {
                    path: path.to_owned(),
                    detail: format!("`{}` has no source", plugin.name),
                });
            }
            if !is_sha256_hex(&plugin.manifest_digest) {
                return Err(PluginCommandError::LockfileMalformed {
                    path: path.to_owned(),
                    detail: format!(
                        "`{}` has a manifest_digest that is not 64 lowercase hex characters",
                        plugin.name
                    ),
                });
            }
            validate_permissions(path, &plugin.name, &plugin.permissions)?;
        }

        Ok(())
    }

    /// The entry for `name`, if it is recorded.
    #[must_use]
    pub(crate) fn find(&self, name: &str) -> Option<&InstalledPlugin> {
        self.plugins.iter().find(|plugin| plugin.name == name)
    }

    /// Insert or replace one plugin, keeping the list sorted by name.
    pub(crate) fn upsert(&mut self, plugin: InstalledPlugin) {
        match self
            .plugins
            .iter_mut()
            .find(|existing| existing.name == plugin.name)
        {
            Some(existing) => *existing = plugin,
            None => self.plugins.push(plugin),
        }
        self.plugins
            .sort_by(|left, right| left.name.cmp(&right.name));
    }

    /// Remove one plugin and return what was recorded, or `None` when it was absent.
    pub(crate) fn remove(&mut self, name: &str) -> Option<InstalledPlugin> {
        let index = self.plugins.iter().position(|plugin| plugin.name == name)?;
        Some(self.plugins.remove(index))
    }

    /// The recorded names as a comma-separated list, or `none`.
    #[must_use]
    pub(crate) fn known_names(&self) -> String {
        if self.plugins.is_empty() {
            return "none".to_owned();
        }
        self.plugins
            .iter()
            .map(|plugin| plugin.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Write the lockfile deterministically, creating `.aicontext/` if needed.
    pub(crate) fn save(&self, root: &Path) -> Result<(), PluginCommandError> {
        let path = Self::path(root);
        let spelled = project::display(&path);

        let mut sorted = self.clone();
        sorted
            .plugins
            .sort_by(|left, right| left.name.cmp(&right.name));
        for plugin in &mut sorted.plugins {
            sort_permissions(&mut plugin.permissions);
        }

        let mut text = serde_json::to_string_pretty(&sorted).map_err(|error| {
            PluginCommandError::LockfileWrite {
                path: spelled.clone(),
                reason: format!("the record could not be encoded: {error}"),
            }
        })?;
        text.push('\n');

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| PluginCommandError::LockfileWrite {
                path: spelled.clone(),
                reason: error.to_string(),
            })?;
        }
        fs::write(&path, text).map_err(|error| PluginCommandError::LockfileWrite {
            path: spelled,
            reason: error.to_string(),
        })
    }

    /// Delete the lockfile after the last plugin is removed.
    pub(crate) fn delete(root: &Path) -> Result<(), PluginCommandError> {
        let path = Self::path(root);
        let spelled = project::display(&path);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(PluginCommandError::LockfileWrite {
                path: spelled,
                reason: error.to_string(),
            }),
        }
    }
}

/// Sort permissions into the one deterministic order every writer uses.
pub(crate) fn sort_permissions(permissions: &mut [Permission]) {
    permissions.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
}

/// Reject a lockfile that records two permissions with the same identity.
fn validate_permissions(
    path: &str,
    plugin: &str,
    permissions: &[Permission],
) -> Result<(), PluginCommandError> {
    let mut seen = BTreeSet::new();
    for permission in permissions {
        if !seen.insert(permission.identity_key()) {
            return Err(PluginCommandError::LockfileMalformed {
                path: path.to_owned(),
                detail: format!("`{plugin}` records `{permission}` more than once"),
            });
        }
    }
    Ok(())
}

/// Whether a name is safe to use as one directory component on every platform.
///
/// This repeats the SDK's name pattern because a lockfile is read from disk and an edited file is
/// untrusted input. The Windows device names are refused on every platform so a lockfile written on
/// Linux cannot become an undeletable directory on Windows.
#[must_use]
pub(crate) fn is_portable_plugin_name(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];

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
    if !bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return false;
    }
    !RESERVED.contains(&name)
}

/// Whether `value` is a lowercase hex SHA-256 digest.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The diff between the permissions already granted and the ones a manifest now requests.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub(crate) struct PermissionDiff {
    /// Requested now, not granted before.
    pub(crate) added: Vec<Permission>,
    /// Granted before, not requested now. Installing revokes them.
    pub(crate) removed: Vec<Permission>,
    /// Granted before and still requested.
    pub(crate) unchanged: Vec<Permission>,
}

impl PermissionDiff {
    /// Compare the permissions already granted with the ones a manifest requests.
    #[must_use]
    pub(crate) fn between(granted: &[Permission], requested: &[Permission]) -> Self {
        let mut granted = granted.to_vec();
        let mut requested = requested.to_vec();
        sort_permissions(&mut granted);
        sort_permissions(&mut requested);

        let added = requested
            .iter()
            .filter(|permission| !granted.contains(*permission))
            .cloned()
            .collect();
        let removed = granted
            .iter()
            .filter(|permission| !requested.contains(*permission))
            .cloned()
            .collect();
        let unchanged = requested
            .iter()
            .filter(|permission| granted.contains(*permission))
            .cloned()
            .collect();

        Self {
            added,
            removed,
            unchanged,
        }
    }

    /// Whether the diff changes the grant at all.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// One line a human can read before the detailed rows.
    #[must_use]
    pub(crate) fn summary(&self) -> String {
        format!(
            "{} added, {} removed, {} unchanged",
            self.added.len(),
            self.removed.len(),
            self.unchanged.len()
        )
    }
}

/// The absolute directory holding every installed plugin's local state.
#[must_use]
pub(crate) fn plugins_root(root: &Path) -> PathBuf {
    root.join(".aicontext").join("plugins")
}

/// Create the per-plugin state directory, refuse a redirected one, and write the manifest copy.
///
/// The check is the same one `remove` makes: the canonical per-plugin directory must be exactly
/// `plugins_root/<name>`. A symlink or junction that points somewhere else is refused before
/// anything is written through it, and the name is a single validated component, so the path cannot
/// climb out of `.aicontext/plugins/`.
pub(crate) fn write_state(
    root: &Path,
    name: &str,
    manifest: &[u8],
) -> Result<PathBuf, PluginCommandError> {
    let plugins = plugins_root(root);
    fs::create_dir_all(&plugins).map_err(|error| PluginCommandError::StateWrite {
        name: name.to_owned(),
        path: project::display(&plugins),
        reason: error.to_string(),
    })?;
    let plugins = plugins
        .canonicalize()
        .map_err(|error| PluginCommandError::StateWrite {
            name: name.to_owned(),
            path: project::display(&plugins_root(root)),
            reason: error.to_string(),
        })?;
    let expected = plugins.join(name);

    let exists = match fs::symlink_metadata(&expected) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(PluginCommandError::StateWrite {
                name: name.to_owned(),
                path: project::display(&expected),
                reason: error.to_string(),
            });
        }
    };

    if exists {
        let canonical =
            expected
                .canonicalize()
                .map_err(|error| PluginCommandError::StateWrite {
                    name: name.to_owned(),
                    path: project::display(&expected),
                    reason: error.to_string(),
                })?;
        if canonical != expected {
            return Err(PluginCommandError::UnsafeStatePath {
                path: project::display(&canonical),
                plugins: project::display(&plugins),
            });
        }
    } else {
        fs::create_dir(&expected).map_err(|error| PluginCommandError::StateWrite {
            name: name.to_owned(),
            path: project::display(&expected),
            reason: error.to_string(),
        })?;
        let canonical =
            expected
                .canonicalize()
                .map_err(|error| PluginCommandError::StateWrite {
                    name: name.to_owned(),
                    path: project::display(&expected),
                    reason: error.to_string(),
                })?;
        if canonical != expected {
            return Err(PluginCommandError::UnsafeStatePath {
                path: project::display(&canonical),
                plugins: project::display(&plugins),
            });
        }
    }

    let copy = expected.join("manifest.json");
    fs::write(&copy, manifest).map_err(|error| PluginCommandError::StateWrite {
        name: name.to_owned(),
        path: project::display(&copy),
        reason: error.to_string(),
    })?;
    Ok(copy)
}

/// Delete the per-plugin state directory, refusing a path that resolves outside the plugin root.
///
/// Returns whether anything existed to delete. A missing directory is not an error: the lockfile
/// entry is still the record of the grant, and removing it is the point of the command.
pub(crate) fn remove_state(root: &Path, name: &str) -> Result<bool, PluginCommandError> {
    let plugins = plugins_root(root);
    match fs::symlink_metadata(&plugins) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(PluginCommandError::StateRemove {
                name: name.to_owned(),
                path: project::display(&plugins),
                reason: error.to_string(),
            });
        }
    }
    let plugins = plugins
        .canonicalize()
        .map_err(|error| PluginCommandError::StateRemove {
            name: name.to_owned(),
            path: project::display(&plugins),
            reason: error.to_string(),
        })?;
    let expected = plugins.join(name);

    match fs::symlink_metadata(&expected) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(PluginCommandError::StateRemove {
                name: name.to_owned(),
                path: project::display(&expected),
                reason: error.to_string(),
            });
        }
    }
    let canonical = expected
        .canonicalize()
        .map_err(|error| PluginCommandError::StateRemove {
            name: name.to_owned(),
            path: project::display(&expected),
            reason: error.to_string(),
        })?;
    if canonical != expected {
        return Err(PluginCommandError::UnsafeStatePath {
            path: project::display(&canonical),
            plugins: project::display(&plugins),
        });
    }

    fs::remove_dir_all(&expected).map_err(|error| PluginCommandError::StateRemove {
        name: name.to_owned(),
        path: project::display(&expected),
        reason: error.to_string(),
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use aicontext_plugin_sdk::{AuthSource, Effect, PluginProtocol};
    use tempfile::TempDir;

    use super::{
        FilesystemAccess, InstalledPlugin, LOCKFILE_VERSION, Lockfile, Permission, PermissionDiff,
        SignatureStatus, is_portable_plugin_name, remove_state, write_state,
    };

    fn permission() -> Permission {
        Permission::Tool {
            tool: "demo.items.read".to_owned(),
            effect: Effect::Read,
            destructive: false,
        }
    }

    fn plugin(name: &str) -> InstalledPlugin {
        InstalledPlugin {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            plugin_api_version: "1".to_owned(),
            protocol: PluginProtocol::AiPlugin,
            description: Some("A demo".to_owned()),
            minimum_aicontext_version: None,
            manifest_digest: "a".repeat(64),
            source: "/plugins/demo/manifest.json".to_owned(),
            signature: SignatureStatus::Unsigned,
            permissions: vec![permission()],
        }
    }

    #[test]
    fn a_lockfile_round_trips_and_sorts_plugins_and_permissions() {
        let root = TempDir::new().expect("temp dir");
        let mut lockfile = Lockfile::empty();
        lockfile.upsert(plugin("zebra"));
        lockfile.upsert(plugin("alpha"));
        lockfile.plugins[0].permissions.push(Permission::Network {
            host: "api.example.com".to_owned(),
        });
        lockfile.plugins[0]
            .permissions
            .push(Permission::Filesystem {
                access: FilesystemAccess::Write,
                path: "out".to_owned(),
            });
        lockfile.plugins[0]
            .permissions
            .push(Permission::Filesystem {
                access: FilesystemAccess::Read,
                path: "src".to_owned(),
            });
        lockfile.plugins[0].permissions.push(Permission::Auth {
            name: "DEMO_TOKEN".to_owned(),
            source: AuthSource::Env,
        });
        lockfile.save(root.path()).expect("save");

        let text = fs::read_to_string(Lockfile::path(root.path())).expect("read");
        assert!(text.ends_with('\n'));
        assert!(
            text.find("\"alpha\"").expect("alpha") < text.find("\"zebra\"").expect("zebra"),
            "plugins are sorted by name: {text}"
        );

        let loaded = Lockfile::load(root.path()).expect("load");
        assert_eq!(loaded.plugins[0].name, "alpha");
        assert_eq!(loaded.plugins[1].name, "zebra");
        let kinds: Vec<&str> = loaded.plugins[0]
            .permissions
            .iter()
            .map(|permission| match permission {
                Permission::Tool { .. } => "tool",
                Permission::Filesystem { .. } => "filesystem",
                Permission::Network { .. } => "network",
                Permission::Auth { .. } => "auth",
            })
            .collect();
        assert_eq!(
            kinds,
            ["tool", "filesystem", "filesystem", "network", "auth"]
        );
    }

    #[test]
    fn an_absent_lockfile_is_empty_and_a_malformed_one_is_an_error() {
        let root = TempDir::new().expect("temp dir");
        assert_eq!(
            Lockfile::load(root.path()).expect("absent is empty"),
            Lockfile::empty()
        );

        fs::create_dir_all(root.path().join(".aicontext")).expect("create");
        fs::write(Lockfile::path(root.path()), "not json").expect("write");
        let error = Lockfile::load(root.path()).expect_err("must refuse");
        assert_eq!(error.code(), "PLUGIN-006");
    }

    #[test]
    fn an_unknown_permission_kind_is_refused_rather_than_ignored() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".aicontext")).expect("create");
        let text = format!(
            r#"{{
  "lockfile_version": {LOCKFILE_VERSION},
  "plugins": [
    {{
      "name": "demo",
      "version": "1.0.0",
      "plugin_api_version": "1",
      "manifest_digest": "{}",
      "source": "/plugins/demo/manifest.json",
      "signature": "unsigned",
      "permissions": [{{"kind": "kernel", "module": "vmlinux"}}]
    }}
  ]
}}"#,
            "a".repeat(64)
        );
        fs::write(Lockfile::path(root.path()), text).expect("write");

        let error = Lockfile::load(root.path()).expect_err("must refuse");
        assert_eq!(error.code(), "PLUGIN-006");
        assert!(
            error.to_string().contains("kernel"),
            "the message must name the unknown kind: {error}"
        );
    }

    #[test]
    fn a_wrong_version_and_duplicate_names_are_refused() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".aicontext")).expect("create");

        let mut lockfile = Lockfile::empty();
        lockfile.lockfile_version = 2;
        fs::write(
            Lockfile::path(root.path()),
            serde_json::to_string(&lockfile).expect("encode"),
        )
        .expect("write");
        assert_eq!(
            Lockfile::load(root.path()).expect_err("version").code(),
            "PLUGIN-006"
        );

        let mut lockfile = Lockfile::empty();
        lockfile.plugins = vec![plugin("demo"), plugin("demo")];
        fs::write(
            Lockfile::path(root.path()),
            serde_json::to_string(&lockfile).expect("encode"),
        )
        .expect("write");
        let error = Lockfile::load(root.path()).expect_err("duplicate");
        assert_eq!(error.code(), "PLUGIN-006");
        assert!(error.to_string().contains("more than once"), "{error}");
    }

    #[test]
    fn a_name_that_is_not_portable_is_refused() {
        for name in [
            "con",
            "nul",
            "com1",
            "UPPER",
            "../escape",
            "a",
            "with space",
        ] {
            assert!(!is_portable_plugin_name(name), "{name} must be refused");
        }
        for name in ["aws", "demo-plugin", "a1", "plugin-2"] {
            assert!(is_portable_plugin_name(name), "{name} must be accepted");
        }
    }

    #[test]
    fn the_diff_separates_added_removed_and_unchanged() {
        let granted = vec![
            permission(),
            Permission::Network {
                host: "old.example.com".to_owned(),
            },
        ];
        let requested = vec![
            permission(),
            Permission::Filesystem {
                access: FilesystemAccess::Read,
                path: "src".to_owned(),
            },
        ];

        let diff = PermissionDiff::between(&granted, &requested);
        assert_eq!(diff.unchanged, vec![permission()]);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.added.len(), 1);
        assert!(matches!(diff.removed[0], Permission::Network { .. }));
        assert!(matches!(diff.added[0], Permission::Filesystem { .. }));
        assert_eq!(diff.summary(), "1 added, 1 removed, 1 unchanged");
    }

    #[test]
    fn write_and_remove_state_keep_the_manifest_copy_and_delete_it() {
        let root = TempDir::new().expect("temp dir");
        let copy = write_state(root.path(), "demo", b"{\"name\":\"demo\"}").expect("write");
        let canonical_root = root.path().canonicalize().expect("canonical root");
        assert!(copy.starts_with(&canonical_root), "{copy:?}");
        assert_eq!(
            copy.file_name().and_then(|name| name.to_str()),
            Some("manifest.json")
        );
        assert_eq!(
            fs::read(&copy).expect("read"),
            b"{\"name\":\"demo\"}".to_vec()
        );

        assert!(remove_state(root.path(), "demo").expect("remove"));
        assert!(!copy.exists());
        assert!(
            !remove_state(root.path(), "demo").expect("missing is not an error"),
            "nothing remained to delete"
        );
    }

    #[test]
    fn removing_an_absent_plugin_root_does_nothing() {
        let root = TempDir::new().expect("temp dir");
        assert!(!remove_state(root.path(), "demo").expect("nothing to remove"));
    }
}
