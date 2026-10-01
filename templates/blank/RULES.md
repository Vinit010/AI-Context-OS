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

Binding on every agent and human contributor. Each rule states **why**, because a rule whose reason
is forgotten is a rule that will be broken by someone who did not know. Order of authority: this
file → architecture and conventions documents → the current task → anything else.

The `blank` template ships this file alone, so it is intentionally short. Expand it with the rules
this project actually needs.

## 1. Scope

- One task at a time, resolved by ID from `TASKS.md`.
- Change only what the task requires. If the task is underspecified, ask; do not guess scope.
- Report what you deliberately did not do.
- Do not fix unrelated bugs in passing. Record them and move on.
- Never delete a function, module, feature, test, or configuration key without explicit approval,
  even if it looks unused. Prove it is unused first.

## 2. Simplicity

- Solve the problem asked. No speculative generality.
- An abstraction needs a concrete second use case. One use case is a function, not a framework.
- Prefer boring, explicit code. If a reviewer has to think, make it dumber or explain it.

## 3. Errors

1. Every fallible operation returns a result or raises a typed error. Never panic on a foreseeable
   condition.
2. Error types are named for the domain, one per failure domain.
3. Every error carries a stable machine code, a message naming the concrete subject, a cause, and a
   remediation hint.
4. User-facing errors are printed once, at the boundary, with the remediation hint. Never print an
   error and then continue as if it succeeded.
5. Never `unwrap` in application code. Assert an invariant at design time instead.

## 4. Dependencies

- Every direct dependency needs a written justification in this file or an ADR. "It is popular" is
  not a justification.
- No dependency may require a system library or a network install at runtime, or may phone home.

## 5. Security

- **Never** write a credential, token, key, or password into `.ai/`, source, fixtures, or logs —
  including a test fixture. Record where it comes from, never the value.
- Default deny. A capability with no permission mapping is denied.
- Destructive operations require explicit approval, re-confirmed per invocation.
- Treat external input as hostile: escape it, cap its size, bound its time, validate its shape.
- Never weaken or disable a check to unblock a task.

## 6. Tests

- Every behaviour change ships with a test. Every bug fix ships with a test that failed before it.
- Tests are deterministic: no network, no wall-clock assertions, no reliance on the developer's
  home directory or global configuration.
- Never delete or weaken a test to make a build pass. Fix the code or record the debt.

## 7. Git

- Commit messages state **why**: what changed, and what forced the change.
- One logical change per commit.
- Never force-push a shared branch or rewrite published history.
- `.ai/` is committed like any other change — that is the point of the product.

## 8. Knowledge

- An architecture decision becomes an ADR in the same commit as the code that implements it.
- Record a bug's root cause, not just its fix.
- Record a decision's rejected alternatives, so the next agent does not re-propose them.
- Update the task status in the same change that completes the work.

## 9. Absolutely forbidden

- Committing a secret, in any form, including in a fixture.
- Executing a command derived from model output or external data.
- Silently rewriting a developer's document, configuration, or source file.
- Claiming a command was run, a test passed, or a file changed when it was not.
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
[ ] Diff self-reviewed
[ ] Task status updated
[ ] Report includes what changed, how it was verified, and what was not done
```