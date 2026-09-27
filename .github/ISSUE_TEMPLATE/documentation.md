name: Task or spec correction
description: The project's own documentation is wrong, inconsistent, or out of date
title: "docs: "
labels: ["documentation", "needs-triage"]
body:
  - type: markdown
    attributes:
      value: |
        AI Context OS documents itself under `.ai/`, and those documents are product data, not
        incidental notes. This template is for when they are wrong.

        Common cases:

        - `RULES.md` says something the code does not do.
        - A task's acceptance criteria are met but the status was never updated.
        - Two documents define the same concept differently.
        - An ADR records a decision that has since been reversed without a superseding ADR.

  - type: textarea
    id: what-is-wrong
    attributes:
      label: What is wrong
      description: Quote the text that is incorrect, and say what it should say.
      placeholder: |
        `.ai/TASKS.md` says `**Current task:** TASK-010` but TASK-010 is `status: DONE`.
    validations:
      required: true

  - type: textarea
    id: evidence
    attributes:
      label: Evidence
      description: What in the repository or history shows this?
      placeholder: "`git log --oneline -5` shows TASK-010 was completed in the last commit."
    validations:
      required: true

  - type: input
    id: task
    attributes:
      label: Related task or ADR
      description: Task ID, ADR ID, or spec section, if one applies.
      placeholder: TASK-011
    validations:
      required: false
