---
id: CTX-stack
type: context
title: "{{project_name}} — Detected stack"
status: active
created: {{date}}
updated: {{date}}
---

# Detected stack

`aicontext init` observed these signals when it created this skeleton and printed them. They are
**advisory**: they come from files existing at the top level, not from a dependency graph, so they
can be wrong. `init` deliberately does not write what it found into any other document, so this
table is the only place detection is recorded.

Fill it in if `init` could not tell you, and correct it when reality differs. It exists so a later
reader can tell the difference between "the project uses Rust" and "a `Cargo.toml` was found".

## Signals

| Signal | Evidence | Confidence |
|--------|----------|------------|
| *language* | *the file that implies it* | high |

## What was not detected

*Languages, frameworks, package managers, CI systems, and infrastructure this run found no evidence
of. An empty list is a real answer: it means nothing was found, not that nothing is used.*

## Re-run discovery

Discovery is a command, not a document. Re-run it after the project changes rather than editing this
file by hand.

```sh
aicontext init --dry-run
```