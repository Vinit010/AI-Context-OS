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

Rust starter conventions. Naming, layout, and format standards; `RULES.md` holds the binding rules.

---

## 1. Naming

| Thing | Convention | Example |
|-------|-----------|---------|
| Crate | `<domain>-<role>`, kebab-case | `payments-ledger` |
| Module, file, function, variable | `snake_case` | `resolve_payment` |
| Type, trait, enum variant | `PascalCase` | `PaymentState` |
| Constant, static | `SCREAMING_SNAKE_CASE` | `MAX_RETRY_ATTEMPTS` |

**Booleans** read as predicates: `is_settled`, `has_retry`, `can_refund`. Never `flag`, `check`, or a
bare `valid`.

**Error types** are named for the domain: `PaymentError`, `LedgerError` — not `ProcessFailedError`.

## 2. File and folder layout

- One responsibility per file. If a file needs a comment saying what part is which, split it.
- `mod.rs` stays thin: re-exports and `mod` declarations only, no logic.
- The module tree mirrors the crate tree, so tests sit next to what they test.
- `tests/` holds integration tests only; fixtures live in one place.
- Generated output goes to an ignored directory, never the source tree.

## 3. API shape

- Infallible constructors are `new`. Anything that can fail gets a distinct constructor and returns
  a `Result`.
- Prefer borrowing to owning: `&str` over `String`, `&[T]` over `Vec<T>`.
- Mark a public type `Debug`. Anything crossing a process or wire boundary is `Serialize` with
  explicit field names, never relying on declaration order.
- Public items carry doc comments stating **what**, **why**, and **what invariant the caller must
  uphold**.

## 4. Errors

- One error enum per failure domain, deriving `thiserror::Error`. Never `Box<dyn Error>` in a public
  signature.
- Every error variant carries a stable code, a message naming the concrete subject, a `#[source]`
  cause, and a remediation hint. Never lose the source chain: it is the difference between a
  five-minute and a two-hour fix.
- Never `unwrap` or `expect` outside tests and `main`. If an invariant cannot fail, make the
  function total instead of asserting at runtime.
- `unsafe` is forbidden in this repository. If it seems necessary, that is an architecture decision.

## 5. Tests

- Unit tests in the same file, under `#[cfg(test)] mod tests`. Integration tests in `tests/`, going
  through the public API.
- Test names state the behaviour, not the function: `rejects_unknown_currency`, not `test_4`.
- Table-driven for matrices; property tests for parsers and anything with a round trip.
- No test reads the network, the wall clock, the developer's home directory, or global
  configuration.

## 6. Comments and documentation

- Comments explain **why**, not **what**. The code already says what.
- Every item that cannot be removed in an emergency carries a comment saying what breaks without it.
- Public documentation states the contract, not the implementation: a caller should never have to
  read the body to use it correctly.