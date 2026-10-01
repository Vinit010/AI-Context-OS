---
id: DESIGN-001
type: design
title: "{{project_name}} — Design System"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# DESIGN

> Starter document. Keep only the sections this project actually has. A design system that documents
> colours nobody uses is a document that will be ignored.

Companion documents: `PRD.md` (what), `ARCHITECTURE.md` (how), `RULES.md` (constraints).

---

## 1. Design principles

| # | Principle | Consequence |
|---|-----------|-------------|
| D1 | *what matters most to the user* | *what it forces you to do* |

## 2. Colour palette

### 2.1 Semantic tokens

| Token | Meaning | Used for |
|-------|---------|-----------|
| *name* | *what it means* | *where it appears* |

### 2.2 Contrast requirements

*What must be legible against what, and how that is verified — by measurement, not by eye.*

## 3. Typography

| Level | Size / weight | Use |
|-------|---------------|-----|
| *name* | *value* | *where it appears* |

## 4. Spacing and layout

*A scale, not arbitrary values. State the base unit.*

## 5. Components

| Component | States | Notes |
|-----------|--------|-------|
| *name* | *every state it can be in* | *what decides the state* |

## 6. Output design

*The CLI, any logs, and any generated text a human reads.*

```text
$ *your-command*

*the real shape of a real run, with the identity line, the content, and the summary*
```

Rules:

- Identity first, then content, then a summary line with counts and the next command.
- Never truncate a path silently — wrap it, or end with an ellipsis and print the full value under a
  verbose flag.
- Colour is opt-out, and always redundant with a glyph and a severity word.

## 7. Accessibility

- **Contrast:** verified in CI, not by eye.
- **Keyboard:** every action reachable and operable by keyboard.
- **Language:** plain, literal wording. No idiom, no pun, no exclamation. Error messages say what
  happened and what to do.

## 8. What this system does not do

- *Anything a reader might otherwise assume. This section prevents more wasted work than any other.*