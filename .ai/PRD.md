---
id: PRD-001
type: prd
title: AI Context OS — Product Requirements
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-09-27
---

# PRD — AI Context OS

## 1. Product overview

**AI Context OS** is a Git-native context, memory, governance, and tool-integration layer for
software development.

It gives AI coding agents a persistent, structured, reviewable understanding of a project —
what the product is, how it is built, what the rules are, what is being built right now, what was
decided, and what has already gone wrong — and it gives the developer a control plane over what
those agents are allowed to do.

Three promises:

1. **AI models change. Project knowledge persists.** The knowledge lives in the repository, in
   plain Markdown, not inside a vendor's chat history.
2. **Code changes often. Decisions stay traceable.** Every architectural choice and every fixed
   bug is a versioned artifact with a recorded reason.
3. **AI assists; the developer decides.** Dangerous actions require explicit human approval, and
   every consequential action is auditable.

The deliverable is a local-first CLI named `aicontext`, written in Rust, shipping a single static
binary.

---

## 2. Problem statement

AI coding assistants are individually capable and collectively unreliable, because they are
**stateless with respect to the project**:

| Problem | Consequence for the developer |
|---------|------------------------------|
| Context is lost between sessions | The same project is re-explained daily |
| Solved mistakes are re-made | "We already decided against that" is said weekly |
| No durable architectural memory | Inconsistent structures, parallel competing patterns |
| Unjustified dependency growth | Every session adds a library |
| Duplicate components are created | The codebase fragments |
| Rationale is lost | Nobody knows why a decision was made |
| Provider-specific context | Switching vendor loses all project knowledge |
| Business rules live only in people's heads | AI confidently breaks domain logic |
| Scope creep | Unrelated files get modified |
| Cloud access is unbounded | A mistaken action can affect production |
| Knowledge is scattered | Docs, issues, chat, and code disagree |

The root cause is not model quality. It is that **project knowledge has no home, no format, no
version control, and no access control**.

---

## 3. Target users

| User | Need |
|------|------|
| **Primary — solo developer / small team** using Cursor, Copilot, Claude Code, or a local model | Persistent project memory, fewer repeated explanations, consistent output |
| **Secondary — tech lead / architect** | Traceable decisions, enforced boundaries, reviewable AI output |
| **Secondary — platform/DevOps engineer** | Governed, audited AI access to cloud and SaaS systems |
| **Tertiary — AI tool builder** | A stable plugin/tool contract to expose external systems to agents |

Explicitly **not** a target in the MVP: enterprise compliance buyers, non-developers, and
organisations seeking a full project-management replacement.

---

## 4. Goals

**G1 — Durable project context.** Any agent, in any session, can reconstruct what the project is,
how it is structured, and what is currently being worked on, from files in the repository.

**G2 — Deterministic context assembly.** The system decides *which* context is relevant using
rules that are inspectable and reproducible, not opaque scoring. Every assembled context carries
its own provenance.

**G3 — Developer control.** Read operations are cheap and mostly automatic. Write operations are
permission-gated. Destructive operations require explicit per-action approval. Nothing is
autonomous by default.

**G4 — Provider independence.** The context engine, the CLI, and the tool gateway must not
contain provider-specific logic. A new model provider is an adapter, not a rewrite.

**G5 — Portability.** The project context is plain files. Deleting the tool leaves the knowledge
intact. No proprietary database, no required cloud account, no lock-in.

**G6 — Extensibility without fragility.** Plugins are versioned, permission-declared, and
isolated. A third party can add a tool without the core changing shape.

**G7 — Security as a first-class feature.** No secrets in the repository, default-deny
permissions, tamper-evident audit, and hard defence against prompt injection from external data.

---

## 5. Core features

### F1 — Structured project knowledge (`.ai/`)
A versioned directory holding `AI.md`, `PRD.md`, `ARCHITECTURE.md`, `RULES.md`,
`CONVENTIONS.md`, `DESIGN.md`, `TASKS.md`, `MEMORY.md`, plus `specs/`, `context/`, `tasks/`,
`decisions/`, `bugs/`, `changes/`, `workflows/`, `agents/`, `integrations/`, `permissions/`,
`schemas/`.

### F2 — Task management
Phases, epics, tasks, subtasks, dependencies, priority, status, acceptance criteria, and links to
specifications, code, bugs, and decisions. Machine-readable via front matter, validated by schema.

### F3 — Context engine
Project discovery, document parsing, a deterministic index, relevance-based retrieval, a fixed
priority ladder, conflict detection, and a token budget with graceful degradation. Every result is
explainable: which documents, why, with what score.

### F4 — Context validation and health
`aicontext doctor` finds missing, malformed, stale, and contradictory knowledge. `health` reports
coverage metrics. Diagnostics are transparent and specific; there are no arbitrary quality scores.

### F5 — AI provider abstraction
One `AIProvider` interface; adapters for OpenAI-compatible endpoints (covering OpenAI, Ollama,
llama.cpp, vLLM, and gateways), Anthropic, and Google Gemini. The engine never branches on provider.

### F6 — Plugin and tool system
A versioned manifest, a standardised tool contract (`provider.service.action`), an out-of-process
runtime, and MCP compatibility so the tool layer interoperates with the existing ecosystem rather
than replacing it.

### F7 — Permission and approval system
`ALLOW` / `DENY` / `APPROVAL` / `EXPLICIT_APPROVAL` modes, resolved default-deny, with a human
approval broker for TTY sessions and safe denial for non-interactive ones.

### F8 — Audit log
Append-only, hash-chained records of tool calls, approvals, and outcomes — timestamp, actor,
provider, agent, plugin, tool, resource, arguments digest, approval, result, status.

### F9 — Git integration
Branch, status, changed files, recent commits, and diff awareness, so context and code changes
stay correlated. Git is the collaboration and rollback substrate.

### F10 — Export / import / portability
Round-trip the full context. Files remain the source of truth. No export may be lossy in a way
that requires the tool to read it back.

---

## 6. User stories

- **US-1** As a developer, I run `aicontext init` in a new repository and get a complete, valid
  context skeleton with detected stack hints, without the tool rewriting any of my code.
- **US-2** As a developer, I ask an agent to fix a bug; the agent reads the relevant spec, task,
  ADR, and prior bug reports — not the entire repository — and cites what it used.
- **US-3** As a developer, I see `aicontext status` and immediately know the branch, the current
  phase, the current task, and what is modified.
- **US-4** As a developer, I run `aicontext doctor` and learn that ARCHITECTURE.md names a
  database the project does not use, and that a task points at a missing spec.
- **US-5** As a developer, I connect the GitHub plugin, and the agent can read issues and PRs
  after I approve the declared permissions.
- **US-6** As a developer, the agent asks to stop an EC2 instance; I see the exact action, target,
  and reason, and I approve or reject it. Nothing happens silently.
- **US-7** As a developer, I read a past bug report and understand why a mistake was made, so it is
  not repeated.
- **US-8** As a developer, I `git clone` a project onto a new machine and the agent behaves
  consistently, because the rules travelled with the code.
- **US-9** As a reviewer, I read a hash-chained audit log and can verify that no tool call
  happened outside approval policy.
- **US-10** As a developer, I delete the tool and keep the `.ai/` directory. Nothing is lost.

---

## 7. Functional requirements

**Context**
- FR-1 Discover a project by walking up from the CWD to the nearest `.ai/` or VCS root.
- FR-2 Detect languages, frameworks, package managers, test/CI/container/cloud configuration —
  as *hints*, never as auto-applied truth.
- FR-3 Parse `.ai` documents into typed records using YAML front matter validated against
  `.ai/schemas/*.schema.json`.
- FR-4 Build a content-hash-keyed index; rebuild only what changed.
- FR-5 Retrieve by explicit task links, front-matter filters, path globs, and lexical relevance.
- FR-6 Assemble context on the fixed priority ladder with per-tier token caps.
- FR-7 Emit provenance for every included and every excluded document.
- FR-8 Detect contradictions between higher and lower tiers and report them without silently
  rewriting the higher tier.

**Tasks**
- FR-9 Support the status set `BACKLOG, TODO, IN_PROGRESS, BLOCKED, IN_REVIEW, TESTING, DONE,
  CANCELLED`, with legal-transition validation.
- FR-10 Enforce that an `IN_PROGRESS` task has acceptance criteria and a linked specification or
  an explicit waiver.

**Tools**
- FR-11 Load plugins from a manifest declaring name, version, API version, entry point,
  permissions, and authentication requirements.
- FR-12 Reject a plugin whose manifest fails schema validation or requests permissions not already
  granted — the install diff must be shown and confirmed.
- FR-13 Route every tool call through the permission resolver, then the approval broker, then the
  runtime, then the audit log. No bypass path may exist.
- FR-14 Enforce timeouts, output size caps, and a filesystem jail per plugin.

**Providers**
- FR-15 Expose a single `AIProvider` interface; adding a provider must not touch the context
  engine, CLI, or plugin runtime.
- FR-16 Read credentials from environment or the OS keychain only; never from `.ai/`.
- FR-17 Degrade predictably when a provider is unavailable: report, do not retry silently.

**CLI**
- FR-18 Every command supports `--json` for automation and has documented, stable exit codes.
- FR-19 All commands operate offline except those that explicitly require network.

**Audit**
- FR-20 Log every state-changing tool call unconditionally, with argument digests rather than
  secrets. Log read calls at debug level.
- FR-21 Detect chain tampering on read and refuse to append to a broken chain.

---

## 8. Non-functional requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR-1 | `aicontext init` on a 10k-file repository | < 3 s |
| NFR-2 | `aicontext status` / `doctor` warm | < 300 ms |
| NFR-3 | `aicontext context` assembly, warm index | < 1 s |
| NFR-4 | Cold index of 5,000 context documents | < 10 s, incremental thereafter |
| NFR-5 | Single static binary, no runtime prerequisite | per platform |
| NFR-6 | Core commands function with no network and no account | mandatory |
| NFR-7 | Cross-platform: Linux, macOS, Windows | mandatory for MVP |
| NFR-8 | Test coverage on core crates | ≥ 80 % lines, 100 % of permission-resolver branches |
| NFR-9 | Secrets never written to `.ai/`, stdout, audit log, or error text | enforced by test + review |
| NFR-10 | Every command's behaviour is scriptable and documented in `docs/CLI_SPEC.md` | mandatory |
| NFR-11 | Plugin API version changes are additive within a major version | mandatory |
| NFR-12 | No telemetry by default; opt-in, aggregate-only, documented | mandatory |

---

## 9. Constraints

- **C-1 Local-first.** No account, no mandatory cloud service, no mandatory network.
- **C-2 Git-native.** Context is files in the repository and is diffable, reviewable, branchable.
- **C-3 Provider-agnostic core.** No provider SDK types in core interfaces.
- **C-4 Developer authority.** Destructive actions are never automatic.
- **C-5 No secrets in the repository**, ever.
- **C-6 Modularity over cleverness.** A modular monolith now; services only when a measured
  constraint demands one.
- **C-7 Minimal dependencies.** Every direct dependency needs a recorded justification.
- **C-8 Portability.** `.ai/` remains valid and readable without this tool.

---

## 10. Non-goals (MVP)

- Custom or fine-tuned models.
- A large plugin marketplace or public registry.
- Multi-agent orchestration (explicitly deferred until the single-agent loop is reliable).
- Autonomous production deployment or autonomous destructive cloud operations.
- A vector database as a prerequisite for retrieval.
- Enterprise SSO, SCIM, or a full RBAC service.
- A hosted/SaaS offering.
- Replacing Jira/Linear, or a general project-management tool.
- A mobile application.
- Autonomous code execution with no human checkpoint.

---

## 11. Success criteria for the MVP

The MVP succeeds when a developer, unaided, can:

1. Create a normal Git repository.
2. Run `aicontext init` and receive a valid, schema-checked context skeleton.
3. Define the product, the architecture, and the rules.
4. Create a task with acceptance criteria.
5. Connect one AI provider.
6. Ask the AI to implement the task.
7. Observe that the AI used **targeted** context and can show what it used.
8. Observe that the AI inspected existing code before writing new code.
9. Observe a plan before implementation.
10. Have the AI implement only that scope.
11. Have the AI run tests and report real results.
12. Have the AI report changed files.
13. Have the AI update `TASKS.md`.
14. Have the AI record durable memory.
15. Review the changes, and have Git hold the complete history — context included.

Measured outcome: **the second session on a project requires no re-explanation of context that was
already written down.** That is the whole thesis, and it is falsifiable.

---

## 12. Roadmap

| Phase | Outcome | Gate to proceed |
|-------|---------|-----------------|
| 0 — Specification | This document set | Internal consistency review |
| 1 — Context MVP | `init`, `status`, `doctor`; `.ai/` skeleton | §11 steps 1–4 |
| 2 — Context engine | Discovery, parse, retrieve, prioritise, validate, health | §11 steps 6–7 |
| 3 — Provider layer | Interface + OpenAI-compatible adapter first | §11 step 5 |
| 4 — Plugin system | Manifest, SDK, tool contract, permissions, approval | §11 step 6 |
| 5 — GitHub plugin | Repos, issues, PRs, branches, commits, files | Read path proven |
| 6 — AWS plugin | Read-only EC2/S3/RDS/CloudWatch/Cost first | Audit + approval proven |
| 7 — Security hardening | Enforcement, audit, credentials, injection defence, sandbox | Phase 6 review |
| 8 — Dashboard | Read-oriented local UI | Core CLI proven |
| 9 — Multi-agent | Planner/Architect/Developer/Tester/Reviewer/DevOps | Single-agent loop stable |
| 10 — Marketplace & cloud | Registry, teams, organisations, SSO, RBAC | Core stable and stable API |

See `TASKS.md` for the task-level breakdown.
