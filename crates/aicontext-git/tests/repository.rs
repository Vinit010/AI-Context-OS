//! The git wrapper, exercised against real repositories.
//!
//! The unit tests beside each module parse hand-written porcelain bytes. They prove the *parser* is
//! right about the format; they cannot prove the format is what `git` actually writes. These tests
//! close that gap: every case here runs a real `git` over a real working tree built by
//! `aicontext-testkit`, so a wrong invocation, a wrong format assumption, or a path-handling bug on
//! one platform fails here rather than in `aicontext status` (TASK-013) later.
//!
//! # Two different test strategies, on purpose
//!
//! `git status` output is not stable across versions the way its *shape* is, so asserting on the
//! exact bytes of a live repository would make the suite fail on the next git release. Instead each
//! test asks the wrapper a question and asserts the typed answer - a `Head`, a `Change`, a `Commit` -
//! because those are the contract. The hand-written parser tests are what pin the bytes.
//!
//! # Hermetic, per RULES.md 8
//!
//! `Git::new` inherits the process environment, which is right in production and wrong in a test: a
//! developer's `core.autocrlf`, `init.defaultBranch`, `safe.directory`, or credential helper must not
//! decide whether this suite passes. Every wrapper here is built with `Git::with_environment` and the
//! fixture's own environment, so the child sees exactly what `TempRepository` pins and nothing else.

use std::path::PathBuf;

use aicontext_core::ErrorCode;
use aicontext_git::{Change, FileStatus, Git, GitError, Head, Repository};
use aicontext_testkit::{TempProject, TempRepository};

/// A wrapper that runs the fixture's git, with the fixture's hermetic environment.
#[must_use]
fn git_in(repo: &TempRepository) -> Git {
    Git::with_environment(repo.path(), repo.git_program(), repo.environment())
}

/// Opens the fixture as a repository, panicking with a readable message if it is not one.
#[must_use]
fn open(repo: &TempRepository) -> Repository {
    git_in(repo).open().expect("the fixture is a repository")
}

/// The one change a test created, failing if there is not exactly one.
#[must_use]
fn only_change(repo: &TempRepository) -> Change {
    let changes = open(repo).snapshot().expect("a snapshot").changes;
    assert_eq!(
        changes.len(),
        1,
        "expected exactly one change in the working tree, got {changes:?}"
    );
    changes.into_iter().next().expect("checked exactly one")
}

#[test]
fn a_directory_that_is_not_a_repository_is_reported_as_one() {
    // "Works outside a repository" is an acceptance criterion, so the not-a-repository path is tested
    // against a real directory that really is outside one. The fixture that provides the git
    // executable and the hermetic environment is separate from the directory under question.
    let anchor = TempRepository::new("anchor").expect("a repository");
    let plain = TempProject::new("plain").expect("a project");
    let git = Git::with_environment(plain.path(), anchor.git_program(), anchor.environment());

    assert!(!git.is_repository().expect("git answers"));

    let error = git
        .open()
        .expect_err("a plain directory is not a repository");
    assert_eq!(error.code(), ErrorCode::GIT_002);
    assert!(matches!(error, GitError::NotARepository { .. }));
}

#[test]
fn a_fresh_repository_is_on_a_branch_with_no_changes() {
    let repo = TempRepository::new("fresh").expect("a repository");

    let repository = open(&repo);
    let snapshot = repository.snapshot().expect("a snapshot");

    let head = &snapshot.head;
    let Head::Branch { name, commit } = head else {
        panic!("a fresh repository is on a branch, got {head:?}");
    };
    assert_eq!(name, "main");
    assert_eq!(commit.len(), 40, "a full hash is forty hex characters");
    assert!(snapshot.is_clean());
    assert!(snapshot.changed_paths().is_empty());

    assert_eq!(repository.root(), repo.path());
    assert_eq!(
        repository.branch().expect("a branch"),
        Some("main".to_owned())
    );
    assert_eq!(
        repository.head_short_hash().expect("a hash").as_deref(),
        commit.get(..7),
        "the short hash is the first seven characters of the full hash"
    );
}

#[test]
fn an_untracked_file_is_reported_as_a_change() {
    let repo = TempRepository::new("untracked").expect("a repository");
    repo.write("new.txt", "content").expect("writes");

    let snapshot = open(&repo).snapshot().expect("a snapshot");

    assert_eq!(snapshot.changes.len(), 1);
    assert!(snapshot.changes[0].is_untracked());
    assert_eq!(snapshot.changes[0].path(), "new.txt");
    assert_eq!(snapshot.changes[0].original_path(), None);
    assert_eq!(snapshot.changed_paths(), vec![PathBuf::from("new.txt")]);
    assert!(!snapshot.is_clean());
}

#[test]
fn a_staged_modification_is_reported_on_the_index_side() {
    let repo = TempRepository::new("staged").expect("a repository");
    repo.write("README.md", "# Fixture repository, edited\n")
        .expect("writes");
    repo.git(&["add", "README.md"]).expect("stages");

    let change = only_change(&repo);

    assert_eq!(change.path(), "README.md");
    assert!(
        matches!(
            change,
            Change::Tracked {
                index: Some(FileStatus::Modified),
                worktree: None,
                ..
            }
        ),
        "a staged edit and an unstaged edit are different states, got {change:?}"
    );
}

#[test]
fn an_unstaged_modification_is_reported_on_the_worktree_side() {
    let repo = TempRepository::new("unstaged").expect("a repository");
    repo.write("README.md", "# Fixture repository, edited\n")
        .expect("writes");

    let change = only_change(&repo);

    assert_eq!(change.path(), "README.md");
    assert!(
        matches!(
            change,
            Change::Tracked {
                index: None,
                worktree: Some(FileStatus::Modified),
                ..
            }
        ),
        "an edit that was never staged is the other half, got {change:?}"
    );
}

#[test]
fn a_rename_keeps_both_paths() {
    let repo = TempRepository::new("rename").expect("a repository");
    repo.git(&["mv", "README.md", "renamed.md"])
        .expect("moves and stages the rename");

    let change = only_change(&repo);

    assert_eq!(change.path(), "renamed.md");
    assert_eq!(
        change.original_path(),
        Some("README.md"),
        "the path before the rename is not discarded"
    );
    assert!(matches!(
        change,
        Change::Tracked {
            index: Some(FileStatus::Renamed),
            ..
        }
    ));
}

#[test]
fn recent_commits_reads_the_fixture_commit() {
    let repo = TempRepository::new("history").expect("a repository");
    let expected_hash = repo.commits(1).expect("a history").remove(0);

    let commits = open(&repo).recent_commits(10).expect("a history");

    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].hash, expected_hash);
    assert_eq!(commits[0].short_hash, expected_hash[..7]);
    assert_eq!(commits[0].subject, "initial commit");
    assert_eq!(commits[0].author, "AI Context OS tests");
    assert_eq!(commits[0].author_email, "tests@aicontext.invalid");
    assert!(
        commits[0].authored.starts_with("2020-01-02T03:04:05"),
        "the fixture pins the author date so the hash is reproducible, got {}",
        commits[0].authored
    );
}

#[test]
fn a_limit_returns_only_the_newest_commits() {
    let repo = TempRepository::new("limit").expect("a repository");
    repo.write("second.txt", "written for the second commit")
        .expect("writes");
    repo.commit_all("second commit").expect("commits");

    let repository = open(&repo);
    let all = repository.recent_commits(10).expect("a history");
    let newest = repository.recent_commits(1).expect("a history");

    assert_eq!(all.len(), 2);
    assert_eq!(all[0].subject, "second commit", "newest first");
    assert_eq!(all[1].subject, "initial commit");
    assert_eq!(newest.len(), 1);
    assert_eq!(newest[0].subject, "second commit");
}

#[test]
fn an_empty_repository_is_unborn_rather_than_broken() {
    // The second acceptance criterion: a repository with no commits is a state to render, not a
    // failure, and the wrapper distinguishes it from a real error.
    let repo = TempRepository::init_empty("empty").expect("a repository");

    let repository = open(&repo);
    let snapshot = repository
        .snapshot()
        .expect("a repository with no commits still has a status");

    assert_eq!(
        snapshot.head,
        Head::Unborn {
            name: "main".to_owned()
        }
    );
    assert!(!snapshot.head.has_commits());
    assert_eq!(snapshot.head.branch(), Some("main"));
    assert_eq!(snapshot.head.commit(), None);

    let error = repository
        .recent_commits(5)
        .expect_err("there is no log yet");
    assert_eq!(error.code(), ErrorCode::GIT_004);
    assert!(matches!(error, GitError::NoCommits { .. }));
}

#[test]
fn a_detached_head_reports_its_commit_and_no_branch() {
    let repo = TempRepository::new("detached").expect("a repository");
    let hash = repo.commits(1).expect("a history").remove(0);
    repo.git(&["checkout", "--quiet", hash.as_str()])
        .expect("detaches HEAD");

    let snapshot = open(&repo).snapshot().expect("a snapshot");

    assert!(matches!(snapshot.head, Head::Detached { .. }));
    assert_eq!(snapshot.head.branch(), None);
    assert_eq!(snapshot.head.commit(), Some(hash.as_str()));
    assert!(snapshot.head.has_commits());
}

#[test]
fn the_wrapper_can_be_pointed_at_a_subdirectory() {
    let repo = TempRepository::new("nested").expect("a repository");
    let nested = repo.join("crates/inner");
    std::fs::create_dir_all(&nested).expect("creates the subdirectory");
    let git = Git::with_environment(nested, repo.git_program(), repo.environment());

    let snapshot = git
        .open()
        .expect("a subdirectory is inside the repository")
        .snapshot()
        .expect("a snapshot");

    assert!(matches!(snapshot.head, Head::Branch { .. }));
}

#[test]
fn a_second_read_sees_a_change_made_after_the_first() {
    // The crate documentation promises it does not cache: a caller that reads a status, writes a file,
    // and reads again must see the change. Caching would be faster and would make `aicontext status`
    // after `aicontext init` report a clean tree that is not clean.
    let repo = TempRepository::new("fresh-eyes").expect("a repository");
    let repository = open(&repo);

    assert!(
        repository.snapshot().expect("a snapshot").is_clean(),
        "a fixture starts clean"
    );

    repo.write("later.txt", "written after the first read")
        .expect("writes");

    let snapshot = repository.snapshot().expect("a second snapshot");
    assert!(
        !snapshot.is_clean(),
        "the wrapper must not cache a status across calls"
    );
    assert_eq!(snapshot.changes[0].path(), "later.txt");
}
