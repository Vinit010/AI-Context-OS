# CONTEXT_SPEC — Knowledge format and the context engine

Normative. Implements `ADR-003` and `ADR-004`. Anything not specified here is an implementation
detail and may change without a version bump.

---

## 1. Document kinds and locations

| Kind | Path | ID | Schema |
|------|------|----|--------|
| Agent entry point | `.ai/AI.md` | — (fixed path) | `ai-entrypoint.schema.json` |
| Product | `.ai/PRD.md` | `PRD-NNN` | `prd.schema.json` |
| Architecture | `.ai/ARCHITECTURE.md` | `ARCH-NNN` | `architecture.schema.json` |
| Rules | `.ai/RULES.md` | `RULES-NNN` | `rules.schema.json` |
| Conventions | `.ai/CONVENTIONS.md` | `CONV-NNN` | `conventions.schema.json` |
| Design | `.ai/DESIGN.md` | `DESIGN-NNN` | `design.schema.json` |
| Task register | `.ai/TASKS.md` | `TASKS-NNN` | `tasks.schema.json` |
| Memory register | `.ai/MEMORY.md` | `MEMORY-REG` | `memory.schema.json` |
| Specification | `.ai/specs/<slug>.md` | `SPEC-<slug>` | `spec.schema.json` |
| Task | `.ai/tasks/TASK-NNN-<slug>.md` or inline in `TASKS.md` | `TASK-NNN` | `task.schema.json` |
| Decision | `.ai/decisions/ADR-NNN-<slug>.md` or inline | `ADR-NNN` | `decision.schema.json` |
| Bug | `.ai/bugs/BUG-NNN-<slug>.md` or inline | `BUG-NNN` | `bug.schema.json` |
| Change | `.ai/changes/CHG-NNN-<slug>.md` or inline | `CHG-NNN` | `change.schema.json` |
| Context note | `.ai/context/<slug>.md` | `CTX-<slug>` | `context-note.schema.json` |
| Workflow | `.ai/workflows/<slug>.md` | `WF-<slug>` | `workflow.schema.json` |
| Agent profile | `.ai/agents/<slug>.md` | `AGENT-<slug>` | `agent.schema.json` |
| Permission policy | `.ai/permissions/permissions.yaml` | — (fixed path) | `permission-policy.schema.json` |
| AI config | `.ai/ai.yaml` | — (fixed path) | `ai-config.schema.json` |

Schemas are authored in `schemas/` and copied to `.ai/schemas/` by `aicontext init`. `doctor`
verifies the copies are identical to the source.

---

## 2. Front matter contract

```markdown
---
id: TASK-014
type: task
title: Implement the lexical retriever
status: IN_PROGRESS
priority: HIGH
phase: 2
depends_on: [TASK-032]
spec: SPEC-context
touches: ["crates/aicontext-context/src/**"]
created: 2026-09-27
updated: 2026-09-27
tags: [retrieval, context]
---

Human-readable body. Never parsed.
```

**Rules**

1. The front-matter block is optional only for `AI.md` and free-form notes. Every other document
   requires `id`, `type`, and `title`.
2. `id` must match the document's location convention and, for inline entities, the enclosing
   level-2 heading. A mismatch is error `CTX-004`.
3. `type` must match the schema for that location. A mismatch is error `CTX-005`.
4. Key order: `id`, `type`, `title`, `status`, type-specific keys, `created`, `updated`, `tags`.
5. Unknown keys are **warnings** (`CTX-006`), not errors, so a newer schema does not break an older
   binary. Unknown keys are preserved on read and preserved on write.
6. Dates are ISO-8601 `YYYY-MM-DD`. Timestamps are RFC-3339 with an explicit offset.
7. `created` is immutable. `updated` is set by the tool when it writes the file, never guessed.
8. The body is free text and is never parsed. A human may restructure it freely.
9. The `spec` key holds either a `SPEC-*` entity ID or a path to a normative specification under
   `docs/` (`docs/CONTEXT_SPEC.md`, `docs/SECURITY.md`, `docs/PLUGIN_SPEC.md`,
   `docs/AI_PROVIDER_SPEC.md`). Both forms are resolved by `doctor`; an unresolvable value is
   `CTX-007`.
10. Line endings are normalised to `\n` on write. A file is written atomically (temp file + rename).
11. Maximum front-matter size: 64 KiB. Maximum document size: 1 MiB, and the excess is reported
    rather than silently truncated.

### 2.1 Inline entity blocks

```markdown
## TASK-014

```yaml
id: TASK-014
type: task
title: …
status: IN_PROGRESS
```

Body.
```

The parser locates a level-2 heading, then takes the **first** fenced `yaml` block that follows it
within the heading's section. A missing or duplicated block is an error, not a guess.

---

## 3. Type-specific keys

```yaml
# task
status: BACKLOG | TODO | IN_PROGRESS | BLOCKED | IN_REVIEW | TESTING | DONE | CANCELLED
priority: CRITICAL | HIGH | MEDIUM | LOW
phase: <int>
epic: <string | null>
depends_on: [<ID>, …]
blocks: [<ID>, …]
spec: <ID | null>
touches: [<glob>, …]
acceptance: [<string>, …]        # required, min 1, unless waived: true
waived: <bool>                   # an IN_PROGRESS task with no acceptance requires waived: true
```

```yaml
# decision
status: proposed | accepted | deprecated | superseded
date: <YYYY-MM-DD>
deciders: [<string>, …]
supersedes: <ID | null>
superseded_by: <ID | null>
```

```yaml
# bug
status: open | investigating | fixed | verified | wont_fix | duplicate
severity: critical | high | medium | low
symptoms: <string>               # required
root_cause: <string | null>      # required before status: fixed
files_changed: [<path>, …]
prevention: <string | null>
```

```yaml
# change
date: <YYYY-MM-DD>
feature: <string>
reason: <string>
architecture_impact: none | minor | major
breaking: <bool>
migration: <string | null>
```

```yaml
# memory entry (inline in MEMORY.md)
category: constraint | lesson | fact | preference | limitation | bug
scope: <string>
confidence: low | medium | high
status: active | superseded
supersedes: <ID | null>
recorded: <YYYY-MM-DD>
```

```yaml
# spec
status: draft | review | accepted | deprecated
feature: <string>
```

**Legal task transitions.** Enforced by `task` commands and reported by `doctor`:

```text
BACKLOG   → TODO, CANCELLED
TODO      → IN_PROGRESS, BLOCKED, CANCELLED
IN_PROGRESS → IN_REVIEW, BLOCKED, TESTING, TODO, CANCELLED
BLOCKED   → TODO, IN_PROGRESS, CANCELLED
IN_REVIEW → TESTING, IN_PROGRESS, TODO
TESTING   → DONE, IN_PROGRESS, TODO
DONE      → (terminal; reopening creates a new task referencing this one)
CANCELLED → (terminal)
```

---

## 4. Document graph

Edges are expressed as ID lists in front matter and resolved by `doctor`.

```text
Task ──spec───────▶ Spec
Task ──depends_on─▶ Task          (acyclic; a cycle is error CTX-010)
Task ──touches────▶ path globs    (unresolved glob is a warning)
Task ──relates_to─▶ Bug | Change
Decision ──supersedes──▶ Decision
Decision ──relates_to▶ Task | Spec
Bug ──prevented_by──▶ Change | Memory
Change ──impacts────▶ Spec | Decision
Spec  ──depends_on─▶ Spec          (acyclic)
```

---

## 5. Project discovery

`discover` inspects and reports. It **never** modifies the project and **never** writes findings
into a document without `--apply`.

```text
Signals                     Evidence                          Reported as
language                    package.json, Cargo.toml,         languages[]
                            pyproject.toml, go.mod, pom.xml,
                            build.gradle
framework                   deps in the manifest, config     frameworks[]
package manager             lockfiles                        package_manager
version control             .git                             vcs: git
tests                       test dirs, test scripts           testing: true|false
CI / CD                     .github/, .gitlab-ci.yml,         ci: [<name>]
                            Jenkinsfile, .circleci/
containers                  Dockerfile, compose files         containers: [<name>]
infrastructure as code      *.tf, k8s manifests, charts       iac: [terraform, k8s]
cloud configuration         aws/, .azure/, gcp/, serverless   cloud_hints: [aws]
monorepo                    workspaces, pnpm-workspace,       packages: [<path>]
                            go.work, cargo workspace members
```

Output is a `ProjectProfile`:

```yaml
languages: [rust]
frameworks: []
package_manager: cargo
vcs: git
testing: true
ci: [github-actions]
containers: [docker]
iac: []
cloud_hints: []
confidence: high          # per-signal
unrecognised: false       # true when nothing was detected
```

A project that matches nothing yields `unrecognised: true`. It is never guessed at, and `init` says
so rather than filling in a plausible stack.

---

## 6. Index

```rust
struct IndexEntry {
    id: String,
    kind: DocumentKind,
    path: PathBuf,          // repo-relative
    title: String,
    front_matter: Map<String, Value>,
    headings: Vec<Heading>, // for lexical scoring only
    body_bytes: u64,
    token_estimate: u32,
    content_hash: [u8; 32],
    links: Vec<String>,     // referenced IDs
}
```

- Keyed by `content_hash`; a file whose hash is unchanged is not re-read.
- Persisted to `.aicontext/index.json`. It is a **cache**, fully rebuildable with
  `doctor --rebuild-index`, and is git-ignored.
- The index never contains external references; it is fully derivable from the repository.
- `token_estimate` is a heuristic (bytes/4 plus heading and front-matter weighting). It is used for
  budgeting only and is never presented to a user as an exact count.

---

## 7. Retrieval

### 7.1 Retrieval plan

```rust
struct RetrievalPlan {
    task: Option<TaskId>,
    query: Option<String>,
    mandatory: Vec<PathBuf>,      // AI.md, RULES.md, and any --include
    linked_ids: Vec<String>,      // resolved from the task graph
    filters: Vec<FrontMatterPredicate>,
    globs: Vec<String>,
    include_kinds: Vec<DocumentKind>,
    exclude_ids: Vec<String>,
    budget: ContextBudget,
}
```

`--explain` prints the resolved plan before retrieval, so a human can see what was asked for.

### 7.2 Scoring

`LexicalRetriever` uses BM25 (`k1 = 1.2`, `b = 0.75`) over a field-weighted document:

| Field | Weight |
|-------|--------|
| `title` | 3.0 |
| `id` | 2.5 |
| `tags` | 2.0 |
| headings | 1.5 |
| front-matter values | 1.2 |
| body | 1.0 |

Deterministic tie-breaking: by score descending, then by `id` ascending. There is no randomness
and no dependence on hash-map iteration order.

### 7.3 Provenance

Every candidate carries a `RetrievalTrace`:

```yaml
id: SPEC-context
tier: 4
score: 12.44
reason: "linked from TASK-037 (spec); lexical match on 'retrieval budget'"
source_path: .ai/specs/context.md
included: true
dropped_reason: null      # or "budget", "excluded", "lower tier", "denied"
```

### 7.4 Priority ladder

```text
1  current user instruction          (not a document; injected by the caller)
2  project policy                    RULES.md, CONVENTIONS.md
3  architecture                      ARCHITECTURE.md, DESIGN.md
4  specification                     specs/
5  current task                      TASKS.md, tasks/TASK-*.md
6  decisions (accepted)              decisions/
7  memory                            MEMORY.md
8  source code                       matched by globs
9  history                           changes/, deprecated/superseded ADRs, git log
```

**Conflict rule.** When a lower tier contradicts a higher tier on an asserted value, the higher tier
wins, the conflict is recorded in the packet, and `doctor` reports it as a warning. The higher-tier
document is never modified. Detection covers:

- structural facts (detected stack vs `ARCHITECTURE.md` claims),
- same-key front-matter assertions across documents,
- a task whose `spec` points at a document of a different `feature`.

### 7.5 Budget and degradation

```rust
struct ContextBudget {
    max_tokens: u32,
    per_tier: BTreeMap<Tier, u32>,   // optional overrides
    reserved_policy_tokens: u32,     // tiers 1-2 are never dropped
}
```

Degradation order, strictly: drop tier 9 → tier 8 → tier 7 → tier 6. Within a tier, drop the
lowest-scoring document first. Tiers 1–5 are never dropped; if they do not fit, assembly fails with
`CTX-030` and an actionable message. Every drop is reported with a reason.

### 7.6 Context packet

```yaml
version: 1
request: { instruction: "...", task: TASK-037 }
policy:                                  # tier 1-2, always present, never truncated
  - { id: RULES-001, tier: 2, content_ref: ".ai/RULES.md" }
retrieved:
  - { id: SPEC-context, tier: 4, score: 12.44, reason: "...", trust: untrusted,
      content_ref: ".ai/specs/context.md" }
conflicts:
  - { key: database, asserted_by: { tier: 3, id: ARCH-001, value: mongodb },
      conflicts_with: { tier: 9, id: CHG-004, value: postgresql },
      resolution: higher-tier-wins }
budget:
  requested: 32000
  used: 21400
  dropped: [ { id: CHG-004, tier: 9, reason: budget } ]
provenance: [ ... RetrievalTrace entries ... ]
untrusted_content_ids: [SPEC-context, BUG-002]
```

`untrusted_content_ids` lists everything the caller must place in a data role. A caller that
ignores this field is violating the contract.

---

## 8. `doctor` checks

| Code | Severity | Check |
|------|----------|-------|
| `CTX-001` | error | `AI.md` missing |
| `CTX-002` | error | Front matter absent or unparseable |
| `CTX-003` | error | Schema validation failed |
| `CTX-004` | error | ID does not match its location, or duplicates another document |
| `CTX-005` | error | `type` does not match the schema for that location |
| `CTX-006` | warning | Unknown front-matter key |
| `CTX-007` | error | Reference to a document that does not exist |
| `CTX-008` | warning | Reference to a deprecated or superseded document |
| `CTX-009` | warning | `IN_PROGRESS` task with no acceptance criteria and no waiver |
| `CTX-010` | error | Dependency cycle in tasks or specs |
| `CTX-011` | error | Illegal task status transition in history |
| `CTX-012` | warning | Schema copy in `.ai/schemas` differs from the source |
| `CTX-013` | warning | `ARCHITECTURE.md` contradicts the discovered project profile |
| `CTX-014` | warning | Deprecated decision with no successor |
| `CTX-015` | warning | Memory entry older than the file it describes, by `updated` date |
| `CTX-016` | error | Credential-shaped content in `.ai/` (see `SECURITY.md` §6) |
| `CTX-017` | warning | Generated file was hand-edited |
| `CTX-018` | warning | Document larger than the configured maximum |
| `CTX-019` | info | Index cache is stale or missing |
| `CTX-020` | info | Document recommends the inline-to-split switch |

`--strict` promotes every warning to an error. `--explain <code>` prints the rationale and a fix.

---

## 9. `health` metrics

Every metric is a count with the command that reproduces it. There is no composite score.

| Metric | Definition |
|--------|-----------|
| Context completeness | Documents present ÷ documents expected for an initialised project |
| Architecture consistency | Contradictions between `ARCHITECTURE.md` and the discovered profile, and between tiers |
| Documentation freshness | Documents whose `updated` predates the newest change to the code they describe |
| Task consistency | Tasks with acceptance criteria, valid transitions, and resolvable references |
| Decision coverage | Architectural boundaries and dependencies covered by an accepted ADR |
| Broken references | Count by check code |
| Stale memory | Active memory entries contradicted by a newer document |
| Plugin health | Manifest validity, granted permissions, last successful call |

---

## 10. Versioning

The packet carries `version: 1`. Additive fields are a minor change. A removed or retyped field is a
major change and requires a `plugin_api_version`-style compatibility declaration from consumers.
`aicontext context --json` output is covered by snapshot tests so accidental shape changes fail CI.
