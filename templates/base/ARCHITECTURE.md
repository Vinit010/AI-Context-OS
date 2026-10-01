---
id: ARCH-001
type: architecture
title: "{{project_name}} — System Architecture"
status: active
version: 0.1.0
created: {{date}}
updated: {{date}}
---

# ARCHITECTURE

> Starter document. Architecture is the set of decisions that are expensive to reverse, written down
> with the reason they were made. Anything that is easy to change does not belong here; it belongs in
> `CONVENTIONS.md`.

Companion documents: `PRD.md` (what), `RULES.md` (constraints on how), `TASKS.md` (plan),
`MEMORY.md` (learned constraints), `decisions/` (one decision, one reason).

---

## 1. Architecture drivers

Decisions in this document are driven by these forces, in this order of precedence.

| # | Driver | Consequence |
|---|--------|-------------|
| D1 | *what matters most here* | *what it forces* |

Where two drivers conflict, the higher one wins. That is the tie-breaker for every ambiguous
decision in this repository.

## 2. Technology stack

**Runtime and language.** *Version, why this language, and what it costs.*

### 2.1 Dependency policy

Direct dependencies are allowed only with a written justification recorded in this table or in an
ADR. "It is popular" is not a justification.

| Dependency | Purpose | Justification |
|------------|---------|---------------|
| *name* | *what it is for* | *why hand-writing it would be worse* |

## 3. Repository layout

```text
.
├── *source directories*
├── docs/                     # normative specifications
└── .ai/                      # project knowledge
```

### 3.1 Module dependency rules

```text
*the direction dependencies point, and the rule that keeps it a DAG*
```

## 4. Runtime architecture

### 4.1 Component map

```text
*the components, and the direction of every call between them*
```

### 4.2 Primary flows

**Flow A — *the first thing a user does***

```text
*the steps, in order, including what it refuses to do*
```

## 5. Data and schema model

*What is stored, where, in what shape, and what is authoritative. State which side is derived.*

## 6. Error model

*One error type per failure domain, and what the caller does with each. Errors carry a code, a
message naming the concrete subject, a cause, and a remediation hint.*

## 7. Testing strategy

| Layer | Approach |
|-------|----------|
| *pure logic* | *what proves it correct* |
| *parsers* | *round-trip plus malformed input* |
| *integration* | *what runs it for real* |

## 8. Risks and unknowns

| # | Risk | Impact | Likelihood | Mitigation | Owner |
|---|------|--------|------------|------------|-------|
| R-1 | *what could go wrong* | *worst case* | *how likely* | *what you will do* | *who* |

**Unknowns that need a spike, not a guess**

- U-1: *the question, and how you will answer it with evidence*