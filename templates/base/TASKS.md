---
id: TASKS-001
type: tasks
title: "{{project_name}} — Task Register"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# TASKS

**Current phase:** *the phase this project is in*
**Current task:** *the one task in flight, or none*
**Rules:** one task at a time; do not start a task whose dependencies are not `DONE`.

Status values: `BACKLOG` · `TODO` · `IN_PROGRESS` · `BLOCKED` · `IN_REVIEW` · `TESTING` · `DONE` ·
`CANCELLED`

---

## How to read this file

Each task is a level-2 heading with its ID, then a fenced `yaml` block holding the machine-readable
record, then a body for humans. The `yaml` block is the only machine-read source; the body is free
text. Once this file exceeds 300 tasks or 2,000 lines, tasks move to `.ai/tasks/TASK-NNN-<slug>.md`
and this file becomes a generated index.

```yaml
id: TASK-NNN
title: <imperative summary>
status: TODO
priority: HIGH          # CRITICAL | HIGH | MEDIUM | LOW
phase: 1
depends_on: [TASK-NNN]
touches: []             # path globs this task is allowed to change
acceptance:             # all must be true before the task is DONE
  - <criterion>
```

**Legal status transitions.** Enforced by tooling and reported by `doctor`:

```text
BACKLOG    → TODO, CANCELLED
TODO       → IN_PROGRESS, BLOCKED, CANCELLED
IN_PROGRESS→ IN_REVIEW, BLOCKED, TESTING, TODO, CANCELLED
BLOCKED    → TODO, IN_PROGRESS, CANCELLED
IN_REVIEW  → TESTING, IN_PROGRESS, TODO
TESTING    → DONE, IN_PROGRESS, TODO
DONE       → (terminal; reopening creates a new task referencing this one)
CANCELLED  → (terminal)
```

---

## Phase 1 — *name the phase*

**Gate:** *the falsifiable condition that lets the next phase start.*

### TASK-001 — *first task*

```yaml
id: TASK-001
title: <imperative summary>
status: TODO
priority: CRITICAL
phase: 1
depends_on: []
touches: []
acceptance:
  - <criterion a reviewer can check>
  - <another one>
done:
  - *Filled in by whoever completes the task, listing what was actually delivered.*
```

## Phase 2 — *name the phase*

**Gate:** *the falsifiable condition that lets the next phase start.*