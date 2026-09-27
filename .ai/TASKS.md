---
id: TASKS-001
type: tasks
title: AI Context OS — Task Register
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-09-27
---

# TASKS

**Current phase:** Phase 1 — Context MVP
**Current task:** TASK-011
**Rules:** one task at a time; do not start a task whose dependencies are not `DONE`.

Status values: `BACKLOG` · `TODO` · `IN_PROGRESS` · `BLOCKED` · `IN_REVIEW` · `TESTING` · `DONE` ·
`CANCELLED`

---

## How to read this file

Each task is a level-2 heading with the ID, followed by a fenced `yaml` metadata block, then the
body. The `yaml` block is the machine-readable record (`docs/CONTEXT_SPEC.md` §3); the body is for
humans. Once this file exceeds 300 tasks or 2,000 lines, tasks move to `.ai/tasks/TASK-NNN-<slug>.md`
and this file becomes a generated index (`ARCHITECTURE.md` §5.3).

```yaml
id: TASK-NNN
title: <imperative summary>
status: TODO
priority: HIGH          # CRITICAL | HIGH | MEDIUM | LOW
phase: 1
depends_on: [TASK-NNN]
spec: SPEC-<slug>       # a SPEC-* id or a docs/*.md path, or null
touches: []             # path globs
acceptance:             # all must be true
  - <criterion>
```

---

## Phase 0 — Specification

**Gate:** the document set is internally consistent, cross-references resolve, and every decision
has a recorded alternative.

### TASK-001 — Produce the Phase 0 document set

```yaml
id: TASK-001
title: Produce the Phase 0 document set
status: DONE
priority: CRITICAL
phase: 0
depends_on: []
spec: null
touches: [".ai/**"]
acceptance:
  - PRD, ARCHITECTURE, RULES, CONVENTIONS, DESIGN, TASKS, MEMORY, AI written
  - Each document states its authority and its cross-references
```

### TASK-002 — Write the normative specifications

```yaml
id: TASK-002
title: Write the five normative specifications in docs/
status: DONE
priority: CRITICAL
phase: 0
depends_on: [TASK-001]
spec: null
touches: ["docs/**"]
acceptance:
  - CONTEXT_SPEC, PLUGIN_SPEC, AI_PROVIDER_SPEC, SECURITY, CLI_SPEC written
  - Exit codes, permission modes, and schema keys are defined exactly once
```

### TASK-003 — Record the foundational ADRs

```yaml
id: TASK-003
title: Record the foundational ADRs
status: DONE
priority: HIGH
phase: 0
depends_on: [TASK-001]
spec: null
touches: [".ai/decisions/**"]
acceptance:
  - ADR-001 language, ADR-002 plugin process model, ADR-003 storage contract,
    ADR-004 deterministic retrieval, ADR-005 secrets, ADR-006 no autonomous writes
  - Each records rejected alternatives
```

### TASK-004 — Review the document set for internal consistency

```yaml
id: TASK-004
title: Review the document set for internal consistency
status: DONE
priority: HIGH
phase: 0
depends_on: [TASK-001, TASK-002, TASK-003]
spec: null
touches: [".ai/**", "docs/**"]
acceptance:
  - Every referenced ID, file, and command exists (verified: 47 task IDs, 6 ADRs, 0 dangling refs)
  - No two documents define the same concept differently
  - Open questions in MEMORY.md are either resolved or listed as a risk
```

### TASK-005 — Write README, CONTRIBUTING, and the issue templates

```yaml
id: TASK-005
title: Write README, CONTRIBUTING, and issue templates
status: IN_PROGRESS
priority: MEDIUM
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["README.md", "CONTRIBUTING.md", ".github/**"]
acceptance:
  - A new contributor can build, test, and submit a change from the README alone
progress:
  - README.md written with TASK-010, including honest status, layout, build and test commands
  - Still to do: CONTRIBUTING.md and the issue templates
```

---

## Phase 1 — Context MVP

**Gate:** §11 of the PRD can be completed through step 4. `init`, `status`, and `doctor` work on a
fresh repository with no network.

### TASK-010 — Create the Rust workspace and CI

```yaml
id: TASK-010
title: Create the Rust workspace and CI pipeline
status: DONE
priority: CRITICAL
phase: 1
depends_on: [TASK-004]
spec: null
touches: ["Cargo.toml", "rust-toolchain.toml", ".github/**", ".gitignore", "crates/**"]
acceptance:
  - Workspace builds with cargo build --workspace
  - Every crate root carries forbid(unsafe_code)
  - CI runs fmt, clippy -D warnings, test, and a dependency audit
  - CI enforces the crate dependency direction from ARCHITECTURE.md 3.2
done:
  - Cargo workspace with 5 crates, resolver 3, edition 2024, MSRV 1.85
  - Workspace lints: missing_docs deny, unsafe_code forbid, clippy all + pedantic
  - 23 tests pass, including the crate boundary matrix that enforces ARCHITECTURE.md 3.2
  - CI: fmt, build, test, doc on 3 OSes; clippy, MSRV 1.85, audit, release binary on Linux
```

### TASK-011 — Define the core domain types and error model

```yaml
id: TASK-011
title: Define the core domain types and error model
status: TODO
priority: CRITICAL
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["crates/aicontext-core/**"]
acceptance:
  - DocumentId, TaskId, DecisionId, BugId, ChangeId, PermissionMode, Severity are defined
  - AicontextError carries a code, message, cause, and remediation hint
  - Crate depends on no internal crate and no async runtime or CLI framework
```

### TASK-012 — Implement aicontext init

```yaml
id: TASK-012
title: Implement aicontext init
status: TODO
priority: CRITICAL
phase: 1
depends_on: [TASK-011, TASK-016]
spec: null
touches: ["crates/aicontext-cli/**", "templates/**"]
acceptance:
  - Creates the full .ai skeleton plus .aicontext/ in .gitignore
  - Emits advisory stack-detection results without rewriting project files
  - Idempotent: re-running does not overwrite an edited document
  - --dry-run prints the plan; --json emits the created path list
  - Validates its own output and reports the next commands
```

### TASK-013 — Implement aicontext status

```yaml
id: TASK-013
title: Implement aicontext status
status: TODO
priority: HIGH
phase: 1
depends_on: [TASK-011, TASK-017]
spec: null
touches: ["crates/aicontext-cli/**", "crates/aicontext-git/**"]
acceptance:
  - Shows project, branch, phase, current task, modified files, and pending tasks
  - Degrades gracefully outside a Git repository
  - Warm run under 300 ms
```

### TASK-014 — Implement aicontext doctor (v1)

```yaml
id: TASK-014
title: Implement aicontext doctor
status: TODO
priority: HIGH
phase: 1
depends_on: [TASK-011, TASK-015, TASK-016]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**", "crates/aicontext-cli/**"]
acceptance:
  - Detects a missing AI.md, invalid front matter, and dangling document references
  - Detects a task pointing at a missing specification and a deprecated ADR with no successor
  - Reports a contradiction between ARCHITECTURE.md and the detected stack
  - Exit code 3 when any error-level finding is present; --json emits findings with codes
```

### TASK-015 — Define the JSON Schema set

```yaml
id: TASK-015
title: Define the JSON Schema set
status: TODO
priority: HIGH
phase: 1
depends_on: [TASK-011]
spec: null
touches: ["schemas/**"]
acceptance:
  - A schema for every document type and fixed-path config listed in CONTEXT_SPEC section 1,
    plus action-proposal and project-profile schemas
  - Validated with a Draft 2020-12 meta-schema check in CI
  - init copies them to .ai/schemas and doctor verifies the copies match the source (CTX-012)
```

### TASK-016 — Implement the front matter parser

```yaml
id: TASK-016
title: Implement the front matter parser
status: TODO
priority: CRITICAL
phase: 1
depends_on: [TASK-011]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Parses a leading YAML block into a typed Document
  - Never parses Markdown body structure
  - Malformed input returns a typed error with a line number, never a panic
  - Round-trip and property tests over generated documents
```

### TASK-017 — Implement the Git wrapper

```yaml
id: TASK-017
title: Implement the Git wrapper
status: TODO
priority: HIGH
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["crates/aicontext-git/**"]
acceptance:
  - Branch, status, changed files, and recent commits via porcelain commands
  - Works outside a repository and on a repository with no commits
  - Arguments are passed as an argv array, never through a shell
  - Behaviour is covered by tests against real temporary repositories
```

### TASK-018 — Build the testkit

```yaml
id: TASK-018
title: Build the aicontext testkit
status: TODO
priority: HIGH
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["crates/aicontext-testkit/**"]
acceptance:
  - Helpers to build a temporary project, a temporary Git repository, and a sample .ai tree
  - No helper reads the real home directory, the network, or global Git config
```

### TASK-019 — Add the secret scan to doctor

```yaml
id: TASK-019
title: Add the secret scan to doctor
status: TODO
priority: HIGH
phase: 1
depends_on: [TASK-014]
spec: docs/SECURITY.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Flags credential-shaped keys and values inside .ai
  - Never echoes the matched secret; reports the path, line, and rule only
  - --strict makes it an error rather than a warning
```

### TASK-020 — Implement export and import

```yaml
id: TASK-020
title: Implement aicontext export and aicontext import
status: TODO
priority: MEDIUM
phase: 1
depends_on: [TASK-013]
spec: null
touches: ["crates/aicontext-cli/**"]
acceptance:
  - export writes a deterministic archive of .ai with a manifest and per-file digests
  - import verifies digests, reports conflicts, and never overwrites without --force
  - A round trip is byte-identical for a tree that was not edited between the two
```

---

## Phase 2 — Context engine

**Gate:** `aicontext context --task <id> --explain` returns a relevant, prioritised, provenance-
annotated packet, deterministically, with no network.

### TASK-030 — Implement the discovery engine

```yaml
id: TASK-030
title: Implement the project discovery engine
status: TODO
priority: HIGH
phase: 2
depends_on: [TASK-012]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Detects language, framework, package manager, tests, CI, containers, IaC, and cloud config
  - Produces an advisory ProjectProfile; never mutates the project
  - Unknown stacks yield an empty profile, not a guess
```

### TASK-031 — Build the content-hash index

```yaml
id: TASK-031
title: Build the content-hash context index
status: TODO
priority: CRITICAL
phase: 2
depends_on: [TASK-016]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Indexes .ai documents and respects .gitignore
  - Persists to .aicontext/index.json keyed by content hash
  - A second run over an unchanged tree re-reads nothing and is measurably faster
  - A changed file invalidates only its own entry
```

### TASK-032 — Define the retrieval plan

```yaml
id: TASK-032
title: Define the retrieval plan
status: TODO
priority: HIGH
phase: 2
depends_on: [TASK-031]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - A RetrievalPlan derives mandatory documents, graph links, filters, and globs from the task
  - The plan is inspectable via --explain and is fully serialisable
```

### TASK-033 — Implement the lexical retriever

```yaml
id: TASK-033
title: Implement the lexical retriever
status: TODO
priority: CRITICAL
phase: 2
depends_on: [TASK-032]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - BM25 over headings and body with per-field boosts, fully deterministic
  - No network, no embeddings, no model call
  - Every result carries a score and a human-readable reason
```

### TASK-034 — Implement the linked, filter, and glob retrievers

```yaml
id: TASK-034
title: Implement the linked, filter, and glob retrievers
status: TODO
priority: HIGH
phase: 2
depends_on: [TASK-032]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Linked follows the task graph: spec, decisions, bugs, touched paths
  - Filter matches front-matter predicates; Glob matches repository paths
  - A broken link is reported, not silently skipped
```

### TASK-035 — Implement the priority ladder and conflict detection

```yaml
id: TASK-035
title: Implement the priority ladder and conflict detection
status: TODO
priority: HIGH
phase: 2
depends_on: [TASK-033, TASK-034]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Tiers 1-9 are applied exactly as ARCHITECTURE.md section 6 defines
  - A lower-tier contradiction loses, is recorded in the packet, and is reported by doctor
  - The higher-tier document is never rewritten
```

### TASK-036 — Implement the context budget

```yaml
id: TASK-036
title: Implement the context budget and degradation
status: TODO
priority: HIGH
phase: 2
depends_on: [TASK-035]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Per-tier token caps with a documented degradation order from the lowest tier upward
  - Tiers 1-2 are never dropped; a budget too small for them is an explicit error
  - The report states what was dropped and why
```

### TASK-037 — Implement aicontext context

```yaml
id: TASK-037
title: Implement aicontext context
status: TODO
priority: CRITICAL
phase: 2
depends_on: [TASK-033, TASK-035, TASK-036]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-cli/**", "crates/aicontext-context/**"]
acceptance:
  - --task, --include, --exclude, --budget, --explain, --json
  - Output is a ContextPacket with policy, retrieved items, conflicts, budget report, and provenance
  - Retrieved content is wrapped in a data envelope marked untrusted and never placed in an
    instruction role
  - Warm run under 1 s
```

### TASK-038 — Implement aicontext health

```yaml
id: TASK-038
title: Implement aicontext health
status: TODO
priority: MEDIUM
phase: 2
depends_on: [TASK-014, TASK-037]
spec: null
touches: ["crates/aicontext-cli/**"]
acceptance:
  - Reports context completeness, architecture consistency, documentation freshness, task
    consistency, decision coverage, and broken references
  - Every metric is a transparent count with the commands that reproduce it
  - No opaque composite score
```

---

## Phase 3 — AI provider layer

**Gate:** a provider can be configured from the environment or keychain, and one task can be planned
from assembled context. Plan mode only — no writes.

### TASK-050 — Define the AIProvider trait and registry

```yaml
id: TASK-050
title: Define the AIProvider trait and registry
status: TODO
priority: HIGH
phase: 3
depends_on: [TASK-037]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - CompletionRequest, CompletionResponse, ToolCall, and Usage are provider-neutral types
  - The trait has no provider-specific types in its signature
  - Adding an adapter touches no other crate
```

### TASK-051 — Implement the OpenAI-compatible adapter

```yaml
id: TASK-051
title: Implement the OpenAI-compatible adapter
status: TODO
priority: HIGH
phase: 3
depends_on: [TASK-050]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - Works against OpenAI and against any base_url-compatible local server
  - Timeouts, retries with backoff, and cancellation are bounded and reported
  - A provider error becomes a typed error with a remediation hint, never a panic
  - Contract tests run against a local fake server
```

### TASK-052 — Implement the credential resolver

```yaml
id: TASK-052
title: Implement the credential resolver
status: TODO
priority: CRITICAL
phase: 3
depends_on: [TASK-050]
spec: docs/SECURITY.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - Resolution order: explicit flag, environment, OS keychain
  - Values are never written to .ai, stdout, logs, or the audit log
  - Redaction is applied at the logging boundary, not ad hoc
  - Unit tests assert no credential appears in any error or debug output
```

### TASK-053 — Implement aicontext connect

```yaml
id: TASK-053
title: Implement aicontext connect
status: TODO
priority: HIGH
phase: 3
depends_on: [TASK-052]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-cli/**"]
acceptance:
  - Stores a provider profile and a keychain reference, never a secret
  - Verifies connectivity with an explicit, minimal test call
  - disconnect removes the reference and leaves no residue
```

### TASK-054 — Implement the Anthropic and Gemini adapters

```yaml
id: TASK-054
title: Implement the Anthropic and Gemini adapters
status: BACKLOG
priority: MEDIUM
phase: 3
depends_on: [TASK-051]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - Each maps its tool-call and usage shapes onto the provider-neutral types
  - Both pass the shared contract test suite
```

### TASK-055 — Implement aicontext agent (plan mode only)

```yaml
id: TASK-055
title: Implement aicontext agent run in plan mode
status: TODO
priority: HIGH
phase: 3
depends_on: [TASK-051, TASK-053]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-cli/**"]
acceptance:
  - Assembles context, calls the provider, and writes a plan to .aicontext/plans
  - Modifies nothing in the working tree
  - Reports usage and cost, and honours the token budget
  - Unavailable provider produces a clear failure, not a silent degradation
```

### TASK-056 — Add cost and usage reporting

```yaml
id: TASK-056
title: Add cost and usage reporting
status: BACKLOG
priority: MEDIUM
phase: 3
depends_on: [TASK-055]
spec: null
touches: ["crates/aicontext-providers/**", "crates/aicontext-cli/**"]
acceptance:
  - Per-run token and cost totals, from provider-reported usage
  - Configurable budget ceiling that stops the run before overspend
```

---

## Phase 4 — Plugin system

**Gate:** a plugin can be installed, its permissions granted, a tool called with approval, and the
call appears in a verifiable audit chain.

### TASK-070 — Define the plugin SDK

```yaml
id: TASK-070
title: Define the plugin SDK manifest and protocol types
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-011]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-sdk/**", "schemas/plugin-manifest.schema.json"]
acceptance:
  - Manifest validates against a Draft 2020-12 schema and declares plugin_api_version
  - Protocol types are JSON-RPC 2.0 and MCP-compatible
  - The SDK depends only on aicontext-core and never on the runtime or CLI
  - Version negotiation rejects an incompatible plugin with a clear message
```

### TASK-071 — Implement the permission resolver

```yaml
id: TASK-071
title: Implement the permission model and resolver
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-070]
spec: docs/SECURITY.md
touches: ["crates/aicontext-permissions/**"]
acceptance:
  - Pure: no I/O, no clock, no randomness, no environment access
  - Precedence is DENY over EXPLICIT_APPROVAL over APPROVAL over ALLOW, defaulting to DENY
  - 100 percent branch coverage over the full mode matrix, including unspecified capabilities
  - Property tests over the resolution order
```

### TASK-072 — Implement the approval broker

```yaml
id: TASK-072
title: Implement the approval broker
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-071]
spec: docs/SECURITY.md
touches: ["crates/aicontext-plugin-runtime/**", "crates/aicontext-cli/**"]
acceptance:
  - A TTY prompt shows the tool, target, arguments, and reason, and requires an explicit answer
  - A non-interactive session denies rather than assuming consent
  - EXPLICIT_APPROVAL cannot be granted for a session; it is per invocation
  - Every prompt and every answer is audited
```

### TASK-073 — Implement the plugin runtime supervisor

```yaml
id: TASK-073
title: Implement the plugin runtime supervisor
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-070]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-runtime/**"]
acceptance:
  - Spawns the plugin, performs the handshake, and negotiates the API version
  - Enforces a per-call timeout, an output size cap, and a clean kill on both
  - A crashing, hanging, or flooding plugin yields a typed error and never wedges the CLI
  - Covered by deliberately misbehaving fixture plugins
```

### TASK-074 — Implement the tool gateway

```yaml
id: TASK-074
title: Implement the tool gateway
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-071, TASK-072, TASK-073]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-runtime/**"]
acceptance:
  - Every call passes argument validation, permission resolution, approval, execution, and audit
  - There is exactly one code path from a tool call to a plugin, enforced by review and tests
  - Responses are normalised into an untrusted data envelope with size and time caps
  - No shell execution path exists
```

### TASK-075 — Implement the hash-chained audit log

```yaml
id: TASK-075
title: Implement the hash-chained audit log
status: TODO
priority: HIGH
phase: 4
depends_on: [TASK-073]
spec: docs/SECURITY.md
touches: ["crates/aicontext-audit/**"]
acceptance:
  - Append-only JSONL where each record includes the digest of the previous record
  - Verifies the chain on read and refuses to append to a broken chain
  - Stores argument digests, never raw secret-bearing values
  - aicontext audit show renders a readable timeline and --json a stable shape
```

### TASK-076 — Implement the sandbox

```yaml
id: TASK-076
title: Implement the plugin sandbox
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-073]
spec: docs/SECURITY.md
touches: ["crates/aicontext-plugin-runtime/**"]
acceptance:
  - Per-plugin working directory and an explicit filesystem path allowlist
  - Declared network egress allowlist; denied by default
  - No ambient access to the host home directory or unrelated environment variables
  - Escape attempts fail with a typed, audited error
```

### TASK-077 — Implement aicontext plugin

```yaml
id: TASK-077
title: Implement aicontext plugin list, install, and remove
status: TODO
priority: HIGH
phase: 4
depends_on: [TASK-070, TASK-071]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-cli/**"]
acceptance:
  - install validates the manifest, prints the permission diff, and requires confirmation
  - A plugin requesting an unknown permission is refused
  - remove revokes permissions and deletes the local state it owned
  - A lockfile-style record of installed plugins and their granted permissions
```

### TASK-078 — Verify MCP compatibility

```yaml
id: TASK-078
title: Verify MCP interoperability
status: TODO
priority: MEDIUM
phase: 4
depends_on: [TASK-073, TASK-074]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-runtime/**", "docs/PLUGIN_SPEC.md"]
acceptance:
  - A third-party MCP server can be registered as a plugin with a permission declaration
  - Tool discovery, invocation, and error shapes match the MCP specification
  - Documented limitations are listed explicitly
```

---

## Phase 5 — GitHub plugin (first real plugin)

```yaml
id: TASK-080
title: Implement the GitHub read-only plugin
status: TODO
priority: HIGH
phase: 5
depends_on: [TASK-077]
spec: docs/PLUGIN_SPEC.md
acceptance:
  - Tools github.repo.read, github.issue.list, github.issue.read, github.pr.read,
    github.branch.list, github.commit.list, github.file.read
  - All read tools resolve to ALLOW only after explicit grant; none may write
  - Issue and PR bodies are returned as untrusted data, never as instructions
  - Rate-limit and pagination errors are typed and actionable
```

```yaml
id: TASK-081
title: Add GitHub write tools behind approval
status: TODO
priority: MEDIUM
phase: 5
depends_on: [TASK-080]
spec: docs/PLUGIN_SPEC.md
acceptance:
  - github.issue.create and github.pr.create default to APPROVAL
  - The prompt shows the exact rendered body that will be sent
```

---

## Phase 6 — AWS plugin (read-only first)

```yaml
id: TASK-090
title: Implement the AWS read-only plugin
status: TODO
priority: MEDIUM
phase: 6
depends_on: [TASK-081]
spec: docs/PLUGIN_SPEC.md
acceptance:
  - Tools for EC2 describe/list, S3 list/head, RDS describe, CloudWatch read, Cost Explorer read
  - Least-privilege credentials only; no credential value is stored or logged
  - Every response is untrusted data with a size cap
```

```yaml
id: TASK-091
title: Add guarded AWS write operations
status: TODO
priority: LOW
phase: 6
depends_on: [TASK-090]
spec: docs/SECURITY.md
acceptance:
  - Start and stop resolve to APPROVAL; terminate to EXPLICIT_APPROVAL per invocation
  - Every call is audited with the approval decision and the acting agent
  - A dry-run mode shows the exact API call without sending it
```

---

## Phase 7 — Security hardening

```yaml
id: TASK-100
title: Prove the prompt-injection containment
status: TODO
priority: CRITICAL
phase: 7
depends_on: [TASK-074]
spec: docs/SECURITY.md
acceptance:
  - A named test proves a hostile instruction in a tool result cannot alter policy
  - A named test proves retrieved content never reaches an instruction role
  - A named test proves no model or tool output is ever passed to a shell
```

```yaml
id: TASK-101
title: Add the cloned-repository policy review
status: TODO
priority: HIGH
phase: 7
depends_on: [TASK-014]
spec: docs/SECURITY.md
acceptance:
  - doctor renders a digest and summary of the .ai policy files found in an unfamiliar repository
  - A repository that attempts to weaken permissions is flagged before any write action
```

---

## Phase 8 — Dashboard (deferred)

Read-only local UI over the same crates: overview, PRD, architecture, tasks, specs, memory,
decisions, bugs, changes, workflows, agents, plugins, permissions, audit, settings. No new write
paths. Blocked until Phase 4 exit criteria are met.

---

## Phase 9 — Multi-agent (deferred)

Planner, Architect, Developer, Tester, Reviewer, DevOps, composed from `.ai/agents/*.md` profiles
and the Phase 4 tool gateway. Explicitly blocked until the single-agent loop is reliable in
production use. No new primitive will be invented for it; it composes existing seams.

---

## Phase 10 — Marketplace and cloud (deferred)

Plugin registry, team collaboration, organisations, hosted context, SSO, RBAC. Requires a stable
plugin API (v1) and evidence that the core works without any of it.

---

## Definition of done

A task is `DONE` only when all of the following are true:

```text
[ ] Every acceptance criterion is demonstrably met
[ ] Tests exist and pass, including a failing-before/passing-after test for bug fixes
[ ] cargo fmt --check, cargo clippy -- -D warnings, cargo test are green
[ ] No new dependency without a recorded justification
[ ] Public API and CLI changes are documented in the same change
[ ] An ADR exists if an architectural decision was made
[ ] The task status here is updated in the same change
```
