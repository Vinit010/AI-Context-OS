//! The filesystem allowlist and escape checks.
//!
//! `docs/PLUGIN_SPEC.md` §7 and `docs/SECURITY.md` §7 make the manifest's `filesystem` block the
//! outer jail. A declared relative path is resolved against the project root; an absolute path is
//! taken as written. Both are checked lexically for `..` escapes and then canonically for symlinks
//! that leave the project root, so a manifest cannot name `link` where `link` points outside the
//! repository. The longest existing ancestor is canonicalised and the not-yet-existing tail is
//! appended, which lets a plugin declare an output file that does not exist yet without disabling
//! the symlink check.
//!
//! Paths must also be portable: a component that is a Windows reserved device name, contains an
//! alternate-data-stream colon, ends in a dot or a space, or contains a character Windows forbids
//! is refused on every platform. Accepting it on Unix and failing on Windows would make the same
//! manifest behave differently on a supported platform, which is exactly the ambiguity a security
//! boundary must not have.
//!
//! These checks are used for host-mediated reads and writes. They do not intercept a plugin's own
//! syscalls; the OS-level boundary is platform-dependent (`docs/SECURITY.md` §12, risk R-3).

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::sandbox::SandboxError;

/// The resolved `filesystem.read` and `filesystem.write` lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesystemPolicy {
    project_root: PathBuf,
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

impl FilesystemPolicy {
    /// Resolve and validate the manifest's filesystem scopes against a canonical project root.
    pub(crate) fn build(
        plugin: &str,
        project_root: &Path,
        read: &[String],
        write: &[String],
    ) -> Result<Self, SandboxError> {
        Ok(Self {
            project_root: project_root.to_path_buf(),
            read: resolve_declared(plugin, "read", project_root, read)?,
            write: resolve_declared(plugin, "write", project_root, write)?,
        })
    }

    /// The resolved read paths, in stable order.
    #[must_use]
    pub fn read_paths(&self) -> &[PathBuf] {
        &self.read
    }

    /// The resolved write paths, in stable order.
    #[must_use]
    pub fn write_paths(&self) -> &[PathBuf] {
        &self.write
    }

    /// Refuse `target` unless it lies at or beneath a declared read path.
    ///
    /// A relative target is resolved against the project root, the same base as a declaration.
    /// The returned path is canonical for its existing prefix, so a caller that opens it is using
    /// the path that was checked rather than the spelling the plugin supplied.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Escape`] for a relative target that leaves the project or resolves outside
    /// it through a symlink, [`SandboxError::Denied`] for a target outside the allowlist, and
    /// [`SandboxError::Io`] if the existing prefix cannot be inspected.
    pub fn check_read(&self, plugin: &str, target: impl AsRef<Path>) -> Result<PathBuf, SandboxError> {
        self.check(plugin, "read", &self.read, target.as_ref())
    }

    /// Refuse `target` unless it lies at or beneath a declared write path.
    ///
    /// # Errors
    ///
    /// As [`FilesystemPolicy::check_read`].
    pub fn check_write(
        &self,
        plugin: &str,
        target: impl AsRef<Path>,
    ) -> Result<PathBuf, SandboxError> {
        self.check(plugin, "write", &self.write, target.as_ref())
    }

    fn check(
        &self,
        plugin: &str,
        operation: &'static str,
        allowed: &[PathBuf],
        target: &Path,
    ) -> Result<PathBuf, SandboxError> {
        let resolved = resolve_request(plugin, operation, &self.project_root, target)?;
        if allowed.iter().any(|root| path_within(root, &resolved)) {
            Ok(resolved)
        } else {
            Err(SandboxError::Denied {
                plugin: plugin.to_owned(),
                operation,
                target: display_path(&resolved),
            })
        }
    }
}

fn resolve_declared(
    plugin: &str,
    operation: &'static str,
    project_root: &Path,
    declared: &[String],
) -> Result<Vec<PathBuf>, SandboxError> {
    let mut resolved = BTreeSet::new();
    for raw in declared {
        if raw.trim().is_empty() {
            return Err(SandboxError::InvalidPath {
                plugin: plugin.to_owned(),
                operation,
                detail: "the path is empty".to_owned(),
            });
        }
        let path = Path::new(raw);
        let relative = !path.is_absolute();
        if relative && path.components().any(|part| matches!(part, Component::Prefix(_))) {
            return Err(SandboxError::InvalidPath {
                plugin: plugin.to_owned(),
                operation,
                detail: format!("`{raw}` is a drive-relative path; use an absolute path or one \
                                 relative to the project root"),
            });
        }

        let absolute = if relative {
            project_root.join(path)
        } else {
            path.to_path_buf()
        };
        let lexical = resolve_lexical(&absolute).map_err(|detail| SandboxError::Escape {
            plugin: plugin.to_owned(),
            operation,
            detail: format!("`{raw}` {detail}"),
        })?;
        if relative && !path_within(project_root, &lexical) {
            return Err(SandboxError::Escape {
                plugin: plugin.to_owned(),
                operation,
                detail: format!("`{raw}` leaves the project root"),
            });
        }

        let canonical = canonicalize_with_missing(&lexical).map_err(|source| SandboxError::Io {
            plugin: plugin.to_owned(),
            path: display_path(&lexical),
            source,
        })?;
        if relative && !path_within(project_root, &canonical) {
            return Err(SandboxError::Escape {
                plugin: plugin.to_owned(),
                operation,
                detail: format!(
                    "`{raw}` resolves through a symlink to `{}`, outside the project root",
                    display_path(&canonical)
                ),
            });
        }
        resolved.insert(canonical);
    }
    Ok(resolved.into_iter().collect())
}

fn resolve_request(
    plugin: &str,
    operation: &'static str,
    project_root: &Path,
    target: &Path,
) -> Result<PathBuf, SandboxError> {
    if target.as_os_str().is_empty() {
        return Err(SandboxError::Denied {
            plugin: plugin.to_owned(),
            operation,
            target: "<empty>".to_owned(),
        });
    }
    let relative = !target.is_absolute();
    let absolute = if relative {
        project_root.join(target)
    } else {
        target.to_path_buf()
    };
    let lexical = resolve_lexical(&absolute).map_err(|detail| SandboxError::Escape {
        plugin: plugin.to_owned(),
        operation,
        detail: format!("`{}` {detail}", display_path(target)),
    })?;
    if relative && !path_within(project_root, &lexical) {
        return Err(SandboxError::Escape {
            plugin: plugin.to_owned(),
            operation,
            detail: format!("`{}` leaves the project root", display_path(target)),
        });
    }
    let canonical = canonicalize_with_missing(&lexical).map_err(|source| SandboxError::Io {
        plugin: plugin.to_owned(),
        path: display_path(&lexical),
        source,
    })?;
    if relative && !path_within(project_root, &canonical) {
        return Err(SandboxError::Escape {
            plugin: plugin.to_owned(),
            operation,
            detail: format!(
                "`{}` resolves through a symlink to `{}`, outside the project root",
                display_path(target),
                display_path(&canonical)
            ),
        });
    }
    Ok(canonical)
}

/// Resolve `..` and `.` without touching the filesystem.
///
/// `..` may cancel a normal component, but it may not climb above the filesystem root. Each normal
/// component is checked for the portable-name rules first, so a reserved name is refused even when
/// the path would otherwise resolve cleanly.
fn resolve_lexical(path: &Path) -> Result<PathBuf, String> {
    let mut resolved = PathBuf::new();
    let mut depth = 0_usize;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                if !resolved.as_os_str().is_empty() {
                    return Err("has more than one prefix".to_owned());
                }
                resolved.push(prefix.as_os_str());
            }
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return Err("climbs above the filesystem root".to_owned());
                }
                resolved.pop();
                depth -= 1;
            }
            Component::Normal(part) => {
                validate_component(part)?;
                resolved.push(part);
                depth += 1;
            }
        }
    }
    Ok(resolved)
}

/// Refuse a component that is not portable across the supported platforms.
fn validate_component(component: &OsStr) -> Result<(), String> {
    let Some(name) = component.to_str() else {
        return Err("contains a component that is not valid UTF-8".to_owned());
    };
    if name.is_empty() {
        return Err("contains an empty component".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err(format!("`{}` contains a control character", sanitize(name)));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(format!(
            "`{}` ends in a dot or a space, which Windows strips",
            sanitize(name)
        ));
    }
    if name.contains(':') {
        return Err(format!(
            "`{}` contains a colon, which names an alternate data stream on Windows",
            sanitize(name)
        ));
    }
    if name.contains(['<', '>', '"', '|', '?', '*', '\\']) {
        return Err(format!(
            "`{}` contains a character Windows does not allow in a file name",
            sanitize(name)
        ));
    }

    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
        });
    if reserved {
        return Err(format!(
            "`{}` is a reserved device name on Windows",
            sanitize(name)
        ));
    }
    Ok(())
}

/// Canonicalise the longest existing ancestor and append the rest unchanged.
fn canonicalize_with_missing(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&current) {
            Ok(mut canonical) => {
                for part in missing.iter().rev() {
                    canonical.push(part);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = current.file_name().map(OsStr::to_os_string) else {
                    return Err(error);
                };
                missing.push(name);
                if !current.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Whether `candidate` is `root` or lies beneath it.
fn path_within(root: &Path, candidate: &Path) -> bool {
    #[cfg(windows)]
    {
        let root = components_lower(root);
        let candidate = components_lower(candidate);
        candidate.starts_with(&root)
    }
    #[cfg(not(windows))]
    {
        candidate.starts_with(root)
    }
}

#[cfg(windows)]
fn components_lower(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Render a path for an error without allowing a control character to forge a line.
fn display_path(path: &Path) -> String {
    sanitize(&path.to_string_lossy())
}

fn sanitize(value: &str) -> String {
    value
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

    use super::{FilesystemPolicy, path_within, resolve_lexical};
    use crate::sandbox::SandboxError;

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary project")
    }

    fn policy(root: &std::path::Path, read: &[&str], write: &[&str]) -> FilesystemPolicy {
        let read: Vec<String> = read.iter().map(|path| (*path).to_owned()).collect();
        let write: Vec<String> = write.iter().map(|path| (*path).to_owned()).collect();
        FilesystemPolicy::build("fixture", root, &read, &write).expect("the paths are valid")
    }

    #[test]
    fn a_relative_declaration_is_resolved_against_the_project_root() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");
        fs::create_dir(root.join("src")).expect("the directory is created");

        let filesystem = policy(&root, &["src"], &["build"]);

        assert_eq!(filesystem.read_paths(), [root.join("src")]);
        assert_eq!(filesystem.write_paths(), [root.join("build")]);
    }

    #[test]
    fn a_declared_path_may_not_climb_out_of_the_project() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");

        let error = FilesystemPolicy::build("fixture", &root, &["../outside".to_owned()], &[])
            .expect_err("the path leaves the project");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_009);
        assert!(matches!(error, SandboxError::Escape { .. }), "{error}");
        assert!(error.to_string().contains("../outside"));
    }

    #[test]
    fn non_portable_names_are_refused_on_every_platform() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");

        for bad in ["CON", "con.txt", "aux", "NUL", "COM1", "lpt9", "file.", "file ", "a:b"] {
            let error = FilesystemPolicy::build("fixture", &root, &[bad.to_owned()], &[])
                .expect_err("the name is not portable");
            assert_eq!(
                error.code(),
                aicontext_core::ErrorCode::PLG_011,
                "`{bad}`: {error}"
            );
            assert!(
                matches!(error, SandboxError::InvalidPath { .. }),
                "`{bad}`: {error}"
            );
        }
    }

    #[test]
    fn a_read_is_allowed_at_or_below_a_declared_path_and_denied_elsewhere() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");
        fs::create_dir(root.join("src")).expect("the directory is created");
        fs::write(root.join("src/readme.md"), "hello").expect("the file is written");
        let filesystem = policy(&root, &["src"], &[]);

        assert_eq!(
            filesystem
                .check_read("fixture", "src/readme.md")
                .expect("a declared read"),
            root.join("src/readme.md")
        );
        assert_eq!(
            filesystem
                .check_read("fixture", "src/missing/child.txt")
                .expect("a path that does not exist yet"),
            root.join("src/missing/child.txt")
        );
        assert_eq!(
            filesystem
                .check_read("fixture", root.join("src"))
                .expect("the declared path itself"),
            root.join("src")
        );

        for denied in ["Cargo.toml", "src/../Cargo.toml", "../outside"] {
            let error = filesystem
                .check_read("fixture", denied)
                .expect_err("the path is outside the read scope");
            assert!(
                matches!(
                    error,
                    SandboxError::Denied { .. } | SandboxError::Escape { .. }
                ),
                "`{denied}`: {error}"
            );
        }
    }

    #[test]
    fn a_write_is_judged_against_the_write_list_not_the_read_list() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");
        fs::create_dir(root.join("src")).expect("the directory is created");
        let filesystem = policy(&root, &["src"], &["build"]);

        let error = filesystem
            .check_write("fixture", "src/main.rs")
            .expect_err("reading a path does not grant writing it");
        assert!(matches!(error, SandboxError::Denied { .. }), "{error}");
        assert!(filesystem.check_write("fixture", "build/out").is_ok());
    }

    #[test]
    fn an_absolute_declaration_is_taken_as_written() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");
        let outside = tempfile::tempdir().expect("a second directory");
        let absolute = fs::canonicalize(outside.path()).expect("the directory exists");

        let filesystem = FilesystemPolicy::build(
            "fixture",
            &root,
            &[absolute.to_string_lossy().into_owned()],
            &[],
        )
        .expect("an explicit absolute path is allowed");
        assert_eq!(filesystem.read_paths(), [absolute]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_that_leaves_the_project_is_refused() {
        let project = project();
        let root = fs::canonicalize(project.path()).expect("the project exists");
        let outside = tempfile::tempdir().expect("a directory outside the project");
        std::os::unix::fs::symlink(outside.path(), root.join("link"))
            .expect("the symlink is created");

        let error = FilesystemPolicy::build("fixture", &root, &["link".to_owned()], &[])
            .expect_err("the symlink leaves the project");
        assert_eq!(error.code(), aicontext_core::ErrorCode::PLG_009);
        assert!(matches!(error, SandboxError::Escape { .. }), "{error}");
    }

    #[test]
    fn lexical_resolution_cancels_dot_dot_but_not_the_root() {
        assert_eq!(
            resolve_lexical(std::path::Path::new("a/b/../c")).expect("a relative path"),
            std::path::Path::new("a/c")
        );
        assert!(resolve_lexical(std::path::Path::new("../a")).is_err());
    }

    #[test]
    fn a_path_is_within_itself_and_its_descendants_only() {
        let root = std::path::Path::new("a/b");
        assert!(path_within(root, std::path::Path::new("a/b")));
        assert!(path_within(root, std::path::Path::new("a/b/c")));
        assert!(!path_within(root, std::path::Path::new("a/bc")));
    }
}
