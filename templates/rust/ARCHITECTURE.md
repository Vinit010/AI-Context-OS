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

Rust starter architecture. A modular monolith unless a measured need says otherwise; the dependency
graph is a DAG pointing inward.

---

## 1. Architecture drivers

| # | Driver | Consequence |
|---|--------|-------------|
| D1 | **Correctness under failure** | Errors are typed; partial failure is reported, never swallowed |
| D2 | **No runtime surprises** | A static binary with no runtime prerequisite |
| D3 | **Portability** | Linux, macOS, and Windows are first-class; no `unix:` assumptions |
| D4 | **Low dependency surface** | Every direct dependency carries a written justification |

Where these conflict, the higher one wins.

## 2. Technology stack

**Rust, edition 2024, MSRV 1.85.** One static binary per platform, no runtime prerequisite. Pin the
toolchain in `rust-toolchain.toml` and declare the compatibility floor as `rust-version` in the
workspace manifest, then prove the floor in CI rather than asserting it.

### 2.1 Dependency policy

Direct dependencies are allowed only with a written justification in this table or in an ADR. "It is
popular" is not a justification. No dependency may require a C compiler, a system library, or a
network install at runtime, and none may phone home or read the home directory implicitly.

| Dependency | Purpose | Justification |
|------------|---------|---------------|
| *name* | *what it is for* | *why hand-writing it would be worse* |

Dev-only: *anything used only by tests.*

## 3. Repository layout

```text
.
├── Cargo.toml                 # workspace
├── rust-toolchain.toml        # the pinned toolchain
├── crates/
│   ├── <domain>-core/         # types and errors. Depends on nothing internal.
│   ├── <domain>-<concern>/    # one concern per crate
│   └── <cli-or-binary>/       # the only crate that assembles anything
└── .ai/                       # project knowledge
```

### 3.1 Module dependency rules

```text
                        <cli>
             ┌────────────┼────────────┐
             ▼            ▼            ▼
        <concern-a>  <concern-b>  <concern-c>
             └────────────┼────────────┘
                          ▼
                        <core>
```

1. Dependencies point **inward**. `core` depends on nothing internal.
2. `core` must not depend on a CLI framework, an async runtime, or a provider SDK.
3. No crate may use another crate's private module; public surfaces are `pub` and documented.
4. A crate that cannot do its job without I/O stays out of the dependency-only crates: the tree
   stays testable without a filesystem.

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

*What is stored, where, in what shape, and which side is derived and rebuildable.*

## 6. Error model

- One error enum per failure domain, in that crate. A common error type carries a code, a message,
  a cause chain, and a remediation hint.
- Errors convert into the common type; a `Box<dyn Error>` never appears in a public signature.
- The top level is the only place that turns an error into human output and an exit code.

## 7. Testing strategy

| Layer | Approach |
|-------|----------|
| Pure logic | Exhaustive unit tests, including property tests over the input matrix |
| Parsers | Round-trip over fixtures, plus malformed input that must be refused by name |
| Concurrency | Real threads with a barrier; no sleeps |
| Integration | Tests against real temporary directories and repositories |

`cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` all gate every change, on every
supported platform.

## 8. Risks and unknowns

| # | Risk | Impact | Likelihood | Mitigation | Owner |
|---|------|--------|------------|------------|-------|
| R-1 | *what could go wrong* | *worst case* | *how likely* | *what you will do* | *who* |

**Unknowns that need a spike, not a guess**

- U-1: *the question, and how you will answer it with evidence*