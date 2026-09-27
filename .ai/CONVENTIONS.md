---
id: CONV-001
type: conventions
title: AI Context OS — Conventions
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-09-27
---

# CONVENTIONS

Naming, layout, and format standards. Normative rules live in `RULES.md`; this file is the
reference detail they point to.

---

## 1. Naming

| Thing | Convention | Example |
|-------|-----------|---------|
| Crate | `aicontext-<domain>`, kebab-case | `aicontext-plugin-runtime` |
| Rust module, file, function, variable | `snake_case` | `resolve_permission` |
| Type, trait, enum variant | `PascalCase` | `PermissionMode`, `ExplicitApproval` |
| Constant, static | `SCREAMING_SNAKE_CASE` | `MAX_DOCUMENT_BYTES` |
| Generic parameter | Single capital, descriptive | `T: Provider`, `D: Document` |
| Module file | Match the module: `index.rs`, `parser.rs` | — |
| Test module | `#[cfg(test)] mod tests` in the same file; integration tests in `crates/<c>/tests/` | — |
| Feature flag | `kebab-case`, one per capability, no `+` chains | `plugin-runtime` |

**Booleans** read as predicates: `is_enabled`, `has_changes`, `should_prompt`, `can_approve`.
Never `flag`, `check`, or a bare `valid`.

**Error types** are named for the domain, not the mechanism: `ContextError`, `IndexError`,
`PermissionError` — not `ParseFailedError` or `IoWrapperError`.

**Abbreviations** allowed: `ctx` is not. `ctx: &Context` is acceptable inside a single function;
prefer a full name across signatures.

---

## 2. File and folder layout

- One responsibility per file. If a file needs a comment saying what part is which, split it.
- `mod.rs` files stay thin: re-exports and `mod` declarations only, no logic.
- A module tree mirrors the crate tree so tests sit next to what they test.
- `tests/` holds integration tests only. `fixtures/` under `aicontext-testkit` holds sample data.
- `templates/` holds `aicontext init` scaffolding. `schemas/` holds JSON Schema source of truth.
- Generated output never lives in the source tree; it goes to `.aicontext/`.
- Line length 100 (rustfmt default is 100; do not fight it).

---

## 3. Rust API shape

```rust
// Constructors, builders, and accessors follow this order.
pub struct ContextIndex { /* … */ }

impl ContextIndex {
    pub fn new() -> Self;                                  // infallible, empty
    pub fn with_documents(docs: Vec<Document>) -> Result<Self, IndexError>;
    pub fn get(&self, id: &DocumentId) -> Option<&Document>;
    pub fn insert(&mut self, doc: Document) -> Result<Option<Document>, IndexError>;
}
```

- Infallible constructors are `new`. Anything that can fail gets a distinct constructor and returns
  `Result`.
- `&self` methods do not mutate. `&mut self` for in-place. Interior mutability only where it is
  justified in a comment.
- `impl Trait` in argument position for simple generic bounds. Explicit `T: Trait` only when the
  bound must be named in the signature for the caller.
- Return `impl Iterator<Item = T>` for lazy sequences over large collections; return `Vec<T>` when
  the caller will almost certainly collect it anyway.

---

## 4. Document and ID conventions

| Entity | ID | File |
|--------|----|------|
| Specification | `SPEC-<slug>` | `.ai/specs/<slug>.md` |
| Task | `TASK-NNN` | `.ai/tasks/TASK-NNN-<slug>.md` |
| Decision | `ADR-NNN` | `.ai/decisions/ADR-NNN-<slug>.md` |
| Bug | `BUG-NNN` | `.ai/bugs/BUG-NNN-<slug>.md` |
| Change | `CHG-NNN` | `.ai/changes/CHG-NNN-<slug>.md` |
| Memory | `MEM-NNN` | `.ai/MEMORY.md` (inline block) |

- IDs are **stable and never reused**, even after deletion or cancellation.
- The file name carries the ID; the ID in front matter is authoritative.
- Slugs are lowercase kebab-case, ASCII, no dates, no status words.
- References between documents use IDs, never file paths, so a rename cannot break a link.

---

## 5. Front matter

Order keys as: `id`, `type`, `title`, `status`, then type-specific keys, then `created`,
`updated`, `tags`. Always `type` and `title`. See `docs/CONTEXT_SPEC.md` for the per-type schema.

---

## 6. Git conventions

- Branches: `feature/add-context-engine`, `fix/doctor-broken-ref`, `docs/plugin-spec`.
- Commits: Conventional Commits — `feat(context): add lexical retriever`,
  `fix(doctor): report dangling task references`, `docs: record ADR-006`.
- One logical change per commit. A commit that needs "also" in its message is two commits.
- PR description: what, why, how verified, what was deliberately not done, linked task ID.
- Reference the task in the commit subject when one exists: `feat(tasks): … (TASK-014)`.

---

## 7. CLI conventions

- Command names are verbs, nouns are arguments: `aicontext task create`, not `aicontext create-task`.
- Every command accepts `--json` and produces a stable, documented JSON shape.
- Human output goes to stdout; diagnostics and progress go to stderr. `--json` keeps stdout pure
  JSON so it can be piped.
- Exit codes are fixed (`docs/CLI_SPEC.md` §5). Never invent one.
- Help text states what the command does, what it changes, and what it needs.
- Confirm before any interactive prompt; never prompt for a non-destructive action.
- Output is line-oriented and diff-friendly, so a human can read a change between two runs.

---

## 8. Testing conventions

- Unit tests live in the same file, under `#[cfg(test)] mod tests`.
- Integration tests live in `crates/<crate>/tests/<behaviour>.rs` and test through the public API.
- Fixtures come from `aicontext-testkit`: `fixture_project()`, `fixture_repo()`, `fake_provider()`.
- Table-driven for matrices; property tests for parsers, budgets, and the permission resolver.
- No test touches the network, the real clock, or the developer's home directory.

---

## 9. Documentation conventions

- Sentence case for headings. No exclamation marks. No marketing tone.
- Present tense, active voice, second person for instructions.
- One fact, one home. Link instead of repeating.
- Every normative statement uses "must", "must not", "should", or "may" — deliberately.
- Examples must be runnable or explicitly marked as illustrative.
