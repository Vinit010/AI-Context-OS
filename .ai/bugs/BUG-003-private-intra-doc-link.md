---
id: BUG-003
type: bug
title: A public doc comment linked to a private item and failed cargo doc under -D warnings
status: fixed
severity: medium
symptoms: >-
  CI's verify job failed its Documentation step on every platform:
  `error: public documentation for environment links to private item TempRepository::command`,
  promoted by `RUSTDOCFLAGS: -D warnings`. A local `cargo doc` without those flags printed it as a
  warning and still succeeded, so the failure existed only in CI.
root_cause: >-
  TempRepository::environment is public and its doc comment used an intra-doc link,
  `[TempRepository::command]`, to the private command builder. Rustdoc cannot resolve a public
  document's link to a private item, so the link is a private-intra-doc-links warning, and CI turns
  every rustdoc warning into an error.
files_changed:
  - crates/aicontext-testkit/src/repo.rs
prevention: >-
  Describe the private helper in prose rather than linking to it, since a public document must not
  depend on an item a reader cannot open. The guard is to run the documentation gate with the same
  flags CI uses: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, which fails
  on the defect where the bare `cargo doc` does not.
created: 2026-10-09
updated: 2026-10-09
tags: [testkit, rustdoc, ci, documentation]
---

# BUG-003 — A public doc linked to a private helper

## What was observed

The `verify` job's Documentation step failed on ubuntu, macos, and windows:

```
error: public documentation for `environment` links to private item `TempRepository::command`
   --> crates\aicontext-testkit\src\repo.rs:172:11
   = note: `-D rustdoc::private-intra-doc-links` implied by `-D warnings`
```

## Why it happened

`TempRepository::environment` is public, and its doc comment linked to `TempRepository::command`,
which is private. Rustdoc will not resolve a link from a public document to a private item — the
reader of the public docs has no such item to open — so it emits a `private-intra-doc-links`
warning. CI sets `RUSTDOCFLAGS: -D warnings` on the Documentation step, which makes that warning a
build failure.

## Why local runs missed it

The command run outside CI, `cargo doc --workspace --no-deps`, prints the warning and exits 0, so
the defect was invisible unless the flags were set. The failure lived in the gap between the local
command and the CI command.

## The fix

State the helper in prose instead of linking to it. The paragraph still explains that the type's own
command builder sets an empty value rather than removing it; it simply no longer points at an item
the public reader cannot see.

## Prevention

Run the documentation gate with CI's flags:

```
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

A warning that is only a warning locally is an error where it counts, so the local command should
carry the same flags as the one that enforces them.
