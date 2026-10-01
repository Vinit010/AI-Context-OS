---
id: MEMORY-001
type: memory
title: "{{project_name}} — Durable Memory"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# MEMORY

Durable knowledge only. If a line here would be false in three months, it does not belong here.
Conversation transcripts, command output, and anything already in `git log` are excluded.

Entries use the inline block form: a level-2 heading, then a fenced `yaml` block, then prose saying
what the constraint means in practice.

```yaml
id: MEM-001
category: constraint | lesson | fact | preference | limitation | bug
scope: <the area it applies to>
status: active | superseded
confidence: low | medium | high
recorded: <YYYY-MM-DD>
supersedes: null
```

---

## MEM-001 — *the first durable thing you learned*

```yaml
id: MEM-001
category: constraint
scope: <area>
status: active
confidence: high
recorded: {{date}}
supersedes: null
```

*What was learned, why it is a constraint rather than a preference, and what would have to change for
it to stop being true.*

## MEM-002 — *a cost someone will otherwise pay twice*

```yaml
id: MEM-002
category: lesson
scope: <area>
status: active
confidence: high
recorded: {{date}}
supersedes: null
```

*What it cost, and the symptom that will identify it next time.*

---

## Open questions

| # | Question | Blocks | Resolve by |
|---|----------|--------|-----------|
| Q-1 | *a question this project cannot answer yet* | *what it blocks* | *when it must be answered* |

---

## Bugs encountered

*None yet. Record the symptom, the cause, and the fix that actually works — a fix without a cause
teaches nothing.*