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

TypeScript-on-Node starter conventions. `RULES.md` holds the binding rules.

---

## 1. Naming

| Thing | Convention | Example |
|-------|-----------|---------|
| Package | `kebab-case`, scoped when publishable | `@scope/payments-ledger` |
| File and folder | `kebab-case` | `resolve-payment.ts` |
| Module member | `camelCase` | `resolvePayment` |
| Type, class, enum member | `PascalCase` | `PaymentState` |
| Constant | `SCREAMING_SNAKE_CASE` | `MAX_RETRY_ATTEMPTS` |

**Booleans** read as predicates: `isSettled`, `hasRetry`, `canRefund`. Never `flag`, `check`, or a
bare `valid`.

**React components** are `PascalCase` files. Hooks are `useX`. Hooks are called from components and
other hooks only — never from a condition, a loop, a callback, or a module-level scope.

## 2. File and folder layout

```text
src/
├── <domain>/           # one folder per domain: its model, its use cases, its tests
├── <entrypoint>        # bin, server, or worker entry
├── lib/                # cross-domain helpers only. If it needs a domain type, it is in a domain.
└── index.ts            # the public surface, and nothing else
```

- One responsibility per file. An index file re-exports; it does not implement.
- Tests live in `src` next to what they test. Integration tests live in `tests/`.
- Generated output goes to an ignored `dist/`; generated types to a committed, generated file.

## 3. Types

- `strict` mode is not optional, and no escape hatch (`any`, `as unknown as`) in application code.
- `unknown` is the type of untrusted input; narrow it before use, never cast it.
- Discriminated unions over optional fields: exactly one of `|`, not all of `?`.
- Interfaces for objects you are given, types for objects you build and combinations of them.
- Runtime validation is required at every boundary: HTTP, environment, files, queues. A type
  annotation is not a check.

## 4. Errors

- One error class or `Error` subclass per failure domain, carrying a stable machine `code` and a
  `cause`.
- Every error thrown across a layer boundary names the concrete subject and says what to do.
- Throw errors; never return `null` or `undefined` to signal failure. Reserve `undefined` for
  "absent", and say so in the type.
- Never swallow a catch: either rethrow with context, handle it, or remove the try.

## 5. Tests

- Test names state the behaviour: `rejects an unknown currency`, not `test 4`.
- Table-driven for matrices; property tests for parsers, resolvers, and anything with a round trip.
- No test reads the network, the wall clock, the developer's home directory, or global
  configuration. Inject all four.
- Prefer real temporary directories over mocks for filesystem work; mocks test the mock.

## 6. Comments and documentation

- Comments explain **why**, not **what**. The code already says what.
- Every exported item carries a doc comment stating its contract, and states the invariant the
  caller must uphold.
- `TODO` is a tracked task, not a comment. Put it in `TASKS.md` and leave the pointer.