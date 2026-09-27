---
id: MEMORY-001
type: memory
title: AI Context OS — Durable Memory
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-09-27
---

# MEMORY

Durable knowledge only. If a line here would be false in three months, it does not belong here.
Conversation transcripts, command output, and anything already in `git log` are excluded.

Entries use the inline block form from `docs/CONTEXT_SPEC.md` §3. Each entry has a stable ID and a
`supersedes` field when it replaces an earlier claim.

---

## MEM-001 — Determinism beats cleverness in retrieval

```yaml
id: MEMORY-REG
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

Retrieval in the MVP is lexical and rule-based (BM25 + field boosts + front-matter filters + path
globs), not embedding-based. Embeddings are non-deterministic across runs, opaque to audit, and
require a network or a local model. A permission-gated agent that must explain *why* it read a
file cannot use a ranking function nobody can inspect. This buys a second property: a reviewer can
reproduce the exact context set by hand from the plan. A future `EmbeddingRetriever` may implement
the same trait, but it must never be the default.

## MEM-002 — Plugins are processes, not libraries

```yaml
id: MEM-002
category: constraint
scope: plugin-system
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

An in-process plugin ABI was rejected. It would couple every plugin to the Rust toolchain version
and the crate graph, and it would make a malicious plugin a compromise of the host rather than a
contained process. The process boundary costs ~100 ms per call, which is irrelevant next to a
network round trip, and buys isolation, resource limits, cross-language plugins, and a versioned
wire protocol. See `ADR-002`.

## MEM-003 — `.ai/` is policy, but it arrives untrusted

```yaml
id: MEM-003
category: constraint
scope: security
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

`RULES.md` is treated as binding project policy **only within a repository the developer has
chosen to work in**. A cloned repository is attacker-controlled input that happens to contain
policy-shaped text. So `.ai/` is trusted-as-policy and untrusted-as-code simultaneously, and
`doctor` renders a digest of the policy files found in an unfamiliar repository before any write
action is permitted. This is the single most important security subtlety in the product: the
mechanism that makes AI consistent is also an injection vector against the human.

## MEM-004 — Retrieval is data, never instruction

```yaml
id: MEM-004
category: constraint
scope: security
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

No text from a tool result, a web page, an issue, a commit message, or a source comment can change
policy. Implementation: retrieved content is wrapped in a data envelope carrying
`{source, path, tier, trust: "untrusted"}`, is placed in a data role, and cannot raise its own
priority tier. The platform has no shell tool, so there is no path from model text to execution.
Absence of a human is a deny, never an inferred consent.

## MEM-005 — The product's own repository is its first test fixture

```yaml
id: MEM-005
category: preference
scope: project
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

This repository dogfoods `.ai/`. Every feature must work against its own context, because a
platform that cannot index a real project is a platform that only works on toy input. Any change
that would make `.ai/` harder to read by hand is a regression, even if it improves the schema.

## MEM-006 — Rust was chosen for isolation, not speed

```yaml
id: MEM-006
category: fact
scope: project
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

The deciding factor was dependency-minimalism plus a credible path to running untrusted plugin code
with tight resource control, on a static binary with no runtime prerequisite. The cost is
implementation velocity: expect ~2× the code of a TypeScript or Go implementation for the same
functionality, and offset it by refusing abstractions that have only one use.

## MEM-007 — Two YAML/schema dependencies are unresolved risks

```yaml
id: MEM-007
category: limitation
scope: build
status: active
confidence: medium
recorded: 2026-09-27
supersedes: null
```

Front matter needs a YAML 1.2 implementation and validation needs a JSON Schema Draft 2020-12
validator. Both are external crates with non-trivial transitive weight, and the mainstream YAML
crate for Rust has had a turbulent maintenance history. Both are wrapped behind internal traits
(`YamlCodec`, `Validator`) chosen at implementation time with a recorded justification, so a bad
choice costs a contained swap rather than a rewrite. See `RISK R-1` and `RISK R-2` in
`ARCHITECTURE.md` §13. Resolve during Phase 1 (TASK-015, TASK-016).

## MEM-008 — MVP success is falsifiable

```yaml
id: MEM-008
category: fact
scope: product
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

The thesis is testable in one sentence: *the second session on a project requires no
re-explanation of context that was already written down.* Every feature is judged against that
sentence. A feature that adds capability without removing a re-explanation is not MVP work, however
impressive it is.

---

## Open questions

| # | Question | Blocks | Resolve by |
|---|----------|--------|-----------|
| Q-1 | Which YAML crate? `serde_yaml` is widely used but its maintenance status needs checking; a maintained fork may be required | TASK-016 | Phase 1 |
| Q-2 | JSON Schema validator crate choice, and whether we accept its transitive weight | TASK-015 | Phase 1 |
| Q-3 | Should `doctor` fail the build on a deprecation warning, or only on errors? | TASK-014 | Phase 1 |
| Q-4 | Is the approval prompt a full-screen TUI, or a line-based prompt that composes with pipes? | TASK-072 | Phase 4 |
| Q-5 | Does an audit record need a stable third-party timestamp authority to resist backdating? | TASK-075 | Phase 4 |
| Q-6 | Task inline-vs-split threshold is set at 300 entities; confirm against real usage before Phase 4 | TASK-038 | Phase 2 |

---

## Bugs encountered

None yet. Record under `MEM-0NN` with `category: bug` when the first one appears, and always file
the full root-cause analysis under `.ai/bugs/BUG-NNN.md`.
