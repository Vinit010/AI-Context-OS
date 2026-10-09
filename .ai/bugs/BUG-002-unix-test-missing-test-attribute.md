---
id: BUG-002
type: bug
title: A platform-gated test was missing its test attribute and never ran on Unix
status: fixed
severity: medium
symptoms: >-
  CI on Linux failed to build the test target: `function
  a_backslash_is_part_of_a_unix_filename_and_is_left_alone is never used`, promoted to an error by
  `-D warnings`. The function was meant to be a test; it had no `#[test]`, so the Unix-only branch of
  `normalise` was compiled but never executed.
root_cause: >-
  The Windows and Unix cases of `normalise` are covered by two functions gated with `#[cfg(windows)]`
  and `#[cfg(not(windows))]`. The Windows one carried `#[test]`; the Unix one carried only the `cfg`,
  so on Unix it was an ordinary private function with no caller - dead code. On Windows the item is
  removed by `not(windows)`, so a local `cargo test` and `cargo clippy` on the developer's machine
  never saw it, and the missing attribute was invisible until a Linux runner compiled the branch.
files_changed:
  - crates/aicontext-git/src/porcelain.rs
prevention: >-
  Add the `#[test]` attribute so the function is a test on the platform it targets. The general guard
  is to type-check the non-default target when a change is platform-gated: `cargo clippy --workspace
  --all-targets --target x86_64-unknown-linux-gnu --locked -- -D warnings` compiles the branch this
  machine excludes, and would have failed here. A `#[cfg]` on a test is a test only where it is
  compiled; the platform that does not run it is exactly the platform that hides whether it is one.
created: 2026-10-09
updated: 2026-10-09
tags: [git, testing, cfg, cross-platform]
---

# BUG-002 — A Unix test that was never a test

## What was observed

The Linux CI job failed while building `aicontext-git`'s test target:

```
error: function `a_backslash_is_part_of_a_unix_filename_and_is_left_alone` is never used
   --> crates/aicontext-git/src/porcelain.rs:800:8
   = note: `-D dead-code` implied by `-D warnings`
```

## Why it happened

`normalise` has two implementations behind `#[cfg(windows)]` and `#[cfg(not(windows))]`. Each is
covered by a dedicated test function, and each test is gated on the same platform. The Windows test
was written as `#[test] #[cfg(windows)]`; the Unix test was written as `#[cfg(not(windows))]` with no
`#[test]` — a function that is only compiled off Windows and, without the attribute, is never called.

On Windows the item is removed entirely by `not(windows)`, so the developer's `cargo test` and
`cargo clippy` never compiled it and never reported the dead code. Only a Linux build compiles the
branch, and there `-D warnings` turns the dead-code lint into a build failure.

## Why it matters

The defect is not merely a lint. The Unix behaviour of `normalise` — that a `\` is left alone because
it is a legal character in a Unix filename — was claimed by a test that had never executed on any
machine. The suite counted a test that could not fail.

## The fix

Add `#[test]` above `#[cfg(not(windows))]`, matching the Windows case. The item is then a registered
test wherever it is compiled, and the dead-code lint has nothing to report.

## Prevention

Type-check the platform the local machine excludes when a change is gated on `cfg`:

```
cargo clippy --workspace --all-targets --target x86_64-unknown-linux-gnu --locked -- -D warnings
```

That command compiles the `not(windows)` branch on a Windows host and fails on the original defect,
so the gap is closable locally rather than only on CI. The durable rule: a `#[cfg]`-gated test is a
test only on the platform that compiles it, and that platform is the one a developer on the other
platform cannot see.
