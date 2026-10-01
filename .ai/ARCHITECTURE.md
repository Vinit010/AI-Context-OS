---
id: ARCH-001
type: architecture
title: AI Context OS — System Architecture
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-09-27
---

# ARCHITECTURE

> Companion documents: `PRD.md` (what), `RULES.md` (constraints on how), `DESIGN.md` (presentation),
> `TASKS.md` (plan), `MEMORY.md` (learned constraints).
> Detailed contracts live in `docs/`: `CONTEXT_SPEC.md`, `PLUGIN_SPEC.md`, `AI_PROVIDER_SPEC.md`,
> `SECURITY.md`, `CLI_SPEC.md`.

---

## 1. Architecture drivers

Decisions in this document are driven by exactly these forces, in this order of precedence:

| # | Driver | Consequence |
|---|--------|-------------|
| D1 | **Developer authority** | Writes are gated, destructive actions are per-action approved, nothing autonomous by default |
| D2 | **No secrets in the repository** | Credentials live in the environment or OS keychain; `.ai/` holds references only |
| D3 | **Portability** | Knowledge is files. The tool is a reader/writer of those files, not their owner |
| D4 | **Determinism and explainability** | Retrieval is rule-based and inspectable; conflicts are reported, never silently resolved |
| D5 | **Provider neutrality** | No provider type appears in core interfaces |
| D6 | **Isolation of untrusted code** | Plugins are out-of-process, permission-declared, jailed, and audited |
| D7 | **Simplicity** | A modular monolith. No microservices, no message bus, no service split without a measured need |
| D8 | **Low dependency surface** | Every direct dependency carries a written justification |

Where D1–D3 conflict with convenience, D1–D3 win. That is the tie-breaker for every ambiguous
decision in this repository.

---

## 2. Technology stack

### 2.1 Language and runtime

**Rust, edition 2024, MSRV 1.85.** Single static binary per platform. No runtime prerequisite.

Why Rust over Go, TypeScript, and Python for this specific product:

| Criterion | Rust | Go | TypeScript | Python |
|-----------|------|----|------------|--------|
| Single static binary, zero runtime deps | Yes | Yes | No | No |
| Strong isolation primitives for untrusted plugin work | Yes (catch, `unsafe` audit, WASM host options) | Moderate | Weak | Weak |
| Cross-platform (Linux/macOS/Windows) first-class | Yes | Yes | Requires Node | Requires Python |
| Dependency-minimalism achievable | Yes | Good | Poor | Poor |
| Ecosystem for AI/provider HTTP clients | Adequate (reqwest) | Good | Best | Best |
| Cost of writing lots of code | Highest | Moderate | Lowest | Low |

The deciding factor is **D6 plus D8**. The product's security model depends on running
third-party code out-of-process with tight resource and permission control, and on not inheriting a
large transitive dependency tree. Rust delivers both; the higher implementation cost is accepted and
mitigated by keeping the crate count small and the interfaces boring.

### 2.2 Dependency policy

Direct dependencies are allowed only with a justification recorded here or in an ADR. The target
list for Phase 1–2:

| Crate | Purpose | Justification |
|-------|---------|---------------|
| `clap` (derive) | CLI parsing, completions | De-facto standard; hand-rolling is a bad use of effort |
| `serde`, `serde_json` | Wire + file formats | Ubiquitous; schema-first serialisation is required anyway |
| `yaml_serde` (0.10) | Front matter | Required by the storage contract, and it is the only maintained crate that is published by a steward rather than an individual. Pinned in `ADR-007`; reached only through the internal `YamlCodec` trait, so a swap is contained |
| `jsonschema` (Draft 2020-12) | Schema validation | Required by the machine-readable metadata requirement. Wrapped behind an internal trait so it can be replaced — **RISK R-2**. In `TASK-015` it is a **dev-dependency only**, with `default-features = false`, used by the CI meta-schema check; the runtime dependency decision is deferred to `TASK-014` |
| `thiserror` | Error types | Removes boilerplate; errors must be typed (RULES §5) |
| `anyhow` | CLI boundary only | Contextual error chaining at the top, nowhere else |
| `tokio` | Async runtime | Provider HTTP and plugin process supervision are I/O-bound |
| `reqwest` + `rustls` | HTTP | Provider calls and plugin transports. `rustls` avoids a system OpenSSL dependency |
| `keyring` | OS keychain | Credential storage without writing secrets to disk in our own format |
| `sha2` | Content hashing | Index invalidation, audit chain, argument digests. Widely audited |
| `ignore` / `walkdir` | File discovery | Respects `.gitignore`; avoids walking `target/` and `node_modules/` |
| `tracing`, `tracing-subscriber` | Diagnostics | Structured, level-controlled, stderr-only by default |

Dev-only: `tempfile`, `assert_cmd`, `predicates`, `insta` (snapshot tests for CLI output),
`proptest` (parsers and the permission resolver).

Explicitly rejected for the MVP: a vector database, an ORM, a web framework, a state-machine or
workflow-orchestration library, a CLI framework beyond `clap`, and any telemetry SDK.

### 2.3 Standards adopted rather than reinvented

- **JSON Schema Draft 2020-12** for all machine-readable metadata.
- **MCP** for the tool transport and tool-definition shape, so the tool layer interoperates with
  the existing agent ecosystem instead of forking it.
- **JSON-RPC 2.0** framing for the plugin protocol.
- **Semantic Versioning** for the CLI, the plugin API, and the schema set.
- **Git** for version control, collaboration, and audit anchoring.

---

## 3. Repository layout

```text
AI-Context-OS/
│
├── .ai/                          # Project knowledge — COMMITTED, dogfoods the product
│   ├── AI.md                     # Agent entry point + mandatory workflow
│   ├── PRD.md
│   ├── ARCHITECTURE.md
│   ├── RULES.md
│   ├── CONVENTIONS.md
│   ├── DESIGN.md
│   ├── TASKS.md
│   ├── MEMORY.md
│   ├── specs/                    # Feature specifications
│   ├── context/                  # Domain knowledge, terminology, environments
│   ├── tasks/                    # One file per task (TASK-NNN.md)
│   ├── decisions/                # ADRs
│   ├── bugs/                     # BUG-NNN.md
│   ├── changes/                  # CHG-NNN.md
│   ├── workflows/                # Recurring procedures
│   ├── agents/                   # Agent role profiles
│   ├── integrations/             # External system notes
│   ├── permissions/              # permissions.yaml
│   └── schemas/                  # JSON Schema, materialised by `aicontext init`
│
├── docs/                         # Normative specifications (this product's own design docs)
│   ├── CONTEXT_SPEC.md
│   ├── PLUGIN_SPEC.md
│   ├── AI_PROVIDER_SPEC.md
│   ├── SECURITY.md
│   └── CLI_SPEC.md
│
├── schemas/                      # Schema source of truth; `init` copies into .ai/schemas
├── templates/                    # `aicontext init` scaffolding templates
│   └── ai/…
│
├── crates/
│   ├── aicontext-core/           # Domain types, errors, IDs. No I/O framework, no CLI, no provider types
│   ├── aicontext-context/        # Discovery, parsing, index, retrieval, validation, health
│   ├── aicontext-git/            # Git porcelain wrapper, change detection
│   ├── aicontext-permissions/    # Policy model + resolver (pure, fully tested)
│   ├── aicontext-audit/          # Hash-chained append-only log
│   ├── aicontext-plugin-sdk/     # Manifest + protocol types for plugin authors
│   ├── aicontext-plugin-runtime/ # Process supervisor, jail, approval broker wiring
│   ├── aicontext-providers/      # AIProvider trait + adapters
│   ├── aicontext-cli/            # The `aicontext` binary
│   └── aicontext-testkit/        # Fixtures: temp repos, sample .ai trees, fake provider
│
├── Cargo.toml                    # Workspace
├── rust-toolchain.toml
├── README.md
├── CONTRIBUTING.md
├── LICENSE
└── .gitignore
```

### 3.1 State separation — committed vs local

This is a hard boundary, and `doctor` enforces it.

| Path | Contents | Git |
|------|----------|-----|
| `.ai/` | Knowledge intended for humans, agents, and teammates | **Committed** |
| `.aicontext/` | Audit log, index cache, local config, credential *references*, approval history, session state | **Ignored** |
| `~/.config/aicontext/` | User-global config, installed plugins, provider profiles | Outside the repo |

`.aicontext/` is added to `.gitignore` by `init`. If a secret-shaped file ever appears under
`.ai/`, `doctor` reports it as an error and `doctor --strict` exits non-zero.

### 3.2 Crate dependency rules

```text
                          aicontext-cli
                                │
        ┌───────────┬───────────┼────────────┬──────────────┐
        ▼           ▼           ▼            ▼              ▼
  aicontext-   aicontext-  aicontext-  aicontext-    aicontext-
  context      git         providers   plugin-runtime  audit
        │           │           │            │
        └───────────┴─────┬─────┴────────────┘
                          ▼
                   aicontext-core
                          ▲
                          │
                 aicontext-plugin-sdk   (depends on core only)
                 aicontext-permissions  (depends on core only)
                 aicontext-testkit     (dev only)
```

Hard rules, checked in CI:

1. Dependencies point **inward** toward `aicontext-core`. `core` depends on nothing internal.
2. `aicontext-core` must not depend on `tokio`, `clap`, `reqwest`, or any provider SDK.
3. `aicontext-context` must not depend on `aicontext-providers`. The context engine assembles a
   `ContextPacket`; it never calls a model.
4. `aicontext-permissions` is **pure**: no I/O, no clock, no randomness. Fully branch-covered tests.
5. `aicontext-plugin-sdk` must not depend on the runtime or the CLI. Plugin authors get the SDK,
   never our internals.
6. No crate may `use` another crate's private module. Public surfaces are `pub` and documented.
7. `aicontext-context` owns the front-matter value type and keeps the YAML library to itself. `Value`
   is `#[non_exhaustive]`, so only a parsed document can produce one, and a test scans `src/` and
   fails if `yaml_serde` appears in any module but `codec.rs`. See `ADR-007` and `MEM-012`.

---

## 4. Runtime architecture

### 4.1 Component map

```text
┌──────────────────────────────────────────────────────────────────────────┐
│                              aicontext CLI                               │
│  init · status · doctor · health · plan · task · memory · decision ·     │
│  bug · context · plugin · connect · agent · export · import · audit      │
└───────────────┬──────────────────────────────────────────────────────────┘
                │
      ┌─────────▼──────────┐
      │  Workspace / Repo  │  ProjectHandle: root, .ai paths, .aicontext paths
      └─────────┬──────────┘
                │
   ┌────────────▼────────────────────────────────────────────────────┐
   │                     Context Engine                              │
   │  discover → parse → index → plan → retrieve → prioritise →     │
   │  budget → assemble → annotate                                  │
   │  (ContextSpec, ContextParser, ContextIndex, Retriever,         │
   │   PriorityResolver, ContextBudget, ContextAssembler)           │
   └───────┬───────────────────────────────────┬────────────────────┘
           │ ContextPacket                    │ ContextDiagnostics
           ▼                                   ▼
   ┌───────────────┐                 ┌──────────────────┐
   │ Provider      │                 │  Validator /     │
   │ (AIProvider)  │                 │  Doctor          │
   └───────┬───────┘                 └──────────────────┘
           │ model output → typed Action Proposal
           ▼
   ┌───────────────────────────────────────────────────────────────┐
   │                      Tool Gateway                              │
   │  ActionProposal → PermissionResolver → ApprovalBroker →       │
   │  PluginRuntime(supervise) → AuditLog.append                    │
   └───────┬───────────────────────────────────────────────────────┘
           │ JSON-RPC 2.0 / stdio (MCP-compatible)
   ┌───────▼────────┐  ┌──────────────┐  ┌──────────────┐  ┌────────┐
   │ github plugin  │  │ aws plugin   │  │ local plugin  │  │  ...   │
   │ (subprocess)   │  │ (subprocess)  │  │ (subprocess)  │  │        │
   └────────────────┘  └──────────────┘  └──────────────┘  └────────┘
```

### 4.2 Connection map — who calls what

| Caller | Callees | Contract |
|--------|---------|----------|
| `cli` | every crate | Only crate that wires dependencies together |
| `context::ContextEngine` | `context::ContextSpec`, `Validator`, `core` | Returns `ContextPacket` / `ContextDiagnostics`; never I/O to models |
| `context::Retriever` impls | `ContextIndex` | `fn retrieve(&self, plan: &RetrievalPlan, index: &ContextIndex) -> Vec<ScoredDocument>` |
| `context::ContextAssembler` | `Retriever`, `PriorityResolver`, `ContextBudget` | `fn assemble(&self, packet) -> AssembledContext` with full provenance |
| `providers::ProviderRegistry` | `AIProvider` impls | `async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse>` |
| `cli` (agent loop) | `ContextEngine`, `ProviderRegistry`, `ToolGateway` | The agent loop lives in the CLI/application layer, not in `core` |
| `ToolGateway` | `permissions::Resolver`, `ApprovalBroker`, `plugin-runtime::Supervisor`, `audit::Log` | Single choke point; there is no second path to a plugin |
| `plugin-runtime::Supervisor` | plugin process, `aiplugin-sdk` types | Spawn, handshake, health-check, timeout, kill, jail |
| `git::Repository` | `git` CLI | Porcelain wrapper; no libgit2 in the MVP |
| `audit::Log` | filesystem | Append-only JSONL, hash-chained |

### 4.3 Primary flows

**Flow A — `aicontext init`**

```text
resolve CWD → detect VCS root → detect stack (advisory only)
  → render templates from templates/ai/ into .ai/
  → copy schemas/ into .ai/schemas/
  → add .aicontext/ to .gitignore (never overwrite an existing entry)
  → validate the result with the Validator
  → print a summary + suggested next commands
Never modifies files outside .ai/ and .gitignore.
```

**Flow B — `aicontext status` / `doctor`**

```text
ProjectHandle → ContextSpec → parse (cached by content hash)
  → git status/branch/diff summary
  → doctor: validate documents, resolve cross-references, run consistency checks
  → emit human or --json output
```

**Flow C — context assembly (`aicontext context --task TASK-001`)**

```text
task id → document graph (task → spec → adr → bugs → files)
  → mandatory set (AI.md, RULES.md) always included
  → lexical + filter retrieval over the index for the rest
  → priority ladder assignment
  → conflict detection across tiers
  → token budget: drop from the lowest tier down, never tiers 1–2
  → assemble ContextPacket { policy, retrieved, conflicts, budget, provenance }
  → render (human) or emit (--json) or hand to a provider
```

**Flow D — agent run (`aicontext agent run --task TASK-001 [--execute]`)** — opt-in, gated

```text
assemble ContextPacket → provider.complete()
  → model returns either text or a typed ActionProposal
  → ActionProposal is validated against the action schema
  → every proposed tool call → PermissionResolver → ApprovalBroker → Supervisor → Audit
  → plan mode (default): write plan to .aicontext/plans/, modify nothing
  → --execute mode: edits restricted to a declared scope allowlist, never commits, never pushes
  → final report per the AI.md output contract
```

**Flow E — a tool call**

```text
call{plugin, tool, resource, args}
  → schema-validate args against the plugin's declared input schema   (reject on failure)
  → resolve permission: DENY > EXPLICIT_APPROVAL > APPROVAL > ALLOW, default DENY
  → audit: append "requested" record
  → if approval required:
        interactive TTY → prompt with human-readable diff → approve/reject
        non-interactive   → DENY (exit 5) unless an explicit scoped grant exists
  → spawn/attach plugin process, enforce timeout + output cap + path jail
  → normalise response into a data envelope (untrusted)
  → audit: append "completed" record with result status and result digest
```

---

## 5. Data and schema model

### 5.1 Storage contract

> Rationale and alternatives: `decisions/ADR-003-knowledge-storage-contract.md`.

1. **Every `.ai` document MAY begin with a YAML front-matter block.** Front matter is the **only**
   machine-read source. The Markdown body is the human view.
2. **The core never parses Markdown structure** — no heading scraping, no table scraping, no
   regex over prose. If a value must be machine-read, it goes in front matter.
3. Front matter is validated against a JSON Schema named by the document's location.
4. A document with invalid or missing required front matter is a **validation error**, not a
   best-effort parse. `doctor` reports it; strict mode fails.
5. Generated files (indexes, tables of contents) are marked with a `generated: true` front-matter
   key and a "do not edit" header. Editing them is a no-op that `doctor` reports.

Common front-matter keys: `id`, `type`, `title`, `status`, `version`, `created`, `updated`,
`tags`. Type-specific keys are declared in `docs/CONTEXT_SPEC.md`.

### 5.2 Entity model

```text
Project 1──n Document
Document subtypes (each with its own schema):
  Spec        requirements, business rules, api, data model, acceptance criteria
  Task        status, priority, phase, dependencies, acceptance criteria, links
  Decision    status (proposed|accepted|deprecated|superseded), context, decision, consequences
  Bug         severity, symptoms, root cause, solution, files changed, prevention
  Change      date, feature, reason, files, architecture impact, breaking, migration
  Memory      category (constraint|lesson|fact|preference|limitation), scope, confidence
  Agent       role, responsibilities, allowed, forbidden, required context, output format
  Workflow    trigger, steps, gates
Schema       JSON Schema definitions
PermissionPolicy  per-tool modes
PluginManifest    plugin API version, entry point, permissions, auth
```

Relationships are expressed as front-matter ID lists and resolved by `doctor`:

```text
Task ──requires──▶ Spec
Task ──blocked_by▶ Task
Task ──informs───▶ Decision
Task ──relates_to▶ Bug
Task ──touches───▶ CodePath (glob)
Decision ──supersedes──▶ Decision
Bug ──prevented_by──▶ Change | Memory
Change ──impacts──▶ Spec | Decision
```

### 5.3 Storage scale policy

`TASKS.md`, `bugs/`, `decisions/`, and `changes/` each have two modes:

- **Inline mode (default until the file exceeds 300 entities or 2,000 lines):** all entities live in
  the index file as front-matter-delimited blocks.
- **Split mode:** the index file is generated (a table of links) and each entity moves to
  `tasks/TASK-NNN.md`, `bugs/BUG-NNN.md`, `decisions/ADR-NNN-slug.md`, `changes/CHG-NNN.md`.

Both modes read identically through the API. `doctor` recommends the switch; it never performs it
silently. The `id` is the stable identity; the file path is not.

---

## 6. Context engine architecture

Full contract in `docs/CONTEXT_SPEC.md`. Structure:

```text
ContextEngine
├── ContextSpec        static description of the .ai layout
├── ContextParser      front matter → typed Document
├── ContextIndex       content-hash keyed, in-memory, persisted to .aicontext/index.json
├── Discovery          stack/CI/container detection → advisory ProjectProfile
├── RetrievalPlan      derived from task, flags, and explicit queries
├── Retriever (trait)
│   ├── AlwaysRetriever        mandatory documents (AI.md, RULES.md)
│   ├── LinkedRetriever        graph edges from the current task
│   ├── FilterRetriever        front-matter predicates
│   ├── GlobRetriever          path patterns
│   └── LexicalRetriever       BM25 over headings + body, field boosts
├── PriorityResolver    fixed ladder + conflict detection
├── ContextBudget       per-tier token caps, degradation order
└── ContextAssembler    produces ContextPacket with provenance
```

**Retrieval is deterministic and lexical in the MVP** (`ADR-004`). No embeddings, no network, no
opaque ranking. Every included document carries `{tier, score, reason}`; every exclusion is
countable and reportable via `aicontext context --explain`. A future `EmbeddingRetriever` is a new
implementation of the same trait and cannot change the packet format.

**Priority ladder** (fixed, and the tie-breaker for conflicts):

```text
1  current user instruction
2  approved project rules (RULES.md, CONVENTIONS.md)
3  architecture
4  specification
5  current task
6  approved decisions (ADRs)
7  relevant memory
8  relevant source code
9  historical information (changes, superseded ADRs, git log)
```

**Conflict rule:** when a lower tier asserts a value that a higher tier contradicts, the higher
tier wins, the conflict is recorded in the packet, and `doctor` reports it as a warning. The system
never rewrites the higher-tier document to resolve a conflict.

**Injection containment:** retrieved content is wrapped in a data envelope carrying `{source, path,
tier, trust: "untrusted"}` and is placed in a data role, never an instruction role. No retrieved
text can raise its own priority, and no retrieved text is ever passed to a shell.

---

## 7. Plugin architecture

Full contract in `docs/PLUGIN_SPEC.md`. Summary of the decisions (`ADR-002`):

- **Execution model: out-of-process.** A plugin is an executable that speaks JSON-RPC 2.0 over
  stdio. The tool contract is `provider.service.action` (e.g. `github.issue.create`,
  `aws.ec2.stop`).
- **Why not in-process:** a native in-process ABI would couple every plugin to the Rust version,
  the crate graph, and the host's memory. It would make a malicious plugin a compromise of the
  host. Out-of-process isolation, resource limits, and a versioned wire protocol are worth the
  process-management cost.
- **MCP compatibility is the point.** The runtime speaks MCP-compatible tool definitions and
  transports so existing MCP servers and clients interoperate. A plugin is, in effect, a
  capability-scoped MCP server with a declared permission set.
- **Permissions are declared, granted, and enforced by the host.** A plugin cannot widen its own
  permissions. Requests for unmapped capabilities fail.
- **Jail:** per-plugin working directory, an explicit read/write path allowlist, network egress
  allowlist, output size caps, and hard timeouts. No ambient `HOME` secrets are exposed.
- **SDK:** `aicontext-plugin-sdk` provides manifest types, protocol types, and a small server
  helper. It is a convenience, never a requirement — the wire protocol is the contract.

---

## 8. Security architecture

Full model in `docs/SECURITY.md`. The load-bearing ideas:

**Trust zones**

```text
Z0  Platform     compiled, signed, trusted
Z1  Local config  .aicontext/ + ~/.config/aicontext — trusted, developer-owned, never committed
Z2  Project policy .ai/ — trusted AS POLICY, but untrusted AS CODE until a human reviews it
Z3  Developer input  the instruction being acted on — highest authority after Z0
Z4  External data   tool results, web, issues, PR text, commit messages, source comments — DATA
```

Invariants:

- **I1** Z4 never becomes an instruction. No amount of text in a tool result can change policy.
- **I2** Every mutation of the workspace or an external system passes the Tool Gateway.
- **I3** Default deny. An unmapped capability is denied, not allowed.
- **I4** Destructive actions require `EXPLICIT_APPROVAL` for that specific action, at that moment.
- **I5** No secret is ever written to `.ai/`, stdout, the audit log, or an error message.
- **I6** The audit chain is hash-linked; a broken chain is reported and blocks further appends.
- **I7** Nothing is executed as a shell command. There is no shell tool in the MVP.
- **I8** A cloned repository's `.ai/RULES.md` is displayed for review by `doctor` before it is
  treated as binding policy for write actions.

---

## 9. Error model and exit codes

- One error enum per crate, in that crate. `aicontext-core::AicontextError` is the common
  denominator; each crate extends it with a typed variant carrying a code, message, cause chain,
  and remediation hint.
- The CLI is the only place allowed to convert errors into human output and exit codes.
- Exit codes are a stable contract (`docs/CLI_SPEC.md` §5). Adding a code is a minor change;
  changing an existing code is a breaking change.

---

## 10. Distribution

- `cargo install --path crates/aicontext-cli`, plus prebuilt binaries per platform.
- No installer script, no daemon, no background process, no telemetry.
- `aicontext` never writes outside the project root or the user config dir, and says so when it
  needs to.

---

## 11. Testing strategy

| Layer | Approach |
|-------|----------|
| Pure logic (permissions, priorities, conflict rules, budget) | Exhaustive unit tests, including property tests over the permission matrix |
| Parsers and index | Round-trip tests over fixture `.ai` trees, including malformed input |
| CLI | Integration tests via `assert_cmd`, plus output snapshots |
| Providers | Contract tests run against a local fake server implementing the OpenAI-compatible shape |
| Plugin runtime | Integration tests with a deliberately misbehaving fixture plugin: hangs, floods output, exits early, writes outside its jail |
| Security regressions | Named tests for each invariant I1–I8; they must fail if the invariant is broken |
| Git | Tests against real temporary repositories created by the testkit |

Enforcement: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, and a dependency
audit in CI. Rust is single-crate-unit by nature, which makes test placement non-negotiable — the
module tree mirrors the crate tree so tests live next to what they test.

---

## 12. Extensibility seams reserved now

Present in the MVP, unused until later phases, so no later phase requires a breaking change:

- `Retriever` trait — embeddings, call-graph, and language-specific retrievers later.
- `AIProvider` trait + `ProviderRegistry` — routing, caching, and cost policy later.
- `PluginManifest.plugin_api_version` + capability negotiation — marketplace and third-party
  ecosystem later.
- `ContextBudget` — per-model cost control later.
- `ApprovalBroker` trait — web dashboard approval, CI approval, and team workflows later.
- `audit::Log` as a trait-backed sink — remote/OTel export later, opt-in only.
- `AgentProfile` documents — multi-agent orchestration later.

Not reserved: no `dashboard/` crate, no `cloud/` crate, no plugin-authoring framework scaffolding.
Empty abstractions are worse than absent ones (RULES §2).

---

## 13. Risks and technical unknowns

| # | Risk | Impact | Likelihood | Mitigation | Owner task |
|---|------|--------|------------|------------|-----------|
| R-1 | **YAML crate maintenance.** The mainstream Rust YAML crate has had churn in maintenance; a bad choice forces a late swap | Parser rewrite across the storage contract | Low | **Mitigated in `TASK-016`**: `yaml_serde` 0.10 chosen in `ADR-007` and reached only through the `YamlCodec` trait, which is now implemented rather than merely planned. No YAML type appears in a public signature, so a swap touches one module. Residual risk is an incompatible 1.0 release; Cargo package renaming is the escape | TASK-016 |
| R-2 | **JSON Schema validator weight.** A Draft 2020-12 validator pulls a large transitive tree, against D8 | Build time, binary size, supply-chain surface | Medium | **Answered in `TASK-015`, by splitting the two claims**: CI validates the schemas with `jsonschema` 0.58.3 as a dev-dependency, which costs build time and nothing else — no shipped binary grows. The runtime `Validator` trait, and therefore the production dependency decision, is deferred to `TASK-014`, where a validator is first needed to validate a document rather than a schema. `default-features = false` is what keeps the network and TLS tree (`resolve-http`, `resolve-file`, `reqwest`, `rustls`, `aws-lc-rs`) out of the workspace entirely; every schema is self-contained, so none of those resolvers has anything to do. Residual risk is the runtime one: if `TASK-014` finds the tree too heavy behind the trait, the schemas themselves are unaffected | TASK-015 (CI), TASK-014 (runtime) |
| R-3 | **Plugin sandbox strength is OS-dependent.** Path and network enforcement differs across Linux, macOS, and Windows | Weaker isolation than documented on some platforms | High | Declare the guarantee per platform; fail closed where it cannot be enforced; document the gap rather than imply parity | TASK-076 |
| R-4 | **Context relevance without semantics.** Lexical retrieval will miss paraphrased intent | Agents re-read too much or miss the right document | Medium | Accept it in the MVP and measure: log which documents were retrieved vs. used, then let evidence pick the next retriever | TASK-033, TASK-038 |
| R-5 | **Audit tamper-resistance is local.** A local hash chain resists accidental edit, not an attacker with write access to the machine | Weaker audit guarantee than "tamper-resistant" implies | High | State the guarantee precisely; offer optional anchoring to git; never overclaim | TASK-075 |
| R-6 | **The `.ai/` policy trust problem.** A cloned repo's `RULES.md` is attacker-controlled text shaped like policy | Agent may obey hostile rules | High | `MEM-003`; digest review in `doctor`; platform policy always outranks project policy | TASK-101 |
| R-7 | **Static binary supply chain.** Rust's build pulls many crates | Compromised build | Low | `cargo audit` in CI, minimal and justified dependency set, lockfile committed, release artefacts checksummed | TASK-010 |
| R-8 | **Approval fatigue.** Too many prompts train a developer to hit "yes" | The approval control degrades into theatre | High | Default most tools to read-only `ALLOW`; prompts only where the consequence is real; group related calls; measure prompt frequency | TASK-072 |
| R-9 | **Scope creep from the roadmap.** Phases 5–10 pull attention from the core | The MVP never ships | High | Phase gates in `TASKS.md`; Phase 9 is explicitly blocked on single-agent reliability | all |
| R-10 | **Rust velocity.** The core is a lot of code to write for one developer | Slower to first value | Medium | Small crate count, boring interfaces, fixtures before features, no premature abstraction | TASK-010 |
| R-11 | **Windows parity.** Path handling, git invocation, and process kill semantics differ | Broken experience on a supported platform | Medium | Test on Windows in CI from the first commit; no `unix:` assumptions | TASK-010, TASK-017 |

**Unknowns that need a spike, not a guess**

- U-1: Whether a single `ContextPacket` can carry a large enough codebase slice for real work
  before the budget forces a hierarchical summarise-then-expand loop. Resolve with a real
  repository during Phase 2, not a synthetic one.
- U-2: Whether MCP's tool-definition shape can carry our permission model cleanly, or whether the
  permission layer must sit strictly outside the protocol. Resolve in TASK-078 before Phase 5.
- U-3: Whether the dashboard (Phase 8) can be served entirely from the same crates with no
  additional write paths. Defer until Phase 4 is proven; a "no" here changes the roadmap.
