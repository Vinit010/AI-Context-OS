name: Bug report
description: Something in AI Context OS behaves incorrectly
title: "bug: "
labels: ["bug", "needs-triage"]
body:
  - type: markdown
    attributes:
      value: |
        Thanks for the report. The most useful bug reports include the exact command, the exit
        code, and the output that surprised you.

        **Never paste credentials, tokens, `.env` contents, or connection strings.** Redact them
        first. If you are unsure whether something is sensitive, describe where it came from
        instead of its value.

  - type: textarea
    id: what-happened
    attributes:
      label: What happened
      description: What did you observe, and what did you expect instead?
      placeholder: |
        `aicontext status --json` reported `"phase": null` on a repository whose
        .ai/TASKS.md has `**Current phase:** Phase 1`. I expected it to report Phase 1.
    validations:
      required: true

  - type: textarea
    id: reproduce
    attributes:
      label: Steps to reproduce
      description: Starting from a fresh clone, the exact steps.
      render: shell
      placeholder: |
        git clone https://github.com/Vinit010/AI-Context-OS.git
        cd AI-Context-OS
        aicontext status --json
    validations:
      required: true

  - type: input
    id: command
    attributes:
      label: Exact command
      placeholder: aicontext status --json
    validations:
      required: true

  - type: input
    id: exit-code
    attributes:
      label: Exit code
      placeholder: "0"
    validations:
      required: true

  - type: textarea
    id: output
    attributes:
      label: Output
      description: Output that shows the problem. Trim it, but keep the error and the exit code.
      render: text
    validations:
      required: true

  - type: textarea
    id: environment
    attributes:
      label: Environment
      description: >
        Operating system and version, `rustc --version`, `aicontext --version`, and the repository
        layout if it is not a fresh clone.
      render: text
      placeholder: |
        OS:        Windows 11
        rustc:     1.95.0
        aicontext: 0.1.0 (dev)
    validations:
      required: true

  - type: textarea
    id: diagnostics
    attributes:
      label: Diagnostics
      description: >
        If the bug involves context assembly, paste `aicontext health` and
        `aicontext context explain` output. It is usually the fastest route to a diagnosis.
      render: shell
    validations:
      required: false

  - type: checkboxes
    id: checks
    attributes:
      label: Checks
      options:
        - label: I searched existing issues and this is not already reported.
          required: true
        - label: I removed secrets, tokens, and `.env` values from everything above.
          required: true
        - label: This is a bug report, not a feature request.
          required: false
