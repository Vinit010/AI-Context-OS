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

Naming, layout, and format standards. Normative rules live in `RULES.md`; this file is the
reference detail they point to.

---

## 1. Naming

| Thing | Convention | Example |
|-------|-----------|---------|
| Module, file, function, variable | *your rule* | *example* |
| Type, interface, enum variant | *your rule* | *example* |
| Constant | *your rule* | *example* |

**Booleans** read as predicates: `is_enabled`, `has_changes`, `can_approve`. Never `flag`, `check`,
or a bare `valid`.

**Error types** are named for the domain, not the mechanism: `ContextError`, `IndexError` — not
`ParseFailedError` or `IoWrapperError`.

## 2. File and folder layout

- One responsibility per file.
- Generated output never lives in the source tree; it goes to an ignored directory.
- *Any directory naming rule this project has.*

## 3. API shape

```text
*Paste a real function from this codebase. A convention nobody follows is not a convention.*
```

- Infallible constructors are `new`. Anything that can fail gets a distinct constructor.
- Prefer borrowing to owning: `&str` over `String`, `&[T]` over `Vec<T>`.
- Public types are `Debug`. Types crossing a boundary are serialisable, with explicit field names.

## 4. Document and ID conventions

| Entity | ID | File |
|--------|----|------|
| Specification | `SPEC-<slug>` | `.ai/specs/<slug>.md` |
| Task | `TASK-NNN` | `.ai/tasks/TASK-NNN-<slug>.md` |
| Decision | `ADR-NNN` | `.ai/decisions/ADR-NNN-<slug>.md` |
| Bug | `BUG-NNN` | `.ai/bugs/BUG-NNN-<slug>.md` |
| Change | `CHG-NNN` | `.ai/changes/CHG-NNN-<slug>.md` |

- IDs are **stable and never reused**, even after deletion or cancellation.
- The file name carries the ID; references between documents use IDs, never paths, so a rename
  cannot break a link.
- Slugs are lowercase kebab-case, ASCII, no dates, no status words.

## 5. Front matter

Key order: `id`, `type`, `title`, `status`, then type-specific keys, then `created`, `updated`,
`tags`. `type` and `title` are always present. `created` is immutable; `updated` is set by the tool
when it writes the file, never guessed.

## 6. Commits and branches

- Branch: `<type>/<short-description>`.
- Commit: `type(scope): summary`, imperative mood, 72 characters or fewer.
- One logical change per commit.

## 7. CLI conventions

- Command names are verbs; nouns are arguments: `tool task create`, not `tool create-task`.
- Every command accepts `--json` and produces a stable, documented shape.
- Human output to stdout; diagnostics to stderr. `--json` keeps stdout pure JSON.
- Exit codes are fixed and documented. Never invent one.
- Output is line-oriented and diff-friendly, so a human can read a change between two runs.

## 8. Testing conventions

- Unit tests live next to what they test; integration tests test through the public API.
- Table-driven for matrices; property tests for parsers and resolvers.
- Fixtures come from one place, so a test never invents its own scaffolding.
- No test touches the network, the real clock, or the developer's home directory.

## 9. Documentation conventions

- Sentence case for headings. No exclamation marks. No marketing tone.
- Present tense, active voice, second person for instructions.
- One fact, one home. Link instead of repeating.
- Every normative statement uses "must", "must not", "should", or "may" — deliberately.
- Examples must be runnable or explicitly marked as illustrative.