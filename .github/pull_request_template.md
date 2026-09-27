## What changed

<!-- One or two sentences. The diff is not the summary. -->

## Why

<!-- The problem, and why this is the right shape of solution.
     If this changes an architectural boundary, name the ADR here. -->

Task: <!-- TASK-NNN, or "none, trivial fix" -->

## How it was verified

<!-- Exact commands and their result. "Tests pass" is not verification. -->

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## What I deliberately did not do

<!-- Out-of-scope work you noticed, with a task or bug ID. See RULES.md 1. -->

## Checklist

- [ ] `cargo fmt --all -- --check`, `cargo clippy -- -D warnings`, and `cargo test --workspace` pass
- [ ] Every behaviour change has a test; every bug fix has a test that failed before the fix
- [ ] No new dependency, or a written justification in `ARCHITECTURE.md` §2.2 or an ADR
- [ ] Public CLI, schema, or Rust API change is reflected in `docs/` and the doc comment
- [ ] Architecture change has an ADR recording the rejected alternatives
- [ ] Task status updated in `.ai/TASKS.md` in this same change
- [ ] No secret, token, or `.env` value anywhere in the diff, including fixtures
- [ ] No permission, validation, or security control weakened
- [ ] Nothing unrelated reformatted, re-ordered, or tidied
