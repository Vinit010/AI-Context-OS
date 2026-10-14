---
id: BUG-005
type: bug
title: A second public doc comment linked to private constants, so the documentation gate failed at HEAD
status: fixed
severity: medium
symptoms: >-
  The Documentation step of the verify job, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace
  --no-deps --locked`, failed in `aicontext-context`: "public documentation for `discovery` links to
  private item `MAX_ENTRIES_SCANNED`" and the same for `MAX_MANIFEST_BYTES`. The bare `cargo doc`
  printed both as warnings and exited 0, so the failure was invisible outside CI. It was found while
  closing TASK-051, a crate unrelated to the defect.
root_cause: >-
  `pub mod discovery` describes its scan bound with two intra-doc links to `MAX_ENTRIES_SCANNED` and
  `MAX_MANIFEST_BYTES`, which are private `const`s. Rustdoc will not resolve a link from a public
  document to an item a public reader cannot open, so it emits `rustdoc::private-intra-doc-links`, and
  CI's `-D warnings` turns that warning into a build error. It is the same class as BUG-003: the rule
  was applied at the one site CI had already caught and not at the next one.
files_changed:
  - crates/aicontext-context/src/discovery.rs
prevention: >-
  A public document must describe a private item in prose rather than link to it. The durable guard is
  to run the documentation gate with CI's flags - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace
  --no-deps --locked` - as part of the routine, not only when a documentation change is made, because
  the defect appears in whichever crate the current task is not touching. BUG-003 recorded the same
  flag-carrying command; this recurrence shows a lesson written down is not yet a habit.
created: 2026-10-14
updated: 2026-10-14
tags: [context, rustdoc, ci, documentation]
---

# BUG-005 — A public doc linked to private constants in `discovery`

## What was observed

While closing `TASK-051`, the documentation gate was run with the flags CI uses, and it failed in a
crate `TASK-051` does not touch:

```
error: public documentation for `discovery` links to private item `MAX_ENTRIES_SCANNED`
error: public documentation for `discovery` links to private item `MAX_MANIFEST_BYTES`
    = note: `-D rustdoc::private-intra-doc-links` implied by `-D warnings`
```

`cargo doc --workspace --no-deps --locked` without `RUSTDOCFLAGS` printed both as warnings and exited
0, so the defect was green locally and red in the `verify` job's Documentation step, which sets
`RUSTDOCFLAGS: -D warnings`.

## Why it happened

`crates/aicontext-context/src/lib.rs` declares `pub mod discovery`, so the module's own documentation
is public. That document bounded the scan with links to `MAX_ENTRIES_SCANNED` and
`MAX_MANIFEST_BYTES`, and both are private `const`s in `discovery.rs`. Rustdoc will not resolve a link
from a public document to a private item — the reader of the public API has no such item to open — so
it emits `rustdoc::private-intra-doc-links`, and `-D warnings` promotes it to an error.

## Why it was missed

The same class of defect was found once before and fixed at its one site (`BUG-003`,
`crates/aicontext-testkit/src/repo.rs`). The lesson recorded there was to run the documentation gate
with CI's flags, but that command was not part of the routine, so the next occurrence — in a different
crate — went unobserved until a task with no relation to it ran the full gate. A bare `cargo doc`
cannot surface it, because the command that enforces the rule is the one carrying the flags.

## The fix

Describe the two private bounds in prose, giving the concrete values so the public reader still learns
what the limits are, and drop the intra-doc links:

- `crates/aicontext-context/src/discovery.rs`: the module document now reads "at most 512 top-level
  entries, and a manifest larger than 256 KiB is not read at all (`RULES.md` §11)". The constants and
  their own documentation are unchanged.

## Prevention

Run the documentation gate with CI's flags, over the whole workspace, as a routine step:

```
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
```

This is the second time this class of defect has been found (`BUG-003`), and both were invisible to
the weaker local command. A public document must never link to a private item; the values of a private
constant belong in prose in the public document.
