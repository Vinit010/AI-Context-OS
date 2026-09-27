---
id: ADR-002
type: decision
title: Plugins run out-of-process over a versioned JSON-RPC protocol
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-002 — Plugins run out-of-process over a versioned JSON-RPC protocol

## Context

The plugin system is the extension point for every external system the product will ever touch:
GitHub, AWS, databases, issue trackers, chat platforms. A plugin declares permissions, exposes
tools named `provider.service.action`, and receives calls from an AI agent through the tool gateway.

The unresolved question in the original specification was the **execution model**. It affects
security, the plugin API's stability, language choice for plugin authors, distribution, and
whether we can interoperate with the existing MCP ecosystem.

Candidates: (a) in-process dynamic linking, (b) out-of-process subprocess over stdio, (c) sandboxed
WASM, (d) network service.

## Decision

A plugin is an **executable that speaks JSON-RPC 2.0 over stdio**. The runtime supervises the
process, performs a versioned handshake, enforces timeouts, output caps, a filesystem path jail,
and a network egress allowlist, and audits every call.

The protocol and tool-definition shape are **MCP-compatible**, so an existing MCP server can be
registered as a plugin by wrapping it with a permission declaration.

`aicontext-plugin-sdk` provides manifest types, protocol types, and a server helper in Rust. The SDK
is a convenience. **The wire protocol is the contract** — a plugin may be written in any language.

## Reason

1. **Isolation is the product's core promise.** An in-process plugin shares the host's memory, its
   credentials, and its file handles. A malicious or buggy plugin is then a compromise of the
   developer machine, not a contained failure. Everything in `docs/SECURITY.md` is unachievable
   without a boundary the plugin cannot cross.
2. **Version stability.** A Rust ABI changes with the compiler and the crate graph. A wire protocol
   changes only when we choose. Plugin API v1 must stay stable for years (product requirement
   NFR-11); a wire protocol is the only way to actually keep that promise across toolchain upgrades.
3. **Interoperability.** MCP already defines the tool-call and tool-definition shapes for agents.
   Forging our own would strand us. Matching it means third-party MCP servers become plugins for
   free, and our plugins can be consumed by other MCP-aware agents.
4. **The cost is negligible.** Roughly 100 ms of process overhead per call, against a network round
   trip that already costs 100 ms–2 s. We are not paying this cost in a hot path.
5. **Language neutrality.** Plugin authors are not forced into Rust. A Python or TypeScript plugin
   is a normal case, not a second-class citizen.

## Alternatives considered

- **(a) In-process dynamic linking** — fastest, simplest distribution. Rejected: no isolation, ABI
  coupled to the toolchain, and a plugin crash takes down the CLI.
- **(c) WASM sandbox** — excellent for pure computation, near-zero overhead, capability-based by
  construction. Not rejected, **deferred**: WASM hosts are a poor fit for plugins that need
  network egress and credentialed access, which is most of what this product exists to do. A WASM
  `Retriever` or prompt-transformer is a realistic Phase 4+ addition. See TASK-076.
- **(d) Long-running network service** — amortises startup for high-volume use. Rejected for the
  MVP: it introduces a daemon, port allocation, lifecycle management, and a new class of
  remote-attack surface, in exchange for solving a latency problem we do not have.

## Rejected alternatives and why

- **A bespoke "tool protocol".** Rejected: it would have been less work short-term and would have
  made every third-party integration a custom adapter forever.
- **A shell script or arbitrary-exec plugin model.** Rejected: it is arbitrary code execution with
  no capability declaration, which is exactly the class of design the permission system exists to
  prevent.

## Consequences

**Positive** — a real security boundary; long-lived plugin API stability; language-neutral
ecosystem; MCP interoperability; a plugin crash is an error, not an outage.

**Negative** — ~100 ms per call; process lifecycle management (spawn, health check, reap, kill on
timeout) is real code that must be correct; plugins must be distributed as executables, so
packaging and versioning are the author's problem; debugging crosses a process boundary.

**Follow-up** — TASK-070 (SDK and manifest), TASK-073 (supervisor), TASK-074 (gateway),
TASK-076 (sandbox), TASK-078 (verify MCP interoperability; unknown U-2 must be answered before
Phase 5).

## Related tasks

TASK-070, TASK-073, TASK-074, TASK-076, TASK-078
