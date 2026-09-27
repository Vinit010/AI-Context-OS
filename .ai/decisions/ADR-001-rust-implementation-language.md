---
id: ADR-001
type: decision
title: Rust as the implementation language
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-001 — Rust as the implementation language

## Context

AI Context OS ships a local CLI that must run on Linux, macOS, and Windows with no runtime
prerequisite; it must load and constrain third-party plugin code; and it must keep its dependency
tree small enough to audit. The core is a text-processing, file-scanning, and process-supervision
system — it is not compute-bound.

Candidate stacks: Rust, Go, TypeScript/Node, Python.

## Decision

Implement the core, the CLI, the plugin runtime, and the provider layer in **Rust, edition 2024,
MSRV 1.85**, as a Cargo workspace of small crates, distributed as a single static binary per
platform.

## Reason

The deciding constraints were isolation and dependency surface, not ergonomics:

1. **Isolation of untrusted code.** Plugins are third-party code that will touch credentials and
   external systems. A language where the safe subset is enforced by the compiler, `unsafe` can be
   forbidden crate-wide, and process/resource limits are first-class, reduces the number of ways
   the platform can be wrong.
2. **A single static binary with zero runtime prerequisites.** Directly satisfies the "no
   installation friction" requirement and keeps `aicontext` usable in a restricted environment.
3. **Minimal transitive dependencies.** This is a security product; every crate is supply-chain
   risk. Rust makes a small tree achievable without heroic effort.
4. **One language across CLI and dashboard.** Phase 8 can reuse the core crates in a web backend
   without a second implementation of the domain logic.

The cost is real: roughly twice the implementation cost of TypeScript for the same functionality,
and a smaller talent pool. We accept it and offset it by refusing abstractions with a single use
case.

## Alternatives considered

- **Go** — excellent single binary and a pleasant CLI story. Rejected: weaker story for
  constraining and running untrusted in-process code, and process-level isolation was the point.
  Viable fallback if Rust velocity proves fatal (see R-10).
- **TypeScript on Node** — fastest to build, best provider ecosystem, easiest future dashboard.
  Rejected: requires a runtime, the largest transitive dependency tree of the four, and weakest
  isolation primitives.
- **Python** — best AI ecosystem, `json` and `jsonschema` in the standard distribution. Rejected:
  packaging and startup are poor for a CLI, plugin distribution is awkward, and the dashboard would
  need a second language.

## Rejected alternatives and why

- **Rust with a plugin ABI in-process (dynamic linking).** Rejected in ADR-002; it would couple
  plugins to the exact toolchain build.
- **Rust + WASM for plugins only.** Not rejected, deferred. It is a strong future sandbox for
  pure-computation plugins, but it cannot replace a process boundary for plugins that need network
  and credential access. See TASK-076.

## Consequences

**Positive** — single static binary; strong isolation posture; auditable dependency tree; one
language for CLI and dashboard; no runtime version conflicts on a developer machine.

**Negative** — slower feature velocity; a smaller contributor pool; every dependency choice is
slowly reviewed; FFI is not available for exotic integrations, which must be a subprocess instead.

**Follow-up** — TASK-010 establishes the workspace; TASK-018 builds the testkit first, because in
Rust the module tree determines where tests can live.

## Related tasks

TASK-010, TASK-018, TASK-052, TASK-076
