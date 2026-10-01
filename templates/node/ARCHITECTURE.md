---
id: ARCH-001
type: architecture
title: "{{project_name}} — System Architecture"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# ARCHITECTURE

TypeScript-on-Node starter architecture. A modular monolith until a measured need says otherwise.

---

## 1. Architecture drivers

| # | Driver | Consequence |
|---|--------|-------------|
| D1 | **One deployable unit** | Services are libraries first; a split needs a named owner and a migration date |
| D2 | **Explicit I/O at the edge** | Business logic is pure and testable; I/O lives in adapters |
| D3 | **Typed boundaries** | Every boundary validates at runtime, not only at compile time |
| D4 | **Predictable operations** | No hidden network calls inside a domain function |

Where these conflict, the higher one wins.

## 2. Technology stack

**TypeScript on Node, ESM.** Record the Node major version here and pin it in the engine field; an
unstated runtime version is a latent outage.

### 2.1 Dependency policy

Direct dependencies are allowed only with a written justification in this table or in an ADR. "It is
popular" is not a justification. No dependency may require a native toolchain, a system library, or a
network install at runtime, and none may phone home or read the home directory implicitly.

| Dependency | Purpose | Justification |
|------------|---------|---------------|
| *name* | *what it is for* | *why hand-writing it would be worse* |

Dev-only: *anything used only by tests.*

## 3. Repository layout

```text
.
├── src/
│   ├── <domain>/
│   ├── adapters/            # I/O: http, db, filesystem, queues
│   ├── entrypoint/
│   └── index.ts
├── tests/
└── .ai/                     # project knowledge
```

### 3.1 Module dependency rules

```text
entrypoint ──► adapters ──► domain ──► (nothing)
                  └────────► shared primitives
```

1. Dependencies point **inward**. A domain module imports no adapter, framework, or transport.
2. A domain module performs no I/O. If it needs data, it takes it as an argument.
3. `shared/` holds primitives only — result, money, clock, id. It holds nothing domain-specific.
4. Cross-domain imports are explicit and reviewed; there is no barrel that hides them.

## 4. Runtime architecture

### 4.1 Component map

```text
*the components, and the direction of every call between them*
```

### 4.2 Primary flows

**Flow A — *the first thing a user does***

```text
*the steps in order, including what it refuses to do*
```

## 5. Data and schema model

*What is stored, where, in what shape, and which side is derived and rebuildable. State which
boundary validates what.*

## 6. Error model

- One error type per failure domain, carrying a stable code, a message, a cause, and a remediation
  hint.
- Errors convert into a common shape at the boundary; `Box<dyn Error>` never appears in a public
  signature.
- The top level is the only place that turns an error into a response or a log line.

## 7. Testing strategy

| Layer | Approach |
|-------|----------|
| Domain logic | Pure unit tests, no I/O, no clock, no mocks |
| Adapters | Against real temporary infrastructure, never an in-memory fake of it |
| Boundaries | Contract tests for every validation entry point |
| End to end | The few user journeys the product promises |

`lint`, `typecheck`, and `test` all gate every change, and `typecheck` is treated as a test.

## 8. Risks and unknowns

| # | Risk | Impact | Likelihood | Mitigation | Owner |
|---|------|--------|------------|------------|-------|
| R-1 | *what could go wrong* | *worst case* | *how likely* | *what you will do* | *who* |

**Unknowns that need a spike, not a guess**

- U-1: *the question, and how you will answer it with evidence*