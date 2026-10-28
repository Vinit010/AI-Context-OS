//! Resolving a local plugin reference, validating its manifest, and extracting its permissions.
//!
//! `docs/PLUGIN_SPEC.md` §8 fixes the order: validate the manifest, show the permission diff, show
//! the auth requirements, confirm, then record. This module owns the first step. It reads a
//! directory or a `manifest.json` file, applies the SDK's own rules, then applies the schema bounds
//! the SDK cannot express without a JSON Schema engine — maximum lengths and `uniqueItems`. The
//! `jsonschema` crate stays a dev-dependency: the installer needs the rules, not a second parser.
//!
//! A registry reference is refused by name. This build has no registry client, so accepting one
//! would mean inventing a download that cannot happen; the refusal says so instead of reporting a
//! missing local path as if the developer mistyped it.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use aicontext_plugin_sdk::{
    ApiVersion, MAX_MANIFEST_BYTES, PluginError, PluginManifest,
};

use crate::archive::sha256_hex;
use crate::plugin::error::PluginCommandError;
use crate::plugin::lockfile::{
    FilesystemAccess, InstalledPlugin, Permission, SignatureStatus, auth_source_name,
    is_portable_plugin_name, sort_permissions,
};
use crate::project;

/// A manifest that passed every check install makes.
#[derive(Debug)]
pub(crate) struct ValidatedManifest {
    /// The parsed declaration.
    pub(crate) manifest: PluginManifest,
    /// The exact bytes that were validated, so the copy on disk is the copy that was checked.
    pub(crate) bytes: Vec<u8>,
    /// Lowercase hex SHA-256 of [`bytes`](Self::bytes).
    pub(crate) digest: String,
    /// The manifest path as a report spells it.
    pub(crate) source: String,
    /// Every permission the manifest requests, sorted deterministically.
    pub(crate) permissions: Vec<Permission>,
}

impl ValidatedManifest {
    /// Load and validate the manifest named by `reference`, resolved against `root`.
    pub(crate) fn load(root: &Path, reference: &str) -> Result<Self, PluginCommandError> {
        let path = resolve(root, reference)?;
        let bytes = read(&path)?;
        let text = std::str::from_utf8(&bytes).map_err(|error| PluginCommandError::ManifestRead {
            path: project::display(&path),
            reason: format!("it is not UTF-8: {error}"),
        })?;

        let manifest =
            PluginManifest::from_json(text).map_err(PluginCommandError::Manifest)?;
        manifest
            .negotiate(ApiVersion::CURRENT)
            .map_err(PluginCommandError::Manifest)?;
        check_schema_bounds(&manifest).map_err(PluginCommandError::Manifest)?;
        if !is_portable_plugin_name(&manifest.name) {
            return Err(PluginCommandError::Manifest(rule(
                "name",
                format!(
                    "`{}` is not a portable directory name; Windows reserved device names such as \
                     `con`, `nul`, and `com1` are refused",
                    manifest.name
                ),
            )));
        }

        let permissions = permissions_from(&manifest);
        let digest = sha256_hex(&bytes);
        Ok(Self {
            manifest,
            bytes,
            digest,
            source: project::display(&path),
            permissions,
        })
    }

    /// The lockfile entry that records this manifest and the permissions a human confirmed.
    #[must_use]
    pub(crate) fn installed_plugin(&self) -> InstalledPlugin {
        InstalledPlugin {
            name: self.manifest.name.clone(),
            version: self.manifest.version.clone(),
            plugin_api_version: self.manifest.plugin_api_version.clone(),
            description: self.manifest.description.clone(),
            minimum_aicontext_version: self.manifest.minimum_aicontext_version.clone(),
            manifest_digest: self.digest.clone(),
            source: self.source.clone(),
            signature: SignatureStatus::Unsigned,
            permissions: self.permissions.clone(),
        }
    }

    /// The auth requirements as lines a human can read, or an empty list.
    #[must_use]
    pub(crate) fn auth_lines(&self) -> Vec<String> {
        let Some(auth) = &self.manifest.auth else {
            return Vec::new();
        };
        let mut lines: Vec<String> = auth
            .references
            .iter()
            .map(|reference| {
                format!(
                    "{} ({})",
                    reference.name,
                    auth_source_name(reference.source)
                )
            })
            .collect();
        if let Some(instructions) = &auth.instructions {
            lines.push(instructions.clone());
        }
        lines
    }
}

/// Resolve a reference to a manifest file.
///
/// A relative reference resolves against the project root, matching `export`, `import`, and the
/// rest of the CLI rather than the process's working directory. An existing directory is searched
/// for `manifest.json`; an existing file is read directly. A reference that names nothing is
/// either a registry reference (refused by name) or a missing local path.
fn resolve(root: &Path, reference: &str) -> Result<PathBuf, PluginCommandError> {
    let candidate = {
        let path = Path::new(reference);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };

    if candidate.is_dir() {
        let manifest = candidate.join("manifest.json");
        if manifest.is_file() {
            return Ok(manifest);
        }
        return Err(PluginCommandError::ManifestNotFound {
            path: project::display(&manifest),
        });
    }
    if candidate.is_file() {
        return Ok(candidate);
    }

    if looks_like_registry(reference) {
        return Err(PluginCommandError::RegistryUnsupported {
            reference: reference.to_owned(),
        });
    }
    Err(PluginCommandError::ManifestNotFound {
        path: project::display(&candidate),
    })
}

/// Whether a reference is a registry name rather than a local path.
///
/// A bare plugin name is the registry's shortest reference (`aws`), and `name@version`, an HTTP
/// URL, and an explicit `registry:` prefix are the other forms the documentation uses. Anything
/// with a path separator is treated as a local path, because the two spellings would otherwise be
/// indistinguishable and a path is the only one this build can act on.
fn looks_like_registry(reference: &str) -> bool {
    let lower = reference.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("registry:")
        || reference.contains('@')
        || is_portable_plugin_name(reference)
}

/// Read the manifest with the SDK's byte cap applied before the file is parsed.
fn read(path: &Path) -> Result<Vec<u8>, PluginCommandError> {
    let spelled = project::display(path);
    let metadata = fs::metadata(path).map_err(|error| PluginCommandError::ManifestRead {
        path: spelled.clone(),
        reason: error.to_string(),
    })?;
    if metadata.len() > MAX_MANIFEST_BYTES as u64 {
        return Err(PluginCommandError::ManifestRead {
            path: spelled,
            reason: format!(
                "it is {} bytes; the maximum is {MAX_MANIFEST_BYTES}",
                metadata.len()
            ),
        });
    }

    let mut file = fs::File::open(path).map_err(|error| PluginCommandError::ManifestRead {
        path: spelled.clone(),
        reason: error.to_string(),
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_MANIFEST_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| PluginCommandError::ManifestRead {
            path: spelled.clone(),
            reason: error.to_string(),
        })?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(PluginCommandError::ManifestRead {
            path: spelled,
            reason: format!("it is larger than the {MAX_MANIFEST_BYTES}-byte maximum"),
        });
    }
    Ok(bytes)
}

/// Apply the schema bounds the SDK's type and cross-field checks do not cover.
///
/// The schema is the normative declaration (`schemas/plugin-manifest.schema.json`); these are the
/// `maxLength`, `maxItems`, and `uniqueItems` rules a serde type cannot state. A manifest that
/// exceeds one is refused with the SDK's `PLG-011`, so the author sees the same code and
/// remediation whichever check caught it.
fn check_schema_bounds(manifest: &PluginManifest) -> Result<(), PluginError> {
    check_len(
        "plugin_api_version",
        &manifest.plugin_api_version,
        4,
    )?;
    check_len("version", &manifest.version, 256)?;
    check_optional_len("description", manifest.description.as_deref(), 2000)?;
    check_optional_len("vendor", manifest.vendor.as_deref(), 200)?;
    check_optional_len("license", manifest.license.as_deref(), 200)?;
    check_optional_len(
        "minimum_aicontext_version",
        manifest.minimum_aicontext_version.as_deref(),
        256,
    )?;
    check_len("entry.command", &manifest.entry.command, 4096)?;
    if manifest.entry.args.len() > 256 {
        return Err(rule(
            "entry.args",
            format!(
                "{} arguments are declared; the maximum is 256",
                manifest.entry.args.len()
            ),
        ));
    }
    for argument in &manifest.entry.args {
        check_len("entry.args[]", argument, 4096)?;
    }
    if let Some(auth) = &manifest.auth {
        check_optional_len("auth.instructions", auth.instructions.as_deref(), 4000)?;
    }

    for (field, paths) in [
        ("filesystem.read", &manifest.filesystem.read),
        ("filesystem.write", &manifest.filesystem.write),
    ] {
        check_unique(field, paths)?;
        for path in paths {
            check_len(field, path, 4096)?;
        }
    }
    check_unique("network.egress", &manifest.network.egress)?;
    for host in &manifest.network.egress {
        check_len("network.egress", host, 253)?;
    }

    Ok(())
}

/// Refuse a string longer than the schema's `maxLength`, counted in Unicode scalar values.
fn check_len(field: &'static str, value: &str, max: usize) -> Result<(), PluginError> {
    let length = value.chars().count();
    if length > max {
        return Err(rule(
            field,
            format!("the value is {length} characters; the maximum is {max}"),
        ));
    }
    Ok(())
}

/// Refuse an optional string longer than the schema's `maxLength`.
fn check_optional_len(
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), PluginError> {
    match value {
        Some(value) => check_len(field, value, max),
        None => Ok(()),
    }
}

/// Refuse a list with a repeated item, which the schema states as `uniqueItems`.
fn check_unique(field: &'static str, values: &[String]) -> Result<(), PluginError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value.as_str()) {
            return Err(rule(
                field,
                format!("`{value}` appears more than once"),
            ));
        }
    }
    Ok(())
}

/// Build the permission set a manifest requests.
///
/// A tool carries its effect and destructive flag; filesystem read and write are separate
/// permissions; a network host and an auth reference are each one permission. The result is sorted
/// so the diff and the lockfile cannot depend on the order the manifest happened to use.
fn permissions_from(manifest: &PluginManifest) -> Vec<Permission> {
    let mut permissions = Vec::new();
    for capability in &manifest.capabilities {
        permissions.push(Permission::Tool {
            tool: capability.tool.clone(),
            effect: capability.effect,
            destructive: capability.destructive,
        });
    }
    for path in &manifest.filesystem.read {
        permissions.push(Permission::Filesystem {
            access: FilesystemAccess::Read,
            path: path.clone(),
        });
    }
    for path in &manifest.filesystem.write {
        permissions.push(Permission::Filesystem {
            access: FilesystemAccess::Write,
            path: path.clone(),
        });
    }
    for host in &manifest.network.egress {
        permissions.push(Permission::Network { host: host.clone() });
    }
    if let Some(auth) = &manifest.auth {
        for reference in &auth.references {
            permissions.push(Permission::Auth {
                name: reference.name.clone(),
                source: reference.source,
            });
        }
    }
    sort_permissions(&mut permissions);
    permissions
}

/// The SDK's rule error, so a schema-bound failure carries the same code and remediation.
fn rule(rule: &'static str, detail: String) -> PluginError {
    PluginError::ManifestRule { rule, detail }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use aicontext_plugin_sdk::Effect;
    use tempfile::TempDir;

    use super::ValidatedManifest;
    use crate::plugin::lockfile::Permission;

    const CANONICAL: &str = r#"{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "name": "demo",
  "version": "1.0.0",
  "description": "A demo plugin",
  "entry": { "command": "demo-plugin", "args": ["serve"] },
  "capabilities": [
    { "tool": "demo.items.read", "effect": "read" },
    { "tool": "demo.items.write", "effect": "write", "destructive": true }
  ],
  "auth": {
    "type": "env_or_credential_command",
    "references": [{ "name": "DEMO_TOKEN", "source": "env" }],
    "instructions": "set DEMO_TOKEN"
  },
  "filesystem": { "read": ["docs"], "write": ["out"] },
  "network": { "egress": ["api.example.com"] },
  "limits": { "call_timeout_ms": 5000, "max_output_bytes": 1048576 }
}"#;

    fn project() -> TempDir {
        TempDir::new().expect("temp dir")
    }

    fn write_manifest(root: &std::path::Path, text: &str) -> std::path::PathBuf {
        let directory = root.join("plugin");
        fs::create_dir_all(&directory).expect("create plugin dir");
        let manifest = directory.join("manifest.json");
        fs::write(&manifest, text).expect("write manifest");
        manifest
    }

    #[test]
    fn a_directory_reference_loads_and_extracts_every_permission() {
        let root = project();
        write_manifest(root.path(), CANONICAL);

        let validated = ValidatedManifest::load(root.path(), "plugin").expect("valid");
        assert_eq!(validated.manifest.name, "demo");
        assert_eq!(validated.permissions.len(), 6);
        assert!(validated.permissions.iter().any(|permission| matches!(
            permission,
            Permission::Tool { tool, effect, destructive }
                if tool == "demo.items.write" && *effect == Effect::Write && *destructive
        )));
        assert!(validated.permissions.iter().any(|permission| matches!(
            permission,
            Permission::Filesystem { path, .. } if path == "docs"
        )));
        assert!(validated.permissions.iter().any(|permission| matches!(
            permission,
            Permission::Network { host } if host == "api.example.com"
        )));
        assert!(validated.permissions.iter().any(|permission| matches!(
            permission,
            Permission::Auth { name, .. } if name == "DEMO_TOKEN"
        )));
        assert_eq!(validated.auth_lines()[0], "DEMO_TOKEN (env)");
        assert_eq!(validated.digest.len(), 64);
    }

    #[test]
    fn a_manifest_file_reference_loads_directly() {
        let root = project();
        let manifest = write_manifest(root.path(), CANONICAL);
        let validated =
            ValidatedManifest::load(root.path(), &manifest.to_string_lossy()).expect("valid");
        assert_eq!(validated.manifest.name, "demo");
    }

    #[test]
    fn a_missing_reference_is_a_named_refusal() {
        let root = project();
        let error = ValidatedManifest::load(root.path(), "not-here").expect_err("missing");
        assert_eq!(error.code(), "PLUGIN-002");
        assert_eq!(error.exit().code(), 2);
    }

    #[test]
    fn a_registry_reference_is_refused_by_name() {
        let root = project();
        for reference in ["demo", "demo@1.0.0", "https://registry.example/demo"] {
            let error = ValidatedManifest::load(root.path(), reference).expect_err("registry");
            assert_eq!(error.code(), "PLUGIN-003", "{reference}");
        }
    }

    #[test]
    fn an_unknown_effect_is_plg_011() {
        let root = project();
        let text = CANONICAL.replace("\"effect\": \"read\"", "\"effect\": \"delete\"");
        write_manifest(root.path(), &text);
        let error = ValidatedManifest::load(root.path(), "plugin").expect_err("unknown effect");
        assert_eq!(error.code(), "PLG-011");
        assert_eq!(error.exit().code(), 3);
    }

    #[test]
    fn an_unknown_field_is_plg_011() {
        let root = project();
        let text = CANONICAL.replace(
            "\"limits\": {",
            "\"permissions\": [\"all\"],\n  \"limits\": {",
        );
        write_manifest(root.path(), &text);
        let error = ValidatedManifest::load(root.path(), "plugin").expect_err("unknown field");
        assert_eq!(error.code(), "PLG-011");
    }

    #[test]
    fn a_schema_bound_the_sdk_does_not_check_is_enforced() {
        let root = project();
        let long = "x".repeat(2001);
        let text = CANONICAL.replace("\"A demo plugin\"", &format!("\"{long}\""));
        write_manifest(root.path(), &text);
        let error = ValidatedManifest::load(root.path(), "plugin").expect_err("too long");
        assert_eq!(error.code(), "PLG-011");
        assert!(error.to_string().contains("description"), "{error}");
    }

    #[test]
    fn duplicate_filesystem_paths_are_refused() {
        let root = project();
        let text = CANONICAL.replace("\"read\": [\"docs\"]", "\"read\": [\"docs\", \"docs\"]");
        write_manifest(root.path(), &text);
        let error = ValidatedManifest::load(root.path(), "plugin").expect_err("duplicate");
        assert_eq!(error.code(), "PLG-011");
        assert!(error.to_string().contains("filesystem.read"), "{error}");
    }

    #[test]
    fn a_non_portable_name_is_refused() {
        let root = project();
        let text = CANONICAL.replace("\"name\": \"demo\"", "\"name\": \"con\"");
        write_manifest(root.path(), &text);
        let error = ValidatedManifest::load(root.path(), "plugin").expect_err("reserved");
        assert_eq!(error.code(), "PLG-011");
    }

    #[test]
    fn an_unsupported_api_version_is_plg_004() {
        let root = project();
        let text = CANONICAL.replace("\"plugin_api_version\": \"1\"", "\"plugin_api_version\": \"2\"");
        write_manifest(root.path(), &text);
        let error = ValidatedManifest::load(root.path(), "plugin").expect_err("api version");
        assert_eq!(error.code(), "PLG-004");
    }
}
