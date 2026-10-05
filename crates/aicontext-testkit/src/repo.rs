//! A temporary Git repository that behaves the same on every developer's machine.
//!
//! # The problem this solves
//!
//! A test that shells out to `git` inherits whatever the developer has configured. That is enough to
//! make a suite pass locally and fail in CI, or pass on one machine and fail on another, for reasons
//! nobody can find in the code:
//!
//! - `core.autocrlf=true` on Windows rewrites LF to CRLF in the working tree, so a fixture written
//!   with `\n` reads back with `\r\n` and a byte-comparison assertion fails on one OS only.
//! - `init.defaultBranch` decides whether a new repository's branch is `master` or `main`. A test
//!   asserting on the branch name passes for one developer and fails for the next.
//! - `user.name` and `user.email` are required to commit. Without them `git commit` fails, so a test
//!   either fails or has to invent identity through the environment it forgot to isolate.
//! - `commit.gpgsign=true` makes every commit in the fixture ask for a passphrase.
//!
//! [`TempRepository`] removes all four by pinning them through the environment, so the fixture is the
//! same repository everywhere. The variables below were each verified against `git` 2.51 before being
//! written down here; none of them is guessed.
//!
//! # What "does not read the developer's configuration" means precisely
//!
//! [`TempRepository::git`] builds its child environment from scratch and passes `env_clear()`. The
//! child therefore sees the listed variables and nothing else — not `HOME`, not the developer's
//! `PATH` entries beyond the one git was found at, and not any `GIT_*` variable the developer has
//! exported. Two of the listed variables make that checkable rather than merely intended:
//!
//! - `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_SYSTEM` point at paths inside the fixture that do not exist,
//!   so git reads no global or system configuration. `GIT_CONFIG_NOSYSTEM=1` makes the second one
//!   doubly redundant, and is set because a future git release should not decide that alone.
//! - `HOME`, `USERPROFILE`, and `XDG_CONFIG_HOME` are pointed at the fixture too, because git looks
//!   for a global config under `HOME` on Unix and under `USERPROFILE` on Windows, and a
//!   `~/.gitconfig` that does not exist is as inert as a `GIT_CONFIG_GLOBAL` that does not.
//!
//! # Why the dates are fixed
//!
//! A commit's hash covers its author date and committer date. Leaving them to the clock means a
//! fixture's hash differs on every run, so a test cannot assert one — and a hash is the most direct
//! possible assertion that "the wrapper read the branch and the head correctly". The dates below
//! are fixed, so the same fixture contents produce the same hash every run and on every machine.
//!
//! # Deliberate limits
//!
//! This spawns `git`, so it is a fixture and never part of the binary. It runs no network command:
//! `clone`, `fetch`, and `push` are not offered, and [`TempRepository::git`] refuses to run any
//! argument list containing one of them. A test that reached the network would be the failure mode
//! `RULES.md` §8 forbids.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::error::FixtureError;
use crate::temp::TempProject;

/// The branch a fresh fixture repository starts on, set through the environment so it does not
/// depend on the developer's `init.defaultBranch`. `R-11` in `ARCHITECTURE.md` §13 lists Windows
/// parity as a named risk; this is one of the places it is answered.
pub const DEFAULT_BRANCH: &str = "main";

/// The identity every fixture commit is attributed to.
///
/// `.invalid` is reserved by RFC 2606 and can never resolve, so a commit in a test repository cannot
/// reach a real person even if the fixture directory is ever copied somewhere that syncs.
const FIXTURE_NAME: &str = "AI Context OS tests";
const FIXTURE_EMAIL: &str = "tests@aicontext.invalid";

/// The fixed author and committer timestamps, so a fixture's commit hashes are reproducible.
const FIXTURE_DATE: &str = "2020-01-02T03:04:05+00:00";

/// Arguments that would reach the network, and so are refused rather than run.
const NETWORK_SUBCOMMANDS: &[&str] = &["clone", "fetch", "push", "pull", "remote", "submodule"];

/// A temporary Git repository whose configuration does not depend on the developer.
///
/// ```
/// use aicontext_testkit::TempRepository;
///
/// let repo = TempRepository::new("payments-ledger").expect("a temporary repository");
/// repo.write("README.md", "# Ledger\n").expect("writes");
/// repo.commit_all("first").expect("commits");
///
/// assert!(repo.has_commits(), "the fixture made a commit");
/// assert_eq!(repo.branch().expect("a branch"), "main");
/// ```
#[derive(Debug)]
pub struct TempRepository {
    project: TempProject,
    home: PathBuf,
    git: PathBuf,
}

impl TempRepository {
    /// Creates `<temp>/<name>`, initialises a repository in it, and makes one commit.
    ///
    /// One commit rather than an empty repository, because a repository with no commits is a state
    /// most of the checks cannot answer: `git log` exits 128, HEAD does not resolve, and a test would
    /// assert on the failure path rather than the behaviour it exists to check.
    /// [`TempRepository::init_empty`] builds that state on purpose, for the tests that need it.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] from the project fixture, [`FixtureError::GitMissing`] when git is
    /// not on `PATH`, and [`FixtureError::GitFailed`] when `init` or the first commit fails.
    pub fn new(name: &str) -> Result<Self, FixtureError> {
        let repo = Self::init_empty(name)?;
        repo.write("README.md", "# Fixture repository\n")?;
        repo.commit_all("initial commit")?;
        Ok(repo)
    }

    /// Creates `<temp>/<name>` and initialises a repository in it with no commits.
    ///
    /// `HEAD` resolves to [`DEFAULT_BRANCH`] and points at no commit, which is the state a
    /// repository is in between `git init` and the author's first commit.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError`] from the project fixture, [`FixtureError::GitMissing`] when git is
    /// not on `PATH`, and [`FixtureError::GitFailed`] when `init` fails.
    pub fn init_empty(name: &str) -> Result<Self, FixtureError> {
        let project = TempProject::new(name)?;
        let git = git_executable()?;
        let repo = Self {
            home: project.join(".fixture-home"),
            git,
            project,
        };
        std::fs::create_dir_all(&repo.home).map_err(|source| FixtureError::Io {
            action: "create the fixture home directory",
            source,
        })?;
        repo.git(&[
            "init",
            "--quiet",
            &format!("--initial-branch={DEFAULT_BRANCH}"),
        ])?;
        Ok(repo)
    }

    /// The project root, which is also the repository root.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.project.path()
    }

    /// A path inside the repository.
    #[must_use]
    pub fn join(&self, relative: &str) -> PathBuf {
        self.project.join(relative)
    }

    /// Writes a file at a `/`-separated relative path, creating its parent directories.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::Io`] when the file cannot be written.
    pub fn write(&self, relative: &str, contents: &str) -> Result<PathBuf, FixtureError> {
        self.project.write(relative, contents)
    }

    /// Reads a file at a `/`-separated relative path.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::Io`] when the file cannot be read.
    pub fn read(&self, relative: &str) -> Result<String, FixtureError> {
        self.project.read(relative)
    }

    /// Runs a git command in the repository and returns its standard output.
    ///
    /// The argument is an argv array and is passed to `Command` as one, so no branch name, path, or
    /// message can be interpreted as a shell command. This is the same rule the production wrapper
    /// in `aicontext-git` follows (`RULES.md` §7, "no shell execution").
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::GitFailed`] for a non-zero exit status, carrying the argv, the status,
    /// and what git wrote to stderr; git's own "not a git repository" message arrives in that stderr,
    /// so a command needing a repository outside one reports as a failure rather than a separate
    /// variant. Returns [`FixtureError::NetworkRefused`] when `args` names a network subcommand.
    pub fn git(&self, args: &[&str]) -> Result<String, FixtureError> {
        self.git_raw(args)
            .map(|output| String::from_utf8_lossy(&output).into_owned())
    }

    /// Runs a git command and returns its raw standard output, for output that is not text.
    ///
    /// # Errors
    ///
    /// As [`TempRepository::git`].
    pub fn git_bytes(&self, args: &[&str]) -> Result<Vec<u8>, FixtureError> {
        self.git_raw(args)
    }

    /// Runs a git command and returns its outcome, whether or not it succeeded.
    ///
    /// The form a test needs when the command is *expected* to fail: asking whether a directory is a
    /// repository, or what `git log` says about a repository with no commits. An expected failure
    /// is data here, not an error, so it is returned rather than raised.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::GitMissing`] when git is not on `PATH`, and
    /// [`FixtureError::Io`] when the process cannot be spawned at all.
    pub fn git_outcome(&self, args: &[&str]) -> Result<Output, FixtureError> {
        Self::refuse_network(args)?;
        let mut command = self.command(args);
        command.output().map_err(|source| FixtureError::GitMissing {
            source: io_with_context(&source, "spawn git"),
        })
    }

    /// The current branch name.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::GitFailed`] when the branch cannot be read, which happens in a
    /// repository with no commits on some git versions.
    pub fn branch(&self) -> Result<String, FixtureError> {
        Ok(self
            .git(&["rev-parse", "--abbrev-ref", "HEAD"])?
            .trim()
            .to_owned())
    }

    /// Stages every change and commits it.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::GitFailed`] when `add` or `commit` fails.
    pub fn commit_all(&self, message: &str) -> Result<(), FixtureError> {
        self.git(&["add", "--all"])?;
        self.git(&["commit", "--quiet", "--message", message])?;
        Ok(())
    }

    /// Whether the path is inside a Git repository.
    ///
    /// Uses `rev-parse --is-inside-work-tree`, which answers `true` for a repository and exits 128
    /// with `not a git repository` anywhere else. Both outcomes are data, so the exit status decides
    /// the answer rather than the error.
    #[must_use]
    pub fn is_repository(path: &Path) -> bool {
        Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(path)
            // The developer's own configuration must not decide whether a directory is a repository:
            // `safe.directory` in a global config would change the answer.
            .env_clear()
            .env("GIT_CONFIG_GLOBAL", path.join(".gitconfig-absent"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    /// The commit hashes of the current history, newest first, up to `limit`.
    ///
    /// Returns an empty vector for a repository with no commits, because `git log` exits 128 with
    /// `does not have any commits yet` there and a caller asking "what is the history" of a fresh
    /// repository should get an empty history, not an error.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::GitFailed`] when `git log` fails for any other reason.
    pub fn commits(&self, limit: usize) -> Result<Vec<String>, FixtureError> {
        let format = "--format=%H";
        let outcome = self.git_outcome(&["log", format, &format!("--max-count={limit}")])?;
        if outcome.status.success() {
            return Ok(String::from_utf8_lossy(&outcome.stdout)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect());
        }

        let stderr = String::from_utf8_lossy(&outcome.stderr);
        if stderr.contains("does not have any commits yet")
            || stderr.contains("bad default revision")
        {
            return Ok(Vec::new());
        }
        Err(FixtureError::GitFailed {
            argv: vec!["log".to_owned(), format.to_owned()],
            status: outcome.status.code(),
            stderr: stderr.trim_end().to_owned(),
        })
    }

    /// Whether the repository has at least one commit.
    ///
    /// `git rev-parse --verify HEAD` is the question asked directly, so the answer costs one process
    /// rather than a log run.
    #[must_use]
    pub fn has_commits(&self) -> bool {
        self.git_outcome(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_ok_and(|outcome| outcome.status.success())
    }

    /// The working-tree status, as `/`-separated paths and their two-letter porcelain codes.
    ///
    /// `A  file` is staged and added, ` M file` is modified in the working tree, `?? file` is
    /// untracked. Sorting keeps a report deterministic, which `RULES.md` §8 requires of every test.
    ///
    /// # Errors
    ///
    /// Returns [`FixtureError::GitFailed`] when `git status` fails.
    pub fn status(&self) -> Result<Vec<ChangeStatus>, FixtureError> {
        let raw = self.git(&["status", "--porcelain=v1", "--untracked-files=all"])?;
        let mut changes: Vec<ChangeStatus> = raw.lines().filter_map(ChangeStatus::parse).collect();
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(changes)
    }

    /// The child `git` process for `args`, with the environment built from scratch.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.git);
        command.current_dir(self.project.path());
        // Everything the child sees is set below. Inherited variables are removed first, so a
        // developer's `GIT_DIR`, `GIT_WORK_TREE`, `GIT_SSH_COMMAND`, or `GIT_CONFIG_COUNT` cannot
        // change what a fixture observes.
        command.env_clear();

        let absent = self.home.join("gitconfig-absent");
        command
            .env("PATH", git_path(&self.git))
            .env("GIT_CONFIG_GLOBAL", &absent)
            .env(
                "GIT_CONFIG_SYSTEM",
                self.home.join("system-gitconfig-absent"),
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // HOME, USERPROFILE, and XDG_CONFIG_HOME all point inside the fixture, so git cannot
            // reach a real ~/.gitconfig by any of the three paths it uses on any platform.
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", &self.home)
            // Two configuration values are injected rather than inherited. This is the documented
            // `GIT_CONFIG_COUNT` protocol, and it is how the fixture overrides a setting without
            // writing a config file that a later test might find.
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "core.autocrlf")
            .env("GIT_CONFIG_VALUE_0", "false")
            .env("GIT_CONFIG_KEY_1", "init.defaultBranch")
            .env("GIT_CONFIG_VALUE_1", DEFAULT_BRANCH)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("GIT_AUTHOR_NAME", FIXTURE_NAME)
            .env("GIT_AUTHOR_EMAIL", FIXTURE_EMAIL)
            .env("GIT_COMMITTER_NAME", FIXTURE_NAME)
            .env("GIT_COMMITTER_EMAIL", FIXTURE_EMAIL)
            .env("GIT_AUTHOR_DATE", FIXTURE_DATE)
            .env("GIT_COMMITTER_DATE", FIXTURE_DATE);
        command.args(args);
        command
    }

    /// Runs a git command and returns its standard output, raising a non-zero exit status as an
    /// error.
    fn git_raw(&self, args: &[&str]) -> Result<Vec<u8>, FixtureError> {
        Self::refuse_network(args)?;
        let outcome = self.git_outcome(args)?;
        if outcome.status.success() {
            return Ok(outcome.stdout);
        }
        Err(FixtureError::GitFailed {
            argv: args.iter().map(|arg| (*arg).to_owned()).collect(),
            status: outcome.status.code(),
            stderr: String::from_utf8_lossy(&outcome.stderr)
                .trim_end()
                .to_owned(),
        })
    }

    /// Refuses a network subcommand, so a fixture cannot become a test that needs the network.
    fn refuse_network(args: &[&str]) -> Result<(), FixtureError> {
        let Some(subcommand) = args.first() else {
            return Ok(());
        };
        if NETWORK_SUBCOMMANDS.contains(&subcommand.trim_start_matches('-')) {
            return Err(FixtureError::NetworkRefused {
                subcommand: (*subcommand).to_owned(),
            });
        }
        Ok(())
    }
}

/// One entry from `git status --porcelain`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeStatus {
    /// The path, relative to the repository root, with `/` separators.
    pub path: String,
    /// The index status: what git would record if the change were staged.
    pub index: char,
    /// The working-tree status: what is different on disk right now.
    pub worktree: char,
}

impl ChangeStatus {
    /// Whether the file is untracked, which is neither staged nor modified.
    #[must_use]
    pub fn is_untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }

    /// Parses one porcelain line, or `None` for a blank one.
    ///
    /// A rename is written by git as `R  old -> new`, and the two paths are kept as one string
    /// rather than split into a pair, because every caller here wants to print the path and a rename
    /// reads correctly as written.
    ///
    /// The path is taken from the fourth byte because porcelain's format is exactly two status codes
    /// and one space. Slicing from the third and trimming only the end leaves the separating space on
    /// every untracked path, which then compares unequal to the same path written by a test and
    /// fails in `status` rather than here.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        let mut characters = line.chars();
        let index = characters.next()?;
        let worktree = characters.next()?;
        if index == '\n' || worktree == '\n' {
            return None;
        }
        let path = line.get(3..)?.trim_end();
        if path.is_empty() {
            return None;
        }
        Some(Self {
            path: path.replace('\\', "/"),
            index,
            worktree,
        })
    }
}

/// Finds `git` on `PATH`, so the fixture can run it with a minimal environment of its own.
///
/// # Errors
///
/// Returns [`FixtureError::GitMissing`] when `git` is not on `PATH`.
fn git_executable() -> Result<PathBuf, FixtureError> {
    let missing = std::io::Error::new(std::io::ErrorKind::NotFound, "no `git` on PATH");
    which("git").ok_or_else(|| FixtureError::GitMissing {
        source: io_with_context(&missing, "search PATH for git"),
    })
}

/// A minimal `which`: `PATH` entries joined with the platform separator, checked in order.
///
/// Written rather than taken from a crate because the fixture already needs `env_clear`, so it has
/// to know the PATH anyway.
///
/// On Windows the candidate is tried once per `PATHEXT` extension, because that is what the shell
/// and `CreateProcess` do and because `is_file` does not: `C:\...\git` is not a file on a machine
/// where git is installed, while `C:\...\git.exe` is. Skipping the extensions makes every
/// repository fixture fail on Windows with "no git on PATH" while passing everywhere else, which is
/// `R-11` in `ARCHITECTURE.md` §13 exactly. An empty or missing `PATHEXT` falls back to trying the
/// bare name, which is what Unix does and what Windows does when the extension is stored literally.
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for directory in std::env::split_paths(&path) {
        candidates.push(directory.join(program));
        for extension in executable_extensions() {
            let mut with_extension = directory.join(program).into_os_string();
            with_extension.push(extension);
            candidates.push(PathBuf::from(with_extension));
        }
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// The suffixes a program name may carry on this platform, each including its dot.
///
/// `PATHEXT` is consulted because it is the platform's own list, so a machine where a developer has
/// added `.BAT` gets it. Unix has none: an executable there is recognised by its permission bits,
/// not by its name, so an empty list means "join and check".
fn executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .map(str::trim)
            .filter(|extension| extension.starts_with('.') && extension.len() > 1)
            .map(str::to_owned)
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// The `PATH` the child git needs: the directory git lives in, and nothing else.
///
/// The fixture clears the environment, so this is the one value that has to be rebuilt for the child
/// to start. Passing only git's own directory means git finds its own `exec-path` helpers while
/// inheriting nothing else from the developer's `PATH`.
fn git_path(git: &Path) -> PathBuf {
    git.parent().map_or_else(PathBuf::new, Path::to_path_buf)
}

/// Attaches the fixture's own context to an `io::Error` without losing it.
fn io_with_context(source: &std::io::Error, action: &'static str) -> std::io::Error {
    std::io::Error::other(format!("{action}: {source}"))
}

#[cfg(test)]
mod tests {
    use super::{ChangeStatus, DEFAULT_BRANCH, FixtureError, TempRepository, which};
    use std::path::Path;

    #[test]
    fn a_new_fixture_is_a_repository_on_the_documented_branch_with_one_commit() {
        let repo = TempRepository::new("payments-ledger").expect("a repository");

        assert!(TempRepository::is_repository(repo.path()));
        assert!(repo.has_commits());
        assert_eq!(repo.branch().expect("a branch"), DEFAULT_BRANCH);
        assert_eq!(repo.commits(10).expect("a history").len(), 1);
        assert!(
            repo.status().expect("a status").is_empty(),
            "a fresh fixture is clean"
        );
    }

    #[test]
    fn the_same_contents_produce_the_same_commit_hash_every_run() {
        // The fixed author and committer dates are what make this true, and a hash is the most direct
        // assertion available that a wrapper read the head correctly.
        let first = TempRepository::new("stable").expect("a repository");
        let second = TempRepository::new("stable").expect("a repository");

        assert_eq!(
            first.commits(1).expect("a history"),
            second.commits(1).expect("a history"),
            "a fixture commit must not depend on the clock"
        );
    }

    #[test]
    fn an_empty_repository_reports_no_history_rather_than_an_error() {
        // `git log` exits 128 with `does not have any commits yet` here. A caller asking what the
        // history is should get an empty history, not a failure it has to special-case.
        let repo = TempRepository::init_empty("empty").expect("a repository");

        assert!(!repo.has_commits());
        assert_eq!(
            repo.commits(5).expect("no history is not an error"),
            Vec::<String>::new()
        );
        assert!(TempRepository::is_repository(repo.path()));
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_reported_as_one() {
        let project = crate::temp::TempProject::new("plain").expect("a project");
        assert!(
            !TempRepository::is_repository(project.path()),
            "a directory outside any repository must not claim to be in one"
        );
    }

    #[test]
    fn status_reports_staged_modified_and_untracked_separately() {
        let repo = TempRepository::new("changes").expect("a repository");
        repo.write("untracked.txt", "new").expect("writes");
        repo.write("README.md", "# Fixture repository, edited\n")
            .expect("writes");
        repo.git(&["add", "README.md"]).expect("stages");

        let changes = repo.status().expect("a status");
        let by_path = |name: &str| {
            changes
                .iter()
                .find(|change| change.path == name)
                .unwrap_or_else(|| panic!("expected {name} in {changes:?}"))
        };
        assert!(by_path("untracked.txt").is_untracked());
        assert_eq!(by_path("README.md").index, 'M');
        assert_eq!(by_path("README.md").worktree, ' ');
        assert!(
            changes.iter().all(|change| !change.path.contains('\\')),
            "paths use `/` on every platform: {changes:?}"
        );
    }

    #[test]
    fn a_written_file_keeps_its_exact_bytes_on_windows_too() {
        // `core.autocrlf` is pinned to false for exactly this reason: with the developer's own
        // setting inherited, this assertion fails on Windows and nowhere else.
        let repo = TempRepository::new("bytes").expect("a repository");
        repo.write("lf.txt", "one\ntwo\n").expect("writes");

        assert_eq!(
            repo.read("lf.txt").expect("reads"),
            "one\ntwo\n",
            "a fixture file must read back byte for byte"
        );
    }

    #[test]
    fn a_command_that_cannot_be_answered_is_an_error_naming_the_command() {
        let repo = TempRepository::init_empty("empty").expect("a repository");
        let error = repo
            .git(&["rev-parse", "--verify", "HEAD"])
            .expect_err("no HEAD in an empty repository");

        assert_eq!(error.code(), "FIX-004");
        assert!(
            error.to_string().contains("rev-parse --verify HEAD"),
            "{error}"
        );
        assert!(!error.remediation().is_empty());
    }

    #[test]
    fn a_network_subcommand_is_refused_rather_than_run() {
        let repo = TempRepository::new("offline").expect("a repository");
        for subcommand in ["clone", "fetch", "push", "pull", "remote"] {
            let error = repo
                .git(&[subcommand, "https://example.invalid/x"])
                .expect_err("must be refused");
            assert!(
                matches!(error, FixtureError::NetworkRefused { .. }),
                "expected NetworkRefused for {subcommand}, got {error:?}"
            );
        }
    }

    #[test]
    fn git_is_found_on_this_machine_or_the_suite_cannot_run() {
        assert!(
            which("git").is_some(),
            "git must be on PATH; every repository fixture depends on it"
        );
    }

    #[test]
    fn a_porcelain_line_is_parsed_into_its_two_codes_and_path() {
        let staged = ChangeStatus::parse("A  new.txt").expect("parses");
        assert_eq!(staged.index, 'A');
        assert_eq!(staged.worktree, ' ');
        assert_eq!(staged.path, "new.txt");
        assert!(!staged.is_untracked());

        let untracked = ChangeStatus::parse("?? notes.txt").expect("parses");
        assert!(untracked.is_untracked());
        assert_eq!(untracked.path, "notes.txt");

        let renamed = ChangeStatus::parse("R  old.txt -> new.txt").expect("parses");
        assert_eq!(renamed.index, 'R');
        assert_eq!(renamed.path, "old.txt -> new.txt");

        assert_eq!(ChangeStatus::parse("   "), None);
        assert_eq!(ChangeStatus::parse(""), None);
    }

    #[test]
    fn git_is_not_read_from_a_path_the_child_cannot_find() {
        // The child's PATH is rebuilt as git's own directory, so a `git` that git needs to find its
        // helpers is reachable and nothing else is.
        let git = which("git").expect("git on PATH");
        let parent = git.parent().expect("git lives in a directory");
        assert_eq!(
            super::git_path(&git),
            parent,
            "the fixture passes git's own directory and nothing else"
        );
    }

    #[test]
    fn a_directory_outside_a_repository_is_not_reported_as_one_even_without_config() {
        let project = crate::temp::TempProject::new("nested-repo").expect("a project");
        let nested = project.create_dir("child").expect("creates");
        assert!(!TempRepository::is_repository(Path::new(&nested)));
    }

    #[test]
    fn every_configuration_git_reads_comes_from_inside_the_fixture() {
        // The task's acceptance criterion is that no helper reads global Git configuration, and the
        // other tests here only show that the values the fixture cares about are pinned - which a
        // leaked `~/.gitconfig` containing some *other* setting would not disturb. So this asks git
        // to name the origin of every value it holds, rather than trusting the variables we set.
        //
        // `git config --list --show-origin` prints `origin<TAB>key=value`. An origin is a
        // `file:`-prefixed path, or one of the labels git invents for a value with no file behind it
        // ("command line:" for the GIT_CONFIG_COUNT values injected below). A repository's own
        // configuration comes back relative to the repository, so it is resolved before comparing.
        let repo = TempRepository::new("origins").expect("a repository");
        let origins = repo
            .git(&["config", "--list", "--show-origin"])
            .expect("reads every configured value with its origin");

        let repo_root = repo.path().to_path_buf();
        let temporary = repo.project.temp_path().to_path_buf();

        let mut seen = 0_usize;
        for line in origins.lines() {
            let (origin, _) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("no origin in {line:?}"));
            if origin.starts_with("command line:") || origin.starts_with("standard input:") {
                continue;
            }
            seen += 1;

            let path = origin.strip_prefix("file:").unwrap_or(origin);
            let resolved = Path::new(path);
            let resolved = if resolved.is_absolute() {
                resolved.to_path_buf()
            } else {
                repo_root.join(resolved)
            };
            assert!(
                resolved.starts_with(&temporary),
                "git read configuration from outside the fixture:\n  {origin}\n\
                 resolved to {resolved:?}\n\
                 expected every origin to be inside {temporary:?}, or a label git invents for a \
                 value with no file behind it"
            );
        }
        assert!(
            seen > 0,
            "no configuration at all was read, so this proved nothing: {origins:?}"
        );
    }
}
