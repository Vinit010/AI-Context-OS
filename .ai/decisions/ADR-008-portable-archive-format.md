---
id: ADR-008
type: decision
title: Export/import archives are one deterministic JSON document with a digest manifest
status: accepted
date: 2026-10-10
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-008 — Export/import archives are one deterministic JSON document with a digest manifest

Implements `F10` in `PRD.md`. Closes `TASK-020`.

## Context

`F10` requires the context tree to round-trip: "Files remain the source of truth. No export may be
lossy in a way that requires the tool to read it back." A portable form for `.ai/` therefore has to
(a) survive transport without the tool, (b) verify itself before a single write is planned, and
(c) refuse rather than truncate when a bound is reached.

Existing contracts already fix the shape of these requirements:

- `RULES.md` §11 forbids unbounded reads, so the archive and the walk that produces it carry the
  same order of caps `doctor` already applies: 512 entries, 8 MiB per file, 16 path components, and
  64 MiB for the archive itself.
- The audit log already standardised on SHA-256 (`SECURITY.md` §10), so the digest choice should
  match it rather than introduce a second scheme with a different trust profile.
- `ADR-002` / `ADR-006` forbid autonomous writes: overwriting an existing file that differs from
  the archive is exactly the kind of clobber that needs a human, and a human in a pipe is not a
  human.
- Determinism is a stated product value (`ADR-004`, `CONTEXT_SPEC`), and an export that embeds
  timestamps, host names, or absolute paths is undiffable and unreproducible.

Container formats considered:

| Candidate | Verdict |
|-----------|---------|
| Directory copy (".zip of a folder" as a tree) | Rejected: not one portable artifact, no whole-archive integrity unit, transport is multi-file. |
| Tarball (`.tar`, compressed or not) | Rejected: binary by nature, needs an archive parser dependency, and a faithful manifest still has to be embedded inside it. Compressed variants add gzip stream and header nondeterminism. |
| ZIP (store or deflate) | Rejected: local file headers embed timestamps and machine markers by default, so reproducible ZIPs require pinning fields for no benefit. |
| CBOR / bincode / MessagePack | Rejected: binary, not reviewable or diffable in a code review. |
| **One pretty-printed JSON document** | **Chosen.** Text, diffable, reviewable, deterministic when rows are sorted and no machine facts leak; content is already UTF-8 text. |

## Decision

`export` writes a single JSON document, `format` `aicontext-archive`, `version` 1, with a trailing
newline:

- a **manifest** carrying the project name, the file count, the total bytes, one digest row per
  file sorted by path, and a SHA-256 digest over the whole manifest;
- the **contents verbatim, in the same order** — stored as JSON strings, so read-back is exactly
  the bytes `export` read.

`import` reads nothing into memory beyond the 64 MiB cap, runs `verify` over the whole document
(format, version, headline counts, manifest digest, path shape, duplicates, per-file sizes, and
per-file digests), and only then plans writes. An archive that fails any check exits 3 and writes
nothing.

Determinism rules, enforced by byte-diff snapshot tests:

- rows are sorted by path;
- nothing about the machine leaks: no timestamps, no absolute paths, no host or user names;
- serialisation is a stable pretty-print with a trailing newline.

Import semantics:

- **Merge, not mirror.** Files the archive does not mention are left alone; `import` never deletes.
  An existing file that already holds the archived bytes is reported `unchanged`.
- **Conflicts are reported, not resolved.** `--force` replaces a differing file and requires an
  interactive terminal; a denied approval leaves the tree untouched (exit 5). `--yes` never
  satisfies the requirement.
- **Relative `<archive>` resolves against the project root**, so the archive name is understood the
  way `doctor` and `status` understand the project, not the raw working directory.

Digest choice is SHA-256, matching the audit log: pre-image resistance sufficient for tamper
detection, not yet cryptographically compromised the way MD5 and SHA-1 are, available as pure Rust
via `sha2` (RustCrypto), and already the only digest scheme the repository uses.

## Reason

1. **Verifiable before acting is a review contract.** A text document means the manifest, the
   paths, and the digests are inspected by a human in a code review without running `import`.
   Binary containers force every reviewer to trust the tool.
2. **Determinism is a test and a product feature at once.** Identical bytes for an unchanged tree
   is the `F10` round trip, and byte-diff tests make format drift fail loudly rather than silently.
3. **The tool must not be required to read its own export — but must be able to verify it.** The
   whole-archive digest is what lets `import` say "this archive is intact" before touching a file;
   the per-file digests are what let it say "these specific files differ".
4. **Bounded reads are inherited, not invented.** Reusing `doctor`'s cap order means a hostile or
   accidental archive cannot exhaust memory, and a cap violation is a finding that refuses the run
   rather than a silent truncation.
5. **SHA-256 is the house standard.** Introducing a second digest would add a second trust surface
   for no benefit; `F10` does not ask for more than practical tamper detection.

## Consequences

- The format is a storage contract from the first byte: a `version` field guards it, and changing
  the shape means a version bump and visible diffs in the snapshot tests, keeping the bytes honest.
- Archives are diffable and committable — an export behaves like a text rather than a blob.
- Determinism means the same tree always exports the same bytes on any machine, so backups and
  reviews are reproducible without a network.
- The caps are refuses, not truncators: an oversize archive (EXP-012/IMP-012), an over-large file
  (EXP-011), too many entries (EXP-013), or too deep a path (EXP-014/IMP-013) fails the run.
- Relative-path resolution against the project root is a deliberate divergence from the raw working
  directory and is documented in `CLI_SPEC.md` §4.
- `F10` is otherwise unchanged in scope: archival moves files in and out of `.ai/` but never edits
  them, so the files remain the source of truth.
- The decision that redefines trust later is if someone wants compression or binary content
  (e.g. images) in `.ai/`; that is a new F-a requirement, not a performance tweak to this format.