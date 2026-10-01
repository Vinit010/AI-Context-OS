---
id: CONV-001
type: conventions
title: "{{project_name}} — Conventions"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# CONVENTIONS

Python starter conventions. `RULES.md` holds the binding rules.

---

## 1. Naming

| Thing | Convention | Example |
|-------|-----------|---------|
| Package | `kebab-case` in distribution, `snake_case` in import | `payments-ledger` |
| Module, function, variable | `snake_case` | `resolve_payment` |
| Class, exception, enum | `PascalCase` | `PaymentState` |
| Constant | `SCREAMING_SNAKE_CASE` | `MAX_RETRY_ATTEMPTS` |

**Booleans** read as predicates: `is_settled`, `has_retry`, `can_refund`. Never `flag`, `check`,
or a bare `valid`.

## 2. File and folder layout

```text
src/
├── <domain>/
│   ├── __init__.py        # public surface for this domain
│   ├── model.py           # types/entities for the domain
│   ├── usecases.py        # business logic (pure)
│   └── adapters.py        # I/O (edge only)
├── app/                   # assembly, no business logic
└── cli.py                 # top-level CLI entry if applicable
tests/
```

- One responsibility per file. If it needs "and", split it.
- Tests live in `tests/` at the root and mirror the module tree.
- Generated output goes to an ignored directory, never `src/`.

## 3. Typing

- Type hints are required on all public functions, classes, and return types.
- The project uses `pyright` in strict mode. No `# type: ignore` in committed code without an ADR
  and an explicit reason.
- Untrusted input is `Any` or a validated dataclass, and it is **narrowed** before use. Never cast
  to silence the checker.
- Prefer `Protocol` over inheritance to define shapes. Prefer immutable dataclasses where state is
  fixed.

## 4. Errors

- One exception hierarchy per failure domain. Never `Exception("failed")`.
- Every raised exception carries a stable machine code, a message naming the concrete subject, a
  cause (`from` chain preserved), and a remediation hint.
- Exceptions are caught only at the boundary that can act on them. Never catch and `pass`.
- Do not mix return codes and exceptions for the same kind of failure; exceptions are for
  exceptional paths and failures, return types for control flow where appropriate.

## 5. Tests

- Test names state the behaviour, not the function: `test_rejects_unknown_currency`.
- Table-driven with `@pytest.mark.parametrize` for matrices; property tests for parsers and anything
  with a round trip.
- No test reads the network, the wall clock, the developer's home directory, or global
  configuration. Inject all four using fixtures.
- Fixtures live in `conftest.py` and do not leak implementation details.

## 6. Style and tooling

- `ruff check` and `ruff format` are the only allowed formatters/linters for style.
- Import order is enforced; no wildcard imports. Absolute imports over relative.
- Docstrings follow a single style for public items and state the contract and invariants.
- Comments explain **why**, not **what**. The code already says what.