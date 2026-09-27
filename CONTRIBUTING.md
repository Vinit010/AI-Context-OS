# Contributing to AI Context OS

Thanks for being here. This project is small, opinionated, and early, so a little structure up
front will save everyone time.

**Read [`.ai/AI.md`](.ai/AI.md) and [`.ai/RULES.md`](.ai/RULES.md) before your first change.**
`RULES.md` is binding on every human and every agent here, and it states the reason behind each
rule. This file is the short practical version; where the two disagree, `RULES.md` wins.

---

## The one rule that matters most

**Work on one task at a time, and take it from the register.**

Do not start a task whose dependencies are not `DONE`. Do not fix unrelated bugs while you are in
the area — record them in `.ai/TASKS.md` and move on. An unrelated diff is a review blocker.

```sh
# 1. Find the current task
grep -A2 '^\*\*Current task' .ai/TASKS.md

# 2. Read it, plus the files it names
```

Update `TASKS.md` in the same change that completes the work, not in a later commit.

---

## Getting set up

You need [Rust](https://www.rust-lang.org/tools/install) and `git`. The exact toolchain is pinned in
`rust-toolchain.toml`, so `rustup` installs the right version the first time you run `cargo`.

```sh
git clone https://github.com/Vinit010/AI-Context-OS.git
cd AI-Context-OS
cargo build --workspace
cargo test --workspace
```

That is the whole setup. There is no code generation step, no database, and no third-party
dependency to install — the workspace currently vendors nothing.

## The gate your change must pass

Run all three before you open a pull request. CI runs the same checks on Linux, macOS, and Windows.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
```

If a lint fires, fix the code. Do not `#[allow]` it without a `// reason:` comment saying why the
lint is wrong here, and never add `#[allow(clippy::too_many_arguments)]` to a public API — that is
a signal the function needs a parameter struct.

Two structural rules are checked by the test suite rather than by review:

- **No `unsafe` anywhere.** Every crate root carries `#![forbid(unsafe_code)]`. If you need it, that
  is an architecture decision requiring an ADR.
- **Dependencies point inward.** The crate graph is a DAG toward `aicontext-core`, enforced by
  `crates/aicontext-testkit/tests/crate_boundaries.rs`. Adding a workspace member without adding it
  to the boundary matrix fails `cargo test`.

## Adding a dependency

Almost certainly you should not. Before you do:

1. Write the ~30 lines of your own code it would replace and confirm the replacement is worse.
2. Record that reasoning in the task.
3. Add a written justification to `ARCHITECTURE.md` §2.2 or an ADR.

"It is popular" is not a justification. A dependency must not require a C compiler, a system
library, or a network install at runtime, and must not phone home or read your home directory
implicitly.

---

## Making a change

**Branch:** `<type>/<short-description>`, lowercase and hyphenated. Type is one of `feature`, `fix`,
`docs`, `refactor`, `chore`, `test`, `security`.

**Commit:** Conventional Commits — `type(scope): summary`, imperative mood, 72 characters or fewer.
One logical change per commit; a commit touching unrelated modules is two commits.

```sh
git switch -c fix/context-budget-overflow
```

**Tests ship with the change.** Every behaviour change comes with a test, and every bug fix comes
with a test that fails before the fix. Name the behaviour, not the function:
`rejects_manifest_with_unknown_permission`, not `test_manifest_3`. Tests are deterministic: no
network, no wall-clock assertions, no dependence on your home directory or global git config.

Use `aicontext-testkit` for temporary repositories and sample `.ai` trees.

---

## What needs a decision record

Some changes are not just code. If yours is any of the following, the record comes **first**, in the
same commit as the code:

| Change | Where it goes |
|--------|---------------|
| An architectural boundary, crate direction, or a rejected approach | `.ai/decisions/ADR-NNN-*.md` |
| A public CLI command, flag, or exit code | `docs/CLI_SPEC.md` |
| A public Rust API | the doc comment, in the same commit |
| A schema change | the schema; additive only unless it is a breaking version |
| A bug's root cause | `.ai/bugs/BUG-NNN.md` — the cause, not just the fix |
| Durable knowledge that will still be true in three months | `.ai/MEMORY.md` |

An ADR that does not record the **rejected alternatives** is incomplete. The next agent will
otherwise re-propose the idea you already turned down.

Never commit a credential, token, key, or `.env` content anywhere — including in fixtures and
snapshots. `.ai/integrations/` may record *where* a credential comes from, never the value.

---

## Pull requests

Fill in the template. Reviewers need three things:

1. **What changed** and **why**, in the summary.
2. **How you verified it** — the exact commands you ran and their result. "Tests pass" is not
   verification; `cargo test --workspace` is.
3. **What you deliberately did not do**, and any task or bug ID you filed for it.

A pull request that touches `.ai/` should say how the task status changed. That is the product
working as intended, not noise in the diff.

## Reporting a bug or asking for a feature

Use the issue templates in [`.github/ISSUE_TEMPLATE`](.github/ISSUE_TEMPLATE). Bug reports should
include the exact command, the exit code, and the `aicontext --json` output where it is safe to
paste — redacting anything that looks like a credential. If the bug is in context assembly,
`aicontext health` and `aicontext context explain` output is usually the fastest route to a
diagnosis.

## Code of conduct

Be direct and be kind. Review the code, not the person. Assume the other person had a reason you
have not seen yet, and ask before assuming it was a mistake.

## Licence

Contributions are accepted under the [Apache License 2.0](LICENSE), the same as the project.
