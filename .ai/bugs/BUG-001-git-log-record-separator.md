---
id: BUG-001
type: bug
title: The git log reader desynchronised on the newline git appends between commits
status: fixed
severity: high
symptoms: >-
  `Repository::recent_commits` failed with `UnexpectedOutput` naming `log` and the detail
  `a commit needs 6 fields, got 1: ["\n"]`. It happened for every repository with at least one
  commit, so the log reader was unusable, not merely wrong at the edges.
root_cause: >-
  `git log --format=<fields with %x00 separators>` appends a newline after each commit, but the
  reader treats every byte between two NULs as a field and chunks the flat stream by a fixed
  `FIELDS_PER_COMMIT` of 6. The trailing newline of the first commit therefore became a seventh,
  non-empty token and every later commit was read shifted by one field. The 39 unit tests could not
  see it because they fed the parser hand-written bytes that ended at the last `%x00` and never
  contained the separator git actually emits; the code was right about the bytes it was given.
files_changed:
  - crates/aicontext-git/src/query.rs
prevention: >-
  Ask git for the record separator we already assume: `-z` makes the commit boundary a NUL to match
  the `%x00` field boundary, and the trailing `%x00` in the format is dropped so the two cannot
  double up. The parser is unchanged. The guard is the integration suite in
  crates/aicontext-git/tests/repository.rs, which runs real git against a real temporary repository;
  `recent_commits_reads_the_fixture_commit` and `a_limit_returns_only_the_newest_commits` failed
  before the fix and pass after it. The general lesson is recorded in the task register: a parser
  tested only against bytes the author chose can agree with itself.
created: 2026-10-09
updated: 2026-10-09
tags: [git, parsing, porcelain, testing]
---

# BUG-001 — The log reader trusted git to end a commit the way it was told to

## What was observed

`Repository::recent_commits` returned `GitError::UnexpectedOutput` for any repository that had at
least one commit. The error named the `log` command and the detail
`a commit needs 6 fields, got 1: ["\n"]`.

## Why it happened

The reader requests a fixed shape from git: six fields separated by NUL, produced by `%x00` escapes
in a `--format` string. It then splits the whole output on NUL and chunks the resulting flat token
stream six at a time. The design assumes the only bytes in the stream are fields, with one NUL
between each.

That is not what `git log --format` produces. Git appends a newline after every commit, so the
stream was `…subject\0\n…subject\0\n…`. The newline after the first commit's subject became a
seventh, non-empty token; the chunker read the next commit starting one field late, and the failure
surfaced as soon as the token count stopped dividing evenly. For a single-commit repository the
leftover was exactly the `["\n"]` the message names.

## Why the unit tests missed it

The 39 unit tests exercise the parser against byte strings the author wrote by hand. Those fixtures
modelled the format as written, not the format as git emits it, so both the fixtures and the parser
were wrong in the same direction and agreed. Nothing in the crate ran real git. That is the gap
TASK-017's fourth acceptance criterion — tests against real temporary repositories — was written to
close, and the gap closed on the suite's first run.

## The fix

Pass `-z` to `git log`, which separates commits with a NUL instead of a newline, and remove the
trailing `%x00` from the format string. Field boundary and record boundary are then the same byte,
which is what the tokenizer always assumed; the parser itself did not change. A stacked `%x00` plus
`-z` would have left an empty token between commits that the tokenizer reads as the end of the
stream, so the two must not both be present.

## Prevention

`crates/aicontext-git/tests/repository.rs` runs the real binary against real temporary repositories
built by `aicontext-testkit`. Two tests, `recent_commits_reads_the_fixture_commit` and
`a_limit_returns_only_the_newest_commits`, failed against the old code and pass against the fix, so
a regression is a red test rather than a silent mis-read.

The durable lesson: a parser validated only against inputs the author generated is validated
against the author's model of the format, not the format. Where an external program defines the
bytes, at least one test must let that program define them.
