---
id: RULES-001
type: rules
title: "{{project_name}} — Rulebook"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# RULES

> These rules are binding on every agent and every human contributor in this repository.
> Each rule states **why it exists**, because a rule whose reason is forgotten is a rule that will be
> broken by someone who did not know.

Order of authority: `RULES.md` → `ARCHITECTURE.md` → `CONVENTIONS.md` → the current task →
anything else.

---

## 1. Scope discipline

**Do**

- Work on exactly one task at a time, resolved by ID from `TASKS.md`.
- Change only what the task requires. If the task is underspecified, ask — do not guess scope.
- Report what you deliberately did not do.

**Do not**

- Modify files outside the task's declared scope, "while you are there".
- Fix unrelated bugs in passing. Record them and move on.
- Delete a function, a module, a feature, a test, or a configuration key without explicit approval,
  even if it appears unused. Prove it is unused first.

## 2. Simplicity and abstraction

- Solve the problem asked. No speculative generality, no "we might need this later".
- An abstraction requires a concrete second use case. One use case is a function, not a trait.
- Prefer boring, explicit code over clever code. If a reviewer needs to think, add a comment or make
  it dumber.
- Do not add a feature flag without a named removal date and an owner.

## 3. Error handling

1. Every fallible operation returns a result. Never panic for a foreseeable condition.
2. Error types are named for the domain, one per failure domain.
3. Every error carries: a stable machine code, a message naming the concrete subject, a cause, and
   a remediation hint.
4. User-facing errors are printed once, at the boundary, with the remediation hint. Never print an
   error and then continue as if it had succeeded.
5. Never `unwrap` or `expect` outside tests, or assert an invariant at runtime instead of making the
   function total.
6. Distinguish *expected* failures from *bugs*. Expected failures are error variants with codes.

## 4. Dependencies

- Every direct dependency needs a written justification in `ARCHITECTURE.md` §2 or an ADR.
- Before adding one, write the code it would replace and confirm the replacement is worse. Record
  that reasoning.
- No dependency may require a C compiler, a system library, or a network install at runtime.
- No dependency that phones home, collects telemetry, or reads the home directory implicitly.

## 5. Security

- **Never** write a credential, token, key, password, or connection string into `.ai/`, source,
  fixtures, logs, or audit output — including in a test fixture.
- Credentials come from environment variables or the OS keychain. `.ai/integrations/` may record
  *where* a credential comes from, never the value.
- Default deny. A capability with no permission mapping is denied.
- Destructive operations require explicit approval, re-confirmed per invocation.
- Treat every external input as hostile: escape it, cap its size, bound its time, validate its shape.
- Do not weaken, bypass, or "temporarily disable" a check to unblock a task. Fix it and record why.

## 6. Testing

- Every behaviour change ships with a test. Every bug fix ships with a test that fails before it.
- Tests are deterministic: no network, no wall-clock assertions, no dependence on the developer's
  home directory or global configuration.
- Test names state the behaviour, not the function: `rejects_manifest_with_unknown_permission`.
- Table-driven tests for matrices. Property tests for parsers and resolvers.
- No test may be deleted or weakened to make a build pass. Fix the code or record the debt.

## 7. Git

- Branch: `<type>/<short-description>`, lowercase and hyphenated.
- Commit: Conventional Commits — `type(scope): summary`, imperative mood, 72 characters or fewer.
- One logical change per commit. A commit needing "also" in its message is two commits.
- Never force-push a shared branch, rewrite published history, or amend someone else's commit.
- `.ai/` changes are committed like any other change — that is the point of the product.

## 8. Documentation and knowledge

- Architecture change → a new ADR, same commit. Decision first, code second.
- Public interface change → the doc comment or the specification, in the same commit.
- Record a bug's **root cause**, not just its fix.
- Record a decision's **rejected alternatives**. The next agent will otherwise re-propose them.
- `MEMORY.md` holds only durable knowledge. If it would be false in three months, it does not go
  there.
- Update `TASKS.md` status in the same change that completes the work.

## 9. Absolutely forbidden

- Committing a secret, in any form, including in a fixture or snapshot.
- Executing a command derived from model output or external data.
- Weakening or disabling a safety or validation check to unblock a task.
- Silently rewriting a developer's document, configuration, or source file.
- Claiming a command was run, a test passed, or a file was changed when it was not.
- Marking a task done while its acceptance criteria are unmet.

## 10. Pre-flight checklist

```text
[ ] Read AI.md, RULES.md, and the current task
[ ] Inspected existing code before writing new code
[ ] Plan stated; approval obtained if it was not trivially small
[ ] Scope respected; no unrelated files touched
[ ] No new dependency without a written justification
[ ] No secret written anywhere
[ ] format / lint / test all pass
[ ] Error paths handled with typed errors and remediation
[ ] Diff self-reviewed
[ ] Task status updated
[ ] ADR / BUG / MEMORY recorded where warranted
[ ] Report includes what changed, how it was verified, and what was not done
```