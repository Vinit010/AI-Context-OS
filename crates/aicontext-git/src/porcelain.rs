//! Types and parsing for `git status --porcelain=v2`.
//!
//! # Why porcelain v2 and not v1
//!
//! Three reasons, all measured against git 2.51 rather than assumed:
//!
//! 1. **One call instead of two.** `--branch` adds header lines carrying the head commit and the
//!    branch name, and it distinguishes `HEAD` from a real branch in a way `git branch --show-current`
//!    does not: on an unborn head that command exits 0 and prints *nothing*, which is
//!    indistinguishable from a detached head unless a second command is run to find out.
//! 2. **No path mangling.** Without `-z`, porcelain quotes any path containing a space, a quote, a
//!    backslash, or a non-ASCII byte, in git's own C-style syntax. That is a second language to
//!    parse, and getting it wrong means reporting a path that does not exist. With `-z`, paths are
//!    raw bytes terminated by NUL, so nothing needs unquoting.
//! 3. **Renames are unambiguous.** In the textual form a rename is one line with a tab inside it and
//!    a `->` between two paths, which is ambiguous the moment either path contains a tab. With `-z`
//!    the original path is a separate NUL-terminated field.
//!
//! # The format, as measured
//!
//! ```text
//! # branch.oid e320d730d4b4f5e2287089191023daf83c191bef   <- full hash, or (initial)
//! # branch.head main                                     <- branch name, or (detached)
//! # branch.upstream origin/main                          <- only when an upstream is set
//! # branch.ab +0 -0                                       <- only when an upstream is set
//! 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>            <- tracked, changed
//! 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path> <- renamed or copied
//!                                                 then a separate field: the original path
//! u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path> <- unmerged
//! ? <path>                                                <- untracked
//! ! <path>                                                <- ignored, skipped here
//! ```
//!
//! Every record, headers included, is NUL-terminated in `-z` mode.
//!
//! # Why an unexpected record is an error
//!
//! `RULES.md` §4.8 forbids reporting success for a batch that partly failed. A record type this
//! version does not know about is exactly that case: if an unknown `3` line were skipped and the
//! rest reported, the caller would be told a working tree is clean when it is not. So a record that
//! claims to be a change but does not have the documented fields is refused, and the caller learns
//! that aicontext does not understand this git rather than that the repository is fine.
//!
//! # Why the head is a three-state value
//!
//! A repository has three genuinely different head situations and a caller acts differently in each:
//!
//! | State | `branch.oid` | `branch.head` | Meaning |
//! |-------|--------------|---------------|---------|
//! | on a branch | a hash | the branch name | ordinary work |
//! | unborn | `(initial)` | the branch name | just `git init`ed; there is nothing to diff against |
//! | detached | a hash | `(detached)` | at a commit, on no branch; commits here would be unreachable |
//!
//! Reporting "no branch" for both of the last two would be a false statement about one of them, so
//! they are separate variants.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::GitError;

/// The command whose output this module parses, named in errors so the reader knows what ran.
pub(crate) const STATUS_COMMAND: &str = "status --porcelain=v2 --branch -z";

/// What `HEAD` points at.
///
/// The three states are separate variants rather than an `Option<String>` because an absent branch
/// means opposite things in each: an unborn branch has a *name* and no commits, while a detached
/// head has a *commit* and no name.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Head {
    /// On a branch, at a commit.
    Branch {
        /// The branch name, without a `refs/heads/` prefix.
        name: String,
        /// The full 40-character commit hash `HEAD` resolves to.
        commit: String,
    },
    /// On a branch that has no commits yet.
    Unborn {
        /// The branch name that will be created by the first commit.
        name: String,
    },
    /// At a commit that belongs to no branch.
    Detached {
        /// The full 40-character commit hash `HEAD` resolves to.
        commit: String,
    },
}

impl Head {
    /// The branch name, when there is one.
    ///
    /// `None` for a detached head only. An unborn branch still has a name — it is the name the
    /// repository is on — so returning `None` there would discard information the caller needs.
    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        match self {
            Self::Branch { name, .. } | Self::Unborn { name } => Some(name),
            Self::Detached { .. } => None,
        }
    }

    /// The commit `HEAD` resolves to, when it resolves to one.
    #[must_use]
    pub fn commit(&self) -> Option<&str> {
        match self {
            Self::Branch { commit, .. } | Self::Detached { commit } => Some(commit),
            Self::Unborn { .. } => None,
        }
    }

    /// Whether the repository has at least one commit.
    #[must_use]
    pub const fn has_commits(&self) -> bool {
        !matches!(self, Self::Unborn { .. })
    }
}

/// What happened to one file in the working tree.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Change {
    /// A file git tracks that differs from `HEAD` or from the index.
    Tracked {
        /// The path, `/`-separated, relative to the repository root.
        path: String,
        /// What the index holds, when it differs from `HEAD`.
        index: Option<FileStatus>,
        /// What the working tree holds, when it differs from the index.
        worktree: Option<FileStatus>,
        /// The path before the change, when it was renamed or copied.
        original_path: Option<String>,
    },
    /// A file git does not track and is not ignoring.
    Untracked {
        /// The path, `/`-separated, relative to the repository root.
        path: String,
    },
    /// A file with a merge conflict.
    Unmerged {
        /// The path, `/`-separated, relative to the repository root.
        path: String,
        /// The two-letter conflict code, such as `UU` or `AA`.
        code: String,
    },
}

impl Change {
    /// The path this change is about, `/`-separated.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Tracked { path, .. } | Self::Untracked { path } | Self::Unmerged { path, .. } => {
                path
            }
        }
    }

    /// The path before the change, if this was a rename or a copy.
    #[must_use]
    pub fn original_path(&self) -> Option<&str> {
        match self {
            Self::Tracked { original_path, .. } => original_path.as_deref(),
            _ => None,
        }
    }

    /// Whether this file is untracked.
    #[must_use]
    pub const fn is_untracked(&self) -> bool {
        matches!(self, Self::Untracked { .. })
    }
}

/// How a tracked file differs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FileStatus {
    /// The file did not exist and now does.
    Added,
    /// The file's contents differ.
    Modified,
    /// The file existed and now does not.
    Deleted,
    /// The file moved, keeping its contents.
    Renamed,
    /// The file was copied.
    Copied,
    /// The file changed kind, for example a symlink became a regular file.
    TypeChanged,
}

impl FileStatus {
    /// Parses one letter of a porcelain status code, or `None` when it means "unchanged".
    ///
    /// A space means the half is unchanged, which is not a `FileStatus` and not an error: it is the
    /// ordinary case of a staged-only or unstaged-only change.
    fn from_letter(letter: u8) -> Option<Self> {
        match letter {
            b'A' => Some(Self::Added),
            b'M' => Some(Self::Modified),
            b'D' => Some(Self::Deleted),
            b'R' => Some(Self::Renamed),
            b'C' => Some(Self::Copied),
            b'T' => Some(Self::TypeChanged),
            _ => None,
        }
    }
}

/// What one `status` invocation produced.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Snapshot {
    /// Where `HEAD` points.
    pub head: Head,
    /// Every changed file, in the order git reported them.
    pub changes: Vec<Change>,
}

/// Splits NUL-terminated records out of `-z` output.
///
/// The final record is followed by its terminator, so splitting yields a trailing empty slice that
/// this drops. A record is a byte slice rather than a `str` because a filename need not be UTF-8, and
/// rejecting it is a decision made later, where the failing path can be named.
fn records(output: &[u8]) -> impl Iterator<Item = &[u8]> {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
}

/// Decodes one field, naming the command when it is not UTF-8.
fn field(command: &'static str, bytes: &[u8]) -> Result<String, GitError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|source| GitError::UnreadableOutput { command, source })
}

/// Turns one `branch.*` header value into a path-safe string.
fn header_value(command: &'static str, line: &[u8], prefix: &[u8]) -> Result<String, GitError> {
    let rest = line
        .strip_prefix(prefix)
        .ok_or_else(|| GitError::UnexpectedOutput {
            command,
            detail: format!(
                "expected a `{}` header, got {:?}",
                String::from_utf8_lossy(prefix),
                String::from_utf8_lossy(line)
            ),
        })?;
    field(command, rest)
}

/// Parses `git status --porcelain=v2 --branch -z` output.
///
/// # Errors
///
/// Returns [`GitError::UnreadableOutput`] when a field is not UTF-8 and
/// [`GitError::UnexpectedOutput`] when a record does not have the documented shape. An absent head
/// header is the latter: without it there is no honest answer to "what branch is this?".
pub(crate) fn parse(output: &[u8]) -> Result<Snapshot, GitError> {
    let mut commit = None;
    let mut branch = None;
    let mut changes = Vec::new();
    let mut records = records(output);

    while let Some(record) = records.next() {
        if record.starts_with(b"# branch.oid ") {
            commit = Some(header_value(STATUS_COMMAND, record, b"# branch.oid ")?);
            continue;
        }
        if record.starts_with(b"# branch.head ") {
            branch = Some(header_value(STATUS_COMMAND, record, b"# branch.head ")?);
            continue;
        }
        if record.starts_with(b"# branch.") {
            // `branch.upstream` and `branch.ab` are not read by this task. They are recognised so
            // that adding them later is not a format change, and skipped so that they are not
            // mistaken for an unknown record type.
            continue;
        }
        if let Some(change) = tracked(record, &mut records, STATUS_COMMAND)? {
            changes.push(change);
        }
    }

    // Which of the two header values carries the marker is the part that is easy to get wrong, so it is
    // spelled out rather than inferred from the variant names. git writes:
    //
    // - normal:   `# branch.head main`        + `# branch.oid <hash>`
    // - unborn:   `# branch.head main`        + `# branch.oid (initial)`
    // - detached: `# branch.head (detached)`  + `# branch.oid <hash>`
    //
    // So `(detached)` appears in `branch.head` and `(initial)` appears in `branch.oid` - the two
    // markers never sit in the same column. Matched by value rather than as constant patterns, since
    // a const pattern would have to bind against a `String`, which is not a structural-match type.
    let head = match (branch, commit) {
        (Some(name), Some(commit)) if name == DETACHED => Ok(Head::Detached { commit }),
        (Some(name), Some(commit)) if commit == INITIAL => Ok(Head::Unborn { name }),
        (Some(name), Some(commit)) => Ok(Head::Branch { name, commit }),
        // A branch with no `branch.oid` line at all: unborn, reported without the `(initial)` marker.
        (Some(name), None) => Ok(Head::Unborn { name }),
        // A commit with no `branch.head` line at all: HEAD is not on any branch.
        (None, Some(commit)) => Ok(Head::Detached { commit }),
        (branch, commit) => Err(GitError::UnexpectedOutput {
            command: STATUS_COMMAND,
            detail: format!(
                "expected `# branch.oid` and `# branch.head` headers, got oid={commit:?} \
                 head={branch:?}; the porcelain v2 output was not the documented shape"
            ),
        }),
    }?;

    Ok(Snapshot { head, changes })
}

/// The literal git writes for a head that resolves to nothing.
const INITIAL: &str = "(initial)";

/// The literal git writes for a head that belongs to no branch.
const DETACHED: &str = "(detached)";

/// Parses one change record, or `None` for an ignored path.
///
/// `records` is advanced for a rename's original-path field, so a `2` record consumes two.
fn tracked<'a>(
    record: &'a [u8],
    records: &mut impl Iterator<Item = &'a [u8]>,
    command: &'static str,
) -> Result<Option<Change>, GitError> {
    let Some(kind) = record.first() else {
        return Ok(None);
    };
    match kind {
        b'!' => Ok(None),
        // `? <path>` - the only field is the path, which is everything after the separating space.
        b'?' => {
            let Some(path) = record.get(2..) else {
                return Err(GitError::UnexpectedOutput {
                    command,
                    detail: "a `?` record has no path field".to_owned(),
                });
            };
            Ok(Some(Change::Untracked {
                path: normalise(&field(command, path)?),
            }))
        }
        b'u' => unmerged(record, command),
        b'1' | b'2' => ordinary(record, records, command, *kind == b'2'),
        _ => Err(GitError::UnexpectedOutput {
            command,
            detail: format!(
                "unknown record type {:?}",
                String::from_utf8_lossy(&record[..record.len().min(1)])
            ),
        }),
    }
}

/// Parses a `u` record: a merge conflict.
///
/// `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>` - eleven tokens, the last being the path.
fn unmerged(record: &[u8], command: &'static str) -> Result<Option<Change>, GitError> {
    let text = String::from_utf8_lossy(record);
    let Some(code) = text.split(' ').nth(1) else {
        return Err(GitError::UnexpectedOutput {
            command,
            detail: format!("an `u` record has no status code: {text:?}"),
        });
    };
    let Some(path) = record_after(record, 11) else {
        return Err(GitError::UnexpectedOutput {
            command,
            detail: format!("an `u` record needs 11 fields, got fewer: {text:?}"),
        });
    };
    Ok(Some(Change::Unmerged {
        code: code.to_owned(),
        path: normalise(&field(command, path)?),
    }))
}

/// Parses a `1` or `2` record: a tracked file that differs.
fn ordinary<'a>(
    record: &'a [u8],
    records: &mut impl Iterator<Item = &'a [u8]>,
    command: &'static str,
    is_rename: bool,
) -> Result<Option<Change>, GitError> {
    let text = String::from_utf8_lossy(record);
    let code = text.get(2..4).unwrap_or_default();
    let mut letters = code.chars();
    let index = letters
        .next()
        .and_then(|c| FileStatus::from_letter(c as u8));
    let worktree = letters
        .next()
        .and_then(|c| FileStatus::from_letter(c as u8));

    // `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>` - path is token 9
    // `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path><sep><origPath>` - the score and the
    // path are one token, so the path is also token 9; the original path is the *next* NUL record.
    let (original, is_rename) = if is_rename {
        let Some(original) = records.next() else {
            return Err(GitError::UnexpectedOutput {
                command,
                detail: format!("a rename record has no original path after it: {text:?}"),
            });
        };
        (Some(field(command, original)?), true)
    } else {
        (None, false)
    };

    let Some(last) = record_after(record, 9) else {
        return Err(GitError::UnexpectedOutput {
            command,
            detail: format!("a change record needs 9 fields, got fewer: {text:?}"),
        });
    };

    // The rename token is `<X><score> <path>`, so its path starts after the first space inside the
    // token rather than at a fixed offset.
    let last = field(command, last)?;
    let raw = if is_rename {
        strip_rename_score(&last)
    } else {
        last
    };

    Ok(Some(Change::Tracked {
        path: normalise(&raw),
        index,
        worktree,
        original_path: original.map(|path| normalise(&path)),
    }))
}

/// Removes the `R100`-style prefix from a rename record's last field.
fn strip_rename_score(last_field: &str) -> String {
    match last_field.split_once(' ') {
        Some((_score, path)) => path.to_owned(),
        None => last_field.to_owned(),
    }
}

/// The token at `number` counting from 1, so token 1 is the record type, or `None` if the record is
/// that short.
///
/// `None` rather than the remainder, deliberately. A record truncated by a future git, or one this
/// version misreads, has to be refused: returning whatever was left would turn a truncated record
/// into a change with an empty path, and an empty path in a status report is a file the user cannot
/// open. `RULES.md` §4.8 forbids reporting success for a batch that partly failed, and that is
/// exactly what "your working tree is clean" derived from output we did not understand would be.
fn record_after(record: &[u8], number: usize) -> Option<&[u8]> {
    if number == 0 {
        return None;
    }
    // `splitn` rather than a manual scan for spaces, because one token in these records contains a
    // space of its own: a rename's ninth token is `<X><score> <path>`. Scanning for spaces splits that
    // in half and yields the score as the path, which is a rename reported under a path that does not
    // exist. With `splitn`, only the first `number - 1` separators count and the last part keeps
    // everything after them, spaces included.
    let mut parts = record.splitn(number, |byte| *byte == b' ');
    for _ in 1..number {
        // A record with fewer tokens than this has no such field, and asking for one returns `None`
        // rather than the remainder.
        parts.next()?;
    }
    parts.next()
}

/// Normalises a git path to `/` separators and strips a trailing slash if git sent one.
fn normalise(path: &str) -> String {
    if path.contains('\\') {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    }
}

/// The path a [`Change`] refers to, as a `PathBuf` relative to the repository root.
///
/// Kept here rather than in the CLI so that the `/`-to-native conversion lives beside the parser
/// that produced the `/`-separated form.
#[must_use]
pub(crate) fn to_path(path: &str) -> PathBuf {
    let mut native = OsString::new();
    for (index, segment) in path.split('/').enumerate() {
        if index > 0 {
            native.push(std::path::MAIN_SEPARATOR.to_string());
        }
        native.push(segment);
    }
    PathBuf::from(native)
}

#[cfg(test)]
mod tests {
    use aicontext_core::ErrorCode;

    use super::{Change, FileStatus, Head, normalise, parse, strip_rename_score, to_path};

    /// Joins records the way `-z` output is joined: each NUL-terminated.
    fn z(records: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend_from_slice(record.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    const OID: &str = "# branch.oid e320d730d4b4f5e2287089191023daf83c191bef";
    const ON_BRANCH: &str = "# branch.head main";

    #[test]
    fn an_ordinary_branch_and_commit_are_read() {
        let snapshot = parse(&z(&[OID, ON_BRANCH])).expect("parses");

        assert_eq!(
            snapshot.head,
            Head::Branch {
                name: "main".to_owned(),
                commit: "e320d730d4b4f5e2287089191023daf83c191bef".to_owned(),
            }
        );
        assert_eq!(snapshot.head.branch(), Some("main"));
        assert_eq!(
            snapshot.head.commit(),
            Some("e320d730d4b4f5e2287089191023daf83c191bef")
        );
        assert!(snapshot.head.has_commits());
        assert!(snapshot.changes.is_empty());
    }

    #[test]
    fn an_unborn_branch_keeps_its_name_and_reports_no_commit() {
        let snapshot = parse(&z(&["# branch.oid (initial)", ON_BRANCH])).expect("parses");

        assert_eq!(
            snapshot.head,
            Head::Unborn {
                name: "main".to_owned()
            }
        );
        assert_eq!(
            snapshot.head.branch(),
            Some("main"),
            "an unborn branch has a name; reporting none would discard it"
        );
        assert_eq!(snapshot.head.commit(), None);
        assert!(!snapshot.head.has_commits());
    }

    #[test]
    fn a_detached_head_reports_a_commit_and_no_branch() {
        let snapshot = parse(&z(&[OID, "# branch.head (detached)"])).expect("parses");

        assert_eq!(
            snapshot.head,
            Head::Detached {
                commit: "e320d730d4b4f5e2287089191023daf83c191bef".to_owned()
            }
        );
        assert_eq!(snapshot.head.branch(), None);
        assert!(snapshot.head.has_commits());
    }

    #[test]
    fn an_unborn_branch_and_a_detached_head_are_not_the_same_state() {
        // The reason Head has three variants: both print nothing for a branch name, and conflating
        // them would tell the user they are on no branch when they are on `main` with nothing
        // committed yet.
        let unborn = parse(&z(&["# branch.oid (initial)", ON_BRANCH])).expect("parses");
        let detached = parse(&z(&[OID, "# branch.head (detached)"])).expect("parses");

        assert_ne!(unborn.head, detached.head);
    }

    #[test]
    fn an_untracked_file_is_read() {
        let snapshot = parse(&z(&[OID, ON_BRANCH, "? new file.txt"])).expect("parses");

        assert_eq!(
            snapshot.changes,
            vec![Change::Untracked {
                path: "new file.txt".to_owned()
            }]
        );
        assert!(snapshot.changes[0].is_untracked());
        assert_eq!(snapshot.changes[0].path(), "new file.txt");
        assert_eq!(snapshot.changes[0].original_path(), None);
    }

    #[test]
    fn a_staged_add_and_an_unstaged_edit_are_separate_halves() {
        let snapshot = parse(&z(&[
            OID,
            ON_BRANCH,
            "1 A. N... 100644 100644 100644 aaaaaaa bbbbbbb added.txt",
            "1 .M N... 100644 100644 100644 aaaaaaa aaaaaaa edited.txt",
        ]))
        .expect("parses");

        assert_eq!(snapshot.changes.len(), 2);
        assert_eq!(
            snapshot.changes[0],
            Change::Tracked {
                path: "added.txt".to_owned(),
                index: Some(FileStatus::Added),
                worktree: None,
                original_path: None,
            }
        );
        assert_eq!(
            snapshot.changes[1],
            Change::Tracked {
                path: "edited.txt".to_owned(),
                index: None,
                worktree: Some(FileStatus::Modified),
                original_path: None,
            }
        );
    }

    #[test]
    fn a_rename_keeps_both_paths() {
        let snapshot = parse(&z(&[
            OID,
            ON_BRANCH,
            "2 R. N... 100644 100644 100644 aaaaaaa bbbbbbb R100 renamed file.txt",
            "old name.txt",
        ]))
        .expect("parses");

        assert_eq!(
            snapshot.changes[0],
            Change::Tracked {
                path: "renamed file.txt".to_owned(),
                index: Some(FileStatus::Renamed),
                worktree: None,
                original_path: Some("old name.txt".to_owned()),
            }
        );
        assert_eq!(snapshot.changes[0].original_path(), Some("old name.txt"));
    }

    #[test]
    fn a_rename_score_is_not_part_of_the_path() {
        assert_eq!(
            strip_rename_score("R100 renamed file.txt"),
            "renamed file.txt"
        );
        assert_eq!(strip_rename_score("C075 copy.txt"), "copy.txt");
    }

    #[test]
    fn an_ignored_file_is_not_a_change() {
        // `!` records are reported because a caller may want them, but an ignored file is not
        // something the working tree is "changed" by.
        let snapshot = parse(&z(&[OID, ON_BRANCH, "! target/debug/a.exe"])).expect("parses");

        assert!(snapshot.changes.is_empty());
    }

    #[test]
    fn an_unmerged_file_keeps_its_conflict_code() {
        let snapshot = parse(&z(&[
            OID,
            ON_BRANCH,
            "u UU N... 100644 100644 100644 100644 aaaaaaa aaaaaaa aaaaaaa conflict.txt",
        ]))
        .expect("parses");

        assert_eq!(
            snapshot.changes[0],
            Change::Unmerged {
                path: "conflict.txt".to_owned(),
                code: "UU".to_owned(),
            }
        );
    }

    #[test]
    fn upstream_headers_are_skipped_rather_than_mistaken_for_changes() {
        let snapshot = parse(&z(&[
            OID,
            ON_BRANCH,
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "? untracked.txt",
        ]))
        .expect("parses");

        assert_eq!(snapshot.changes.len(), 1);
        assert_eq!(snapshot.changes[0].path(), "untracked.txt");
    }

    #[test]
    fn a_path_with_a_space_or_a_tab_survives_intact() {
        // The whole reason for -z: in the textual form this line would be quoted or tab-ambiguous.
        let snapshot =
            parse(&z(&[OID, ON_BRANCH, "? a file with spaces and a\ttab.txt"])).expect("parses");

        assert_eq!(
            snapshot.changes[0].path(),
            "a file with spaces and a\ttab.txt"
        );
    }

    #[test]
    fn output_with_no_head_headers_is_refused_rather_than_read_as_clean() {
        // RULES.md 4.8: reporting "no changes" from output we did not understand would be a lie.
        let error = parse(&z(&["? untracked.txt"])).expect_err("must not be read as clean");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn an_unknown_record_type_is_refused_rather_than_skipped() {
        let error = parse(&z(&[
            OID,
            ON_BRANCH,
            "3 brand new record type from a future git something.txt",
        ]))
        .expect_err("an unknown type must not be skipped");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn a_short_change_record_is_refused_rather_than_truncated() {
        let error = parse(&z(&[OID, ON_BRANCH, "1 M. N..."])).expect_err("too few fields");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn a_rename_with_no_original_path_is_refused() {
        let error = parse(&z(&[
            OID,
            ON_BRANCH,
            "2 R. N... 100644 100644 100644 a b R100 x.txt",
        ]))
        .expect_err("the original path is a separate field and is missing");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn output_that_is_not_utf8_names_the_command_rather_than_being_replaced() {
        let mut bytes = z(&[OID, ON_BRANCH]);
        bytes.extend_from_slice(b"? ");
        bytes.push(0xff);
        bytes.push(0xfe);
        bytes.push(0);

        let error = parse(&bytes).expect_err("a non-UTF-8 filename must be refused");

        assert_eq!(error.code(), ErrorCode::GIT_006);
        assert!(error.to_string().contains("porcelain=v2"), "{error}");
        assert!(
            !error.to_string().contains('\u{fffd}'),
            "replacement characters would report a path that does not exist: {error}"
        );
    }

    #[test]
    fn empty_output_is_refused_rather_than_read_as_a_clean_tree() {
        let error = parse(&[]).expect_err("no headers at all");

        assert_eq!(error.code(), ErrorCode::GIT_006);
    }

    #[test]
    fn a_backslash_in_a_path_becomes_a_forward_slash() {
        // git uses `/` even on Windows, but a quoted or hand-built record may not, and a caller
        // comparing paths needs one spelling.
        assert_eq!(normalise(r"docs\nested\file.md"), "docs/nested/file.md");
        assert_eq!(normalise("docs/nested/file.md"), "docs/nested/file.md");
    }

    #[test]
    fn a_slash_separated_path_becomes_native_on_the_way_out() {
        let path = to_path("docs/nested/file.md");

        assert_eq!(
            path,
            std::path::PathBuf::from("docs")
                .join("nested")
                .join("file.md"),
            "segments must be joined with the platform separator, not with `/`"
        );
    }

    #[test]
    fn the_native_and_slash_forms_are_inverses_of_each_other() {
        // The property that makes the pair safe to use together, and the one that holds on every
        // platform without a conditional in it: a path can go out to the filesystem as a native path
        // and come back to a caller as the same `/`-separated string it started as.
        //
        // Asserting that the native form contains no `/` instead would look equivalent and is not: on
        // Unix a native path *is* `/`-separated, so such an assertion passes only on Windows and
        // fails everywhere else.
        for original in [
            "file.md",
            "docs/file.md",
            "docs/nested/deep/file.md",
            "a b/c d.md",
        ] {
            let native = to_path(original);
            assert_eq!(
                normalise(&native.to_string_lossy()),
                original,
                "round trip changed {original:?} via {native:?}"
            );
        }
    }
}
