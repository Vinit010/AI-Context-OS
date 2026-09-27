---
id: ADR-003
type: decision
title: Knowledge storage contract — Markdown body, YAML front matter, no body parsing
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-003 — Knowledge storage contract

## Context

The specification asks for two things that appear to conflict:

- Knowledge lives in Markdown files a human reads and edits (§3–§17 of the original brief).
- "Do not make everything dependent on Markdown" — metadata must be machine-readable (§28).

A design that parses Markdown structure — headings, tables, task checkboxes, ADR section names —
turns every reformatting a human performs into a parser bug, and makes the files hostile to edit.
The failure mode is familiar: a wiki that only its own tool can read.

A second question follows: as `TASKS.md`, `bugs/`, and `decisions/` grow, does each stay a single
file, or does each entity get its own file?

## Decision

1. **Every `.ai` document may begin with a YAML front-matter block delimited by `---`.** Front
   matter is the **only** machine-read source of a document's identity and metadata. It is
   validated against a JSON Schema chosen by the document's location.
2. **The platform never parses Markdown body structure.** No heading scraping, no table scraping,
   no regex over prose, no checkbox parsing. If a value must be machine-read, it belongs in front
   matter. The body is the human view and is treated as free text.
3. **Invalid or missing required front matter is an error**, reported by `doctor`, not a
   best-effort parse.
4. **Inline entity blocks** in an index file (`TASKS.md`, `MEMORY.md`) use a level-2 heading
   containing the entity ID, immediately followed by a fenced ` ```yaml ` block.
5. **Scale policy:** an index file holds entities inline until it exceeds 300 entities or 2,000
   lines, at which point each entity moves to its own file (`tasks/TASK-NNN-<slug>.md`,
   `bugs/BUG-NNN.md`, `decisions/ADR-NNN-<slug>.md`, `changes/CHG-NNN.md`) and the index becomes
   a **generated** table of links. The API reads both modes identically. `doctor` recommends the
   switch; it never performs it silently.
6. **The ID is the stable identity. The file path is not.** Cross-document references use IDs, so
   a rename cannot break a link.
7. Generated files are marked `generated: true` in front matter plus a "do not edit" header.

## Reason

- **Humans stay in charge of the format.** Reformatting a document cannot break the tool, because
  the tool never looks at the formatting.
- **One parse target instead of many.** A YAML front-matter parser plus a JSON Schema validator is
  a small, well-testable surface. A Markdown-structure parser is an unbounded surface with an
  adversarial input space.
- **Portability survives.** A developer who deletes this tool still has readable Markdown. A
  database-backed tool that parses prose has already failed this requirement.
- **Validation is possible.** Front matter can be schema-checked, so `doctor` can be precise
  instead of heuristic.
- **The split policy is reversible.** Because the API reads both modes identically, the threshold
  can be tuned from real usage (open question Q-6) without a migration of anyone's content.

## Alternatives considered

- **Parse the Markdown body** (headings, tables, checkboxes). Rejected: brittle, and it makes
  human formatting a correctness risk. This is the most common way wiki-shaped tools rot.
- **Store everything in JSON/YAML, generate Markdown for reading.** Rejected: it inverts the
  product's core promise. Knowledge must be editable by hand in a text editor, with Git showing a
  readable diff. A generated Markdown view is a nice-to-have, not the source of truth.
- **SQLite or another embedded database as the index and source of truth.** Rejected: it creates a
  second source of truth that can diverge from the files, and it violates the "remove the tool and
  keep the knowledge" requirement in spirit. A derived cache in `.aicontext/` is acceptable
  precisely because it can be deleted and rebuilt.
- **One file per entity from day one.** Rejected for now: it produces a directory of 300 one-line
  files for a young project, which is worse to review than one ordered document. The threshold
  makes this a reversible choice rather than a migration.

## Rejected alternatives and why

- **TOML front matter instead of YAML.** Rejected: YAML's block style is far more natural for
  nested front matter, and the ecosystem around it is deeper. Revisit if the YAML crate risk
  (R-1) materialises — this is exactly why `YamlCodec` is an internal trait.
- **A binary/compact cache as the primary store.** Rejected: unmergeable, unreviewable, and it
  would end Git-based collaboration on context.

## Consequences

**Positive** — hand-editable and Git-friendly; a small, testable parser; precise validation;
portability; no format lock-in; content survives any future internal redesign.

**Negative** — front matter is duplicated between the ID in the heading and the ID in the metadata,
so the parser must detect and reject a mismatch; duplicate data within a document is possible and
is the author's responsibility; long single files are awkward to edit past a few thousand lines,
which is what the scale policy exists to solve; two representations of "index" (inline vs.
generated) must be kept behaviourally identical.

**Follow-up** — TASK-015 (schema set), TASK-016 (parser, including the heading/metadata ID
mismatch check), TASK-014 (`doctor` reports it).

## Related tasks

TASK-014, TASK-015, TASK-016, TASK-038
