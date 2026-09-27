name: Feature request
description: Propose a capability, or record a decision that needs an ADR
title: ""
labels: ["needs-triage"]
body:
  - type: markdown
    attributes:
      value: |
        Two different things can land here.

        - **A capability** the CLI or library should have. Describe the outcome you want.
        - **A decision** that changes an architectural boundary. Open an issue first to agree the
          shape, then the ADR is written before the code — the decision comes first.

        If you only want to report a broken behaviour, use the bug report template instead.

  - type: dropdown
    id: kind
    attributes:
      label: What kind of request is this?
      options:
        - A capability the CLI or library should have
        - A change to the architecture, crate boundaries, or a dependency
        - A rule, spec, or schema change
        - A developer-experience improvement
    validations:
      required: true

  - type: textarea
    id: problem
    attributes:
      label: The problem
      description: What are you unable to do today? Describe the situation, not the solution.
      placeholder: >
        I cannot find out why a document was left out of a context packet without reading the
        source, and the packet says only that it was dropped.
    validations:
      required: true

  - type: textarea
    id: proposal
    attributes:
      label: Proposed outcome
      description: What should be true when this is done? Command surface, if any.
      placeholder: >
        `aicontext context explain` shows the plan, the score of each candidate, and why each
        dropped document was dropped.
    validations:
      required: true

  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives you considered
      description: >
        Including "do nothing". If this changes the architecture, name the boundaries it would
        cross — that decides which ADRs and specs need updating.
    validations:
      required: true

  - type: textarea
    id: scope
    attributes:
      label: Scope
      description: >
        Which phase does this belong to? See `.ai/TASKS.md`. Proposals that cannot be traced to a
        task are usually proposals that are not yet understood.
      placeholder: Phase 2 — context engine, near TASK-037
    validations:
      required: false

  - type: checkboxes
    id: checks
    attributes:
      label: Checks
      options:
        - label: I searched existing issues and open PRs first.
          required: true
        - label: >
            If this crosses an architectural boundary, I read `ARCHITECTURE.md` and expect it will
            need an ADR.
          required: false
