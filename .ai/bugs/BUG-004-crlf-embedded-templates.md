---
id: BUG-004
type: bug
title: Embedded templates carried the checkout's line endings, so init wrote CRLF and doctor disagreed
status: fixed
severity: high
symptoms: >-
  CI's verify job failed six tests in the `aicontext` binary on windows-latest only, all reporting
  `CTX-002` against `.ai/TASKS.md` ("front matter has no `type`" and "no `id`"), while the byte-for-byte
  init test and the doctor end-to-end tests failed outright. Every one of these passed on the
  developer's Windows machine, so the failure existed only where git expanded newlines on checkout.
root_cause: >-
  The base templates and the schemas are embedded with `include_str!`, so their bytes are whatever git
  wrote to disk. Git for Windows defaults to `core.autocrlf=true`, which converts LF to CRLF on
  checkout, so on the CI runner the embedded text began `---\r\n`. `init` substituted placeholders and
  wrote the text through unchanged, so a generated document had CRLF endings; the front-matter reader
  then failed its `strip_prefix("---\n")`, and the tests that assert the written document equals the
  rendered template failed on the same bytes. A second, latent defect surfaced once `init` was fixed to
  write LF: `doctor`'s CTX-012 check compared the raw on-disk copy against the raw embedded source, so
  an LF copy no longer matched a CRLF source.
files_changed:
  - crates/aicontext-cli/src/init/render.rs
  - crates/aicontext-context/src/doctor.rs
prevention: >-
  Normalise embedded text to its canonical form at the boundary where it is consumed.
  `docs/CONTEXT_SPEC.md` rule 11 says line endings are normalised to `\n` on write, so `Bindings::apply`
  folds CRLF and lone CR to LF before substitution, and CTX-012 compares the compiled-in source in that
  same canonical form. A local checkout that happens to use LF is not evidence the fix works: the guard
  is an explicit CRLF unit test on each normalisation, or the Windows CI job itself, because the defect
  lives in the gap between the developer's git config and the runner's.
created: 2026-10-09
updated: 2026-10-09
tags: [init, doctor, templates, include_str, line-endings, ci, windows]
---

# BUG-004 — Embedded templates carried the checkout's line endings

## What was observed

The `verify` job's test step failed on `windows-latest` alone, in the `aicontext` binary:

```
test result: FAILED. 95 passed; 6 failed; 0 ignored; 0 measured; 0 filtered out
```

The six were `init::apply::tests::the_written_document_is_the_rendered_template_byte_for_byte`,
`init::tests::a_full_run_creates_the_skeleton_and_the_gitignore_entry`,
`init::tests::json_output_is_a_single_object_naming_the_project`,
`init::validate::tests::a_fresh_skeleton_validates_without_a_single_finding`,
`init::validate::tests::a_generated_document_without_front_matter_is_reported_against_both_keys`, and
`init::validate::tests::every_stack_template_validates`. Each reported `CTX-002` against every
document, with the message "front matter has no `type`". The same suite was green on ubuntu and macos,
and green on the developer's Windows machine.

## Why it happened

`templates/base/*.md` and `schemas/*.json` are embedded with `include_str!`, so the binary holds the
bytes git checked out. Git for Windows defaults to `core.autocrlf=true`, which rewrites LF as CRLF on
checkout; the CI runner did not have the repository's LF form that the developer's machine had. So on
the runner the embedded text began `---\r\n`.

`init` passed that text through `Bindings::apply` and wrote it unchanged, so a generated document had
CRLF endings. The front-matter reader looks for `---\n` and found `---\r\n`, so it reported the front
matter as absent and emitted CTX-002 for both required keys on every document. The same bytes made the
assertion that a written document equals the rendered template fail, and the doctor end-to-end tests -
which run `init` and then check the scaffolded tree is clean - fail with it.

Fixing `init` to write LF exposed a second copy of the same assumption. `doctor`'s CTX-012 check read
the copy in `.ai/schemas` and compared it byte-for-byte to the compiled-in source. With the write path
normalised to LF and the source still CRLF, every freshly initialised schema disagreed with the source
it was copied from, so a clean project reported twenty CTX-012 warnings.

## Why local runs missed it

The normalisation is `\n` inside `docs/CONTEXT_SPEC.md` rule 11, but the embedded bytes were never
normalised to it. The defect is a function of the checkout's git configuration, and the developer's
configuration happened to match the repository's stored form, so the two halves of the comparison
agreed by accident everywhere except the runner.

## The fix

Normalise at the boundary where the text is consumed, so the bytes a checkout supplies cannot reach a
comparison or a write unnormalised:

- `crates/aicontext-cli/src/init/render.rs`: `Bindings::apply` folds `\r\n` to `\n` and a lone `\r` to
  `\n` before substitution, so every generated document is LF regardless of the checkout, as rule 11
  requires.
- `crates/aicontext-context/src/doctor.rs`: `check_schemas` compares the compiled-in source in its
  canonical LF form, so an LF copy matches a source whose checkout bytes were CRLF.

Each has a unit test that feeds CRLF explicitly, because the developer's own checkout cannot reproduce
the condition.

## Prevention

Treat the line endings of embedded text as checkout-dependent input, not as a constant. When a rule
fixes a canonical form for written text, apply it where the text enters the comparison or the writer,
and prove it with an input that carries the other form. The durable guard is the platform that has the
other git configuration, which is exactly the platform a developer is not using.
