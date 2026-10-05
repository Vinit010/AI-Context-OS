//! The questions `aicontext` asks a repository.
//!
//! One function per question, each returning typed data rather than text, and each documented with the
//! git invocation it wraps. The point of the type signatures is that the three awkward repository
//! states — not a repository, no commits yet, detached head — are visible in the types rather than in
//! a string a caller has to recognise. `aicontext status` (TASK-013) is the first consumer, and it
//! degrades outside a repository as one of its acceptance criteria, so that path is built here rather
//! than left to the caller.
//!
//! # Why the invocations are the ones they are
//!
//! Every choice below was measured against git 2.51 rather than taken from the documentation's
//! intent. The alternatives are named where they were rejected, because "why not just call X" is the
//! question a reviewer asks first.
//!
//! | Question | Command | Why not the obvious alternative |
//! |----------|---------|---------------------------------|
//! | branch and changes together | `status --porcelain=v2 --branch -z` | `branch --show-current` exits 0 and prints *nothing* on an unborn head, indistinguishable from a detached head without a second call |
//! | changed files | the `status` above | `diff --name-only` misses untracked files, which are changes |
//! | recent commits | `log --no-color --format=…` | the default format is for humans and changes between versions |
//!
//! # Nothing here writes
//!
//! Every command is a read. The wrapper does not implement `commit`, `checkout`, `push`, or anything
//! else that changes the repository, and the argv array is fixed at each call site rather than
//! accepted from a caller, so there is no argument a caller can pass that turns a read into a write.
//! That is why [`Git::output`] takes `&[&str]` from this module's call sites and the public methods
//! take no arguments at all.

use std::path::PathBuf;

use crate::error::GitError;
use crate::git::Git;
use crate::porcelain::{self, Change, Head};

/// The byte commits and fields are split on, written by git as the `%x00` escape.
///
/// NUL rather than a printable delimiter because a commit subject may contain any printable
/// character - including every delimiter that would be convenient, such as a tab, a pipe, or a
/// percent sign - so no printable choice is safe. It also cannot be a control character: git
/// refuses to *store* a NUL in a commit message, which is the one guarantee stronger than "sanitises
/// newlines" and therefore the only byte a subject provably cannot contain.
///
/// Field and record boundaries both use it, so the output is one flat stream of
/// `FIELDS_PER_COMMIT` tokens per commit and is chunked rather than split into records first. Using
/// one separator for both means the chunk size is the only thing that has to be right, and it is a
/// constant next to this one.
const NUL: u8 = 0;

/// How many tokens one commit occupies in that stream.
const FIELDS_PER_COMMIT: usize = 6;

/// The `--format` that produces exactly those tokens, in the order [`Commit`] declares them.
///
/// Written out once here rather than assembled, so the field order and the struct cannot drift apart
/// silently. `%aI` is the author's date in strict ISO 8601, a fixed format across git versions
/// whereas the default date rendering is not.
const LOG_FORMAT: &str = "--format=%H%x00%h%x00%an%x00%ae%x00%aI%x00%s%x00";

/// One commit, as read from the log.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Commit {
    /// The full 40-character hash.
    pub hash: String,
    /// The abbreviated hash, as git renders it.
    pub short_hash: String,
    /// The author's name, exactly as committed. Not necessarily a current git identity.
    pub author: String,
    /// The author's email address, exactly as committed.
    pub author_email: String,
    /// The author date in ISO 8601, for example `2026-09-27T14:03:22+02:00`.
    ///
    /// Kept as git's own ISO 8601 text rather than parsed into a date type: `RULES.md` §4.2 keeps the
    /// vocabulary of dates in `aicontext-core`, and a git wrapper has no business inventing one. A
    /// caller that needs to compare dates parses them.
    pub authored: String,
    /// The commit subject: the first line of the message.
    pub subject: String,
}

/// A repository read, in one pass.
///
/// Returned by [`Repository::snapshot`] so a caller that needs the branch *and* the changes pays for
/// one `status` rather than two, and so the two can never disagree — a branch and a file list taken
/// from two separate invocations describe two different moments.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Snapshot {
    /// Where `HEAD` points.
    pub head: Head,
    /// Every changed file, in the order git reported them.
    pub changes: Vec<Change>,
}

impl Snapshot {
    /// The changed files, as native paths relative to the repository root.
    ///
    /// Native rather than `/`-separated, because this is the boundary where a parsed path becomes
    /// something a caller hands to the filesystem.
    #[must_use]
    pub fn changed_paths(&self) -> Vec<PathBuf> {
        self.changes
            .iter()
            .map(|change| porcelain::to_path(change.path()))
            .collect()
    }

    /// Whether the working tree has no changes at all.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.changes.is_empty()
    }
}

/// A repository that has been confirmed to exist.
///
/// The distinction from [`Git`] is the whole point of this type: to hold one, a caller has already
/// established that the directory is inside a working tree, so no method here has to re-probe or
/// defend against it. A `Repository` cannot be constructed directly — see
/// [`Git::open`] — which turns "I forgot to check whether this is a repository" from a runtime error
/// message into a compile error at the call site.
#[derive(Clone, Debug)]
pub struct Repository {
    git: Git,
    root: PathBuf,
}

impl Repository {
    /// The directory git is run in.
    #[must_use]
    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// The wrapper this repository was opened with, for callers that need a lower-level question.
    #[must_use]
    pub fn git(&self) -> &Git {
        &self.git
    }

    /// Reads the head and every changed file in one git invocation.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Unavailable`] when git cannot be started, and [`GitError::CommandFailed`]
    /// or [`GitError::UnreadableOutput`] when the output cannot be read. A repository with no commits
    /// is **not** an error: it reports [`Head::Unborn`], because that is a state a caller must render
    /// rather than a failure.
    pub fn snapshot(&self) -> Result<Snapshot, GitError> {
        let bytes = self.git.run(&[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ])?;
        let parsed = porcelain::parse(&bytes)?;
        Ok(Snapshot {
            head: parsed.head,
            changes: parsed.changes,
        })
    }

    /// The branch name, if `HEAD` is on one.
    ///
    /// Returns `None` for a detached head, and `Some` for an unborn branch — which has a name but no
    /// commit. Use [`Repository::head`] when the difference matters.
    ///
    /// # Errors
    ///
    /// As [`Repository::snapshot`].
    pub fn branch(&self) -> Result<Option<String>, GitError> {
        Ok(self
            .snapshot()?
            .head
            .branch()
            .map(str::to_owned))
    }

    /// The head commit's short hash, if there is one.
    ///
    /// # Errors
    ///
    /// As [`Repository::snapshot`].
    pub fn head_short_hash(&self) -> Result<Option<String>, GitError> {
        Ok(self.snapshot()?.head.commit().map(short_hash))
    }

    /// Every changed file.
    ///
    /// # Errors
    ///
    /// As [`Repository::snapshot`].
    pub fn changed_files(&self) -> Result<Vec<Change>, GitError> {
        Ok(self.snapshot()?.changes)
    }

    /// The most recent commits, newest first.
    ///
    /// `limit` is passed to `git log -n` rather than truncating in Rust, so a repository with a
    /// million commits reads one page rather than the whole log. A `limit` of zero is passed through
    /// and git returns nothing, which is why this returns an empty `Vec` rather than an error.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Unavailable`], [`GitError::CommandFailed`] — which is what an empty
    /// repository produces, since `git log` exits 128 with "does not have any commits yet" —
    /// [`GitError::NoCommits`] for that specific case so a caller can tell it from a real failure,
    /// and [`GitError::UnreadableOutput`] when a subject is not UTF-8.
    pub fn recent_commits(&self, limit: usize) -> Result<Vec<Commit>, GitError> {
        // `-n` takes the limit as an argument, so it cannot be a caller-controlled word: the value
        // is formatted into the format string below, never interpolated into the argv as a separate
        // token that a branch name could imitate.
        let count = limit.to_string();
        let args = ["log", "--no-color", "--no-decorate", "-n", &count, LOG_FORMAT];

        let bytes = match self.git.run(&args) {
            Ok(bytes) => bytes,
            Err(GitError::CommandFailed { stderr, .. })
                if mentions_no_commits(&stderr) =>
            {
                return Err(GitError::NoCommits {
                    root: self.root.clone(),
                });
            }
            Err(error) => return Err(error),
        };

        parse_log(&bytes)
    }
}

impl Git {
    /// Confirms `root` is inside a working tree and returns a [`Repository`].
    ///
    /// This is where "works outside a repository" is answered. It returns a typed error rather than
    /// `Ok(false)` so a caller cannot accidentally discard it: the usual bug at this boundary is
    /// `if git.is_repository()? { ... }` silently doing nothing, and a caller that wants that
    /// behaviour writes it explicitly.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::NotARepository`] when `root` is not inside a working tree,
    /// [`GitError::Unavailable`] when git cannot be started, and [`GitError::CommandFailed`] when git
    /// fails for a reason other than the repository being absent — a corrupt `.git` must not be
    /// reported as "not a repository".
    pub fn open(self) -> Result<Repository, GitError> {
        let root = self.root().to_path_buf();
        if self.is_repository()? {
            return Ok(Repository { git: self, root });
        }
        Err(GitError::NotARepository { root })
    }
}

/// Whether git said the repository has no commits.
///
/// Matched against git's own text rather than its exit status, because `git log` on an empty
/// repository and `git log` on a broken one both exit 128 and only the text distinguishes them. The
/// risk is a localised git not producing this phrase, in which case the caller sees `GIT-003` with
/// git's own translated message attached rather than `GIT-004` — degraded, not wrong. The
/// locale-independent check is [`Head::Unborn`] from [`Repository::snapshot`], and a caller that can
/// use it should.
fn mentions_no_commits(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("does not have any commits yet") || lower.contains("no commits yet")
}

/// Abbreviates a full hash the way git does, to seven characters.
fn short_hash(full: &str) -> String {
    full.chars().take(7).collect()
}

/// Parses the NUL-separated output of [`Repository::recent_commits`].
///
/// Chunked by [`FIELDS_PER_COMMIT`] rather than split into records first, because both boundaries use
/// [`NUL`]. A trailing partial chunk is refused rather than padded: git would not produce one, and
/// silently returning a `Commit` with empty fields would be reporting success for output this crate
/// did not understand.
fn parse_log(bytes: &[u8]) -> Result<Vec<Commit>, GitError> {
    const COMMAND: &str = "log";

    // Split before decoding, so invalid UTF-8 in a subject is reported against the right command
    // rather than being replaced and turning into a plausible-looking wrong subject.
    let text = String::from_utf8(bytes.to_vec()).map_err(|source| GitError::UnreadableOutput {
        command: COMMAND,
        source,
    })?;
    let mut tokens = text.split(NUL as char);
    let mut commits = Vec::new();
    loop {
        let mut fields = Vec::with_capacity(FIELDS_PER_COMMIT);
        for _ in 0..FIELDS_PER_COMMIT {
            match tokens.next() {
                // Every commit ends with a NUL, so the stream ends with one empty token.
                None => break,
                Some(token) => {
                    if token.is_empty() {
                        break;
                    }
                    fields.push(token);
                }
            }
        }
        if fields.is_empty() {
            break;
        }
        if fields.len() != FIELDS_PER_COMMIT {
            return Err(GitError::UnexpectedOutput {
                command: COMMAND,
                detail: format!(
                    "a commit needs {FIELDS_PER_COMMIT} fields, got {}: {fields:?}",
                    fields.len()
                ),
            });
        }
        commits.push(Commit {
            hash: fields[0].to_owned(),
            short_hash: fields[1].to_owned(),
            author: fields[2].to_owned(),
            author_email: fields[3].to_owned(),
            authored: fields[4].to_owned(),
            subject: fields[5].to_owned(),
        });
    }
    Ok(commits)
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::{GitError, mentions_no_commits, parse_log, short_hash};

    #[test]
    fn a_commit_record_is_read_in_order() {
        let bytes = b"abc123\x00abc\x00Name\x00n@x.invalid\x002026-09-27T14:03:22+02:00\x00Subject\x00\
                     def456\x00def\x00Other\x00o@x.invalid\x002026-09-28T09:00:00+00:00\x00Second\x00";

        let commits = parse_log(bytes).expect("parses");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc123");
        assert_eq!(commits[0].short_hash, "abc");
        assert_eq!(commits[0].author, "Name");
        assert_eq!(commits[0].author_email, "n@x.invalid");
        assert_eq!(commits[0].authored, "2026-09-27T14:03:22+02:00");
        assert_eq!(commits[0].subject, "Subject");
        assert_eq!(commits[1].subject, "Second");
    }

    #[test]
    fn a_subject_containing_a_printable_delimiter_does_not_shift_the_columns() {
        // A commit subject may contain any printable character, including every delimiter that would
        // have been convenient to split on - a tab, a pipe, a percent sign, even the escape sequence
        // itself. This is the test that says NUL was chosen for that reason.
        let bytes = b"abc\x00abc\x00N\x00n@x.invalid\x002026-01-01T00:00:00+00:00\x00\
                     a subject with \t | %x00 and %x1f inside it\x00";

        let commits = parse_log(bytes).expect("parses");

        assert_eq!(commits.len(), 1);
        assert_eq!(
            commits[0].subject,
            "a subject with \t | %x00 and %x1f inside it"
        );
    }

    #[test]
    fn empty_output_is_no_commits_rather_than_an_error() {
        // `git log -n 0` succeeds and prints nothing, so the empty case must be total.
        assert_eq!(parse_log(b"").expect("parses"), Vec::new());
        assert_eq!(parse_log(b"\x00").expect("parses"), Vec::new());
    }

    #[test]
    fn a_truncated_final_commit_is_refused_rather_than_read_with_empty_fields() {
        // git would not produce this, and padding it out would mean reporting a commit that does not
        // exist.
        let bytes = b"abc\x00abc\x00N\x00n@x.invalid\x002026-01-01T00:00:00+00:00\x00";

        let error = parse_log(bytes).expect_err("five fields, not six");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn a_lone_short_token_is_refused_rather_than_counted_as_a_commit() {
        let error = parse_log(b"garbage\x00").expect_err("one token, not six");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn output_that_is_not_utf8_is_refused() {
        let bytes = vec![b'a', b'b', b'c', 0, 0xff, 0xfe];

        let error = parse_log(&bytes).expect_err("not utf-8");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn a_hash_is_abbreviated_to_seven_characters() {
        assert_eq!(short_hash("e320d730d4b4f5e2287089191023daf83c191bef"), "e320d73");
    }

    #[test]
    fn the_no_commits_message_is_recognised_in_the_forms_git_uses() {
        // git 2.51 words it as "your current branch 'main' does not have any commits yet". A
        // detached-head empty repository words it differently, so both are matched.
        assert!(mentions_no_commits(
            "fatal: your current branch 'main' does not have any commits yet"
        ));
        assert!(mentions_no_commits("fatal: no commits yet"));
        assert!(!mentions_no_commits("fatal: not a git repository"));
        assert!(!mentions_no_commits("error: object file is empty"));
    }

    #[test]
    fn only_the_empty_repository_failure_carries_the_no_commits_code() {
        // The guard against the worst outcome of this whole file: reporting a corrupt repository as
        // a fresh one, which would tell the user everything is fine.
        let empty = GitError::NoCommits {
            root: "/tmp/r".into(),
        };
        let broken = GitError::CommandFailed {
            argv: vec!["log".to_owned()],
            status: 128,
            stderr: "error: object file .git/objects/ab/cdef is empty".to_owned(),
        };

        assert_eq!(empty.code(), ErrorCode::GIT_004);
        assert_eq!(broken.code(), ErrorCode::GIT_003);
        assert!(
            !broken.remediation().contains("first commit"),
            "a corrupt repository must not be told to make a first commit"
        );
    }
}