//! Resolving the project root, naming the project, and spelling a path for a human.
//!
//! Every command that reads or writes a `.ai/` tree needs the same three answers: which directory is
//! the project, what is it called, and how is a path written down. `doctor` is the second consumer
//! after `init`, so the answers live here rather than being written twice — a second copy would be a
//! second opinion about where the project is, which is not a thing a filesystem question should have
//! (`RULES.md` §2).
//!
//! The error is `PROJ-` rather than `INIT-`, because this is not an `init` failure any more: the code
//! space belongs to the question, not to the first command that asked it.

use std::path::{Path, PathBuf};

use crate::exit::Exit;

/// The directory a command was pointed at cannot serve as a project root.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ProjectError {
    /// The path does not exist, is not a directory, or cannot be resolved.
    #[error("cannot use {path} as the project root: {reason}")]
    UnusableRoot {
        /// The path as the user would recognise it.
        path: String,
        /// Why it cannot be used.
        reason: String,
    },
}

impl ProjectError {
    /// A stable code, independent of the wording.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => "PROJ-001",
        }
    }

    /// What the developer can do about it (`RULES.md` §4.3).
    pub(crate) const fn remediation(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => {
                "pass --cwd with an existing directory, or run the command from inside the project"
            }
        }
    }

    /// The exit code this failure produces (`docs/CLI_SPEC.md` §5).
    ///
    /// Usage rather than general: the argument named a directory that cannot be the project, and the
    /// developer can fix it by typing something else.
    pub(crate) const fn exit(&self) -> Exit {
        match self {
            Self::UnusableRoot { .. } => Exit::Usage,
        }
    }

    /// The path and the reason, so a command with its own error type can carry them across.
    pub(crate) fn into_parts(self) -> (String, String) {
        match self {
            Self::UnusableRoot { path, reason } => (path, reason),
        }
    }
}

/// Resolves the project root from `--cwd` or the working directory, and proves it is a directory.
///
/// Canonicalised, because every path a command reports is relative to this one and a relative path
/// computed against `C:\work\proj\..\other` does not round-trip. An unreadable directory is refused
/// rather than reported as empty, so `doctor` cannot mistake a permissions problem for a clean tree.
pub(crate) fn resolve_root(cwd: Option<&Path>) -> Result<PathBuf, ProjectError> {
    let working = || {
        std::env::current_dir().map_err(|error| ProjectError::UnusableRoot {
            path: "the working directory".to_string(),
            reason: format!("it cannot be read ({error})"),
        })
    };

    let candidate = match cwd {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => working()?.join(path),
        None => working()?,
    };

    if !candidate.exists() {
        return Err(ProjectError::UnusableRoot {
            path: display(&candidate),
            reason: "it does not exist".to_string(),
        });
    }
    if !candidate.is_dir() {
        return Err(ProjectError::UnusableRoot {
            path: display(&candidate),
            reason: "it is a file, not a directory".to_string(),
        });
    }
    candidate
        .canonicalize()
        .map_err(|error| ProjectError::UnusableRoot {
            path: display(&candidate),
            reason: format!("it cannot be resolved ({error})"),
        })
}

/// The project name taken from the root directory, which is where a developer expects it to come
/// from.
pub(crate) fn project_name(root: &Path) -> Result<String, ProjectError> {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    // `--cwd /srv/repos/project.git` names the repository, not the checkout directory.
    let name = name.strip_suffix(".git").unwrap_or(&name).to_string();
    if name.is_empty() || name == "." || name == ".." {
        return Err(ProjectError::UnusableRoot {
            path: display(root),
            reason: "it has no directory name to take a project name from".to_string(),
        });
    }
    Ok(name)
}

/// A path in the spelling a person would type, with `/` separators.
///
/// `canonicalize` answers in the verbatim `\\?\` form on Windows, and a drive letter or a UNC share
/// behind that prefix is a Win32 spelling detail rather than something a person wrote. Paths reach
/// the filesystem as [`PathBuf`]s and are only spelled out here, so removing the prefix cannot change
/// what is written; it only stops the report from showing `//?/C:/work/project`.
pub(crate) fn display(path: &Path) -> String {
    let spelled = path.to_string_lossy();
    let spelled = if let Some(unc) = spelled.strip_prefix(r"\\?\UNC\") {
        return format!("//{unc}").replace('\\', "/");
    } else {
        spelled.strip_prefix(r"\\?\").unwrap_or(&spelled)
    };
    spelled.replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{ProjectError, display, project_name, resolve_root};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn the_project_name_comes_from_the_directory() {
        assert_eq!(
            project_name(Path::new("/srv/payments-ledger")).expect("named"),
            "payments-ledger"
        );
        assert_eq!(
            project_name(Path::new("/srv/payments.git")).expect("named"),
            "payments",
            "a bare repository is named by the repository, not the checkout"
        );
        assert!(
            project_name(Path::new("/")).is_err(),
            "a filesystem root has no name to take"
        );
    }

    #[test]
    fn a_directory_that_does_not_exist_is_refused_rather_than_invented() {
        let missing = Path::new("/srv/definitely-not-here");
        let error = resolve_root(Some(&missing)).expect_err("must not invent a directory");
        let (path, reason) = error.into_parts();
        assert!(path.contains("definitely-not-here"), "{path}");
        assert!(reason.contains("does not exist"), "{reason}");
    }

    #[test]
    fn a_file_is_not_a_project() {
        let root = TempDir::new().expect("temp dir");
        let file = root.path().join("notes.txt");
        fs::write(&file, "text").expect("writes");
        let error = resolve_root(Some(&file)).expect_err("a file is not a project");
        assert!(error.to_string().contains("a file, not a directory"), "{error}");
    }

    #[test]
    fn every_failure_carries_a_code_a_fix_and_an_exit() {
        let error = ProjectError::UnusableRoot {
            path: "/tmp/x".to_string(),
            reason: "it does not exist".to_string(),
        };
        assert_eq!(error.code(), "PROJ-001");
        assert!(error.remediation().contains("--cwd"));
        assert_eq!(error.exit().code(), 2);
    }

    #[test]
    fn a_windows_path_is_spelled_the_way_a_person_typed_it() {
        assert_eq!(
            display(Path::new(r"\\?\C:\work\project")),
            "C:/work/project",
            "the verbatim prefix is a Win32 detail, not part of the path"
        );
        assert_eq!(display(Path::new("plain/path")), "plain/path");
    }
}