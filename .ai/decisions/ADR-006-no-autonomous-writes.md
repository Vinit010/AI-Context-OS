---
id: ADR-006
type: decision
title: No autonomous file writes by default — plan first, execute only into a declared scope
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-006 — No autonomous writes by default

## Context

The MVP success criteria in the brief require that a developer can "ask the AI to implement the
task", have the AI "run tests", "report changed files", and "update TASKS.md" — while the product's
most important product rule is "do not build a platform that tries to make AI autonomous for the sake
of autonomy" and "the developer remains the final authority".

That is a real tension, and the specification does not resolve it. It says the AI implements the
task; it also says nothing important happens without the developer. The resolution determines the
entire threat model of the product, so it is recorded here.

## Decision

1. **Plan mode is the default and the only mode available until Phase 3 is complete.** `aicontext
   agent run` assembles context, calls the provider, and writes a **plan** to
   `.aicontext/plans/`. It modifies nothing in the working tree.
2. **`--execute` is opt-in, per invocation, and never persisted.** There is no config key, flag
   file, or "trust this agent" switch that makes execution the default.
3. **When execution is enabled, it is confined to a declared scope allowlist** — the file globs
   from the task's `touches` field, plus explicitly added paths. A write outside the allowlist is
   refused and audited, not warned about.
4. **The agent never commits, never pushes, never branches, and never rewrites history.** Those are
   human actions, always. `RULES.md` §1 makes committing to a "hard stop".
5. **Destructive actions are always a separate, explicit human act**, whatever mode the agent is in.
6. **The agent cannot widen its own permissions.** The permission policy is evaluated outside the
   model's control and applies to the model identically to a human operator.
7. **The plan is the reviewable artifact.** It lists files to modify, create, and delete;
   dependencies affected; schema and data impact; and security impact — before anything is written.

## Reason

- **A wrong autonomous edit is expensive and hard to see.** The failure mode of agentic editing is
  not a crash, it is a plausible-looking change to the wrong subsystem that passes review because it
  looks like the work you asked for somewhere else.
- **The plan is where the human adds the most value.** A developer can correct a plan in seconds;
  they cannot cheaply audit a diff that a model produced across forty files.
- **Scope confinement turns a catastrophic failure into a bounded one.** A confined agent that
  misbehaves damages one task's worth of files, which Git already handles.
- **It preserves the product's core claim.** "Developer-controlled" is a feature. An agent that
  can write anywhere makes the permission system theatre.
- **It is reversible in the right direction.** If plans prove reliable, execution widens. If
  autonomous editing were shipped first and had to be retracted, trust would not recover.

## Rejected alternatives and why

- **Full autonomous execution with a diff summary afterwards.** Rejected: the summary arrives after
  the damage, and "review the diff" is exactly the human judgement the product exists to support,
  not a step to automate away.
- **Autonomous execution with automatic rollback on test failure.** Rejected: tests do not
  determine intent. A change can pass every test and be wrong, and a failed test can be a
  legitimately changed assumption. Reverting silently also destroys the intermediate work a human
  would have learned from.
- **A persistent "autonomous mode" per project or per agent profile.** Rejected: a standing grant
  is how approval fatigue begins (risk R-8). Autonomy must be re-earned per invocation.
- **Committing on the agent's behalf to create a cheap undo point.** Rejected: it puts the agent on
  the Git write path, makes the agent's actions indistinguishable from the human's in history, and
  invites a `reset --hard` reflex that destroys real work.

## Consequences

**Positive** — the developer stays in the loop at the cheapest possible point; failures are
bounded by the scope allowlist; the plan doubles as documentation; the permission system keeps its
meaning; the product's central promise is literally implemented.

**Negative** — more friction than a fully autonomous agent; the developer must read plans, which
some will find tedious; a genuine speed advantage of unattended agentic work is deliberately
forgone in the MVP; the plan step can become ritual if it is not kept short and factual.

**How we will know it is right** — the product succeeds when a developer *chooses* to widen the
scope, not when the tool widens it for them. If plans turn out to be noise, the correct response is
to make plans better, not to remove them.

**Follow-up** — TASK-055 (plan mode), TASK-072 (approval broker), and the `--execute` flag, which
is deliberately unassigned until plan mode has proven itself in real use.

## Related tasks

TASK-055, TASK-072, TASK-100
