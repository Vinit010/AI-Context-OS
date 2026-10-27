---
id: MEMORY-001
type: memory
title: AI Context OS — Durable Memory
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-10-27
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

## MEM-016 - The register's current phase and task are body fields, read by one parser

```yaml
id: MEM-016
category: constraint
scope: context
status: active
confidence: high
recorded: 2026-10-09
supersedes: null
tags: [tasks, register, parsing, doctor, status]
```

`TASKS.md`'s current phase and current task are deliberately not in front matter - the schema leaves
the document-level record to identity and puts the moving cursor in the body as `**Current phase:**`
and `**Current task:**` lines. They move every task, and a front-matter field that changes every task
would make every diff touch two places for one fact.

Both readers of the register now share one inline-entity parser, `aicontext-context`'s private
`body` module: doctor for its document checks and `register` for the current cursor and the task
list. The rule this encodes is RULES 2 applied to Markdown: two commands that read the same file must
not each own a copy of the fenced-block and entity-heading rules, because the copies drift and the
drift is invisible until the two commands disagree about the same file. `register::read` never fails
- a missing or unreadable `TASKS.md` is an empty register - because `status` must describe an absent
register rather than abort, and a command that cannot run before `init` cannot report on a project
that has not been initialised.

## MEM-017 - A register block is YAML, so a plain-scalar entry must be quoted if it holds a `#` or a colon-space

```yaml
id: MEM-017
category: constraint
scope: register
status: active
confidence: high
recorded: 2026-10-09
supersedes: null
tags: [tasks, yaml, parsing, authoring]
```

The fenced block under each task is parsed exactly as front matter is, so it obeys YAML, not
Markdown. A done or acceptance entry written as a plain (unquoted) scalar breaks the block in two
ways that are easy to miss while reading it as prose: `#` begins a comment, so an entry containing
`#[non_exhaustive]` (or any `#`) is truncated at the `#`; and a `: ` inside a plain scalar reads as a
mapping, so an entry containing `notes: value` stops being a scalar. Both make the whole block
unparseable, and the block does not fail alone - doctor reports `CTX-002` against the task and any
consumer that reads the register sees one fewer task.

The rule for editing this file: quote as a single-quoted scalar any entry containing `#` or `: `, and
double an internal `'` to `''`. TASK-014's repair pass quoted the colon cases and missed the first
`#` case in TASK-018, which is why the register parsed for a while and then did not. After editing a
block, the cheap check is that `aicontext doctor` still reports the file as valid.

## MEM-018 - An embedded template's line endings belong to the checkout, not the repository

```yaml
id: MEM-018
category: constraint
scope: project
status: active
confidence: high
recorded: 2026-10-09
supersedes: null
tags: [templates, include_str, line-endings, init, doctor, windows]
```

`include_str!` embeds the bytes git wrote to disk, and those bytes depend on the reader's git
configuration: Git for Windows defaults to `core.autocrlf=true`, which turns the repository's LF into
CRLF on checkout. A rule that fixes a canonical byte form - `docs/CONTEXT_SPEC.md` rule 11, LF on
write - therefore cannot be satisfied by normalising only what the tool produces, because the input
it is compared against arrived from the same checkout. Both sides are brought to the canonical form at
the boundary: `init`'s `Bindings::apply` folds CRLF and a lone CR to LF before substituting, and
`doctor`'s CTX-012 folds the compiled-in source to LF before comparing it to the copy in `.ai/schemas`.

The general rule: embedded text is checkout-dependent input, not a constant. Any code that treats the
bytes of an `include_str!` as author-controlled content must normalise them where they enter, and the
test must supply the form the developer's own checkout cannot produce. This is the shape of defect
that running the suite locally cannot catch, because it lives exactly where the developer's git
configuration and the CI runner's disagree (see `.ai/bugs/BUG-004-crlf-embedded-templates.md`).

## MEM-019 - A credential check must not hold a credential, repeat one, or mistake prose for one

```yaml
id: MEM-019
category: constraint
scope: project
status: active
confidence: high
recorded: 2026-10-09
supersedes: null
tags: [doctor, security, CTX-016, tests, false-positives]
```

`RULES.md` 7 and 12 forbid a credential-shaped literal anywhere - source, fixtures, snapshots. That
rule applies to the credential scan itself, so the CTX-016 tests build each vector at run time (a
prefix and a run of filler joined with `concat`) rather than committing a string any pattern would
call a secret. The scanner is tested without the suite becoming the leak it warns about.

The report must not become a second leak either: a finding names the file, the line, and the rule, and
never the match, so a `doctor` run over a compromised tree, its JSON envelope, and its logs are all
safe to share. A detector's own text is a false-positive risk in its own right - `openai-api-key`
requires a word boundary before `sk-`, because the substring in `task-name` would otherwise report
every document that mentions a task.

## MEM-020 - Discovery is bounded, deterministic, and never guesses

```yaml
id: MEM-020
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-10
supersedes: null
tags: [discovery, context-engine, determinism, project-profile, TASK-030]
```

`discover(root) -> ProjectProfile` is advisory and read-only: it inspects and reports, and never
writes a finding into a document (`docs/CONTEXT_SPEC.md` 5). Three properties are what the next agent
has to keep, and each is a choice rather than an accident.

It is **infallible**. A root that cannot be listed reads as a root with nothing in it, so an
unreadable or missing directory yields the same empty, `unrecognised` profile that a genuinely empty
project does, rather than a `Result`. This is the same call `register::read` makes for a missing
`TASKS.md` (`MEM-016`): a command that cannot report before `init` has run is a command that fails on
the one case it most needs to describe. A caller that must tell "could not look" from "saw nothing"
asks the filesystem directly.

It is **bounded and deterministic**. At most `MAX_ENTRIES_SCANNED` top-level entries are examined and
a manifest past `MAX_MANIFEST_BYTES` is skipped rather than truncated (`RULES.md` 11), every list is
sorted, and no result depends on directory iteration order (`RULES.md` 8). Two runs over an unchanged
tree are equal (`RULES.md` 12).

It **never guesses**. An unknown stack yields `unrecognised: true` with every field empty and
`confidence: None`; nothing is inferred from a directory name alone. Frameworks are read from
manifest *text* with a bounded token scan, not a parsed dependency graph, because no TOML or JSON
parser is available in the crate and adding one was refused for a discovery-only need (`RULES.md` 6).
The scan is deliberately conservative (word boundaries, so `react` never matches `preact`) and every
finding keeps the manifest or config file as its `evidence`, so a wrong hint is attributable and
correctable by hand rather than silently authoritative. The typed `ProjectProfile` replaces the
`Signal` list `init` carried (`MEM-013`); `init`'s own `detect.rs` is a separate, narrower reader and
is not yet rewired to it.

## MEM-021 - The index is a cache: its identity is the content hash, and a scan reads nothing it does not have to

```yaml
id: MEM-021
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-11
supersedes: null
tags: [index, context-engine, incremental, content-hash, TASK-031]
```

`.aicontext/index.json` is a **cache, not a source of truth**. It is fully derived from `.ai/`, it is
git-ignored, and a damaged or foreign-version cache is rebuilt rather than trusted (`RULES.md` 4.10).
Three properties are what the next agent has to keep.

Each entry's **content hash is its identity for invalidation**, and one edit invalidates one entry
(`docs/CONTEXT_SPEC.md` 6). The map is keyed by repository-relative path, but the SHA-256 in the entry
is what a rescan compares: when a file's bytes hash the same, its parsed entry is kept even if the path
or the metadata moved. Keying the map itself by the hash would collapse two documents with identical
bytes into one, which is why the two are separate.

A scan reads as little as possible. A private `(size, mtime-in-nanoseconds)` fingerprint is compared
before a file is opened; on a match the cached entry is reused and the file is never read. Only a moved
fingerprint causes a read, and an unchanged hash then costs a rehash rather than a re-parse. This is
asserted through `ScanReport::reads`, a deterministic count, because `RULES.md` 8 forbids asserting on
the wall clock: "a second run over an unchanged tree re-reads nothing" is `reads() == 0`.

The walk is `.gitignore`-aware and hermetic. `ContextIndex` walks `root/.ai` with the `ignore` crate at
`hidden(false)` (the `.ai` directory is itself hidden, so hidden-off would walk nothing), `parents(true)`
and `git_ignore(true)` with `require_git(false)` so ignore files apply without a `.git` repository, and
`git_global(false)` so a developer's global ignore file cannot change what is indexed. The two fixed-path
YAML files under `.ai/` are config, not documents, and are not indexed. A document larger than
`DOCUMENT_MAX_BYTES`, or a path the walk cannot deliver, is counted in the report and skipped rather than
aborting the scan (`RULES.md` 4.8); the body is never parsed for meaning (`ADR-003`), only scanned for
headings and ID-shaped links.

## MEM-022 - The retrieval plan is a request, not an action

```yaml
id: MEM-022
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-11
supersedes: null
tags: [retrieval, context-engine, plan, TASK-032]
```

`RetrievalPlan` defines what to ask for (`docs/CONTEXT_SPEC.md` §7.1) without reading documents. It
records: the seed task (when one exists), any free-text query, the mandatory set (`.ai/AI.md` and
`.ai/RULES.md`, plus `--include`), graph seeds (`linked_ids` = `spec` + `depends_on` + `relates_to`),
path `globs` (task `touches` merged with `--globs`), `filters` (epic and phase derived from the task,
merged with `--filters`), `include_kinds`, `exclude_ids`, and `ContextBudget`. All are serialisable
and deterministically rendered by `RetrievalPlan::explain()`.

The task's own graph keys are read once, by `register::read` and stored in `TaskEntry` (a public
struct in `aicontext-context::register`). A predicate carries its value as text rather than a sealed
`Value` so a `--filter` flag can be represented naturally; matching projects a document's front
matter to a comparable string form (a scalar becomes its decimal, boolean, or string; a list is
searched for equal items; a string is tested by substring for `contains`). The plan is a value object
(exported from `aicontext-context::retrieval`) and is never the one that walks the filesystem — that
is left to the retrievers and the `ContextIndex`.

## MEM-023 - The lexical retriever is pure, and the body it scores lives in the index

```yaml
id: MEM-023
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-11
supersedes: null
tags: [retrieval, context-engine, bm25, index, TASK-033]
```

`LexicalRetriever` turns a plan and a `ContextIndex` into `Vec<ScoredDocument>` and reads nothing
else. `ScoredDocument` and `RetrievalTrace` (`docs/CONTEXT_SPEC.md` §7.3) carry the id, path, tier,
score, a human-readable reason, and the trace, so a candidate is self-describing. The `Retriever`
trait signature is exactly the seam `ARCHITECTURE.md` §4.2 reserves —
`fn retrieve(&self, plan: &RetrievalPlan, index: &ContextIndex) -> Vec<ScoredDocument>` — so the
Always/Linked/Filter/Glob implementations (`TASK-034`) and a future `EmbeddingRetriever` are new
implementations, not new call sites.

Scoring is BM25F at `k1 = 1.2`, `b = 0.75` over the §7.2 field weights (title 3.0, id 2.5, tags 2.0,
headings 1.5, other front-matter values 1.2, body 1.0). A term's weighted frequency is the sum over
fields of weight × count, and a document's weighted length is the sum over fields of weight × token
count; the non-negative IDF `ln(1 + (N - df + 0.5)/(df + 0.5))` is computed over the same documents
the results come from. Ordering is score descending, then id ascending, then path ascending — a total
order with no randomness and no dependence on hash-map order (`ADR-004`).

Because §7.2 scores the body at weight 1.0 and the retriever never touches the filesystem, the body
text has to be in the cache. `IndexEntry` therefore gains `body: String` (`docs/CONTEXT_SPEC.md` §6),
serde-defaulted and filled at build time, and `CACHE_VERSION` is bumped from 1 to 2 so a version-1
cache is rebuilt rather than read as an all-empty body. The serde default is what lets a v1 cache
deserialize and then fail the version check with the precise `IndexError::Version` instead of a
generic `Format` error. The cost is that the git-ignored cache now roughly mirrors the size of the
`.ai` Markdown; the alternative — reading bodies during retrieval — would make `retrieve` do I/O and
break the one property that makes the packet reproducible.

## MEM-024 - Graph, filter, and glob retrieval is exact; broken links are reported as dropped traces

```yaml
id: MEM-024
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-11
supersedes: null
tags: [retrieval, context-engine, graph, glob, filter, TASK-034]
```

The three non-lexical retrievers (`TASK-034`) are exact, not ranked. `LinkedRetriever` resolves each
`RetrievalPlan` edge (`spec`, `depends_on`, `relates_to`) forward through `ContextIndex::resolve`: a
front-matter `id`, a repository path, or a register heading that defines an inline entity — so a
`depends_on` on an inline task resolves to `.ai/TASKS.md`. Every resolved edge scores 1.0 and the
reason reads `linked from TASK-034 (depends_on)`, which is why the plan stores
`LinkedId { id, edge }` rather than a bare id list: an edge remembers how it was reached instead of
making the retriever re-derive the graph from the id's shape.

An edge that resolves to nothing becomes a dropped trace (`included: false`,
`dropped_reason: "broken link"`) at the tier its target would have sat at (spec → 4, depends_on → 5,
relates_to → 9) — never skipped, the same honesty `doctor`'s `CTX-007` applies to front-matter
references (`RULES.md` §4.8). Filter and glob retrieval keep every document that satisfies all
predicates / matches a glob; the glob matcher is our own (`*` within a segment, `**` across segments,
`?` one character) because the crate must not grow a dependency for the pattern subset a `touches`
list needs (`ARCHITECTURE.md` §2.2). Ordering everywhere is score, then `id`, then path, so merged
results stay reproducible (`ADR-004`).

## MEM-025 - The ladder numbers tier 1 highest; only project-wide scalar assertions can conflict

```yaml
id: MEM-025
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [retrieval, context-engine, priority, conflict, TASK-035]
```

The priority ladder (`TASK-035`, `docs/CONTEXT_SPEC.md` §7.4, `ARCHITECTURE.md` §6) numbers tier 1 the
**highest** priority (the caller's instruction) and tier 9 the lowest (history). So "a lower tier
contradicts a higher tier" means the loser has the **larger** number, and the winner is the
**smallest** tier. `PriorityResolver::conflicts` picks that winner per asserted key; a lower tier that
disagrees becomes one `Conflict { asserted_by, conflicts_with, resolution: higher-tier-wins }`. The
first implementation had this backwards — it treated the largest number as the winner — which is
exactly the kind of inversion the spec's ten-line ladder invites, so it is worth a memory entry: when
reading the ladder, "up" is toward 1.

Detection compares **scalar** front-matter values over one documented allowlist, `ASSERTION_KEYS`
(`store`, `database`), which is the single opt-in point. Comparing every shared key would report a
conflict for each pair of documents that do not happen to agree, because a document's own keys
(`status`, `phase`, `spec`, `feature`, ...) describe that document and are expected to differ. A list
or map is a collection of claims rather than one value, and `null` is the absence of a claim, so
neither is compared. A key asserted once, or by documents that agree, is not a conflict.

Same-tier disagreement is deliberately **not** reported: it is not a lower-versus-higher conflict, so
neither side is made to lose. A same-tier tie-break on the smallest id only decides which assertion
the comparison is made against, which keeps the output a total order and therefore reproducible
(`ADR-004`). `Assertion.path` carries `#[serde(skip)]`, so a conflict serializes as the §7.6
`{tier, id, value}` shape while `doctor` can still point at the loser's file.

`doctor` reuses the existing `CTX-013` rather than adding a code: "a lower tier contradicts a higher
tier" now names both `check_architecture`'s declared-stack-versus-discovery half and
`check_conflicts`'s assertion half. Finally, the third category the spec lists a task whose `spec`
points at a document of a different `feature` is left undetected on purpose: no document carries a
`feature` key today, so there is nothing to compare, and guessing one would be inventing scope
(`RULES.md` §2). It is recorded in §7.4 as deferred, to be implemented when a `feature` field arrives.

## MEM-026 - The budget protects the whole upper half of the ladder, and its cost is the index's one number

```yaml
id: MEM-026
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [retrieval, context-engine, budget, degradation, TASK-036]
```

The context budget (`TASK-036`, `docs/CONTEXT_SPEC.md` §7.5) is a pure, deterministic function from a
candidate list and an index to a kept set and a report (`decisions/ADR-004`). Three properties are what
the next agent has to keep.

The **protected floor is tier 5**, not tier 2. Only tiers 6–9 are droppable, and they go in the order
9 → 8 → 7 → 6, lowest score first within a tier, with id then path as the total tie-break. This is
stricter than the task's acceptance wording ("tiers 1–2 are never dropped") and satisfies it; the flow
was widened deliberately because policy, architecture, specification, and the task itself are as
undroppable as the entry point in practice. Because the ladder numbers tier 1 highest (`MEM-025`), "drop
from the lowest tier upward" means the **largest** number first — the same inversion the conflict rule
warns about, and the reason this entry repeats it: when reading the ladder, "down" is toward 9. A budget
that cannot fit tiers 1–5 is refused with `CTX-030`, never silently truncated; the same code covers a
protected tier that exceeds a `per_tier` cap.

A candidate is **priced from the index**, `IndexEntry::token_estimate` by its portable path, so the
budget charges the exact heuristic §6 already publishes and invents no second number. `apply_with`
exposes the same rule with an explicit cost function, which is the seam `ARCHITECTURE.md` §12 reserves
for a per-model tokenizer rather than a speculative generalisation. `ContextBudget` lives in its own
module `crates/aicontext-context/src/budget.rs` (re-exported from `lib.rs`, so the public path is
unchanged), because §7.5 is its one home (`RULES.md` §2). `reserved_policy_tokens` is documented as
satisfied by construction — tiers 1–2 are kept unconditionally — and reserved for the per-model policy
of a later phase; no numeric semantics were invented for it.

`CTX-030` is the first `CTX` code that is **not a `doctor` check**: it is raised by `aicontext context`,
and `doctor --explain` deliberately returns `Unknown` for it, because the `Explanation` type is scoped
to doctor checks. It is listed in the §8 table anyway, with a note, because that table is the `CTX`
catalogue a reader will look in.

## MEM-027 - The packet is references only; content enters at render time, under an untrusted envelope

```yaml
id: MEM-027
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [retrieval, context-engine, cli, trust, packet, TASK-037]
```

`aicontext context` (`TASK-037`, `docs/CONTEXT_SPEC.md` §7.6) publishes one object: the `ContextPacket`
is the envelope's `data`, unchanged. The CLI approves a second renderer only for the prompt view, and
`--format json` is an alias for the global `--json` that loses to it when both are given — the flag is
the machine contract, so it is the one that wins. The packet holds **references and metadata, never
bodies**: `--json` and the default human view read nothing from a document, and `--format prompt` — the
one content-bearing view — reads each body from the index at render time. `ContextPacket::render_prompt`
lives in the context crate, not the CLI, because wrapping retrieved content in a `BEGIN/END UNTRUSTED
DATA` envelope is the trust boundary (`RULES.md` §9, `MEM-004`), and the boundary belongs beside the
type that decides what is untrusted. `--explain` changes only the human and prompt views, adding
`RetrievalPlan::explain` and every provenance trace; the packet it is rendered from is identical.

The packet splits candidates by the ladder, not by a new rule: tiers 1–2 (the mandatory `AI.md` and
`RULES.md`, plus any `--include`) are the `policy` region; every other candidate is `retrieved` and
carries `trust: untrusted`. `Trust` is a **one-variant enum**, so no code path can mark retrieved
content as instruction — the type makes `MEM-004` unrepresentable rather than merely discouraged.
`include_kinds` and `exclude_ids` are applied **before** the budget, so a document that is both
excludable and over budget is reported `excluded`, not `budget`; the reason records the first thing
that happened to it. `AI.md` and `RULES.md` are exclusion-protected by path, so `--exclude` can never
remove the always-on entry point even when it names their id — the same invariant the budget keeps by
refusing to drop tiers 1–5 (`MEM-026`).

The command keeps the same index contract as `doctor`: load `.aicontext/index.json` when present,
`build` with those entries as fingerprints so a warm run re-reads only what changed, and `save`
best-effort — a failed write is `CTX-019` at **info** level, not an error, because the cache is
derivable and a read-only checkout must still assemble. Two consequences of the index scope carry
forward and are worth restating: the scan covers only `root/.ai`, so a task whose `spec` or `touches`
point at `crates/**` yields a dropped provenance trace (`missing` or a broken link) rather than a
retrieved document, and the packet therefore describes what the index holds, not what the repository
holds. Widening the index was deliberately out of `TASK-037`'s scope.

## MEM-028 - Health reports counts it can source; a metric with no source is absent, not zero

```yaml
id: MEM-028
category: constraint
scope: context-engine
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [health, metrics, cli, observability, TASK-038]
```

`aicontext health` (`TASK-038`, `docs/CONTEXT_SPEC.md` §9) publishes the §9 metrics as counts, each with
the command that reproduces it and no composite score. The constraint that decides every disagreement is
this: a metric is worth showing only if it has a source the command can point at, and a metric with no
source is left out rather than shown as zero. Stale memory and plugin health are therefore absent, not
zero — nothing counts them yet. The same rule makes documentation freshness `n/a` with a reason outside
a Git repository, and makes a document with no `touches` count as neither fresh nor stale: a zero would
be a claim the tree does not support.

The counts are not a second implementation of the rules. The check-derived metrics call the same
`aicontext_context::doctor::run` that `doctor` calls, and the command beside each metric is the exact
`aicontext doctor --only <code>` that reproduces it, so the two commands cannot drift into disagreeing
about the same tree. `health` is read-only, more strictly than `context`: it builds the index in memory
and never saves it, so a report never creates `.aicontext/`. Two consequences carry forward: the
reproducing command is part of the contract rather than a hint, and a future metric should be added only
once its source exists.

## Open questions

| # | Question | Blocks | Resolve by |
|---|----------|--------|-----------|
| Q-1 | Which YAML crate? `serde_yaml` is widely used but its maintenance status needs checking; a maintained fork may be required | TASK-016 | **RESOLVED in TASK-016: `yaml_serde` 0.10, see `ADR-007` and `MEM-010`** |
| Q-2 | JSON Schema validator crate choice, and whether we accept its transitive weight | TASK-015 | **PARTLY RESOLVED in TASK-015: `jsonschema` 0.58.3, `default-features = false`, dev-only for CI meta-validation. The runtime `Validator` remains open for TASK-014** |
| Q-3 | Should `doctor` fail the build on a deprecation warning, or only on errors? | TASK-014 | Phase 1 |
| Q-4 | Is the approval prompt a full-screen TUI, or a line-based prompt that composes with pipes? | TASK-072 | **RESOLVED in TASK-072: a line-based plain-text prompt on stderr, not a TUI; only the literal `yes` approves and every missing answer denies, see `MEM-039`** |
| Q-5 | Does an audit record need a stable third-party timestamp authority to resist backdating? | TASK-075 | Phase 4 |
| Q-6 | Task inline-vs-split threshold is set at 300 entities; confirm against real usage before Phase 4 | TASK-038 | **RESOLVED in TASK-038: the register holds 41 inline tasks, well under the 300-entity threshold, and `aicontext health` now reports the register's task count, so crossing it is observable. Keep 300 and revisit if the count approaches it** |

## MEM-029 - The provider abstraction is honest only if its signature names no provider

```yaml
id: MEM-029
category: constraint
scope: provider-layer
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [providers, ai-provider, abstraction, boundaries, TASK-050]
```

`TASK-050` defines `AIProvider`, the provider-neutral request and response types, and the registry. The
constraint that decides every question is `docs/AI_PROVIDER_SPEC.md` §1: the context engine must not know
that models exist. The test of whether the abstraction is honest is mechanical, not aesthetic — adding an
adapter must touch no other crate. Three shapes enforce it rather than a convention:

- The trait's signature names no provider. Only `CompletionRequest`, `CompletionResponse`,
  `StreamEvent`, `Capabilities`, `ProviderError`, and `ProviderId` appear in it — no URL, header, status,
  or wire type. The trait is `async` and dyn-compatible (the registry holds `Box<dyn AIProvider>`), which
  is why it uses `async-trait`: a native `async fn` in a trait is not object-safe. The crate re-exports
  `async_trait` and `Stream` so an adapter implements the trait with only this crate in scope.
- The two regions are two types, not two fields. Trusted instruction is `PolicyRegion`; external content
  is `DataBlock`, whose `trust` is a one-variant `Trust::Untrusted`. No constructor moves one into the
  other, so "never place external content in a policy region" (T9) is a type rule, not a review habit.
- Failure and refusal are different facts. `ProviderError` carries one variant per transport, HTTP,
  capability, and cancellation failure, each with a stable `PROVIDER_*` code and a `remediation()` hint.
  A model *refusal* is not an error: it is a successful `CompletionResponse` with
  `finish_reason: Refusal`, and an unrecognised finish reason is preserved as `Unknown`, never guessed
  (T12).

`ProviderRegistry` holds `Box<dyn AIProvider>` keyed by a validated `ProviderId`, and `route` is the seam
`docs/AI_PROVIDER_SPEC.md` §8 asks for: in the MVP it returns the configured default and ignores its
request, so adding routing later changes no signature. Two consequences carry forward: the provider layer
has no write capability whatsoever (ADR-006), and mapping a `ProviderError` onto process exit `6` belongs
to the command that runs a provider, not to this crate. The crate deliberately does not depend on
`aicontext-core`: nothing in it uses a core type, and an unused dependency would be a false one.

## MEM-030 - An adapter is honest only if its failures are typed and its retries are bounded and visible

```yaml
id: MEM-030
category: constraint
scope: provider-layer
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [providers, openai, reliability, retries, cancellation, secrets, TASK-051]
```

`TASK-051` adds the OpenAI-compatible adapter: one implementation over a configurable `base_url` that
reaches the hosted endpoint and every local server speaking the same shape. `MEM-029` records the
abstraction's honest signature; this entry records what the first implementation had to get right, and
each is a choice the next adapter must repeat rather than rediscover.

**Translation lives in one module and never leaks a wire type.** `openai/wire.rs` maps to and from
`serde_json::Value` by hand instead of deriving types, so no provider-shaped struct reaches the
neutral API, and `PolicyRegion` becomes `system` messages while every `DataBlock` becomes one
enveloped `user` message. T9 ("never place external content in a policy region") is therefore a
property of the mapping, and a test asserts it on the bytes the fake server received rather than on
the types alone.

**A failure is a typed value with a next step.** Every HTTP status and every transport fault maps onto
a `ProviderError` variant carrying a stable `PROVIDER_*` code and a `remediation()` hint; a body that
is not JSON is `MalformedResponse`, not a panic. The adapter never falls back to another provider or
model: a failure is returned, not hidden.

**Reliability is bounded, and a retry is visible.** Three timeouts (connect, request, total) are each
configurable and each failure names its phase. A retry happens only for an idempotent request (one
with an idempotency key), only for a retryable failure (transport, 429, 5xx), at most
`RetryPolicy::max_retries` times, honours a provider `Retry-After`, and is logged through `tracing` so
it is never silent (`RULES.md` §6). A `CancellationToken` races the whole call and the stream, so
cancellation returns `Cancelled` promptly rather than waiting out a timeout.

Two consequences carry forward. A stream carries no error variant, so the reader task ends the stream
on a failure rather than emitting one, and hands events to the host through a bounded channel - which
is also why `bytes` is not a dependency. And a credential in `Auth::Bearer` is a `Secret` that renders
`[redacted]` in every `Debug` and `Display`, so a config three types up cannot print it (T10); the
adapter never resolves a credential itself, which is `TASK-052`.

The transport itself deviates once from `RULES.md` §6, recorded in `ADR-009`: `reqwest`'s `rustls-tls`
feature resolves to `rustls` with the `ring` provider, which compiles C at build time. `ring` adds a
build-time C toolchain, not a runtime system library, which is the intent that clause protects.

## MEM-031 — A documented CI guard that is not run is not a guard, and the private-doc-link defect recurred

```yaml
id: MEM-031
category: bug
scope: context-engine
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [context, rustdoc, ci, documentation, BUG-005]
```

`BUG-003` fixed one public doc comment that linked to a private item, and recorded the guard for the
class: run the documentation gate with CI's flags. Two milestones later the same defect was found
again, this time in `aicontext-context`'s `discovery` module, whose public documentation linked to the
private constants `MAX_ENTRIES_SCANNED` and `MAX_MANIFEST_BYTES`. It is filed as
`.ai/bugs/BUG-005-private-intra-doc-link-discovery.md`.

The lesson is not the link, which is mechanical to fix, but that a guard written down is only a guard
if it runs. The bare `cargo doc` prints `rustdoc::private-intra-doc-links` as a warning and exits 0,
so the failure exists only under `RUSTDOCFLAGS=-D warnings`, and it surfaces in whichever crate the
task at hand is not touching - here, a provider-layer task found a defect in the context engine.
Run the documentation gate with the flags CI uses, over the whole workspace, as part of the routine
rather than only when a documentation change is made.

---

## MEM-032 — Credentials resolve behind ports, and redaction lives at the one boundary

```yaml
id: MEM-032
category: constraint
scope: providers
status: active
confidence: high
recorded: 2026-10-14
supersedes: null
tags: [credentials, security, adr-005, keyring, redaction]
```

The credential path has two halves and one rule. `Secret` is the type-level half — a value that
renders `[redacted]` and can only be read by asking — and `CredentialResolver` is the decision half.
Its order is fixed by `SECURITY.md` §6 and `ADR-005`: an explicit flag, then an environment variable,
then the OS keychain. `Environment` and `Keychain` are traits rather than calls, for two reasons that
outlast this task: a test must not read the developer's real environment or keychain (`RULES.md` §8),
and `std::env::set_var` is `unsafe` in edition 2024 (`RULES.md` §3), so the only hermetic way to test
resolution is to inject the ports; and `connect` (`TASK-053`) writes through the same seam that
resolution reads through.

Redaction is one type, `Redactor`, applied at one place: the provider's error body, which is the one
string in a call that the provider, not we, built. That placement is what "at the logging boundary,
not ad hoc" (`SECURITY.md` §6) means in practice — the adapter redacts there so no other call site has
to remember to. It masks a known value exactly, a value it was never told about by shape (`sk-`,
`ghp_`, `Bearer`, …), and a value by key name where the key is credential-shaped.

`keyring` supplies the OS keychain. Each backend is the OS credential store and nothing else — the
Windows Credential Manager, the macOS Keychain, the Linux kernel keyring — so `RULES.md` §6 holds with
no C compiler and no third-party system library, and, unlike TASK-051's `rustls`/`ring`, no `ADR`
deviation is needed. `3.6.3` was chosen over `4.2.0` by MSRV-aware resolution: `4.2.0` requires Rust
1.88, above the workspace floor of 1.85.

## MEM-033 — A committed schema constrains the writer: `disconnect` removes the file rather than emptying the list

```yaml
id: MEM-033
category: lesson
scope: cli
status: active
confidence: high
recorded: 2026-10-20
supersedes: null
tags: [schema, ai-yaml, connect, disconnect, ai-config, task-053]
```

`schemas/ai-config.schema.json` declares `providers: minItems 1`. A schema is usually read as a
constraint on whoever *reads* a document; it is equally a constraint on whoever *writes* one, and
`connect` and `disconnect` are the only writers `.ai/ai.yaml` has. TASK-053 reached the case that
made the difference: `disconnect --provider <the-last-one>` cannot write `providers: []`, because an
emptied list would be a document that fails the schema other tools trust (`CONTEXT_SPEC.md` §1).

The resolution was to take the schema's own word for what "no providers" means. `init` never creates
this file, so a fresh project has none — the state a project with no providers is *in* is "no
`.ai/ai.yaml`", and removing the file is how `disconnect` returns to it. That is a stronger reading of
*leaves no residue* than an empty list would have been, and it required no schema edit. Relaxing
`minItems` to 0 was the other candidate and was rejected: it would have widened a committed contract
outside the task's `touches` scope in order to accommodate one command's output.

The general shape: before inventing a new state for a document you write, ask whether the schema
already defines the transition. Three states — absent, one provider, many — are all the schema
describes, and a command that only moves between them can never produce a file a reader rejects.

Related, and learned in the same task: the split between a *reference* and a *value* is what makes a
committed config file safe to write at all. `.ai/ai.yaml` holds `keychain_entry: hosted`, an `env`
variable's *name*, or nothing; the credential itself goes to the OS keychain. `auth.type` is one of
`keychain`, `env_or_keychain`, `none` — three words that say where a value lives, never what it is.
`--token` is the one flag with no `clap` value parser for the same reason: the parser's own error
quotes the value it rejected, and an empty secret is caught in the command, which names the flag
without naming what it held.

## MEM-034 — A contract test records one implementation until a second one can fail it

```yaml
id: MEM-034
category: lesson
scope: providers
status: active
confidence: high
recorded: 2026-10-20
supersedes: null
tags: [providers, adapters, contract-tests, shared-transport, task-054]
```

TASK-051 wrote the provider contract (`AI_PROVIDER_SPEC.md` §11, T1–T12) and built one adapter to
satisfy it. With a single implementation, "the contract is provider-neutral" was a claim about the
future: nothing in the test suite could falsify it, because every wire fact the assertions rested on
was a fact about the OpenAI-compatible shape. TASK-054 added the two adapters that could. They
falsified plenty — Gemini puts the model in the path, spells a turn that proposed a function `STOP`,
and assigns no id to a call at all; Anthropic splits usage across the first and last stream events,
sends the instruction as a top-level `system` string, and has no seed to send. None of that is the
same as anything else, and none of it is the contract.

The resolution has two halves, and they answer different questions. *What is shared* was extracted
once — `src/transport` now holds auth, timeouts, retries, SSE parsing, the stream pump, and error
detail, each with exactly the three users `RULES.md` §2 asks for before code is shared. That the
extraction changed nothing was established by running the old OpenAI suite against the refactored
adapter *before* replacing it, so "no behaviour changed" was a result rather than an intention. *What
differs* became a fixture: `tests/contract.rs` loops three fixtures through every test, and each
fixture owns the endpoint path, the wire spelling of a reply, a tool call, usage and a finish reason,
the header a credential lands in, where the policy and the data sit in the body, and the bytes of a
stream.

The test file that results is the point. Everything in it is an assertion the spec says is the same
wherever the call goes; every branch lives in a fixture, where a wire fact belongs. Adding an adapter
therefore means adding a fixture and editing no assertion — which is what makes "all three pass the
shared suite" a checkable statement instead of a description of intent. The general lesson: a
contract test earns its name only when a second implementation can fail it, and the measure of one is
not that it passes but what you must touch to make a new implementation pass. Two smaller habits came
with it — assertion messages name the adapter they came from, so one red test still says which wire
format broke, and the list of what exists lives in one place (`Adapter::ALL`) that callers read
rather than re-state.

## MEM-035 — A plan is a rendering of a validated proposal, or the answer verbatim, and never the model's own word that it is one

```yaml
id: MEM-035
category: lesson
scope: cli
status: active
confidence: high
recorded: 2026-10-20
supersedes: null
tags: [cli, agents, plan-mode, validation, schema, task-055]
```

`agent run` has to write something to `.aicontext/plans/` from a model's answer, and the tempting
shortcut is to trust the answer: if it looks like JSON, parse it; if it parses, act on it. The
provider contract (`AI_PROVIDER_SPEC.md` §9) refuses that. A *structured* plan is one the model
claims is an `action_proposal` **and** that validates against `schemas/action-proposal.schema.json`
— kind, version, at least one action, and every action a `plan` with a summary or a `tool_call` with
a tool and an object of arguments. A claim that fails any of that is not partially honoured and not
silently downgraded: it exits 3 (`AGENT-005`) naming every problem, and nothing is written.
Everything that is not a claim of being a proposal is a plan too, kept verbatim as prose, because a
model that answered in sentences has still planned.

The validation is hand-rolled to mirror the schema, which is a second definition of one contract and
therefore a drift risk. It is held together by a test that loads the committed schema and asserts
the two agree on every fixture, so the schema may stay a dev-only dependency: nothing in the runtime
path parses JSON Schema, and the package a user installs does not grow to check its own answer.

Two smaller decisions carry the same instinct — the parser decides, not the file. The plan path is
`.aicontext/plans/<task>.md` with the task id sanitised to `[A-Za-z0-9._-]`, so an id can never walk
out of the directory, and the file is overwritten deterministically rather than versioned, because a
plan is a snapshot of one attempt and the register is the history. And a credential the profile
*names* but the machine cannot produce is a failure (`CREDENTIAL_KEYCHAIN_MISSING`, exit 1), not a
call made without one: `auth: none` is the only way to say "no credential", so that every free and
local provider runs with none resolved and no keychain read, while a misconfigured hosted one stops
loudly. The general lesson: when a machine's answer is about to become an artifact, the artifact is a
rendering of what *validated*, and the validation is a contract with a test holding it to its schema
(`MEM-034`).

## MEM-036 — A cost is unavailable or exact, never zero, and a ceiling that cannot be projected is not a ceiling

```yaml
id: MEM-036
category: constraint
scope: cli
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [providers, cli, cost, budget, money, anthropic, task-056]
```

A run reports what it cost only from two facts it actually has: the usage the provider reported and
the prices the developer configured in `.ai/ai.yaml` (`budget.max_run_usd` for the ceiling, a
per-provider `pricing` block for the rates of USD per million tokens). No vendor price table is
compiled in, because a hard-coded price is wrong the day the vendor changes it and wrong silently.
When either half is missing the answer is **unavailable**, never `$0`: a provider with no `pricing`
block, or a response with no usage, is unknown, and an unknown cost must not look free. A configured
price of zero is a real zero, and the types keep the two apart — `Some(Cost::ZERO)` versus `None`.

Money is integer micro-USD on the whole path. The product is `u128` before it saturates into a `u64`,
each component rounds **up**, and the YAML number is parsed from its decimal rendering rather than
through `f64`, so a price is the number that was written and a reported cost is never below the
arithmetic truth. Six fixed decimals are printed so a human comparing a cost with a ceiling does not
have to count zeros. `Usage.cached_input_tokens` is a subset of `input_tokens`; the adapters
normalise to that shape because the vendors disagree on the wire, and the Anthropic adapter adds
cache reads and cache writes into the input total while reporting only cache reads as cached. A cache
*write* is therefore charged at the input price — usually an under-estimate, and documented as one
rather than papered over with an invented fourth price.

The ceiling is checked in both directions. Before the call, the input side is estimated from the
JSON-escaped bytes of the request (over four) plus a small scaffolding allowance, and the output side
is charged at the full output ceiling; a projection above the limit is `AGENT-012` and **no call is
made**, which is the only check that can actually stop overspend. After the call, the
provider-reported total is compared with the ceiling: an overage is `AGENT-013` and exits 3, but the
plan is still written because the spend already happened and destroying its result helps nobody.
Usage too incomplete to price is the `AGENT-014` warning, so an unverifiable ceiling says so.
A ceiling with no prices is `AGENT-011` and fails closed before any call, including a dry run.

Two smaller consequences. `connect` may rewrite a provider entry, so its `upsert` merges rather than
replaces: a hand-written `pricing` block, a `default_model`, or a future adapter's key is
configuration the writer does not own and must survive a reconnect. And because an error finding can
now fail a run, `agent run` derives its exit code from the findings as `doctor` and `export` already
do — the plan-mode path had no way to report a problem after a successful call until now. The general
lesson: a budget is only a budget when both halves of the arithmetic are present and the check
happens before the money is spent; a check after the fact is a receipt, not a limit.

## MEM-037 — A plugin API version is negotiated, not pinned by the schema, and MCP compatibility is an input alias, not a second output dialect

```yaml
id: MEM-037
category: constraint
scope: plugin-sdk
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [plugin, sdk, manifest, json-rpc, mcp, versioning, task-070]
```

`plugin_api_version` is deliberately **not** a schema constant. If the manifest schema pinned it to
`"1"`, a plugin built for version 2 would fail schema validation before negotiation, and its author
would get a generic schema error instead of the `PLG-004` that names both versions. The schema
therefore accepts one decimal version, the SDK parses it into `ApiVersion`, and `negotiate` makes the
compatibility decision. The split is semantic: a malformed version is a property of the manifest and
is `PLG-011`; a well-formed version the host does not implement is a property of the *pair* and is
`PLG-004`. A JSON Schema can express the first and cannot express the second. The current range is
four digits, which is why every schema-valid value also fits the SDK's `u16` and the two layers
cannot disagree about what "valid" means.

The manifest types are deliberately stricter than the schema in the places a schema cannot reach:
`deny_unknown_fields` rejects a typo instead of defaulting it, duplicate tool names are refused, and
`destructive: true` on a read capability is refused by both layers. `destructive` also raises the
permission floor to `EXPLICIT_APPROVAL` through `Capability::permission_floor()`, so a manifest cannot
lower the approval its own flag requires. The full schema is still validated by the installer
(`TASK-077`); the SDK does not carry a runtime `jsonschema` engine, and the canonical fixture is
validated against the published schema by a dev-only test. A plugin author should not pay for the
host's validator.

The wire protocol is JSON-RPC 2.0, and its MCP compatibility is one-directional on purpose. MCP's
`name`, `description`, `inputSchema`, and `isError` are accepted **on input**, but serialization uses
the plugin contract's `tool`, `summary`, `input_schema`, and `is_error`. There is one output dialect,
so the host and the audit record have one shape to reason about, while a third-party MCP descriptor
still parses. `effect` is required and has no MCP equivalent: defaulting a missing classification to
`read` would understate a write tool, which is exactly the failure a permission system must not make.
Unknown MCP fields are preserved with `flatten` rather than dropped, so a newer MCP revision survives
the bridge; how much of MCP the runtime actually supports is still `TASK-078`'s decision.

`PLG-004` and `PLG-005` are SDK-producible, so they are constants in `aicontext-core` now;
`PLG-011` invalid manifest is added to `docs/PLUGIN_SPEC.md` §4 as an additive code. The runtime's
process-supervision codes wait for `TASK-073`, because core must not carry a code no crate can return.
The family prefix is part of a code's identity: `PLG-004` and `CTX-004` share a number but never a
rendered code, and `docs/CLI_SPEC.md` §6 promises `code` is safe to branch on.

Finally, the plugin manifest schema is published in `schemas/` and listed in the CLI's
`EXPECTED_SCHEMAS`, but it is **not** scaffolded into `.ai/schemas`. It validates an external plugin
artefact, not an `.ai/` document, so `doctor`'s count of 20 document schemas stays correct. If a
future task decides plugins may be authored inside `.ai/`, it must revisit both `init` and the count;
until then, adding the schema to that copy set would make the count mean two different things.

## MEM-038 — An explicit policy default is honored, but an absent policy is deny-all, and a wildcard no step consults is refused rather than inert

```yaml
id: MEM-038
category: constraint
scope: permissions
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [permissions, resolver, policy, wildcard, default-deny, task-071]
```

The policy schema publishes `defaults.mode` as one of four values, so the resolver honors the value
the file states; treating the field as decoration would make the schema lie. The fail-closed
guarantees live at the boundary instead: `Policy::default()` is deny-all, the shipped template writes
`mode: deny`, and a missing or unreadable policy is deny-all. An invalid policy is `CTX-003`, never a
silent fallback. Invariant I3 therefore means "no explicit mapping, including the default mapping,
resolves to deny"; a file that explicitly maps the unmentioned set to allow has said so as policy
and must not be silently overridden.

The old `PLUGIN_SPEC` §5 example used a top-level `permissions:` map and an `aws.*` deny beside
`aws.ec2.describe` allow. Both were wrong: the schema and template require
`version`/`defaults`/`tools`, and step 2 puts wildcard `DENY` ahead of step 7 exact `ALLOW`, so that
example denied everything it appeared to allow. The docs now show the committed shape and a
coherent rule set.

Wildcard rules are accepted only as `provider.service.*`, and only with `deny` or `approval`.
`allow` and `explicit_approval` wildcards are rejected with `UnsupportedWildcardMode` because the
resolution table has no step that could consult them; accepting one would create a rule that looks
load-bearing and can never match. `*`, `*.action`, extra segments, embedded `*`, and an empty
provider or service are invalid. A wildcard covers exactly one action segment, so `aws.ec2.*` does
not cover `aws.ec2` or `aws.ec2.describe.extra`.

File order is not a tie-break. Duplicate patterns are refused, each step matches at most one rule,
and the tests reverse rule order to prove decisions do not depend on it. Exact `ALLOW` losing to
wildcard `APPROVAL` is intentional because step 6 precedes step 7; the earlier "a more specific rule
always beats a wildcard" wording contradicted the table and was removed. `DecisionSource` records
which step matched, including `Destructive`, so an audit entry can explain the result without
re-resolving against a policy that may have changed.

## MEM-039 — An approval prompt is one line-based exchange on stderr, and only the literal `yes` is consent

```yaml
id: MEM-039
category: constraint
scope: approval
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [approval, prompt, cli, non-interactive, audit, deny-by-default, task-072]
```

`Q-4` is resolved by `TASK-072`: the approval prompt is plain text on stderr, not a full-screen TUI.
A line-based prompt composes with pipes, lets a redirected run print the text and deny with exit 5,
and lets a test assert the exact bytes a human sees without a terminal emulator. The prompt order is
fixed - tool, plugin, target, compact-JSON arguments, caller's reason, permission mode - and every
caller-controlled field is escaped so an argument, target, or reason cannot forge another prompt
line. Only the literal word `yes`, after trimming surrounding whitespace, approves; Enter (an empty
line), `y`, `YES`, punctuation, and every other word deny. A non-interactive invocation prints the
prompt and denies without reading stdin, because a pipe is not a human, and the terminal prompter
caps an answer at 64 bytes so an over-long answer denies rather than becoming a truncated `yes`.

The decision matrix is total and per-invocation. `ALLOW` and `DENY` answer without a prompt or an
audit event; `APPROVAL` accepts an exact `--allow-once <tool>` match or prompts; `EXPLICIT_APPROVAL`
always prompts and ignores the flag. There is no blanket `--yes` and no session grant: a future
session grant must be added to the broker deliberately, never inferred from a prior answer. Every
prompt is recorded before it is shown and every decision after it is reached, so an interrupted
prompt still leaves a trace. An audit sink that cannot record either event refuses the call with
`PLG-010`; an approval that cannot be audited is not an approval.

The audit event carries an `ApprovalSummary`, not the request: arguments become `sha256:<hex>` of
their compact JSON and the free-form reason is omitted. The type makes the exclusion structural,
since the summary has no field able to hold either value. The terminal prompter installs no signal
handler and no wall-clock timer; it reports end of input and interrupted reads, and the broker maps
every prompter error - including a reported timeout - to a denial. `TASK-074` still has to wire the
broker and terminal into the single tool gateway and map a denial to exit 5; `TASK-075` still has to
make the audit sink durable.

## MEM-040 — An MSRV is proved by building on 1.85, and nothing else proves it

```yaml
id: MEM-040
category: bug
scope: build
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [msrv, ci, dependencies, let-chains, resolver, BUG-006]
```

`BUG-006` is the eighth defect found before a consumer saw it. The `msrv (1.85)` job failed twice in
sequence: first in `ignore` 0.4.30, whose new incremental walker uses a let-chain but whose manifest
declares no `rust-version`, so the MSRV-aware resolver had nothing to reject it by; then in
`wildcard_matches` in `aicontext-permissions`, written by `TASK-071` as a three-arm let-chain. Both
constructs are accepted by the pinned 1.95 toolchain, so `verify`, `clippy`, and the ordinary test job
were green. Only a build on 1.85 could see either failure.

Two rules follow. While the MSRV is below 1.88, let-chains are forbidden in workspace code; use
`let ... else` or nested matches, which express the same early return and compile on 1.85. A dependency
that declares no `rust-version` must be exact-pinned when a release starts using newer language
features, because "the newest release that keeps our floor" is a claim to prove by building with
`cargo +1.85.0`, not to infer from published metadata.

For every dependency update or language-level change, run the real floor before pushing:

```
cargo +1.85.0 build --workspace --all-targets --locked
cargo +1.85.0 test --workspace --locked
```

The CI job is the detection; a local 1.85 run is the prevention. See
`.ai/bugs/BUG-006-msrv-let-chain-break.md`.

## MEM-041 — A plugin supervisor owns one process, one deadline, and three byte caps

```yaml
id: MEM-041
category: constraint
scope: plugin-system
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [plugin-system, supervisor, process, json-rpc, timeout, output-limit, task-073]
```

`TASK-073` implements the process half of `docs/PLUGIN_SPEC.md` §7 as one synchronous
`PluginProcess`: one child, one request-id sequence, and one deadline per call. There is no call
queue and no second path to the plugin; `TASK-074`'s gateway is the only caller. The timeout and
the output cap come from `limits` in the manifest and are revalidated before spawn, so a command
line cannot widen a plugin's authority for one call and a plugin cannot negotiate a longer deadline
at handshake.

Every failure after spawn kills the child and poisons the pipe. A malformed frame, a timeout, an
output overflow, a broken pipe, or an unexpected exit leaves the stream desynchronised, and later
calls return `NotRunning` rather than guessing that the next frame answers the next request. There
is no retry, no process pool, and no resynchronisation.

The output cap has three surfaces, because one is not enough: a single frame is bounded before it
is buffered (`fill_buf`, not `read_until`, so a line that never ends cannot grow host memory), the
total `log` and `host/request_approval` traffic during one call is counted so logging cannot extend
a deadline, and a result is rejected past 32 levels of nesting. Plugin-initiated approval is always
answered `approved: false` with a reason: the host decides before the call, and a plugin cannot ask
itself for permission. Logs and stderr are bounded separately and controls are stripped from the
stderr tail so diagnostics cannot forge terminal output.

The stdout, stdin, and stderr threads are detached rather than joined. A plugin can spawn a
grandchild that inherits a pipe, and joining would make the host wait for a process it does not
control; dropping the handles lets the threads end with the pipe without ever wedging the CLI.
`TASK-074` still adds the gateway and untrusted-result envelope, and `TASK-076` still adds the
working directory, environment, filesystem, and network controls; the supervisor deliberately
spawns with the host environment and expects the sandbox to narrow it first. The code uses
`let ... else` rather than let-chains and both 1.85 gates pass (`MEM-040`).

## MEM-042 — The tool gateway is one funnel, and every gate is structural rather than advisory

```yaml
id: MEM-042
category: constraint
scope: plugin-system
status: active
confidence: high
recorded: 2026-10-26
supersedes: null
tags: [plugin-system, gateway, json-schema, untrusted-data, audit, task-074]
```

`TASK-074` turns the pieces into the one path `docs/PLUGIN_SPEC.md` §1 requires: a `ToolGateway`
owns the manifest, the policy, the prompter, the audit sink, the `--allow-once` set, and the single
`PluginProcess`. `ToolGateway::call` validates the arguments against the tool's `input_schema`,
resolves the policy, asks the broker, records `Executing`, dispatches exactly once, normalises the
result, and records `Completed`. `PluginProcess::call_tool` stays public because the supervisor is a
library of its own, but a production source scan asserts that `.call_tool(` occurs once, in
`gateway/mod.rs`. A second caller would fail the test before review could miss it.

The manifest is checked against `tools/list` before the first call: an undeclared, missing,
duplicate, or effect/destructive-mismatched descriptor is refused. The resolver receives the
manifest's destructive flag, not the descriptor's, so a plugin cannot lower the `I4` floor it
advertised. `--allow-once` remains exact-match and cannot satisfy `EXPLICIT_APPROVAL`; that is now
proved through the gateway as well as the broker.

Argument validation is a closed subset of Draft 2020-12, not a best effort. Unknown keywords such as
`oneOf` and `$ref` are refused with `PLG-012`, because a constraint the host silently ignored would
be worse than a missing one. The subset supports one type (or a union carrying only `enum`/`const`),
`properties`/`required`/`additionalProperties: false`, arrays, string lengths and patterns, numeric
bounds, `enum`, and `const`. The 32-level depth cap and 64 KiB size caps bound both the schema and
the arguments. A dev-only `jsonschema` test cross-checks every supported keyword against a real
Draft 2020-12 validator, so the subset cannot drift without a failure; the production dependency
stays hand-rolled (`R-2`).

A result becomes `UntrustedToolResult` and nothing else. It carries the literal `trust: untrusted`
marker, sanitised content and extra fields, `is_error` as data, and host-recorded `request_id` and
`duration_ms`; plugin-supplied metadata is dropped. ANSI escapes and control, bidi, and zero-width
characters are replaced rather than concatenating the words around them, and two keys that collide
after sanitising refuse the result instead of silently merging. The envelope is measured after
serialisation against `limits.max_output_bytes`, so host metadata can push a raw frame that fit over
the cap and the call is then `PLG-006`.

Audit is a seam rather than a side effect. Broker events are forwarded through `ApprovalBridge` to
the gateway's `GatewayAudit`, so one sink sees `Prompted`, `Decided`, `Executing`, and `Completed` in
order; a sink failure refuses the call with `PLG-010` before dispatch, or withholds a result already
produced. A policy denial or a human "no" is `GatewayOutcome::Refused`, not an error, but it is still
recorded. The durable hash chain is `TASK-075`; the sandbox is `TASK-076`; and the CLI still has no
plugin execution command, so the gateway is a library edge until `TASK-077` and the Phase 5 loop.

## MEM-043 — The audit chain seals canonical JSON, verifies every read, and refuses to extend a broken log

```yaml
id: MEM-043
category: constraint
scope: plugin-system
status: active
confidence: high
recorded: 2026-10-27
supersedes: null
tags: [audit, hash-chain, jsonl, canonical-json, task-075]
```

`TASK-075` adds `aicontext-audit`, which depends only on `aicontext-core`, `serde`, `serde_json`,
`sha2`, and `thiserror`. The log is `.aicontext/audit.jsonl`, one JSON object per line, `seq` from 1.
`self` is SHA-256 over the record's compact JSON with keys sorted and `self` removed, so key order
and whitespace never change the digest. `prev` is the previous `self`; the first record follows the
fixed genesis sentinel of `sha256:` plus 64 zeroes. A hardcoded digest test pins the canonical form,
so a change to canonicalisation is a deliberate decision rather than a silent break of every
existing log.

Verification is not a mode. Every read checks the record shape, `seq` contiguity, `prev`, the
recomputed `self`, and the timestamp spelling, and returns the verified prefix plus the first bad
line when something fails. There is no trusted fast path. A missing file is an empty valid chain;
an unreadable file is an error, never emptiness. Reads are capped at 64 MiB. Appends verify the
existing log first and refuse with `AUD-005` without writing on any failure; an acknowledged append
is `sync_data`'d. Appends assume one writer per project until the project lock owns concurrent
writers.

The record has no field for raw arguments: `AuditEntry` exposes `with_arguments_digest` and nothing
that accepts an argument value, and `AuditRecord` denies unknown fields, so a hand-written
`arguments` object is malformed. Identifiers reject control and bidi/zero-width characters;
`resource` keeps them because it is free text and the CLI sanitises it for display. The event,
actor kind, effect, approval mode, and result status are bounded words rather than free prose.

`AUD-001`–`AUD-005` are defined in `aicontext-core` with the rest of the catalogue. The CLI's
`audit show|verify|tail` is read-only, publishes one JSON shape across all three views, exits 3 for
a chain that does not verify (rendering the verified prefix), 1 for an unreadable log, and 2 for a
bad root. `verify` prints the head as the external witness `SECURITY.md` §8 requires. Nothing
appends yet: the first command that executes a plugin becomes the writer, and gateway wiring is
deliberately not part of `TASK-075`.

## Bugs encountered

None shipped. Eight defects were found before a consumer saw them. Two were found by running the
binary against a scratch project before TASK-012 was closed and are recorded under `MEM-013`:
reported paths carried the Windows verbatim `\\?\` prefix, and the counts line said `directorys`.
Both are fixed and covered by tests. The third was found by the integration suite on its first run,
before TASK-017 was closed: the git log reader desynchronised on the newline git appends between
commits, filed under `.ai/bugs/BUG-001-git-log-record-separator.md`. The fourth was found by CI on
Linux after TASK-017 was closed: its Unix-only `normalise` test was missing its `#[test]`, so the
branch never ran and the attribute-less function was dead code, filed under
`.ai/bugs/BUG-002-unix-test-missing-test-attribute.md`; it is visible only on the platform the
developer is not using, which is the lesson it records. The fifth was found by the same CI run: a
public doc comment in aicontext-testkit linked to a private item, which `cargo doc` treats as an
error under `-D warnings`, filed under `.ai/bugs/BUG-003-private-intra-doc-link.md`; it was invisible
locally because the bare `cargo doc` prints the warning and exits 0. Both CI defects share one lesson
- a gate that passes locally with weaker flags than CI is a gate that is not being run. The sixth, from
the same Windows job, was that templates embedded with `include_str!` carried the runner's CRLF line
endings, so `init` wrote CRLF documents its own front-matter reader rejected and CTX-012 compared an LF
copy against a CRLF source, filed under `.ai/bugs/BUG-004-crlf-embedded-templates.md`; it adds a second
lesson - an embedded file's bytes are a property of the checkout, not the repository, so text that
enters a comparison must be normalised at the boundary (see `MEM-018`). The seventh, found while
closing `TASK-051`, was the same private-link defect as the fifth in a second crate:
`aicontext-context`'s public `discovery` documents linked to two private constants, filed under
`.ai/bugs/BUG-005-private-intra-doc-link-discovery.md`. Its lesson is that a guard written down is not
yet a habit - the flag-carrying documentation command has to run over the whole workspace every time,
because the defect surfaces in a crate the current task is not touching (`MEM-031`). The eighth, found
by CI when `TASK-072` was pushed, was the 1.85 floor broken twice at once: `ignore` 0.4.30's new
incremental walker and the permission resolver's `wildcard_matches` both used let-chains, which the
pinned 1.95 toolchain accepts but 1.85 does not, filed under
`.ai/bugs/BUG-006-msrv-let-chain-break.md`. Its lesson is that an ordinary build cannot prove an MSRV;
only a build on the declared floor can (`MEM-040`).

Record under `MEM-0NN` with `category: bug` when the next one appears, and always file the full
root-cause analysis under `.ai/bugs/BUG-NNN.md`.
