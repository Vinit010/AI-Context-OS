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

Python starter architecture. A modular monolith with I/O at the edge until a measured need says
otherwise.

---

## 1. Architecture drivers

| # | Driver | Consequence |
|---|--------|-------------|
| D1 | **Testable without infrastructure** | Business logic is pure; I/O lives in adapters |
| D2 | **Explicit dependencies** | Dependency injection is manual and obvious, not magical |
| D3 | **Typed contracts** | Every boundary validated at runtime and typed statically |
| D4 | **Predictable operations** | No hidden network calls inside a domain function |

Where these conflict, the higher one wins.

## 2. Technology stack

**Python.** Record the minimum supported version here and in `pyproject.toml`; an unstated runtime
version is a latent outage.

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
│   ├── <domain>/           # model.py, usecases.py, adapters.py, __init__.py
│   ├── app/                # assembly: wiring, DI, entrypoints
│   └── cli.py              # top-level CLI entry if applicable
├── tests/
├── pyproject.toml
└── .ai/                    # project knowledge
```

### 3.1 Module dependency rules

```text
cli ──► app (assembly) ──► domain ──► (nothing but stdlib/shared)
```

1. Dependencies point **inward**. A domain module imports no adapter, framework, or transport.
2. A domain module performs no I/O. If it needs data, it takes it as an argument.
3. `app/` holds wiring only. No business logic lives there.
4. Cross-domain imports are explicit and reviewed; there is no wildcard re-export.

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

- One exception hierarchy per failure domain: a base `DomainError` with a stable `code`, a message
  naming the concrete subject, a `from` cause chain, and a remediation hint.
- The CLI layer converts exceptions to output and exit codes. Exceptions never escape as raw
  tracebacks to the user for known failures.

## 7. Testing strategy

| Layer | Approach |
|-------|----------|
| Domain logic | Pure unit tests, no I/O, no clock, no mocks |
| Adapters | Against real temporary infrastructure, never an in-memory fake of it |
| Boundaries | Contract tests for every validation entry point |
| End to end | The few user journeys the product promises |

`pytest`, `ruff check`, and `ruff format --check` all gate every change. A failing type check is a
failing build.

## 8. Risks and unknowns

| # | Risk | Impact | Likelihood | Mitigation | Owner |
|---|------|--------|------------|------------|-------|
| R-1 | *what could go wrong* | *worst case* | *how likely* | *what you will do* | *who* |

**Unknowns that need a spike, not a guess**

- U-1: *the question, and how you will answer it with evidence*