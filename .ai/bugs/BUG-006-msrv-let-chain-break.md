---
id: BUG-006
type: bug
title: The MSRV build failed on let-chains in a dependency and in the resolver at once
status: fixed
severity: high
symptoms: >-
  The `msrv (1.85)` CI job failed with E0658 in `ignore` 0.4.30's `incremental.rs`: "`let` expressions
  in this position are unstable". After `ignore` was pinned to 0.4.29, the same error appeared in
  `crates/aicontext-permissions/src/resolver.rs` at `wildcard_matches`, whose `if let ... && let ...`
  chain also needs Rust 1.88. Both compiled on the pinned 1.95 toolchain, so `verify`, `clippy`, and
  the ordinary test job were green.
root_cause: >-
  `ignore` was declared as `0.4`, so the lockfile took 0.4.30. That release has no `rust-version` for
  the MSRV-aware resolver to read, yet its new incremental walker uses let-chains, stable only from
  1.88. The workspace code made the same assumption: TASK-071 wrote `wildcard_matches` as a three-arm
  let-chain, and no local gate used 1.85. The dependency failed first and hid the workspace half;
  the Cargo.toml comment that called 0.4.30 the newest 1.85-compatible release was reasoning from
  the absence of a declaration rather than from a build on 1.85.
files_changed:
  - Cargo.toml
  - Cargo.lock
  - crates/aicontext-permissions/src/resolver.rs
  - .ai/ARCHITECTURE.md
prevention: >-
  While the MSRV is below 1.88, do not use let-chains: use `let ... else` or nested matches. Exact-pin
  a dependency release that declares no `rust-version` and starts using newer language features,
  because the MSRV-aware resolver has nothing to reject it by. Before pushing a dependency update or
  a language-level change, run `cargo +1.85.0 build --workspace --all-targets --locked` and
  `cargo +1.85.0 test --workspace --locked`; the default toolchain and the other CI jobs cannot see
  this class of failure.
created: 2026-10-26
updated: 2026-10-26
tags: [msrv, ci, dependencies, let-chains, ignore, permissions, BUG-006]
---

# BUG-006 — The 1.85 floor was broken by two let-chains at once

## What was observed

The `msrv (1.85)` job failed while building the workspace:

```
error[E0658]: `let` expressions in this position are unstable
   --> ignore-0.4.30/src/incremental.rs:274:16
error[E0658]: `let` expressions in this position are unstable
   --> ignore-0.4.30/src/incremental.rs:337:42
```

Pinning `ignore` to 0.4.29 let the build continue, and the same error appeared in this repository:

```
error[E0658]: `let` expressions in this position are unstable
   --> crates/aicontext-permissions/src/resolver.rs:216:8
```

`cargo build` and `cargo test` on the pinned 1.95 toolchain passed, and so did the other CI jobs.
Only the job that installs 1.85 could see either failure.

## Why it happened

There were two independent causes:

- `Cargo.toml` declared `ignore = "0.4"`. The lockfile therefore took 0.4.30, whose `Cargo.toml` has
  neither a `rust-version` nor a feature gate for the new incremental walker. Cargo's MSRV-aware
  resolver cannot reject a crate that does not state an MSRV, so resolution succeeded and the compiler
  failed instead.
- `wildcard_matches` in `aicontext-permissions` was written as:

  ```rust
  if let Some(prefix) = pattern.strip_suffix(".*")
      && let Some(rest) = tool.strip_prefix(prefix)
      && let Some(action) = rest.strip_prefix('.')
  ```

  Let-chains were stabilized in Rust 1.88, so the 1.85 compiler rejects a construct the 1.95 compiler
  accepts without a warning.

The comment above the dependency claimed `0.4.30` was the newest release that kept the 1.85 floor.
That claim was inferred from version numbers and published metadata rather than proved by building
the crate on 1.85, and it was false.

## Why it was missed

Every local gate used the pinned 1.95 toolchain, and the only job that tests the declared MSRV is the
one that failed. The dependency error appeared first, so the resolver error was invisible until the
dependency was pinned. A language feature that is stable on the development toolchain produces no
warning there; the failure exists only at the compatibility boundary.

## The fix

- `Cargo.toml` pins `ignore = "=0.4.29"`, the newest release before the incremental walker, and records
  why 0.4.30 cannot be used.
- `Cargo.lock` downgrades `ignore` from 0.4.30 to 0.4.29.
- `crates/aicontext-permissions/src/resolver.rs` replaces the let-chain with three `let ... else`
  bindings, which have been stable since Rust 1.65 and express the same "any missing part means no
  match" rule.
- `.ai/ARCHITECTURE.md` §2.2 records the pin and points at this bug.

## Prevention

1. Do not use let-chains while the MSRV is below 1.88. Prefer `let ... else`; it is explicit about
   every early return and compiles on 1.85.
2. Exact-pin a dependency release that declares no `rust-version` when a newer release starts using
   newer language features. The MSRV-aware resolver is only as good as the metadata it can read.
3. Run the real floor before pushing a dependency update or a language-level change:

   ```
   cargo +1.85.0 build --workspace --all-targets --locked
   cargo +1.85.0 test --workspace --locked
   ```

   The ordinary `cargo build`/`cargo test` cannot substitute for this; the entire failure mode is a
   construct the newer compiler accepts.
