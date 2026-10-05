//! A temporary project directory that cleans itself up.
//!
//! Every test in this workspace that touches the filesystem needs the same two things: a directory
//! that is gone when the test ends, and a project root with a name a developer would recognise.
//! [`TempProject`] is both, so no test spells out `TempDir::new()`, joins a name onto it, and hands
//! back a tuple.
//!
//! # Why the name is not the temp directory's own
//!
//! Several checks report a path relative to the project root, and `aicontext status` takes its
//! project name from the root directory. A test whose root is `C:\Users\me\AppData\Local\Temp\.tmpXyZ`
//! therefore reports a name no developer could recognise and one that differs between runs. The
//! project lives at `<temp>/<name>`, so the name is the same on every machine and every run.
//!
//! # Holding the guard
//!
//! The struct owns its [`tempfile::TempDir`], which deletes the tree on drop. A test therefore
//! keeps the [`TempProject`] alive for as long as it needs the path, and drops it — deleting
//! everything — at the end of its scope. Dropping it early deletes files a later assertion was
//! about to read, which fails confusingly rather than loudly.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// A temporary directory that is a project root, removed when this value is dropped.
///
/// ```
/// use aicontext_testkit::TempProject;
///
/// let project = TempProject::new("payments-ledger").expect("a temporary project");
/// project.write("README.md", "# Ledger\n").expect("writes");
///
/// assert!(project.path().join("README.md").is_file());
/// assert_eq!(project.path().file_name().expect("named"), "payments-ledger");
/// ```
#[derive(Debug)]
pub struct TempProject {
    guard: TempDir,
    root: PathBuf,
}

impl TempProject {
    /// Creates `<temp>/<name>` and returns it as a project root.
    ///
    /// `name` is the directory the developer would recognise, and it becomes the project name that
    /// commands derive from the root. A name containing a path separator is rejected rather than
    /// sanitised, because the name is meant to be asserted on and a silently rewritten one would
    /// make a test pass for a different project than it created.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::UnusableName`] for an empty name or one carrying a separator, and
    /// [`FixtureError::Io`] when the directory cannot be created.
    pub fn new(name: &str) -> Result<Self, FixtureError> {
        if name.is_empty() {
            return Err(FixtureError::UnusableName {
                name: name.to_owned(),
                reason: "it is empty, so the project would have no name to report",
            });
        }
        if name.contains(['/', '\\']) {
            return Err(FixtureError::UnusableName {
                name: name.to_owned(),
                reason: "it carries a path separator, so it is a path rather than a project name",
            });
        }

        let guard = TempDir::new().map_err(|source| FixtureError::Io {
            action: "create a temporary directory",
            source,
        })?;
        let root = guard.path().join(name);
        fs::create_dir_all(&root).map_err(|source| FixtureError::Io {
            action: "create the project root inside the temporary directory",
            source,
        })?;

        Ok(Self { guard, root })
    }

    /// The project root, which is where a command under test is run.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// A path inside the project, whether or not anything exists there yet.
    ///
    /// Takes a `/`-separated relative path so a fixture reads the same as a path in a finding, and
    /// joins it as a single component on Windows rather than letting the separator be re-parsed.
    #[must_use]
    pub fn join(&self, relative: &str) -> PathBuf {
        let mut path = self.root.clone();
        for segment in relative.split('/').filter(|segment| !segment.is_empty()) {
            path.push(segment);
        }
        path
    }

    /// Writes a file at a `/`-separated relative path, creating its parent directories.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::Io`] when a parent directory cannot be created or the file cannot be
    /// written.
    pub fn write(&self, relative: &str, contents: &str) -> Result<PathBuf, FixtureError> {
        let path = self.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| FixtureError::Io {
                action: "create the parent directory of a fixture file",
                source,
            })?;
        }
        fs::write(&path, contents).map_err(|source| FixtureError::Io {
            action: "write a fixture file",
            source,
        })?;
        Ok(path)
    }

    /// Reads a file at a `/`-separated relative path.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::Io`] when the file cannot be read.
    pub fn read(&self, relative: &str) -> Result<String, FixtureError> {
        fs::read_to_string(self.join(relative)).map_err(|source| FixtureError::Io {
            action: "read a fixture file",
            source,
        })
    }

    /// Creates a directory at a `/`-separated relative path.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::Io`] when the directory cannot be created.
    pub fn create_dir(&self, relative: &str) -> Result<PathBuf, FixtureError> {
        let path = self.join(relative);
        fs::create_dir_all(&path).map_err(|source| FixtureError::Io {
            action: "create a fixture directory",
            source,
        })?;
        Ok(path)
    }

    /// Every file under the project root, as `/`-separated relative paths, sorted.
    ///
    /// Used to assert that a command wrote nothing it did not declare, which is the only way to
    /// check "writes nothing at all" without trusting the command's own report. `.git` is excluded
    /// because a repository fixture has one by construction and it is not part of the project.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::Io`] when the tree cannot be walked.
    pub fn files(&self) -> Result<Vec<String>, FixtureError> {
        let mut found = Vec::new();
        collect_files(&self.root, &self.root, &mut found)?;
        found.sort();
        Ok(found)
    }
}

/// Walks `directory` recursively, appending `/`-separated paths relative to `root`.
///
/// A walk that cannot read one entry fails rather than skipping it: a fixture that silently omits a
/// file would let a test assert "the command wrote nothing" while the command wrote something.
fn collect_files(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), FixtureError> {
    let entries = fs::read_dir(directory).map_err(|source| FixtureError::Io {
        action: "read a directory while walking the fixture tree",
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| FixtureError::Io {
            action: "read a directory entry while walking the fixture tree",
            source,
        })?;
        let path = entry.path();
        let is_dir = entry.file_type().map_err(|source| FixtureError::Io {
            action: "read the type of a directory entry",
            source,
        })?.is_dir();

        if is_dir {
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            collect_files(root, &path, found)?;
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
            .map_err(|source| FixtureError::Io {
                action: "report a walked file relative to the project root",
                source: std::io::Error::other(source.to_string()),
            })?;
        found.push(relative);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FixtureError, TempProject};

    #[test]
    fn the_root_is_named_so_the_project_has_a_recognisable_name() {
        let project = TempProject::new("payments-ledger").expect("a temporary project");
        assert_eq!(project.path().file_name().expect("named"), "payments-ledger");
    }

    #[test]
    fn the_tree_is_gone_once_the_value_is_dropped() {
        let path = {
            let project = TempProject::new("scoped").expect("a temporary project");
            project.write("a/b.txt", "text").expect("writes");
            project.path().to_path_buf()
        };
        assert!(!path.exists(), "dropping the project must delete its tree");
    }

    #[test]
    fn a_nested_path_is_created_with_its_parents() {
        let project = TempProject::new("nested").expect("a temporary project");
        project
            .write(".ai/decisions/ADR-001-rust.md", "content")
            .expect("writes");

        assert!(project.join(".ai/decisions/ADR-001-rust.md").is_file());
        assert_eq!(
            project.read(".ai/decisions/ADR-001-rust.md").expect("reads"),
            "content"
        );
    }

    #[test]
    fn a_separated_path_is_one_component_on_every_platform() {
        // `join` splits on `/` and pushes each segment, so a `/` in a fixture path is a separator on
        // Windows too rather than a character a file may be named after.
        let project = TempProject::new("separators").expect("a temporary project");
        project.write(".ai/TASKS.md", "tasks").expect("writes");

        let joined = project.join(".ai/TASKS.md");
        assert_eq!(joined.file_name().expect("named"), "TASKS.md");
        assert_eq!(joined.parent().expect("has a parent").file_name(), Some(".ai".as_ref()));
    }

    #[test]
    fn files_lists_the_whole_tree_sorted_and_relative() {
        let project = TempProject::new("listed").expect("a temporary project");
        project.write("z.txt", "z").expect("writes");
        project.write(".ai/AI.md", "ai").expect("writes");
        project.write("a/b/c.txt", "c").expect("writes");

        assert_eq!(
            project.files().expect("walks"),
            vec!["a/b/c.txt", "z.txt", ".ai/AI.md"],
            "paths are relative, use `/`, and are sorted"
        );
    }

    #[test]
    fn a_git_directory_is_not_part_of_the_project_tree() {
        let project = TempProject::new("withgit").expect("a temporary project");
        project.write(".ai/AI.md", "ai").expect("writes");
        project.create_dir(".git/objects").expect("creates");

        assert_eq!(project.files().expect("walks"), vec![".ai/AI.md"]);
    }

    #[test]
    fn an_empty_or_nested_name_is_refused_rather_than_rewritten() {
        // A name is asserted on by the tests that use it, so sanitising one would let a test pass
        // for a project other than the one it created.
        for name in ["", "nested/name", r"nested\name"] {
            let error = TempProject::new(name).expect_err("must be refused");
            assert!(
                matches!(error, FixtureError::UnusableName { .. }),
                "expected UnusableName for {name:?}, got {error:?}"
            );
        }
    }

    #[test]
    fn every_failure_says_what_to_do_about_it() {
        let error = TempProject::new("").expect_err("must be refused");
        assert_eq!(error.code(), "FIX-001");
        assert!(
            error.remediation().contains("project name"),
            "the hint must name what was wrong: {}",
            error.remediation()
        );
    }
}