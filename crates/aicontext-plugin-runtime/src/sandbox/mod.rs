//! The plugin sandbox: working directory, environment, and allowlists.
//!
//! `docs/PLUGIN_SPEC.md` §7 and `docs/SECURITY.md` §7 describe the controls a plugin runs under.
//! This module implements the ones the host can establish structurally:
//!
//! - a per-plugin directory under `state/plugins/<name>` with `work`, `home`, and `tmp` children;
//! - the process working directory set to `work`, never the repository;
//! - an empty child environment with only a minimal base set and the manifest's `env` auth
//!   references, with `HOME` and the temporary-directory variables pointing into the sandbox;
//! - the manifest's filesystem read/write lists resolved and checked for lexical and symlink
//!   escapes;
//! - the manifest's network egress list, with an empty list meaning no network.
//!
//! The filesystem and network checks are the host-mediated operations. They are not a kernel
//! sandbox: a plugin that opens a path directly is stopped only by the operating system's own
//! controls, and those differ across Linux, macOS, and Windows. That limitation is stated rather
//! than hidden (`docs/SECURITY.md` §12, risk R-3); the runtime does not claim a guarantee it
//! cannot enforce.
//!
//! A refusal is always [`SandboxError`] and every variant names a `PLG` code. The gateway records a
//! preparation refusal as `GatewayEvent::SandboxRefused` before returning it, so an attempt to
//! leave the sandbox is auditable rather than a silent startup failure.

mod environment;
mod error;
mod network;
mod path;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aicontext_plugin_sdk::PluginManifest;

pub use environment::Environment;
pub use error::SandboxError;
pub use network::NetworkPolicy;
pub use path::FilesystemPolicy;

/// The two host-chosen roots a sandbox is prepared from.
///
/// `state` is writable runtime state (the default is `.aicontext/`); `project` is the repository the
/// manifest's relative filesystem paths resolve against. Both are supplied by the caller so the
/// sandbox never guesses a path from the current directory or an environment variable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxRoots {
    state: PathBuf,
    project: PathBuf,
}

impl SandboxRoots {
    /// The state and project roots.
    #[must_use]
    pub fn new(state: impl Into<PathBuf>, project: impl Into<PathBuf>) -> Self {
        Self {
            state: state.into(),
            project: project.into(),
        }
    }

    /// The writable state root.
    #[must_use]
    pub fn state(&self) -> &Path {
        &self.state
    }

    /// The project root that relative declarations resolve against.
    #[must_use]
    pub fn project(&self) -> &Path {
        &self.project
    }
}

/// One prepared sandbox. Create it with [`Sandbox::prepare`] and pass it to
/// [`start_plugin`](crate::start_plugin) or [`ToolGateway::start`](crate::ToolGateway::start).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sandbox {
    plugin: String,
    root: PathBuf,
    work: PathBuf,
    home: PathBuf,
    temp: PathBuf,
    filesystem: FilesystemPolicy,
    network: NetworkPolicy,
    environment: Environment,
}

impl Sandbox {
    /// Prepare the per-plugin tree and resolve the manifest's allowlists.
    ///
    /// Existing directories are reused. A symlink where the sandbox needs a directory is refused,
    /// and on Unix every directory the plugin can use is mode `0700`.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Manifest`] (`PLG-011`) for an invalid manifest,
    /// [`SandboxError::Escape`] (`PLG-009`) for a declared path that leaves the project or resolves
    /// through a symlink, [`SandboxError::InvalidPath`] (`PLG-011`) for a non-portable path,
    /// [`SandboxError::InvalidHost`] (`PLG-011`) for a malformed egress entry,
    /// [`SandboxError::Environment`] (`PLG-011`) for an auth variable the sandbox controls,
    /// [`SandboxError::Root`] (`PLG-009`) for an unusable sandbox directory, and
    /// [`SandboxError::Io`] (`PLG-009`) when a directory cannot be created or inspected.
    pub fn prepare(manifest: &PluginManifest, roots: &SandboxRoots) -> Result<Self, SandboxError> {
        manifest.validate().map_err(SandboxError::Manifest)?;
        let plugin = manifest.name.clone();
        let project = canonical_directory(&plugin, &roots.project)?;
        let state = create_directory(&plugin, &roots.state)?;
        let plugins = create_directory(&plugin, &state.join("plugins"))?;
        let root = create_directory(&plugin, &plugins.join(&plugin))?;

        // A canonicalised path is the path the host will actually use. If a filesystem object
        // between the state root and the plugin tree redirected it (a junction, for example),
        // the canonical path leaves the root and the sandbox is refused rather than used.
        for (name, path, parent) in [
            ("plugins", &plugins, &state),
            ("plugin root", &root, &plugins),
        ] {
            if !path.starts_with(parent) {
                return Err(SandboxError::Root {
                    plugin: plugin.clone(),
                    detail: format!(
                        "the {name} `{}` is not inside `{}`",
                        path.display(),
                        parent.display()
                    ),
                });
            }
        }

        let work = create_directory(&plugin, &root.join("work"))?;
        let home = create_directory(&plugin, &root.join("home"))?;
        let temp = create_directory(&plugin, &root.join("tmp"))?;
        for (name, path) in [("work", &work), ("home", &home), ("tmp", &temp)] {
            if !path.starts_with(&root) {
                return Err(SandboxError::Root {
                    plugin: plugin.clone(),
                    detail: format!(
                        "the {name} directory `{}` is not inside the plugin root `{}`",
                        path.display(),
                        root.display()
                    ),
                });
            }
        }

        let filesystem = FilesystemPolicy::build(
            &plugin,
            &project,
            &manifest.filesystem.read,
            &manifest.filesystem.write,
        )?;
        let network = NetworkPolicy::build(&plugin, &manifest.network.egress)?;
        let environment = Environment::build(manifest, &home, &temp)?;

        Ok(Self {
            plugin,
            root,
            work,
            home,
            temp,
            filesystem,
            network,
            environment,
        })
    }

    /// The plugin this sandbox was prepared for.
    #[must_use]
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The per-plugin root, `state/plugins/<name>`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The process working directory, an empty per-plugin directory.
    #[must_use]
    pub fn work(&self) -> &Path {
        &self.work
    }

    /// The sandbox's `HOME` and `USERPROFILE`.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The sandbox's temporary directory.
    #[must_use]
    pub fn temp(&self) -> &Path {
        &self.temp
    }

    /// The resolved filesystem allowlist.
    #[must_use]
    pub fn filesystem(&self) -> &FilesystemPolicy {
        &self.filesystem
    }

    /// The resolved network allowlist.
    #[must_use]
    pub fn network(&self) -> &NetworkPolicy {
        &self.network
    }

    /// The environment the plugin will receive.
    #[must_use]
    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    /// Refuse a host-mediated read outside the manifest's read list.
    ///
    /// # Errors
    ///
    /// As [`FilesystemPolicy::check_read`].
    pub fn check_read(&self, target: impl AsRef<Path>) -> Result<PathBuf, SandboxError> {
        self.filesystem.check_read(&self.plugin, target)
    }

    /// Refuse a host-mediated write outside the manifest's write list.
    ///
    /// # Errors
    ///
    /// As [`FilesystemPolicy::check_write`].
    pub fn check_write(&self, target: impl AsRef<Path>) -> Result<PathBuf, SandboxError> {
        self.filesystem.check_write(&self.plugin, target)
    }

    /// Refuse a host-mediated network request outside the manifest's egress list.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Denied`] when the policy is empty or the host is not declared.
    pub fn check_host(&self, host: &str) -> Result<(), SandboxError> {
        self.network.check(&self.plugin, host)
    }

    /// Refuse to start a manifest the sandbox was not prepared for.
    pub(crate) fn ensure_matches(&self, manifest: &PluginManifest) -> Result<(), SandboxError> {
        if self.plugin == manifest.name {
            Ok(())
        } else {
            Err(SandboxError::Mismatch {
                plugin: manifest.name.clone(),
                sandbox: self.plugin.clone(),
            })
        }
    }

    /// Apply the working directory and the filtered environment to a command.
    pub(crate) fn configure(&self, command: &mut Command) {
        command.current_dir(&self.work);
        self.environment.apply(command);
    }
}

/// Canonicalise an existing directory that must already be present.
fn canonical_directory(plugin: &str, path: &Path) -> Result<PathBuf, SandboxError> {
    let canonical = fs::canonicalize(path).map_err(|source| SandboxError::Io {
        plugin: plugin.to_owned(),
        path: display_path(path),
        source,
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| SandboxError::Io {
        plugin: plugin.to_owned(),
        path: display_path(&canonical),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(SandboxError::Root {
            plugin: plugin.to_owned(),
            detail: format!("`{}` is not a directory", display_path(path)),
        });
    }
    Ok(canonical)
}

/// Create (or reuse) a directory, refuse a symlink, and restrict it on Unix.
fn create_directory(plugin: &str, path: &Path) -> Result<PathBuf, SandboxError> {
    fs::create_dir_all(path).map_err(|source| SandboxError::Io {
        plugin: plugin.to_owned(),
        path: display_path(path),
        source,
    })?;
    let metadata = fs::symlink_metadata(path).map_err(|source| SandboxError::Io {
        plugin: plugin.to_owned(),
        path: display_path(path),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(SandboxError::Root {
            plugin: plugin.to_owned(),
            detail: format!(
                "`{}` is a symlink; the sandbox refuses to follow it",
                display_path(path)
            ),
        });
    }
    if !metadata.is_dir() {
        return Err(SandboxError::Root {
            plugin: plugin.to_owned(),
            detail: format!("`{}` is not a directory", display_path(path)),
        });
    }
    #[cfg(unix)]
    restrict_permissions(plugin, path)?;
    fs::canonicalize(path).map_err(|source| SandboxError::Io {
        plugin: plugin.to_owned(),
        path: display_path(path),
        source,
    })
}

#[cfg(unix)]
fn restrict_permissions(plugin: &str, path: &Path) -> Result<(), SandboxError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
        SandboxError::Io {
            plugin: plugin.to_owned(),
            path: display_path(path),
            source,
        }
    })
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{FFFD}'
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use aicontext_plugin_sdk::{
        Capability, Effect, EntryPoint, FilesystemScope, Limits, NetworkScope, PluginManifest,
    };

    use super::{Sandbox, SandboxRoots};

    fn manifest() -> PluginManifest {
        PluginManifest {
            manifest_version: 1,
            plugin_api_version: "1".to_owned(),
            name: "fixture".to_owned(),
            version: "1.0.0".to_owned(),
            description: None,
            vendor: None,
            license: None,
            entry: EntryPoint {
                command: "fixture".to_owned(),
                args: Vec::new(),
            },
            minimum_aicontext_version: None,
            capabilities: vec![Capability {
                tool: "fixture.echo.run".to_owned(),
                effect: Effect::Read,
                destructive: false,
            }],
            auth: None,
            filesystem: FilesystemScope {
                read: vec!["src".to_owned()],
                write: vec!["build".to_owned()],
            },
            network: NetworkScope {
                egress: vec!["api.example.com".to_owned()],
            },
            limits: Limits {
                call_timeout_ms: 1_000,
                max_output_bytes: 1_024,
            },
        }
    }

    fn roots() -> (tempfile::TempDir, SandboxRoots) {
        let directory = tempfile::tempdir().expect("a temporary root");
        let project = directory.path().join("project");
        fs::create_dir(&project).expect("the project exists");
        let roots = SandboxRoots::new(directory.path().join("state"), project);
        (directory, roots)
    }

    #[test]
    fn prepare_creates_a_private_tree_under_the_state_root() {
        let (_directory, roots) = roots();
        let sandbox = Sandbox::prepare(&manifest(), &roots).expect("the sandbox is prepared");

        assert_eq!(sandbox.plugin(), "fixture");
        let state = fs::canonicalize(roots.state()).expect("the state root exists");
        assert!(sandbox.root().starts_with(&state));
        assert_eq!(
            sandbox.root().file_name().and_then(std::ffi::OsStr::to_str),
            Some("fixture")
        );
        assert!(sandbox.work().is_dir());
        assert!(sandbox.home().is_dir());
        assert!(sandbox.temp().is_dir());
        assert_ne!(sandbox.work(), roots.project());
        let home = sandbox.environment().get_str("HOME").expect("HOME is set");
        assert_eq!(fs::canonicalize(home).expect("HOME exists"), sandbox.home());
        let temp = sandbox
            .environment()
            .get_str("TMPDIR")
            .expect("TMPDIR is set");
        assert_eq!(
            fs::canonicalize(temp).expect("TMPDIR exists"),
            sandbox.temp()
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_per_plugin_directories_are_mode_0700() {
        use std::os::unix::fs::PermissionsExt;

        let (_directory, roots) = roots();
        let sandbox = Sandbox::prepare(&manifest(), &roots).expect("the sandbox is prepared");

        for path in [
            sandbox.root(),
            sandbox.work(),
            sandbox.home(),
            sandbox.temp(),
        ] {
            let mode = fs::metadata(path)
                .expect("the directory exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700, "{}", path.display());
        }
    }

    #[test]
    fn the_allowlists_are_resolved_and_enforced() {
        let (_directory, roots) = roots();
        fs::create_dir(roots.project().join("src")).expect("the read directory exists");
        let sandbox = Sandbox::prepare(&manifest(), &roots).expect("the sandbox is prepared");

        assert!(sandbox.check_read("src/main.rs").is_ok());
        assert!(sandbox.check_write("build/out").is_ok());
        assert!(sandbox.check_host("api.example.com").is_ok());
        assert!(sandbox.check_read("Cargo.toml").is_err());
        assert!(sandbox.check_write("src/main.rs").is_err());
        assert!(sandbox.check_host("evil.example.com").is_err());
    }

    #[test]
    fn a_sandbox_for_one_plugin_is_not_reused_for_another() {
        let (_directory, roots) = roots();
        let sandbox = Sandbox::prepare(&manifest(), &roots).expect("the sandbox is prepared");
        let mut other = manifest();
        other.name = "other".to_owned();
        other.capabilities[0].tool = "other.echo.run".to_owned();

        let error = sandbox
            .ensure_matches(&other)
            .expect_err("the names disagree");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_009);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_where_the_sandbox_needs_a_directory_is_refused() {
        let (directory, roots) = roots();
        let outside = tempfile::tempdir().expect("a directory outside the state root");
        fs::create_dir_all(roots.state()).expect("the state root exists");
        std::os::unix::fs::symlink(outside.path(), roots.state().join("plugins"))
            .expect("the symlink is created");
        let _ = directory;

        let error = Sandbox::prepare(&manifest(), &roots).expect_err("the symlink is refused");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_009);
    }

    #[test]
    fn a_project_root_that_does_not_exist_is_an_io_error() {
        let directory = tempfile::tempdir().expect("a temporary root");
        let roots = SandboxRoots::new(
            directory.path().join("state"),
            directory.path().join("missing-project"),
        );

        let error = Sandbox::prepare(&manifest(), &roots).expect_err("the project is missing");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_009);
    }
}
