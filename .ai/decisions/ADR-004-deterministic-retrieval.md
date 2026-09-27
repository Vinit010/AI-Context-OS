---
id: ADR-004
type: decision
title: Deterministic lexical retrieval for context assembly
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-004 — Deterministic lexical retrieval for context assembly

## Context

The context engine must decide which project knowledge is relevant to a task. The original brief
requires that it "not blindly send the entire project to the AI", and separately forbids building
"massive vector database infrastructure" in the MVP. Those two constraints point at a
non-embedding solution, but the exact mechanism still had to be chosen.

The retrieval result is security-relevant: it decides what text reaches the model, and therefore
what an attacker might try to smuggle in through a document. It is also a cost and latency
budget line, and it is the part of the system a developer is most likely to try to debug by hand.

## Decision

Context assembly is a deterministic pipeline:

```text
discover → parse → index → plan → retrieve → prioritise → budget → assemble → annotate
```

Retrieval is composed of pluggable `Retriever` implementations, each returning
`ScoredDocument { tier, score, reason, source_path }`:

| Retriever | Basis |
|-----------|-------|
| `AlwaysRetriever` | Mandatory documents: `AI.md`, `RULES.md` |
| `LinkedRetriever` | Document-graph edges from the current task (spec, decisions, bugs, touched paths) |
| `FilterRetriever` | Front-matter predicates (type, status, tags) |
| `GlobRetriever` | Repository path patterns |
| `LexicalRetriever` | BM25 over headings and body, with per-field boosts |

The MVP ships the lexical retriever. **No embeddings, no vector store, no model call during
retrieval.** Every included document carries its score and a human-readable reason; every exclusion
is countable and visible via `--explain`.

A fixed **priority ladder** (tiers 1–9) is applied after scoring. A lower-tier document that
contradicts a higher-tier one loses, the conflict is recorded in the packet, and `doctor` reports
it. The higher-tier document is never rewritten.

A **token budget** degrades from the lowest tier upward; tiers 1–2 are never dropped.

Retrieved content is wrapped in an untrusted **data envelope** and placed in a data role.

## Reason

1. **Auditability is a product requirement.** Every tool action is permission-gated, which means a
   human must be able to review what the agent was told. A ranking function nobody can reproduce
   makes that review impossible in principle.
2. **Reproducibility.** The same repository state and the same task must yield the same context.
   Embedding models change between versions, making a recorded context unreproducible.
3. **It is the honest MVP.** Semantic retrieval is a large investment (index lifecycle, model
   choice, chunking strategy, evaluation set) whose benefit can only be measured once real usage
   exists. R-4 predicts lexical retrieval will sometimes miss paraphrased intent — that is a
   measurable, bounded failure, and it is cheaper to hit than an unmeasurable one.
4. **Security containment is easier to state.** If retrieval is a pure function over a local index,
   there is no network call during context assembly and therefore no remote channel through which
   to influence the result.
5. **The seam is the point.** `Retriever` is a trait. A future `EmbeddingRetriever` plugs in without
   changing the packet format, the CLI, the providers, or the security model. We are deferring the
   algorithm, not the architecture.

## Alternatives considered

- **Embedding retrieval with a vector store** (pgvector, Qdrant, a local HNSW index). Rejected for
  the MVP: needs a model, a store, an index lifecycle, and a network or local inference runtime;
  non-deterministic across model versions; and explicitly excluded by the brief.
- **Send everything within a size budget, trimmed by a cheap heuristic.** Rejected: prompt
  injection exposure grows with context size, and cost grows linearly. Retrieval exists to *reduce*
  what is sent, not to select the order of what is already sent.
- **A model classifies relevance.** Rejected: non-deterministic, adds a provider dependency to the
  context engine, and would violate the rule that the context engine must not know models exist.
- **Keyword grep as the retriever.** Rejected as the sole mechanism: too weak for paraphrased
  intent. Retained as the *explanation* mechanism inside `LexicalRetriever`.

## Rejected alternatives and why

- **Hand-tuned per-project retrieval configuration.** Rejected: it makes quality depend on a human
  maintaining configuration, which is exactly the toil the product removes. Retrieval quality must
  come from the structure of the documents, not from tuning.

## Consequences

**Positive** — reproducible and reviewable context; no network during assembly; no model in the
retrieval path; fast; free of index lifecycle management; failures are diagnosable by a human.

**Negative** — paraphrased intent will sometimes be missed; lexical scoring is a poor fit for
synonym-heavy code questions; the ladder is a blunt instrument when two documents of similar
authority disagree; relevance quality has no fallback and no learning.

**How we will know it is good enough** — instrument it: record which documents were retrieved and
which were actually used, and let the evidence decide whether an embedding retriever earns its
place. `aicontext health` and `context --explain` exist to make that measurement possible.

**Follow-up** — TASK-032 (plan), TASK-033 (lexical), TASK-034 (linked/filter/glob), TASK-035
(ladder and conflicts), TASK-036 (budget), TASK-037 (assembly). Unknown U-1 — whether one flat
packet suffices for a large codebase — is answered with a real repository during Phase 2.

## Related tasks

TASK-032, TASK-033, TASK-034, TASK-035, TASK-036, TASK-037, TASK-038
