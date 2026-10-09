---
id: MEMORY-001
type: memory
title: AI Context OS — Durable Memory
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-10-09
---

# MEMORY

Durable knowledge only. If a line here would be false in three months, it does not belong here.
Conversation transcripts, command output, and anything already in `git log` are excluded.

Entries use the inline block form from `docs/CONTEXT_SPEC.md` §3. Each entry has a stable ID and a
`supersedes` field when it replaces an earlier claim.

---

## MEM-001 — Determinism beats cleverness in retrieval

```yaml
id: MEM-001
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

**Partly resolved by TASK-015.** YAML is settled (`yaml_serde` 0.10, `ADR-007`, see `MEM-010`).
The JSON Schema question is now answered for CI: `jsonschema` 0.58.3, `default-features = false`,
dev-dependency only. What is still open is the *runtime* `Validator`, which is deferred to
`TASK-014` — the crate is chosen, the production dependency decision is not, so this entry stays
`active` rather than being closed on the strength of a dev-dependency.

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

## MEM-009 - Dependencies are added one at a time, with the justification already written down

```yaml
id: MEM-009
category: preference
scope: project
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

The workspace does not accumulate dependencies. Each one is added by the task that needs it, and
`ARCHITECTURE.md` 2.2 carries the written justification in advance. "It is popular" is not a
justification, and neither is a future need (RULES.md 6).

State as of TASK-011: **one** third-party dependency, `thiserror`, added by the task that defines
the error model, because `RULES.md` 4.2 requires error types to be enums deriving
`thiserror::Error` and hand-rolling `Display` and `source` for every variant would be strictly
worse code for a strictly worse reason. The other Phase 1-2 candidates stay unadded until their
task needs them.

Amended by TASK-018: a second dependency is now in use, `tempfile`, and it is the reason
`aicontext-testkit` is no longer dependency-free. That crate had been kept at zero dependencies on
purpose, so the change is worth recording rather than leaving as a diff. It was still the right
trade: the crate needed temporary directories whose removal is guaranteed on every platform, and the
Windows case - a just-closed handle keeping a directory from being deleted - is not solvable with
`std::env::temp_dir()`. The justification is written into
`crates/aicontext-testkit/Cargo.toml` where a reader meets it, and the line "every helper reads no
real home directory, no global Git configuration, and no network" is a property of the fixtures
that a test asserts rather than a property of the dependency list.

Consequences already in place:

- `aicontext-testkit` takes `tempfile` and only `tempfile`. Its `FixtureError` writes its own
  `Display` and `Error` impls instead of deriving through `thiserror`, on the grounds that a
  dev-only crate should not pull a production dependency in to shorten five variants. This is the
  one place in the workspace where that rule is deliberately inverted, and the reasoning sits in
  `src/error.rs` rather than only in this entry.
- The one place that reads `Cargo.toml` is `aicontext-testkit`, and it does so with a ~200-line
  hand-rolled reader rather than `toml`. That reader is a liability the moment a real parser is
  added, so it is confined to one module and its scope is documented. Replace it when Q-1/Q-2
  force a YAML or JSON Schema dependency anyway.
- The crate dependency direction in `ARCHITECTURE.md` 3.2 is enforced by a test, not by review
  discipline. Adding a workspace member without adding it to the boundary matrix fails
  `cargo test`.
- `Cargo.lock` is committed, and CI builds with `--locked`.

The toolchain is pinned to 1.95.0 in `rust-toolchain.toml`, while the declared MSRV is 1.85. The
gap is intentional: the pin gives reproducible local runs, the MSRV is the compatibility floor, and
a CI job builds and tests on 1.85 so the floor cannot rot unnoticed.

---

## MEM-010 - No YAML type appears in a public signature, so a YAML swap touches one module

```yaml
id: MEM-010
category: constraint
scope: project
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

The choice of crate is recorded in `ADR-007` and is not repeated here. This entry records the
constraint that choice is protecting, because the constraint outlives the choice and is what the next
agent has to keep.

`yaml_serde` 0.10 is the YAML implementation, chosen in `ADR-007`. It is the actively maintained
fork of `serde_yaml`, published by the official YAML organisation, pure Rust, MIT OR Apache-2.0, and
MSRV 1.82 - comfortably inside our 1.85 floor. `serde_yaml` itself is archived; `serde_yml` is
deprecated and carries `RUSTSEC-2025-0068`; `serde_norway` is plausible but had gone 21 months
without a release. Full comparison in the ADR.

The part that matters more than the crate is the seam. No YAML type appears in a public signature:
the parser speaks our own `Value` enum, and `yaml_serde` is reachable only through the private
`YamlCodec` trait. That is `RISK R-1`'s mitigation, which was previously an intention and is now
implemented.

Consequences:

- The trait must stay thin. If it grows into a general YAML abstraction layer it has become the
  risk instead of the mitigation, and the right move is to delete it and call the crate directly.
  This is stated in the trait's own documentation so the next agent does not inherit a layer that
  grew for its own sake.
- `R-1` likelihood is downgraded Medium to Low. Residual risk is an incompatible 1.0 release; Cargo
  package renaming is the planned escape.
- State as of TASK-016: two production third-party dependencies - `thiserror`, already pre-justified in
  `ARCHITECTURE.md` 2.2, and `yaml_serde` - plus dev-only `proptest`. `serde` is deliberately **not** a
  direct dependency: the parser walks `yaml_serde`'s own value tree into our `Value` rather than
  deriving `Deserialize`, because a derived mapping would drop a repeated key without naming it, and
  rule 5 says nothing written by hand may be silently lost. `serde_json`, `clap`, and the rest stay
  unadded until their task needs them.
- `MEM-009` noted that the ~200-line hand-rolled TOML reader in `aicontext-testkit` should be
  replaced by a real parser "when Q-1/Q-2 force a YAML or JSON Schema dependency anyway." That
  condition is now met for YAML, but replacing it is not `TASK-016`'s scope; it is left as a
  follow-up rather than quietly absorbed into a front-matter task.

---

## MEM-011 — Front matter refuses line endings the file cannot express

```yaml
id: MEM-011
category: fact
scope: project
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
```

YAML ends a line on LF, CR, NEL, LS, and PS. A file's lines are separated by LF alone, so a block
containing a bare CR - or one of the three exotic ones - has *more* lines for the YAML layer than for
the file, and every line number that layer reports after the offender is one or more too high. The
author would be sent past the end of their own document, which is worse than a slightly vague message
because it looks authoritative.

`TASK-016` therefore refuses a bare CR, NEL, LS, or PS anywhere in a block, naming the character and
its line. A CR immediately before the LF is the ordinary CRLF ending and is left alone, so a
Windows-authored document still parses and still renders with LF endings as rule 10 asks. This was
found by fuzzing, not by review: the property that asserts a reported line is never past the end of
the file produced the failing case, and it is the test that keeps it fixed.

The alternative - accepting the input and reporting a line the author cannot use - was rejected. A
title containing a bare LS cannot survive a YAML round-trip in any case, because a quoted scalar
holding one is folded across two lines, so refusing is closer to the truth than accepting and
silently changing the text.

---

## MEM-012 - Front matter is read-only in phase 1, and `Value` is sealed to keep it that way

```yaml
id: MEM-012
category: constraint
scope: project
status: active
confidence: high
recorded: 2026-09-27
supersedes: null
tags: [frontmatter, api, boundary]
```

`Value` is `#[non_exhaustive]`, so no crate outside `aicontext-context` can construct one. The only
way to obtain a value is to read it out of a parsed document.

The reason is an invariant rather than a style preference: the set of values in memory is then
exactly the set the parser can produce. A fabricated value - one no block could ever have contained -
cannot turn up in a document, and a variant added later is additive instead of a breaking change
(`RULES.md` §3). Making the type sealed now is cheap; making it sealed after something constructs
values is not.

The cost is that a future command which *edits* one front-matter field, `aicontext task set
phase=3` being the obvious case, cannot build a `Value` and will need a way in. That is the moment
to add one, deliberately and with tests, rather than to have shaped the API around a guess. Nothing
in phase 1 needs it: `init` writes template text from `templates/` and reads documents to decide
whether to leave them alone, and `doctor` and `status` only read. No constructor is invented now
(`RULES.md` §2).

The other half of the same boundary is enforced rather than promised. A test in
`crates/aicontext-context/tests/frontmatter.rs` scans `src/` and fails if `yaml_serde` appears in any
module other than `codec.rs`, so the `ADR-007` condition cannot rot unnoticed. It was checked by
planting a reference in `error.rs` and confirming the test failed, rather than by assuming it worked.

---

## MEM-013 - `init` decides first, writes second, and re-reads third

```yaml
id: MEM-013
category: preference
scope: project
status: active
confidence: high
recorded: 2026-10-01
supersedes: null
tags: [init, idempotence, templates, cli]
```

The skeleton is built by three separate passes. `plan` compares the rendered templates against the
filesystem and produces one entry per path with an action; `apply` writes only the entries that carry
a write; `validate` re-reads what was written and compares it. `--dry-run` stops after `plan`, which
is why it cannot disagree with a real run about what would happen.

The reason is that idempotence is a property of the comparison, not of the writer. Byte equality
against the rendered template answers "would this file change?", which is the only question that
matters for a second run. Anything cleverer - timestamps, front-matter identity, content hashes -
would be a second definition of sameness that could disagree with the first.

Two consequences are worth keeping. An edited document is preserved and reported as `CTX-017`,
because the tool did not write it and has no claim to have the last word. And `--force` refuses to
overwrite anything unless stdout is a terminal: overwriting a developer's own rules is a decision, and
`--yes` is explicitly not that decision. The test for this is the non-terminal run exiting 5.

Templates are embedded with `include_str!` rather than read at runtime. There is no template
installation step, no path resolution, no "template not found" error, and the binary is
self-contained, which matters for a tool whose whole promise is that the knowledge is in the
repository. The cost is that editing a template means rebuilding; that is the right trade for a set
of files that changes a few times a year.

Discovery stayed a `Signal { kind, value, evidence }` list instead of a typed `ProjectProfile`. The
evidence file is reported to the user and recorded in `context/stack.md`, so a wrong hint is
attributable and correctable by hand. The typed profile is `TASK-030`, where it can be built on
evidence rather than guessed at from four file extensions.

Manual verification caught two defects the automated suite did not: `canonicalize` answers in the
Windows verbatim `\\?\` form, so every path in the report read `//?/C:/work/project`, and the counts
line pluralised `directory` into `directorys`. Both were cosmetic and both would have shipped,
because every test asserted on the action list rather than on the prose. Tests for each now exist.

---

## MEM-014 - An acceptance criterion must not require a task that depends on it

```yaml
id: MEM-014
category: lesson
scope: project
status: active
confidence: high
recorded: 2026-10-01
supersedes: null
```

TASK-015's acceptance required `doctor` to verify the copied schemas (CTX-012). `doctor` is
TASK-014, and TASK-014's `depends_on` includes TASK-015. The criterion was therefore unsatisfiable
from inside the task that carried it: doctor could not be built until TASK-015 was `DONE`, and
TASK-015 could not be `DONE` until doctor verified the copies. `RULES.md` §12 forbids marking a task
`DONE` against unmet acceptance, so the deadlock was real and not a bookkeeping detail.

It was resolved by waiving the clause in writing and naming TASK-014 as its true owner, rather than
by closing the task and leaving the gap unrecorded. The general rule this suggests: when a task's
acceptance names a *check* performed by another task, confirm first that the named task does not
depend on this one. A criterion phrased as "X verifies Y" belongs to X, not to the task that
produced Y.

## MEM-015 - The hermetic git environment has one home, in the testkit

```yaml
id: MEM-015
category: constraint
scope: testing
status: active
confidence: high
recorded: 2026-10-09
supersedes: null
tags: [testkit, git, hermetic, fixtures]
```

A test that spawns git and a test that uses a fixture must run the same git with the same
environment, or the fixture's guarantees end where the raw spawn begins. So `TempRepository` exposes
the git executable it resolved (`git_program`) and the exact name/value set it applies
(`environment`), and its own `command()` builds the child from that one list. `aicontext-git`'s
integration tests construct `Git::with_environment(repo.git_program(), repo.environment())` rather
than re-deriving either.

The rule this encodes: a hermetic property is only as strong as its single definition. Two copies of
the variable set - one in the fixture, one in a test - can drift, and the drift is invisible until a
test goes red on someone else's machine for a reason neither copy explains. `environment` returns
pairs in application order and treats an empty value as unset, which is how `GIT_ASKPASS` is closed
rather than pointed at the empty string.

## Open questions

| # | Question | Blocks | Resolve by |
|---|----------|--------|-----------|
| Q-1 | Which YAML crate? `serde_yaml` is widely used but its maintenance status needs checking; a maintained fork may be required | TASK-016 | **RESOLVED in TASK-016: `yaml_serde` 0.10, see `ADR-007` and `MEM-010`** |
| Q-2 | JSON Schema validator crate choice, and whether we accept its transitive weight | TASK-015 | **PARTLY RESOLVED in TASK-015: `jsonschema` 0.58.3, `default-features = false`, dev-only for CI meta-validation. The runtime `Validator` remains open for TASK-014** |
| Q-3 | Should `doctor` fail the build on a deprecation warning, or only on errors? | TASK-014 | Phase 1 |
| Q-4 | Is the approval prompt a full-screen TUI, or a line-based prompt that composes with pipes? | TASK-072 | Phase 4 |
| Q-5 | Does an audit record need a stable third-party timestamp authority to resist backdating? | TASK-075 | Phase 4 |
| Q-6 | Task inline-vs-split threshold is set at 300 entities; confirm against real usage before Phase 4 | TASK-038 | Phase 2 |

---

## Bugs encountered

None shipped. Three defects were found before the task that introduced them was closed, so none
reached a consumer. Two were found by running the binary against a scratch project before TASK-012
was closed and are recorded under `MEM-013`: reported paths carried the Windows verbatim `\\?\`
prefix, and the counts line said `directorys`. Both are fixed and covered by tests. The third was
found by the integration suite on its first run, before TASK-017 was closed: the git log reader
desynchronised on the newline git appends between commits, and is filed with its root cause under
`.ai/bugs/BUG-001-git-log-record-separator.md`.

Record under `MEM-0NN` with `category: bug` when the next one appears, and always file the full
root-cause analysis under `.ai/bugs/BUG-NNN.md`.
