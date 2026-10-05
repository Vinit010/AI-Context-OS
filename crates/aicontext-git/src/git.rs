//! The git subprocess: one struct, one spawn path, no shell.
//!
//! Everything that reads a repository goes through [`Git::run`]. Concentrating it here means the
//! argv-array rule, the exit-status mapping, and the environment policy are stated once and cannot
//! drift between call sites — the failure mode of a wrapper where each query builds its own
//! `Command` is that one of them forgets an argument and a branch name starts working on some
//! machines.
//!
//! # The environment
//!
//! Two constructors, because two callers need two things and conflating them would make one of them
//! wrong:
//!
//! - [`Git::new`] inherits the process environment. Production must: the developer's `core.autocrlf`,
//!   their `init.defaultBranch`, their `include.path`, and their credential helpers are all real
//!   configuration, and a tool that hid them would misreport their repository.
//! - [`Git::with_environment`] supplies the variables explicitly and inherits nothing. This is what
//!   makes the tests in this crate hermetic — `CONVENTIONS.md` §8 forbids a test from touching the
//!   developer's home directory, and the alternative, mutating the process environment, is unsound in
//!   a test harness that runs tests on several threads.
//!
//! Neither clears anything the other needs: `Git::new` does not clear the environment, and
//! `Git::with_environment` does not clear it either unless it is told to, because a caller building a
//! hermetic environment usually still wants `PATH` so git can be found at all.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::error::GitError;

/// The program name looked up on `PATH` by [`Git::new`].
const GIT: &str = "git";

/// A git repository or working tree that can be asked questions.
///
/// Holds no state beyond where to run and how to run, and never caches a result: a caller that reads
/// the branch and then writes a file and reads the status again must see the change. Caching would be
/// faster and wrong.
#[derive(Clone, Debug)]
pub struct Git {
    /// The directory git is run in. Not necessarily the repository root.
    root: PathBuf,
    /// The program to run.
    program: OsString,
    /// The environment, or `None` to inherit the process environment.
    environment: Option<Vec<(OsString, OsString)>>,
}

impl Git {
    /// A wrapper that runs `git` from `PATH`, in `root`, with the ambient environment.
    ///
    /// This is the constructor production code should use. It does not check that git exists: doing so
    /// would spawn a process on every construction to answer a question the first real query answers
    /// anyway, and a repository that is not there fails with a better message than a probe would.
    ///
    /// `root` need not be the repository root — it is the directory git is run *in*, so a caller can
    /// point at a subdirectory and still ask about the repository containing it.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            program: OsString::from(GIT),
            environment: None,
        }
    }

    /// A wrapper that runs `program` in `root` with exactly the variables given, inheriting nothing.
    ///
    /// Exists so this crate's own tests can be hermetic without touching the process environment,
    /// and so a caller reproducing a specific environment — a hook, a CI runner, a user's own
    /// reproduction case — can do so exactly rather than approximately.
    ///
    /// A variable whose value is an empty [`OsString`] is removed rather than set to empty, because
    /// that is the only way to unset an inherited variable through `Command`, and `GIT_ASKPASS` is
    /// the case that matters: an empty value is still a program git will try to run.
    #[must_use]
    pub fn with_environment(
        root: impl Into<PathBuf>,
        program: impl Into<OsString>,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Self {
        Self {
            root: root.into(),
            program: program.into(),
            environment: Some(
                environment
                    .into_iter()
                    .map(|(name, value)| (name, value))
                    .collect(),
            ),
        }
    }

    /// The directory git is run in.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The program that will be run.
    #[must_use]
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Whether `root` is inside a Git working tree.
    ///
    /// The probe is `git rev-parse --is-inside-work-tree`, which exits 0 both in a repository with
    /// commits and in one without, and 128 outside a repository — measured on git 2.51. That matters
    /// because the obvious alternative, looking for a `.git` directory, is wrong in both directions:
    /// it misses a worktree or submodule whose `.git` is a *file* pointing elsewhere, and it claims a
    /// bare repository is a working tree when it is not.
    ///
    /// This is the call a command makes to decide whether to report repository facts at all, and it
    /// returns `false` rather than an error so that "not a repository" stays a state a caller can
    /// handle: `aicontext status` outside a repository is a normal answer, not a failure.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Unavailable`] when git cannot be started, and [`GitError::CommandFailed`]
    /// when it runs but exits for a reason other than "no repository here" — a corrupt `.git`, for
    /// instance, which is a real problem and must not be reported as "not a repository".
    pub fn is_repository(&self) -> Result<bool, GitError> {
        Ok(self.output(&["rev-parse", "--is-inside-work-tree"])?.status.success())
    }

    /// Runs one git command and returns its raw standard output.
    ///
    /// This is the only place a process is started. The arguments are passed to [`Command`] as an
    /// argv array, so a branch name, a path, or a commit message can never be interpreted as a shell
    /// command — `RULES.md` §7 forbids shell execution and this is what compliance looks like.
    ///
    /// A non-zero exit is not an error here: `log` on an empty repository and `rev-parse` outside one
    /// both fail, and the caller decides what that means. Use [`Git::run`] when a failure should be
    /// one.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Unavailable`] when the process cannot be started at all.
    pub fn output(&self, args: &[&str]) -> Result<Output, GitError> {
        let mut command = Command::new(&self.program);
        command
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(args.iter().map(OsStr::new));
        match &self.environment {
            // Explicit environment: start from nothing, so nothing the caller did not name is
            // inherited. This is the only branch that clears.
            Some(environment) => {
                command.env_clear();
                for (name, value) in environment {
                    if value.is_empty() {
                        command.env_remove(name);
                    } else {
                        command.env(name, value);
                    }
                }
            }
            // No explicit environment: inherit the process environment untouched. Production needs
            // the developer's real git configuration, and clearing here would hide core.autocrlf,
            // init.defaultBranch, include.path, and their credential helpers — reporting the
            // repository as something it is not.
            None => {}
        }

        command.output().map_err(|source| GitError::Unavailable {
            program: self.program.to_string_lossy().into_owned(),
            source,
        })
    }

    /// Runs one git command and returns its standard output, treating a non-zero exit as an error.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Unavailable`] when git cannot be started,
    /// [`GitError::Terminated`] when it was killed before exiting — which is distinguishable from a
    /// failure because a killed process has no exit status — and [`GitError::CommandFailed`]
    /// otherwise, carrying the argv, the status, and git's own stderr.
    pub fn run(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let argv: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let output = self.output(args)?;
        let status = output.status;
        if status.success() {
            return Ok(output.stdout);
        }
        let Some(code) = status.code() else {
            return Err(GitError::Terminated { argv });
        };
        Err(GitError::CommandFailed {
            argv,
            status: code,
            stderr: String::from_utf8_lossy(&output.stderr).trim_end().to_owned(),
        })
    }

    /// Runs one git command and decodes its standard output as UTF-8.
    ///
    /// # Errors
    ///
    /// As [`Git::run`], plus [`GitError::UnreadableOutput`] when the output is not UTF-8.
    pub fn text(&self, args: &[&str]) -> Result<String, GitError> {
        let command = describe(args);
        let bytes = self.run(args)?;
        String::from_utf8(bytes).map_err(|source| GitError::UnreadableOutput { command, source })
    }
}

/// A stable name for a command, used in the message of a decoding failure.
///
/// A `&'static str` is needed because [`GitError::UnreadableOutput`] records which command produced
/// the bytes, and a `String` there would have to be owned by the error for no benefit: the value is
/// only ever rendered.
fn describe(args: &[&str]) -> &'static str {
    match args {
        ["status", ..] => crate::porcelain::STATUS_COMMAND,
        _ => "git",
    }
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn a_status_failure_names_the_full_porcelain_invocation() {
        // The error has to say what produced the bytes, and `status` alone would leave the reader
        // guessing whether the raw or the porcelain form failed.
        assert_eq!(
            describe(&["status", "--porcelain=v2", "--branch", "-z"]),
            "status --porcelain=v2 --branch -z"
        );
    }

    #[test]
    fn any_other_command_is_named_generically() {
        // `describe` cannot borrow a formatted string, so a command it does not recognise gets a
        // generic name. That is a real limitation and the test says so rather than hiding it.
        assert_eq!(describe(&["log", "--oneline"]), "git");
    }
}