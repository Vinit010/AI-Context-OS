---
id: TASKS-001
type: tasks
title: AI Context OS — Task Register
status: active
version: 0.1.0
created: 2026-09-27
updated: 2026-10-26
---

# TASKS

**Current phase:** Phase 4 — Plugin system
**Current task:** TASK-072
**Rules:** one task at a time; do not start a task whose dependencies are not `DONE`.

Status values: `BACKLOG` · `TODO` · `IN_PROGRESS` · `BLOCKED` · `IN_REVIEW` · `TESTING` · `DONE` ·
`CANCELLED`

---

## How to read this file

Each task is a level-2 heading with the ID, followed by a fenced `yaml` metadata block, then the
body. The `yaml` block is the machine-readable record (`docs/CONTEXT_SPEC.md` §3); the body is for
humans. Once this file exceeds 300 tasks or 2,000 lines, tasks move to `.ai/tasks/TASK-NNN-<slug>.md`
and this file becomes a generated index (`ARCHITECTURE.md` §5.3).

```yaml
id: TASK-NNN
title: <imperative summary>
status: TODO
priority: HIGH          # CRITICAL | HIGH | MEDIUM | LOW
phase: 1
depends_on: [TASK-NNN]
spec: SPEC-<slug>       # a SPEC-* id or a docs/*.md path, or null
touches: []             # path globs
acceptance:             # all must be true
  - <criterion>
```

---

## Phase 0 — Specification

**Gate:** the document set is internally consistent, cross-references resolve, and every decision
has a recorded alternative.

### TASK-001 — Produce the Phase 0 document set

```yaml
id: TASK-001
title: Produce the Phase 0 document set
status: DONE
priority: CRITICAL
phase: 0
depends_on: []
spec: null
touches: [".ai/**"]
acceptance:
  - PRD, ARCHITECTURE, RULES, CONVENTIONS, DESIGN, TASKS, MEMORY, AI written
  - Each document states its authority and its cross-references
```

### TASK-002 — Write the normative specifications

```yaml
id: TASK-002
title: Write the five normative specifications in docs/
status: DONE
priority: CRITICAL
phase: 0
depends_on: [TASK-001]
spec: null
touches: ["docs/**"]
acceptance:
  - CONTEXT_SPEC, PLUGIN_SPEC, AI_PROVIDER_SPEC, SECURITY, CLI_SPEC written
  - Exit codes, permission modes, and schema keys are defined exactly once
```

### TASK-003 — Record the foundational ADRs

```yaml
id: TASK-003
title: Record the foundational ADRs
status: DONE
priority: HIGH
phase: 0
depends_on: [TASK-001]
spec: null
touches: [".ai/decisions/**"]
acceptance:
  - ADR-001 language, ADR-002 plugin process model, ADR-003 storage contract,
    ADR-004 deterministic retrieval, ADR-005 secrets, ADR-006 no autonomous writes
  - Each records rejected alternatives
```

### TASK-004 — Review the document set for internal consistency

```yaml
id: TASK-004
title: Review the document set for internal consistency
status: DONE
priority: HIGH
phase: 0
depends_on: [TASK-001, TASK-002, TASK-003]
spec: null
touches: [".ai/**", "docs/**"]
acceptance:
  - Every referenced ID, file, and command exists (verified: 47 task IDs, 6 ADRs, 0 dangling refs)
  - No two documents define the same concept differently
  - Open questions in MEMORY.md are either resolved or listed as a risk
```

### TASK-005 — Write README, CONTRIBUTING, and the issue templates

```yaml
id: TASK-005
title: Write README, CONTRIBUTING, and issue templates
status: DONE
priority: MEDIUM
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["README.md", "CONTRIBUTING.md", ".github/**"]
acceptance:
  - A new contributor can build, test, and submit a change from the README alone
done:
  - README.md: honest status, layout, build/test commands, documentation index
  - CONTRIBUTING.md: setup, the gate, dependency rules, what needs an ADR
  - Issue templates: bug report, feature request, documentation correction
  - config.yml routes security findings to private advisories, blank issues disabled
  - Pull request template carries the RULES.md 14 pre-flight checklist
  - README and CONTRIBUTING cross-link, so the README alone is a usable entry point
```

---

## Phase 1 — Context MVP

**Gate:** §11 of the PRD can be completed through step 4. `init`, `status`, and `doctor` work on a
fresh repository with no network.

### TASK-010 — Create the Rust workspace and CI

```yaml
id: TASK-010
title: Create the Rust workspace and CI pipeline
status: DONE
priority: CRITICAL
phase: 1
depends_on: [TASK-004]
spec: null
touches: ["Cargo.toml", "rust-toolchain.toml", ".github/**", ".gitignore", "crates/**"]
acceptance:
  - Workspace builds with cargo build --workspace
  - Every crate root carries forbid(unsafe_code)
  - CI runs fmt, clippy -D warnings, test, and a dependency audit
  - CI enforces the crate dependency direction from ARCHITECTURE.md 3.2
done:
  - Cargo workspace with 5 crates, resolver 3, edition 2024, MSRV 1.85
  - Workspace lints: missing_docs deny, unsafe_code forbid, clippy all + pedantic
  - 23 tests pass, including the crate boundary matrix that enforces ARCHITECTURE.md 3.2
  - CI: fmt, build, test, doc on 3 OSes; clippy, MSRV 1.85, audit, release binary on Linux
```

### TASK-011 — Define the core domain types and error model

```yaml
id: TASK-011
title: Define the core domain types and error model
status: DONE
priority: CRITICAL
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["crates/aicontext-core/**", "Cargo.toml"]
acceptance:
  - DocumentId, TaskId, DecisionId, BugId, ChangeId, PermissionMode, Severity are defined
  - AicontextError carries a code, message, cause, and remediation hint
  - Crate depends on no internal crate and no async runtime or CLI framework
done:
  - TaskId, DecisionId, BugId, ChangeId as distinct newtypes; DocumentId as the union form
  - 'DocumentId enforces CONTEXT_SPEC 1 exactly: only SPEC, CTX, WF, AGENT take a slug suffix, so TASK-014-extra and the spec placeholders ARCH-NNN and MEMORY-REG are rejected'
  - ErrorCode transcribes the CTX-001 to CTX-020 catalogue; AicontextError carries all four fields
  - Severity defaults to Error so an unannotated finding fails closed
  - PermissionMode defaults to Deny, which is invariant I3
  - 'One dependency added: thiserror, already justified in ARCHITECTURE.md 2.2, required by RULES 4.2'
  - 40 tests in the crate, plus the workspace suite, all passing
```

### TASK-012 — Implement aicontext init

```yaml
id: TASK-012
title: Implement aicontext init
status: DONE
priority: CRITICAL
phase: 1
depends_on: [TASK-011, TASK-016]
spec: null
touches: ["crates/aicontext-cli/**", "templates/**"]
acceptance:
  - Creates the full .ai skeleton plus .aicontext/ in .gitignore
  - Emits advisory stack-detection results without rewriting project files
  - 'Idempotent: re-running does not overwrite an edited document'
  - --dry-run prints the plan; --json emits the created path list
  - Validates its own output and reports the next commands
done:
  - 'Four templates embedded with include_str!: default, plus rust, node, and python overlays, plus blank, which writes only AI.md and RULES.md'
  - The default tree is eight documents, context/stack.md, permissions/permissions.yaml, and nine
    register directories, all rendered with the project name and today's UTC date
  - Writes are confined to .ai/ and one appended .gitignore entry; .aicontext/ is listed, never created
  - Discovery is presence-based and advisory, reported with its evidence file, and recorded only in
    context/stack.md; no project file is rewritten and no template is chosen from what was found
  - Idempotence is byte equality against the rendered template; an edited document is preserved and
    reported as CTX-017, and --force replaces it only after an interactive approval
  - --force refuses with exit 5 when stdout is not a terminal, and --yes never satisfies that check
  - --dry-run prints the plan and writes nothing; --json emits one envelope with data, findings, and
    warnings, so a preserved edit is both listed and counted
  - The run validates what it wrote and prints the next command; a mismatch exits 3
  - Unimplemented commands exit 2 naming their task instead of failing through clap's usage error
  - 'Dependencies added: clap, serde, and serde_json for the command tree and the published envelope, each pre-justified in ARCHITECTURE.md 2.2, plus serde_json and tempfile for the tests'
  - 74 unit tests and 23 end-to-end tests in the crate, the workspace suite, fmt, clippy -D warnings,
    cargo audit, and the MSRV check all passing
  - 'Carried forward, now closed by TASK-015: docs/CONTEXT_SPEC.md showed ARCH-NNN, TASKS-NNN, and MEMORY-REG as valid document ids, which core rejects. TASK-015 corrected the spec to match the code rather than the reverse, and found the same placeholder as a live id in this file''s own MEMORY-001 entry'
```

### TASK-013 — Implement aicontext status

```yaml
id: TASK-013
title: Implement aicontext status
status: DONE
priority: HIGH
phase: 1
depends_on: [TASK-011, TASK-017]
spec: null
touches: ["crates/aicontext-cli/**", "crates/aicontext-git/**", "crates/aicontext-context/**"]
acceptance:
  - Shows project, branch, phase, current task, modified files, and pending tasks
  - Degrades gracefully outside a Git repository
  - Warm run under 300 ms
scope_notes:
  - 'crates/aicontext-context/** was added to touches on developer approval, and with it one new public
    reader. The register''s current phase and current task are body fields, not front matter - the
    schema says so deliberately - and the CLI already depends on aicontext-context for doctor, so the
    alternative was a second Markdown reader in the CLI, which RULES 2 forbids: one fact, one source.
    The amendment is additive: aicontext-context gains the public register module with Register,
    TaskEntry, and read(), and its existing doctor checks are re-expressed on a shared private body
    parser with no change to their public results'
done:
  - 'Two new modules and one shared one. crates/aicontext-context/src/register.rs is the public
    reader: read(root) never fails, returning a default Register when TASKS.md is missing or
    unreadable, because a status command run before init must describe an absent register rather than
    abort; current phase and current task are read from the **Label:** body lines the schema documents,
    and every fenced entity whose id starts with TASK- is a TaskEntry'
  - 'crates/aicontext-context/src/body.rs is the private inline-entity parser both register and doctor
    now use - fenced-block extraction, section ends, entity headings, inline references - so the CLI
    did not grow a second copy of the rules in RULES 2. It is pub(crate); doctor''s public findings,
    codes, and messages are unchanged and its suite still passes without edits'
  - 'crates/aicontext-cli/src/status.rs is the command. Project name and root come from crate::project,
    the register from aicontext_context::register, and the branch and file changes from
    aicontext-git''s Repository::snapshot. Acceptance 1: the human report carries a product, project,
    branch header; the phase and current task; the changed files with their state; and the pending
    count, ending in the DESIGN 6 summary line, the exit code, and one next command'
  - 'Acceptance 2: outside a Git repository the report says there is no repository and still prints the
    register, and a directory that is not a repository is read as the VCS-absent state rather than an
    error - GitError::NotARepository is the clean "none", and any other git failure becomes a warning
    finding (STATUS-001 for an unreadable repository, STATUS-002 for an unreadable working tree) so a
    partial answer is never a crash. An unusable root stays the PROJ-001 usage error, exit 2'
  - 'The --json shape follows CLI_SPEC 6 and 8: data carries root, project, vcs {kind, branch, head,
    detached, changes}, current_phase, current_task, and tasks {total, done, pending, pending_ids}, so
    the documented jq -r ''.data.current_task.id'' resolves. A JSON write failure is STATUS-003 and
    exit 1'
  - 'Acceptance 3: the warm run over this repository measured 175 ms and 191 ms wall clock on the
    second and third invocations against the debug binary, under the 300 ms budget'
  - 'crates/aicontext-cli/tests/status.rs holds 7 end-to-end tests through the real binary, each
    spawning it with the fixture''s hermetic environment so the child git reads only the fixture: a
    full report in a clean repository, the JSON current-task id and counts, graceful degradation
    outside a repository, a modified and an untracked file, a branch in a repository with no commits,
    the closing next-command and exit line, and an unusable root as PROJ-001. The register is committed
    in the clean-tree case so the assertion is about a clean tree and not an untracked fixture'
  - 'The four tests in tests/cli.rs that used status as their example of a not-yet-built command were
    moved to the still-pending health (TASK-038), because status is now in the help tree and would have
    made those tests assert the opposite of the truth'
  - 'Two CI defects were fixed while this task was open. In crates/aicontext-git, inside its touches,
    the Unix-only test for normalise in src/porcelain.rs had no #[test], so it never ran and the Linux
    build treated it as dead code; the attribute was added and the defect filed as BUG-002. In
    crates/aicontext-testkit, outside its touches and approved by the developer, a public doc comment
    linked to a private item and failed the verify job''s cargo doc under -D warnings; the link became
    prose and the defect filed as BUG-003. Both prevention steps are the same shape: run the gate with
    the flags and the target CI uses, not the ones that happen to pass on the local machine'
  - 'Verification: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked
    -- -D warnings clean; cargo test --workspace --locked green - 101 unit and 8 + 23 + 17 + 5 + 7
    integration in aicontext-cli, 82 unit in aicontext-context with its 7 fixtures, 43 frontmatter, and
    4 doc tests, and every other crate''s suite'
```

### TASK-014 — Implement aicontext doctor (v1)

```yaml
id: TASK-014
title: Implement aicontext doctor
status: DONE
priority: HIGH
phase: 1
depends_on: [TASK-011, TASK-015, TASK-016]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**", "crates/aicontext-cli/**"]
acceptance:
  - Detects a missing AI.md, invalid front matter, and dangling document references
  - Detects a task pointing at a missing specification and a deprecated ADR with no successor
  - Reports a contradiction between ARCHITECTURE.md and the detected stack
  - Exit code 3 when any error-level finding is present; --json emits findings with codes
done:
  - 'CTX-001, CTX-002, CTX-007, CTX-012, CTX-013, CTX-014, and CTX-018 are implemented. The other
    thirteen codes in the catalogue are reported as deferred by --explain, each naming what it waits
    for, so nobody reads a missing check as a passing one'
  - 'The checks live in aicontext-context and take an Inputs struct carrying the root, the languages
    discovery observed, and the embedded schema bytes. Parsing, human and JSON rendering, and exit
    mapping stay in aicontext-cli, which is the same split docs/ARCHITECTURE.md 2.1 records'
  - 'Human output lists findings by severity then code, says what was checked afterwards, and exits 0,
    3 for findings, or 2 for a flag or directory it cannot use. JSON emits the CLI_SPEC section 6
    envelope with a checked array beside the findings'
  - '--strict raises warnings to errors and informational findings to warnings, so the exit code moves
    only when the report itself is stricter; --only takes a case-insensitive code prefix and an empty
    value is a usage error; --explain distinguishes implemented, deferred, and unknown codes, and an
    unknown one exits 2 rather than printing an empty rationale'
  - '--rebuild-index is accepted and says there is no cache yet, because the index arrives with
    TASK-031. An unimplemented command still exits 2 naming its task; doctor is no longer in that tree'
  - 'init and doctor now share crate::project for root resolution, project name, and path display, so
    the two commands cannot disagree about which directory they are reading or how it is printed'
  - 'Run over this repository the command found four bugs in itself and one in its own input, all
    fixed: a YAML comment inside a fenced block ended its own section one line early, so the block
    after it was never read; the H1 that repeats a standalone decision id was reported as an inline
    entity heading with no block, failing all seven ADRs; a document could be listed as checked and
    carry a finding from a tree-wide check that ran later; a project with no schema copies produced
    twenty identical lines where one directory-level finding is the true report'
  - 'The four malformed YAML blocks in this file were found by the same run, in TASK-011, TASK-012,
    TASK-015, and TASK-016, and were repaired in place by quoting the values that YAML was reading as
    mappings or comments. The register is 41 tasks and every block parses'
  - '346 tests across the workspace, of which 82 unit and 43 integration in aicontext-context and 23
    end-to-end in crates/aicontext-cli/tests/doctor.rs. fmt, clippy -D warnings on every target, the
    locked workspace suite, cargo doc, and cargo audit over 123 locked dependencies all pass'
  - 'Deferred on purpose: CTX-003 needs the runtime Validator, CTX-017 and CTX-019 need the index from
    TASK-031, and CTX-004 through CTX-006 need schema comparison that depends on CTX-003'
```

### TASK-015 — Define the JSON Schema set

```yaml
id: TASK-015
title: Define the JSON Schema set
status: DONE
priority: HIGH
phase: 1
depends_on: [TASK-011]
spec: null
touches: ["schemas/**", "crates/aicontext-cli/**", "Cargo.toml", "Cargo.lock"]
acceptance:
  - A schema for every document type and fixed-path config listed in CONTEXT_SPEC section 1,
    plus action-proposal and project-profile schemas
  - Validated with a Draft 2020-12 meta-schema check in CI
  - init copies them to .ai/schemas and doctor verifies the copies match the source (CTX-012)
waived: true
done:
  - 'schemas/ holds 20 self-contained Draft 2020-12 schemas: one per document kind and fixed-path config in CONTEXT_SPEC 1, plus action-proposal and project-profile. No schema refers to another file, because no file-fetching resolver is enabled in this project'
  - crates/aicontext-cli/tests/schemas.rs asserts the exact file set, the declared dialect, and
    every schema against the 2020-12 meta-schema, then checks that no $ref leaves its own file and
    that every $id is unique and matches its file name. The meta-schema check was confirmed to
    have teeth by breaking minItems in task.schema.json and watching it fail by name
  - jsonschema 0.58.3 is a dev-dependency with default-features = false, so CI validates without
    pulling the resolve-http/resolve-file/TLS tree; nothing shipped in the binary links it. It
    raised the workspace from 75 to 123 crates, and cargo audit reports no vulnerabilities
  - init embeds the schema set with include_str! and copies all 20 into .ai/schemas/. A scaffolded
    project receives exactly the bytes CI validated, and a unit test compares the embedded text
    against schemas/ so the two cannot drift
  - 'Verified by hand: a fresh init writes 31 documents including the 20 schemas, all byte-identical to the source; a second run reports 39 unchanged and writes nothing; an edited schema is preserved and reported as CTX-017 like any other document'
  - Corrected docs/CONTEXT_SPEC.md, which showed ARCH-NNN, TASKS-NNN and MEMORY-REG as valid
    document ids that core rejects, and reclassified five memory entries that used a `decision`
    category the enum does not define
  - 'R-2 is answered by splitting the claim: the dev-dependency costs build time and no binary weight, and the runtime Validator decision is deferred to TASK-014 where it is first needed'
```

**Why the third acceptance criterion is waived.** It requires `doctor` to verify the copied schemas
(CTX-012), and `doctor` is TASK-014, whose `depends_on` includes TASK-015. The criterion is therefore
unsatisfiable from inside this task: doctor cannot be built until this task is `DONE`, and this task
cannot be `DONE` until doctor verifies the copies. Rather than close the task against an unmet
criterion silently, the deviation is recorded here. CTX-012 is registered in CONTEXT_SPEC §7 and
verifying the copy is TASK-014's own acceptance criterion, so nothing is lost by moving the check
there — it is only recorded in the wrong task.

### TASK-016 — Implement the front matter parser

```yaml
id: TASK-016
title: Implement the front matter parser
status: DONE
priority: CRITICAL
phase: 1
depends_on: [TASK-011]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Parses a leading YAML block into a typed Document
  - Never parses Markdown body structure
  - Malformed input returns a typed error with a line number, never a panic
  - Round-trip and property tests over generated documents
done:
  - Document::parse splits the leading --- block and never looks inside the body; FrontMatter holds
    the well-known keys typed and keeps type-specific keys in extras, so rule 5 loses nothing
  - 'type and status stay strings, and dates stay validated YYYY-MM-DD strings: the vocabulary belongs to TASK-015 and a date type belongs in aicontext-core, so neither was invented here'
  - 64 KiB block and 1 MiB document caps are hard refusals, per RULES 11; the catalogue rates
    CTX-018 a warning, but a parser has no document to attach a warning to
  - 'ContextError is #[non_exhaustive], every variant carries a line, a stable code and a remediation hint, and converts into AicontextError with the cause chain intact'
  - yaml_serde 0.10 is reached only through a private two-method YamlCodec, so no YAML type is
    public; this is R-1's mitigation, implemented rather than noted
  - A repeated key, a non-string key, a tag, a non-finite float, and an integer beyond i64 are each
    refused by name rather than coerced, dropped, or rounded
  - 'render() is fallible: a value with no YAML spelling is reported instead of written as an empty block, which would delete a document''s metadata while leaving a file that still parses'
  - 'tags: [] and an absent tags key survive a round-trip as the different facts they are'
  - 'A bare CR, NEL, LS, or PS in a block is refused by name (MEM-011): YAML counts all five as line breaks, so the reported line would land past the end of the file. CRLF files still parse'
  - 'CONTEXT_SPEC 2.1 inline entity blocks are deliberately not read: that needs Markdown structure, which this task forbids, and it belongs to doctor in TASK-014, which already walks bodies'
  - '`Value` is `#[non_exhaustive]`, so it cannot be constructed outside this crate: the only source of one is a parsed document, and the set of values in memory is exactly the set the parser can produce. Nothing in phase 1 needs to build a document by hand, and a later variant is additive'
  - A test scans `src/` and fails if `yaml_serde` reaches any module but `codec.rs`, so R-1's
    mitigation is enforced rather than asserted; it was checked by planting a reference and
    watching it fail
  - Round-trip fixtures are built from this file's own `Shape` type rather than the crate's `Value`,
    so the property compares two independent descriptions of the format instead of agreeing with
    itself. Floats are included, generated with a `u32` whole part and a short fraction so the
    fixture and the renderer cannot disagree about notation
  - 'A float keeps its decimal point across a rewrite (`phase: 2.0` stays a float, does not return as an integer); CONTEXT_SPEC §2 rule 7 now states that a value''s type survives a rewrite'
  - 89 tests in the crate (42 unit, 43 integration of which 7 are properties, 4 doc) and 152 across
    the workspace; the properties were also run at 50,000 cases each, and the whole suite passes on
    the declared MSRV toolchain 1.85, not only on current stable
  - '`cargo audit` reports no vulnerabilities across the 57 locked dependencies'
```

**Dependency: `yaml_serde` 0.10.** `RULES.md` §6 requires writing the ~30 lines a dependency
replaces. This is what that would have been — a reader for the subset of YAML our own files happen
to use:

```rust
fn read_block(text: &str) -> Result<Vec<(String, String)>, ContextError> {
    let mut pairs = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (key, value) = line.split_once(':').ok_or(ContextError::Malformed {
            line: index + 2, reason: "expected `key: value`".into(),
        })?;
        pairs.push((key.trim().to_string(), unquote(value.trim()).to_string()));
    }
    Ok(pairs)
}

fn unquote(value: &str) -> &str {
    value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value)
}
```

That is genuinely less code, and it is wrong in ways our own documents would hit. It cannot hold a
value that is not a flat scalar, so `depends_on: [TASK-014, TASK-015]` — a list of strings, in
`extras`, in the first week — has nowhere to go. It resolves a duplicate key by last-one-wins, which
`rule 5` forbids outright. `a: b: c` splits on the first colon and yields the key `a`, which is a
YAML syntax error. It cannot tell `title: 2026-09-27` (a date) from `title: 0755` (an octal integer
in YAML 1.1) or from `title: no` (a boolean), so the type of every value is a guess. A `#` inside a
quoted string starts a comment and truncates the value. And it has no line numbers beyond the one it
just read, which is the one thing `ContextError` exists to provide.

The decisive argument is not line count. `docs/CONTEXT_SPEC.md` §2 is a **published storage
contract** that an external tool reads without running our binary. A subset parser makes `.ai`
documents readable only by us, so "we accept slightly different YAML" would become a permanent,
invisible fork of the format the spec claims to be YAML. `ADR-007` records the full candidate
comparison; `MEM-010` records the dependency state.

### TASK-017 — Implement the Git wrapper

```yaml
id: TASK-017
title: Implement the Git wrapper
status: DONE
priority: HIGH
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["crates/aicontext-git/**", "crates/aicontext-core/src/error.rs"]
acceptance:
  - Branch, status, changed files, and recent commits via porcelain commands
  - Works outside a repository and on a repository with no commits
  - Arguments are passed as an argv array, never through a shell
  - Behaviour is covered by tests against real temporary repositories
scope_notes:
  - 'aicontext-core/src/error.rs was added to touches on developer approval. AicontextError requires an
    ErrorCode, ErrorFamily had only Context, and no GIT catalogue existed, so a GIT-NNN code could not
    be produced from this crate at all. The amendment is additive: ErrorFamily is #[non_exhaustive], so
    adding a variant cannot break a caller that matches on it, and the module already documents that a
    family is added by the task introducing its failure domain'
  - 'thiserror was approved as a dependency of aicontext-git, required by RULES 4.2 and already in
    Cargo.lock'
  - 'GIT-005 was originally "the index is held by another process" and was replaced on approval with
    "the git process was terminated by a signal". The original was unreachable: with .git/index.lock
    present, status --porcelain=v1, status --porcelain=v2, diff --name-only HEAD, log, and
    branch --show-current were all measured exiting 0 on git 2.51. Every command this wrapper offers
    is read-only and none acquires the index lock'
  - 'crates/aicontext-testkit/src/repo.rs was added to touches on developer approval. Acceptance 4
    needs tests that spawn the same git binary the fixture resolved and the same hermetic environment
    it applies; re-deriving either inside aicontext-git would re-introduce exactly the machine
    dependence TASK-018 exists to remove. The change is additive: two new public accessors on
    TempRepository (git_program, environment) and its private command() refactored to consume the one
    list, so the fixture has a single definition of its environment instead of two that can drift. No
    dependency was added'
done:
  - 'Five modules: git.rs spawns git with an argv array and captures stdout and stderr; porcelain.rs
    reads branch and status; query.rs reads the log; error.rs is the GIT-001 to GIT-006 catalogue;
    lib.rs is the documented public surface'
  - 'Public read-only API: Git::new, Git::with_environment and Git::open, Repository::branch,
    Repository::changed_files, Repository::snapshot, Repository::recent_commits(limit), and the Head
    (Branch, Unborn, Detached), Change, FileStatus, and Commit types'
  - 'Acceptance 1: branch, status, changed files - with the index, worktree, and unmerged halves kept
    distinct - and recent commits are all read from porcelain, not from the human-readable output'
  - 'Acceptance 2: a directory that is not a repository is GIT-002 rather than a crash, and an empty
    repository returns the distinct GIT-004 so a caller can tell "no history yet" from a real failure.
    Both are named integration tests, as are a detached HEAD and an unborn branch'
  - 'Acceptance 3: every invocation is an argv array passed to Command, with no shell on any path, so
    a branch name or a commit subject cannot become a command'
  - 'Acceptance 4: crates/aicontext-git/tests/repository.rs holds 12 tests against real temporary
    repositories built by aicontext-testkit - a non-repository, a fresh repository, an untracked file,
    a staged and an unstaged modification, a rename, the fixture commit and a limit, an empty
    repository, a detached HEAD, a subdirectory root, and a second read after a change. Each builds
    Git::with_environment(repo.git_program(), repo.environment()), so the wrapper runs against the same
    hermetic git the fixture uses and never reads the developer''s own configuration'
  - 'The integration suite found a real defect on its first run that the 39 unit tests could not,
    because they fed the parser bytes and the parser was right about the bytes it was handed: git log
    --format appends a newline after every commit, but the tokenizer trusts every byte between NULs to
    be a field, so the newline was read as a seventh token and the stream desynchronised from the
    second commit on. Fixed by adding -z (a NUL record separator) and dropping the trailing %x00; the
    two tests that failed first, recent_commits_reads_the_fixture_commit and
    a_limit_returns_only_the_newest_commits, are the failing-before/passing-after evidence. Recorded
    as BUG-001'
  - 'Hermetic by construction: Git::with_environment clears the child environment and applies only the
    pairs it is given, and an empty value means unset, so GIT_ASKPASS is closed rather than set to the
    empty string. The testkit''s environment() is the single definition of that set, consumed by the
    fixture''s own commands and by the integration tests, so the two cannot drift'
  - 'No new production dependency: thiserror only, already approved. The testkit gained the two
    accessors above and no dependency'
  - 'Verification: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked
    -- -D warnings clean; cargo test --workspace --locked green. aicontext-git: 39 unit, 12
    integration, 1 doc. aicontext-testkit: 48 unit, 10 boundary, 3 doc'
```

### TASK-018 — Build the testkit

```yaml
id: TASK-018
title: Build the aicontext testkit
status: DONE
priority: HIGH
phase: 1
depends_on: [TASK-010]
spec: null
touches: ["crates/aicontext-testkit/**", "crates/aicontext-context/tests/fixtures.rs"]
acceptance:
  - Helpers to build a temporary project, a temporary Git repository, and a sample .ai tree
  - No helper reads the real home directory, the network, or global Git config
done:
  - TempProject owns a tempfile::TempDir, names the root after the project, and refuses an empty or
    separator-bearing name rather than sanitising it, since a rewritten name would let a test pass
    for a project other than the one it created
  - 'TempRepository builds its child environment with env_clear() and pins the four things that make a
    suite machine-dependent: core.autocrlf=false, init.defaultBranch=main, a fixed author and
    committer identity, and fixed dates. Fixed dates are also what make commit hashes reproducible, so
    a test can assert a hash at all'
  - 'GIT_CONFIG_GLOBAL, GIT_CONFIG_SYSTEM, GIT_CONFIG_NOSYSTEM, HOME, USERPROFILE, and XDG_CONFIG_HOME
    all point inside the fixture. Global Git configuration is the acceptance criterion here, so it is
    closed from both ends: the paths are inert and the inherited environment is empty'
  - 'The isolation is asserted by asking git where its configuration came from, with
    `git config --list --show-origin`, rather than by trusting the variables that were set: every
    origin must resolve inside the fixture''s own temporary directory. Pointing GIT_CONFIG_GLOBAL at a
    real ~/.gitconfig was tried during development and the test failed and named the file, so it
    would catch a regression rather than pass vacuously'
  - Every git invocation is an argv array through Command with no shell, matching RULES 7, so a branch
    name or commit message cannot become a command
  - 'clone, fetch, push, pull, remote, and submodule are refused with FixtureError::NetworkRefused
    rather than merely discouraged: a fixture that clones leaves a test that passes online and fails
    offline'
  - 'FixtureError is one #[non_exhaustive] enum carrying a stable code, a message, and a remediation,
    with Display and Error written by hand instead of derived from thiserror. That is the one place
    the RULES 4.2 convention is deliberately inverted, and the reasoning is in src/error.rs'
  - 'sample_ai_tree writes the minimum a clean project needs: an AI.md with no front matter (the one
    exempt location), a TASKS.md with front matter and one fenced inline entity whose spec points at
    a file that exists, and the .ai/schemas directory without contents, since CTX-012 compares against
    a source set this crate cannot see'
  - 'The fixture cannot prove itself: aicontext-testkit may not depend on aicontext-context
    (ARCHITECTURE 3.2), so crates/aicontext-context/tests/fixtures.rs asserts zero findings, asserts
    the checked list is non-empty so an empty tree cannot pass for a clean one, and then deletes
    .ai/AI.md to show the check reports CTX-001 and therefore has teeth'
  - tempfile is the crate's only dependency and was approved for it. The Windows case it solves - a
    just-closed handle keeping a directory from being deleted - is not reachable from
    std::env::temp_dir(). MEM-009 was amended to record that the testkit is no longer dependency-free
  - 'The existing hand-rolled Git test helpers in the CLI were left alone: migrating them is outside
    this task''s declared touches, and they are the consumer of this fixture rather than part of it'
  - 'This block was repaired after the task was closed, and it was the one malformed block TASK-014''s
    repair pass did not catch, because its defect was not a colon: the #[non_exhaustive] in the
    FixtureError entry began a YAML comment and truncated the entry, so the block did not parse and
    doctor reported CTX-002 against TASK-018. The entry and three others containing a colon are now
    single-quoted, and doctor reads the block'
```

### TASK-019 — Add the secret scan to doctor

```yaml
id: TASK-019
title: Add the secret scan to doctor
status: DONE
priority: HIGH
phase: 1
depends_on: [TASK-014]
spec: docs/SECURITY.md
touches: ["crates/aicontext-context/**", "crates/aicontext-cli/**"]
acceptance:
  - Flags credential-shaped keys and values inside .ai
  - Never echoes the matched secret; reports the path, line, and rule only
  - It is an error in the catalogue, so the scan fails the run without --strict
scope_notes:
  - 'crates/aicontext-cli/** was added to touches on developer approval so one implementation is
    visible to the command that runs it: CTX-016 was added to IMPLEMENTED_CODES and to the severity
    table''s test, so --explain and doctor --json no longer name it as deferred. Without the
    amendment the crate would run the check while the CLI advertised it as missing, and RULES 2
    forbids the fact living in two places'
  - 'Acceptance 3 was reworded on developer approval. CTX-016 is an error in the catalogue
    (CONTEXT_SPEC 8) and the CLI already classified it as one, so --strict is not what makes it an
    error; the reworded criterion records that it is one without --strict. The severity is asserted
    against the catalogue in every_severity_matches_the_catalogue_in_section_8'
  - 'The CI half of SECURITY 6 is not here. Its bullet 135 promises a repository secret scan in CI as
    a second, independent check, and .gitignore attributes that scan to TASK-019. It is deliberately
    left out: an independent scanner is a third-party CI action, which the agent contract lists as a
    hard stop needing approval, and the acceptance names only the doctor behaviour. doctor now
    enforces CTX-016 on any project; a CI job that runs an independent scanner over this repository,
    and the .gitignore wording that already claims it exists, are a follow-up for the developer'
done:
  - 'crates/aicontext-context/src/doctor.rs gains check_secrets and a rule set. It walks every file
    below .ai/ - not only the Markdown documents - because init also copies permissions.yaml, and a
    secret leaked there is exactly as public. The walk is bounded by the same MAX_DEPTH and
    MAX_DOCUMENTS as the document reader, and a file past the 1 MiB cap or unreadable as UTF-8 is
    skipped rather than loaded'
  - 'Acceptance 1: the value-shaped rules are aws-access-key-id, private-key-block, github-token,
    slack-token, google-api-key, stripe-secret-key, anthropic-api-key, openai-api-key,
    json-web-token, bearer-token, and password-in-url, tried most specific first. The key-shaped
    rule credential-key fires when a password/secret/token/api_key/... key holds a literal value
    rather than a reference (${VAR}, keychain:...), a stub (changeme), a mask (xxxxxx), or structure'
  - 'Acceptance 2: Finding carries the path; the message carries only the line number and the rule
    name ("credential-shaped content on line N matched the `rule` rule") and never the match; the
    remediation is static. The unit tests assert the secret appears nowhere in the serialized
    findings, and the CLI end-to-end test asserts it appears nowhere on stdout'
  - 'CTX-016 moved out of UNIMPLEMENTED into an Implemented rationale, and the CLI IMPLEMENTED_CODES
    gained it, so doctor --explain CTX-016 and doctor --json name it as a check that runs.
    explain_covers_every_code_a_check_can_emit and
    every_code_the_cli_claims_to_run_is_known_to_the_checks both cover the entry'
  - 'Tests: six unit tests in aicontext-context - a value-shaped secret reported and never echoed, a
    key-shaped literal reported, the line number, references/stubs/masks not reported, a non-Markdown
    file still scanned, and an oversize file skipped - and one end-to-end test in
    crates/aicontext-cli/tests/doctor.rs (a credential in .ai/ is an error, exit 3, and stdout does
    not contain the secret). Every credential-shaped value in a test is assembled at run time, so no
    fixture holds a contiguous credential literal (RULES 7 and 12)'
  - 'Verification: doctor over the dogfooded .ai/ still exits 0 with its one pre-existing CTX-012
    warning and no CTX-016, so the scan does not flag this repository. cargo fmt --all --check clean;
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings clean, and the same
    for --target x86_64-unknown-linux-gnu; cargo test --workspace --locked green; cargo doc under
    RUSTDOCFLAGS=-D warnings clean; cargo +1.85 build --workspace --all-targets --locked green'
```

### TASK-020 — Implement export and import

```yaml
id: TASK-020
title: Implement aicontext export and aicontext import
status: DONE
priority: MEDIUM
phase: 1
depends_on: [TASK-013]
spec: null
touches: ["crates/aicontext-cli/**"]
acceptance:
  - export writes a deterministic archive of .ai with a manifest and per-file digests
  - import verifies digests, reports conflicts, and never overwrites without --force
  - A round trip is byte-identical for a tree that was not edited between the two
done:
  - 'crates/aicontext-cli/src/archive.rs is the format layer: build/render/parse for one
    pretty-printed JSON document (format aicontext-archive, version 1, trailing newline),
    manifest_digest over the sorted manifest, sha256_hex for per-file digests, and safe_relative to
    sanitise and bound relative paths. verify() runs the whole audit up front - format, version,
    headline counts, manifest digest, path shape (no .., ., absolute, empty, colon, or control
    characters; at most MAX_DEPTH components), duplicates, per-file sizes, and digests - so an
    import is fully checked before a single write is planned. The caps are the same order of bounds
    doctor applies: MAX_ENTRIES 512, MAX_DEPTH 16, MAX_FILE_BYTES 8 MiB, MAX_ARCHIVE_BYTES 64 MiB'
  - 'Acceptance 1: export walks .ai/ top-down by sorted name, hashes every file, and writes the
    archive. Nothing about the machine leaks into the bytes - no timestamps, absolute paths, host
    names, or user names - so exporting the same tree twice writes identical bytes and a
    same-named fresh root re-exports the same bytes. The manifest carries the project name, the
    file count, the total bytes, a digest row per file sorted by path, and a SHA-256 digest over
    the whole manifest. A relative <archive> path resolves against the project root, matching how
    doctor and status see the project'
  - 'Acceptance 2: import never plans a write before verify() passes; then it compares each
    archived file against the tree, reports intact files as unchanged, and reports differing
    existing files as conflicts. Conflicts without --force are a refusal (exit 5, IMP-020) that
    writes nothing; with --force they are replaced. Files the archive does not mention are left
    alone - an archive is a merge, not a mirror - and an empty archive imports with a note that it
    holds no files'
  - 'Acceptance 3: import writes verified file contents verbatim as UTF-8, and the export + import
    e2e round trip on an unedited tree is byte-identical'
  - 'Force requires a human: --force outside an interactive terminal is a deny (EXP-006/IMP-004,
    exit 5) and --yes never satisfies it; --dry-run still runs every check and every non-write
    guard, returns the exit a real run would return, and writes nothing'
  - 'Codes: fatal export failures are EXP-001..EXP-014 and import failures IMP-001..IMP-015,
    printed as the stderr error report outside --json and as findings in the shared envelope under
    --json. Archive and data.files entry paths are .ai/-relative; findings and the human report
    spell them .ai/-prefixed'
  - 'Wiring: Command::Export | Import in src/args.rs with ExportArgs/ImportArgs, dispatch arms in
    src/main.rs, and the crate module doc lists them. project.rs exposes vcs_label (shared by
    doctor.rs) so the manifest can name the version-control system without duplicating the
    detection'
  - 'Tests: unit tests in archive.rs (deterministic manifest digest, capacity refusals, path
    sandbox including ../../ escapes, split and appended-content detection, parse round trip) and
    in export.rs/import.rs (writers refuse non-UTF-8, empty tree exports a zero-file archive, byte
    determinism), plus 11 e2e tests in tests/export.rs and 12 in tests/import.rs that run the real
    binary - documented archive shape, existing-output and conflict refusals leave the file
    untouched, force-in-a-pipe refuses, force+dry-run writes nothing, relative paths, deep and
    non-UTF-8 refusals, tampered archives write nothing, and the byte-identical round trip'
  - 'Verification: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked
    -- -D warnings clean on native and on x86_64-unknown-linux-gnu; cargo test --workspace --locked
    green on stable and on the 1.85 MSRV; aicontext doctor over this repository still exits 0 with
    its one pre-existing CTX-012 warning; cargo audit 0.22.2 reports no vulnerabilities'
```

---

## Phase 2 — Context engine

**Gate:** `aicontext context --task <id> --explain` returns a relevant, prioritised, provenance-
annotated packet, deterministically, with no network.

### TASK-030 — Implement the discovery engine

```yaml
id: TASK-030
title: Implement the project discovery engine
status: DONE
priority: HIGH
phase: 2
depends_on: [TASK-012]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Detects language, framework, package manager, tests, CI, containers, IaC, and cloud config
  - Produces an advisory ProjectProfile; never mutates the project
  - Unknown stacks yield an empty profile, not a guess
done:
  - 'crates/aicontext-context/src/discovery.rs is the engine: public discover(root) -> ProjectProfile plus Signal and Confidence. A fixed MARKERS table records what a name implies on its own (languages from manifests, vcs, ci, containers, tests, IaC, cloud hints), a LOCKFILES table maps lockfiles to package managers in priority order, and framework tables scan Cargo.toml, package.json, pyproject.toml/requirements.txt, go.mod, and the JVM builds for known dependencies. Framework config files (next.config.*, vite.config.*, manage.py, and the rest) are evidence without a manifest'
  - 'Acceptance 1: one run detects language, framework, package manager, tests, CI, containers, IaC, and cloud config, each field a sorted, de-duplicated list of findings and each finding backed by a project-relative evidence path. package_manager is the highest-priority lockfile present while every lockfile is still recorded as a signal; vcs is git when .git exists; testing is true when any test-shaped signal was found'
  - 'Acceptance 2: discovery never mutates. It only reads metadata and file contents through fs::metadata, fs::read_to_string, and fs::read_dir, and tests/discovery.rs snapshots the whole tree before and after a run and asserts the two are byte-for-byte equal'
  - 'Acceptance 3: an unrecognised project yields an empty profile with unrecognised: true and confidence: None rather than a guess. Nothing is inferred from a directory name alone, and a root that cannot be listed reads as a root with nothing in it, so discover is infallible - the same deliberate choice register::read makes for a missing TASKS.md (MEM-016)'
  - 'Bounded and deterministic: at most MAX_ENTRIES_SCANNED (512) top-level entries are examined and a manifest past MAX_MANIFEST_BYTES (256 KiB) is skipped rather than truncated (RULES 11); every list is sorted and nothing depends on directory iteration order (RULES 8), so two runs over an unchanged tree are equal (RULES 12)'
  - 'Evidence and confidence are the model, not decoration: every Signal carries kind (the spec field name), value, evidence, and a Confidence, and the profile-level confidence is the weakest signal so one doubtful finding cannot be averaged away by certain ones'
  - 'Tests: 13 unit tests in discovery.rs (empty and missing roots, rust and node and python projects, word-boundary framework matching so react never matches preact, the highest-priority lockfile, case-insensitive terraform found by extension, a medium-confidence tests directory, sort and repeatability, an oversize manifest skipped) and 7 integration tests in tests/discovery.rs through the public API (a fully described rust service, an empty profile, a non-mutation snapshot, a node project with TypeScript and its frameworks, a python project, every field keeps its evidence, two runs agree)'
  - 'Verification: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked green. Scoped to crates/aicontext-context/** - the pre-existing init/detect.rs subset is left in place and rewiring init to the typed profile is a follow-up, not part of this task'
```

### TASK-031 — Build the content-hash index

```yaml
id: TASK-031
title: Build the content-hash context index
status: DONE
priority: CRITICAL
phase: 2
depends_on: [TASK-016]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Indexes .ai documents and respects .gitignore
  - Persists to .aicontext/index.json keyed by content hash
  - A second run over an unchanged tree re-reads nothing and is measurably faster
  - A changed file invalidates only its own entry
done:
  - 'crates/aicontext-context/src/index.rs is the index module: public ContextIndex, IndexEntry, DocumentKind, Heading, ScanReport, IndexError, and the INDEX_PATH constant, all re-exported from lib.rs. ContextIndex::refresh(root, cache) loads the existing cache (ignoring any failure), builds, and writes it back; build(root, previous) returns (index, report); load and save are the two halves on their own; empty, entries, get, len, is_empty, and IndexEntry::content_hash_hex complete the surface. Keyed by repository-relative path, each entry carrying its content_hash - the hash is the invalidation identity, not the map key, so two documents with identical bytes do not collapse into one entry'
  - 'Acceptance 1: the scan walks root/.ai through ignore::WalkBuilder with hidden(false) (so the hidden .ai directory is walked), parents(true), git_ignore(true), git_exclude(true), git_global(false), and require_git(false) (so .gitignore applies without a .git repository and no developer-global ignore file can change the result). Only .md files are documents. docs/CONTEXT_SPEC.md 1 locations are read for DocumentKind (kebab-case serde names); a location that does not answer falls back to the front matter type key. The two fixed-path YAML files (ai.yaml, permissions/permissions.yaml) are config, not documents, and are not indexed'
  - 'Acceptance 2: the cache is .aicontext/index.json, a versioned envelope (version: 1) holding entries and fingerprints. It is pretty-printed with a trailing newline, its parent directory is created on demand, and its path keys use forward slashes so the same bytes are written on Windows and Unix. load returns IndexError::Read, ::Format, or ::Version; refresh swallows those and rebuilds, so a corrupt or foreign cache can never be trusted - it is a cache and is fully derivable from the repository (RULES 4.10)'
  - 'Acceptance 3: incrementality is decided by a (size, mtime-in-nanoseconds) fingerprint held in a private fingerprints map. When the fingerprint still matches, the cached entry is reused verbatim and the file is never opened. When it moved, the file is read and hashed: an unchanged hash keeps the parsed entry and counts a rehash, a changed hash rebuilds one entry and counts a parse. tests/index.rs asserts a second scan of an unchanged tree reports reads() == 0 and parsed == 0, and writes byte-identical output - a deterministic proxy for measurably faster, because RULES 8 forbids asserting on the wall clock'
  - 'Acceptance 4: a changed file invalidates only its own entry. tests/index.rs edits one document, rescans, and asserts exactly one parse, one reuse, the changed hash moved, and the untouched entry is bit-for-bit the previous value. A file removed from the tree is counted in report.removed and absent from the rebuilt index'
  - 'Honest partial failure (RULES 4.8): ScanReport counts documents, reused, rehashed, parsed, removed, oversize, and unreadable. A document larger than DOCUMENT_MAX_BYTES (1 MiB) is counted oversize and skipped, matching what the parser accepts (CTX-018), and a path the walk or the read cannot deliver is counted unreadable rather than silently treated as indexed. Nothing aborts the scan'
  - 'The entry records what the body exposes without interpreting it (ADR-003): front matter through the crate Value model (Value gained serde Serialize/Deserialize, untagged, and FrontMatter::ordered_entries became pub(crate) to reuse the canonical key order), ATX headings ignoring fenced blocks, a heuristic token estimate, and the ID-shaped links found in front matter and body. IndexError reports CTX-019 (Index cache is stale or missing) at Info severity and converts into AicontextError, mirroring ContextError'
  - 'Dependencies, each pre-justified in ARCHITECTURE.md 2.2 and commented in both Cargo.toml files: sha2 for the content hash, ignore for a gitignore-respecting walk, and serde plus serde_json for the cache. ignore 0.4 is pinned to the newest release that keeps the 1.85 MSRV floor. No aicontext-providers dependency is added and no unsafe code enters the crate'
  - 'Tests: 15 integration tests in crates/aicontext-context/tests/index.rs through the public API - a fresh scan indexes every document under .ai and nothing outside it, an empty project, a gitignored document excluded while its neighbour is kept, persistence and an identical reload, a no-read second scan, byte-identical rewrite, one changed entry invalidating only itself, removal, a corrupt cache rebuilt, a foreign-version cache rebuilt, kinds from location, front matter and headings and links recorded, a document without front matter still indexed, and an oversize document counted and not indexed'
  - 'Verification: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked green (544 passed, 0 failed). Scoped to crates/aicontext-context/** - wiring the cache to a command (doctor --rebuild-index and CTX-019 reporting, and the aicontext context command in TASK-037) is a follow-up outside this task, as is rewiring init/detect.rs'
```

### TASK-032 — Define the retrieval plan

```yaml
id: TASK-032
title: Define the retrieval plan
status: DONE
priority: HIGH
phase: 2
depends_on: [TASK-031]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - A RetrievalPlan derives mandatory documents, graph links, filters, and globs from the task
  - The plan is inspectable via --explain and is fully serialisable
done:
  - 'crates/aicontext-context/src/retrieval.rs is new: RetrievalPlan, ContextBudget, FrontMatterPredicate, PredicateOp, Tier, PlanOptions, plus constants MANDATORY_ENTRY_POINT, MANDATORY_RULES, DEFAULT_MAX_TOKENS. All are re-exported from lib.rs. The plan holds task, query, mandatory (AI.md and RULES.md are always present; include paths are de-duplicated and order-preserved), linked_ids (spec + depends_on + relates_to), globs, filters, include_kinds, exclude_ids, and budget. RetrievalPlan::new derives the mandatory set from options; RetrievalPlan::for_task seeds from the task and derives edges/filters/globs when the register knows that task.'
  - 'register.rs extended TaskEntry with phase (i64), epic, spec, depends_on, touches, relates_to, tags; the reader extracts them from the task entity block. This keeps one reader of the register format (register::read) and exposes the graph keys the plan needs (TaskEntry is public).'
  - 'FrontMatterPredicate compares using string projection (a document value is equal/contains against a predicate string by normalising Int/Bool/Null/Str; lists are searched for equal items, strings by substring), so command-line --filter values work without constructing Value. matches_document maps a Document to its canonical fields via ordered_entries. PlanOptions captures caller flags (--query, --include, --exclude, --globs, --filters, --include-kinds, --budget) and for_task combines them with task-derived facts (epic and phase become equality filters; globs and linked_ids merge with task.touches/spec/depends_on/relates_to). explain() prints every field deterministically (including (none) markers) so --explain is inspectable and comparable.'
  - 'DocumentKind gains Display (kebab-case matching serde rename_all) so explain can render kinds; a new test asserts Display == serde JSON wire names.'
  - 'Tests in crates/aicontext-context/tests/retrieval.rs (13 tests) cover derivation for a known and unknown task, options merging, JSON round-trip (including per-tier budget), budget defaults/overrides, explain rendering, predicate operators (Equals, NotEquals, Contains, NotContains, Exists, NotExists), integer equality by decimal, string substring contains, list contains, and Tier bounds. tests/index.rs gains one Display/serde agreement test. All pass.'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked 558 passed, 0 failed. The task''s public API additions are confined to crates/aicontext-context/**; no CLI wiring yet (the --explain flag wiring will land with TASK-037), no document-graph resolution beyond seeds (resolution stays in TASK-034).'
```

### TASK-033 — Implement the lexical retriever

```yaml
id: TASK-033
title: Implement the lexical retriever
status: DONE
priority: CRITICAL
phase: 2
depends_on: [TASK-032]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - BM25 over headings and body with per-field boosts, fully deterministic
  - No network, no embeddings, no model call
  - Every result carries a score and a human-readable reason
done:
  - 'crates/aicontext-context/src/retrieval.rs gains the retrieval result types and the retriever: ScoredDocument (id, path, tier, score, reason, trace), RetrievalTrace (the docs/CONTEXT_SPEC.md 7.3 shape - id, tier, score, reason, source_path, included, dropped_reason), the pure Retriever trait, and LexicalRetriever. All are re-exported from lib.rs. The trait matches ARCHITECTURE.md 4.2 exactly: fn retrieve(&self, plan: &RetrievalPlan, index: &ContextIndex) -> Vec<ScoredDocument>, so the Always/Linked/Filter/Glob retrievers named in ARCHITECTURE.md 6 (TASK-034) are additional impls, not a redesign, and the reserved seam in ARCHITECTURE.md 12 is honoured.'
  - 'LexicalRetriever is BM25 with k1 = 1.2 and b = 0.75 (docs/CONTEXT_SPEC.md 7.2) over a field-weighted document. Fields and weights: title 3.0, id 2.5, tags 2.0 (the front-matter tags key), headings 1.5, other front-matter values 1.2, body 1.0. A term''s weighted frequency is the sum over fields of weight x count, and the document''s weighted length is the sum over fields of weight x token count, so a title term is both commoner in the numerator and counted longer in the denominator. A non-negative IDF, ln(1 + (N - df + 0.5)/(df + 0.5)), is computed per query term over the same documents the results come from. tokenize() lowercases ASCII alphanumerics and hyphens and drops every other character, for both the query and each field.'
  - 'Every returned document carries a positive score and a human-readable reason naming the matched terms ("lexical match on ''term'', ''term''"); a document that matches no query term is not returned. Results are ordered by score descending, then id ascending, then path ascending - a total order with no randomness and no dependence on hash-map iteration order, so the same plan and index always produce the same candidates (ADR-004). The retriever reads only &ContextIndex: no network, no embeddings, no model call.'
  - 'Acceptance 1 (BM25 over headings and body, per-field boosts, deterministic): the new integration tests build a real .ai tree, index it through ContextIndex::refresh, and assert a title match scores strictly above a body match, a document with no matching term is dropped, a tie is broken by identifier then path, and two runs produce bit-identical scores. Acceptance 2: the retriever reads only the index and imports nothing that could reach a model. Acceptance 3: every result is asserted to have score > 0 and a reason that names the matched term.'
  - 'The body text is carried in the index so the body weight is real: IndexEntry gains a body: String field (docs/CONTEXT_SPEC.md 6), serde-defaulted and populated from the parsed body at build time. Because that changes the cached shape, CACHE_VERSION in index.rs is bumped 1 -> 2, so a version-1 cache is rebuilt rather than misread as an all-empty body; the serde default lets a v1 cache deserialize and fail the version check with the precise IndexError::Version instead of a generic Format error. tests/index.rs asserts the cache names version 2, and docs/CONTEXT_SPEC.md 6 now lists the body field and the version rule.'
  - 'Tests: 4 new integration tests in crates/aicontext-context/tests/retrieval.rs (title outranks body, tie broken by identifier then path, determinism across two runs, no query yields nothing) alongside the 13 plan tests from TASK-032. They live in tests/ so they reach only the public API.'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean (the trivially-copy-pass-by-ref, match-same-arms, cast-precision-loss, and float-cmp findings are all resolved); cargo test --workspace --locked 562 passed, 0 failed. Scoped to crates/aicontext-context/**; the graph, filter, and glob retrievers remain TASK-034, the priority ladder TASK-035, and the CLI wiring (aicontext context, --explain) TASK-037.'
```

### TASK-034 — Implement the linked, filter, and glob retrievers

```yaml
id: TASK-034
title: Implement the linked, filter, and glob retrievers
status: DONE
priority: HIGH
phase: 2
depends_on: [TASK-032]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Linked follows the task graph: spec, decisions, bugs, touched paths
  - Filter matches front-matter predicates; Glob matches repository paths
  - A broken link is reported, not silently skipped
done:
  - 'The plan now remembers how each edge was reached: retrieval.rs gains LinkedEdge (Spec | DependsOn | RelatesTo, Display renders the YAML key) and LinkedId { id, edge } with spec/depends_on/relates_to constructors, and RetrievalPlan.linked replaces linked_ids: Vec<String> with Vec<LinkedId>. absorb_task keeps the order spec, depends_on, relates_to and deduplicates first-occurrence-wins; explain() renders "id (edge)"; docs/CONTEXT_SPEC.md 7.1 shows the LinkedId shape.'
  - 'ContextIndex gains resolve(target): the entries whose front-matter id equals target, whose repository-relative path equals target, or whose headings define target as an inline entity (so a depends_on on an inline task resolves to .ai/TASKS.md, and a MEM-* id to .ai/MEMORY.md). Pure, path-ordered, deterministic (ADR-004): an id names its document, it is not matched against whatever mentions it.'
  - 'LinkedRetriever follows the task graph: each resolved edge scores 1.0 with a reason reading "linked from TASK-034 (depends_on)" and the tier of the document''s kind. An edge that resolves to nothing is reported, not skipped: a dropped candidate with trace.included = false, dropped_reason = "broken link", score 0.0, and the tier its target would have sat at (spec -> 4, depends_on -> 5, relates_to -> 9), matching doctor''s CTX-007 honesty and RULES.md 4.8.'
  - 'FilterRetriever keeps every document satisfying all plan.filters (a conjunction) with a reason naming each matched predicate ("matches phase = 2, status exists"); GlobRetriever keeps every document whose canonical /-separated path matches a plan glob. The glob matcher is our own deterministic one - * within a segment, ** across whole segments (zero or more), ? for one character - so no glob crate and identical results on every host (ARCHITECTURE.md 2.2). Every match scores 1.0; all three exact retrievers share candidate_for/order_candidates and order by score descending, then id, then path, the same total order as the lexical retriever.'
  - 'Tests: 7 new integration tests in crates/aicontext-context/tests/retrieval.rs (24 total) covering spec/dependency/bug resolution, inline-register resolution by heading, broken links reported as dropped candidates at the right tier, filter conjunction, glob path matching, glob segment boundaries (*, **, ?), and empty-plan behaviour. The three TASK-032 plan-shape assertions moved to the LinkedId shape, and explain asserts "docs/CONTEXT_SPEC.md (spec), TASK-031 (depends_on), BUG-001 (relates_to)".'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked 569 passed, 0 failed (was 562). Scoped to crates/aicontext-context/**; docs/CONTEXT_SPEC.md 7.1-7.2 updated for the plan shape and the exact retrievers. The priority ladder and conflict detection remain TASK-035, the budget TASK-036, and the CLI wiring (aicontext context, --explain) TASK-037.'
```

### TASK-035 — Implement the priority ladder and conflict detection

```yaml
id: TASK-035
title: Implement the priority ladder and conflict detection
status: DONE
priority: HIGH
phase: 2
depends_on: [TASK-033, TASK-034]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**"]
acceptance:
  - Tiers 1-9 are applied exactly as ARCHITECTURE.md section 6 defines
  - A lower-tier contradiction loses, is recorded in the packet, and is reported by doctor
  - The higher-tier document is never rewritten
done:
  - 'crates/aicontext-context/src/resolve.rs is new and is the single home for the ladder and its one conflict rule. PriorityResolver::tier_for_kind maps every DocumentKind to its rung exactly as ARCHITECTURE.md section 6 and docs/CONTEXT_SPEC.md 7.4 name it; tier_of reads an IndexEntry and defaults an unclassified document to tier 8 (source). The mapping moved out of retrieval.rs, whose private tier_for_kind is deleted, and both the lexical and the shared exact-retriever candidate builder now call PriorityResolver::tier_of, so there is one ladder and not two (RULES.md section 2).'
  - 'PriorityResolver::conflicts(&ContextIndex) compares scalar front-matter assertions over ASSERTION_KEYS (store, database), the one place this project-wide-key policy is written. A key asserted once, asserted by documents that agree, carried as a list or map, or written as an explicit null is not a conflict. The winner is the highest-priority rung, which the ladder numbers as the smallest tier; a lower tier (a larger number) that disagrees is reported as Conflict { key, asserted_by, conflicts_with, resolution: higher-tier-wins }. Same-tier disagreement is never reported: a same-tier tie-break on the smallest id only decides which assertion the comparison is made against. The output is a total order (key, winner tier, winner id, loser tier, loser id), so the same index always yields the same list (ADR-004). Assertion carries path with serde skip, so asserted_by and conflicts_with serialize as the 7.6 {tier, id, value} shape while doctor can still point at a file.'
  - 'doctor reports the conflict as CTX-013, the existing code generalized from "ARCHITECTURE.md contradicts the discovered project profile" to "a lower tier contradicts a higher tier". check_conflicts builds the same ContextIndex the retrievers use, so "the same key asserted twice" means the same thing in the report and in a packet, and emits a warning naming the key, both ids, both values, and both tiers, against the loser''s path, with a remediation saying the higher tier wins and is never rewritten. check_architecture is the structural half of the same code; --explain CTX-013 now describes both halves.'
  - 'Acceptance 1 (tiers 1-9 exactly as ARCHITECTURE.md section 6): a unit test asserts each DocumentKind maps to its rung, and an integration test builds a tree with one document per kind and asserts PriorityResolver::tier_of returns that rung, plus an unclassified document at tier 8. Acceptance 2 (a lower-tier contradiction loses, is recorded, and is reported): the 7.6 example is reproduced end to end - ARCHITECTURE.md at tier 3 asserting database: mongodb and CHG-004 at tier 9 asserting postgresql - and the conflict is asserted to name the winner and the loser, to serialize into the packet shape, and to be reported by doctor::run as CTX-013 against the loser. Acceptance 3 (the higher-tier document is never rewritten): conflicts is a pure function over the index and writes nothing, and the test reads the winner back and asserts its bytes are unchanged.'
  - 'Tests: 3 unit tests in resolve.rs (every kind has its rung, only scalars name an assertion, the assertion-key list) and 9 integration tests in the new crates/aicontext-context/tests/resolve.rs (kind to rung through a real index, an unclassified document at source, the 7.6 conflict, agreeing documents, same-tier disagreement, a list-valued key, determinism across two runs, the packet JSON shape without path, and doctor reporting CTX-013). Tests live in tests/ so they reach only the public API.'
  - 'Docs: docs/CONTEXT_SPEC.md 7.4 now defines an assertion (a scalar under a project-wide key) and names ASSERTION_KEYS, and records that the task-spec-versus-feature category is not yet detected because no document carries a feature key; the 8 CTX-013 row is generalized to "a lower tier contradicts a higher tier". The feature-mismatch check is deferred as underspecified, not dropped.'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked 581 passed, 0 failed (was 569). Scoped to crates/aicontext-context/**; the budget remains TASK-036 and the CLI wiring (aicontext context, --explain) TASK-037.'
```

### TASK-036 — Implement the context budget

```yaml
id: TASK-036
title: Implement the context budget and degradation
status: DONE
priority: HIGH
phase: 2
depends_on: [TASK-035]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-context/**", "crates/aicontext-core/src/error.rs", "docs/CONTEXT_SPEC.md"]
acceptance:
  - Per-tier token caps with a documented degradation order from the lowest tier upward
  - Tiers 1-2 are never dropped; a budget too small for them is an explicit error
  - The report states what was dropped and why
done:
  - 'crates/aicontext-context/src/budget.rs is new and is the single home for docs/CONTEXT_SPEC.md 7.5. ContextBudget (max_tokens, per_tier, reserved_policy_tokens) and DEFAULT_MAX_TOKENS moved out of retrieval.rs, and lib.rs re-exports them from budget, so the public path is unchanged; retrieval.rs keeps only the plan''s budget field and imports the type. The degradation floor is the whole protected half of the ladder, tiers 1-5: only tiers 6-9 are droppable, cut in the order 9 -> 8 -> 7 -> 6, and within a tier the lowest score first, with id then path as a total tie-break (ADR-004). This is deliberately stricter than the acceptance wording''s tiers 1-2, and satisfies it.'
  - 'ContextBudget::apply prices each candidate from the index''s token_estimate by its portable path, so a budget charges the same heuristic doc §6 already publishes and invents no second number; apply_with is the identical rule with an explicit cost function, the seam ARCHITECTURE.md §12 reserves for a per-model tokenizer. A candidate retrieval already excluded is not budgeted, and its dropped_reason is left untouched. A budget that cannot hold tiers 1-5 is refused, not truncated: BudgetError::TooSmall, or TierTooSmall when a protected tier exceeds its per_tier cap, both reported as CTX-030 with an actionable remediation.'
  - 'BudgetReport { requested, used, dropped: [DroppedDocument { id, tier, reason }] } is the doc §7.6 budget block. apply writes each cut candidate''s trace with included = false and dropped_reason = "budget", so the packet''s per-item trace and the budget report agree. reserved_policy_tokens is documented as satisfied by construction (tiers 1-2 are kept unconditionally) and reserved for the per-model cost policy of a later phase (§12), rather than given invented semantics.'
  - 'CTX-030 is added to aicontext-core''s ErrorCode catalogue and to the docs/CONTEXT_SPEC.md §8 table with a note that it is raised by aicontext context and never by doctor, so doctor --explain does not describe it (the Explanation design keeps --explain to doctor checks). §7.5 was clarified for per-tier caps and reserved_policy_tokens. Tier gained a Display impl for the error message.'
  - 'Tests: 7 unit tests in budget.rs drive the pure ordering and admission logic through apply_with (protected kept and history cut bottom-up, lowest score first within a tier, a budget below the floor is CTX-030, a protected tier over its cap is CTX-030, a droppable tier bounded by a cap, an already-excluded candidate ignored, the report JSON shape). 8 integration tests in the new crates/aicontext-context/tests/budget.rs exercise apply through a real ContextIndex and TempProject: protected tiers survive a budget that cannot hold history, a budget below the floor is CTX-030, cuts run 9 -> 7 before 6, a candidate is priced from the index, a per-tier cap bounds a droppable tier, a protected tier over its cap is CTX-030, determinism across two runs, and the packet JSON shape.'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked 596 passed, 0 failed (was 581). Scope widened from the declared crates/aicontext-context/** to add crates/aicontext-core/src/error.rs (the CTX-030 constant) and docs/CONTEXT_SPEC.md (the §7.5 and §8 text), a public-API change approved before starting. The CLI wiring (aicontext context, --budget, --explain) remains TASK-037 and health TASK-038.'
```

### TASK-037 — Implement aicontext context

```yaml
id: TASK-037
title: Implement aicontext context
status: DONE
priority: CRITICAL
phase: 2
depends_on: [TASK-033, TASK-035, TASK-036]
spec: docs/CONTEXT_SPEC.md
touches: ["crates/aicontext-cli/**", "crates/aicontext-context/**"]
acceptance:
  - --task, --include, --exclude, --budget, --explain, --json
  - Output is a ContextPacket with policy, retrieved items, conflicts, budget report, and provenance
  - Retrieved content is wrapped in a data envelope marked untrusted and never placed in an
    instruction role
  - Warm run under 1 s
done:
  - 'crates/aicontext-cli/src/context.rs is new and is the single `aicontext context` command, wired through args.rs (Command::Context, ContextArgs, OutputFormat) and main.rs, and removed from args::PENDING. The two views CLI_SPEC §3 would have split into `context show` and `context explain` are one command whose flag shape was approved before starting: the default renders the packet, `--explain` adds RetrievalPlan::explain and every provenance trace, `--format prompt` writes provider input, and `--json` (aliased by `--format json`, which loses to the global flag when both are given) puts the packet in the envelope. The index is loaded from `.aicontext/index.json` when present and rebuilt with the previous entries as fingerprints, so a warm run re-reads only what changed; the save is best-effort and reports CTX-019 at info level when it fails. A budget that cannot hold tiers 1-5 is forwarded as CTX-030 and exits 3.'
  - 'crates/aicontext-context/src/assemble.rs is new and owns the packet: PACKET_VERSION 1, Request, PolicyItem, RetrievedItem, Trust (one Untrusted variant, so the type cannot mark retrieved content as instruction), ContextPacket, and ContextAssembler. Tiers 1-2 (the mandatory AI.md and RULES.md plus any --include) are the policy region; every other candidate is retrieved and untrusted. include_kinds and exclude_ids are applied before the budget so a dropped trace carries `excluded` rather than `budget`; AI.md and RULES.md are exclusion-protected. Conflicts come from PriorityResolver and the budget report from ContextBudget, and provenance holds one RetrievalTrace per candidate considered, merged by max score with reasons joined. render_prompt lives here, at the trust boundary, and wraps each retrieved document in a BEGIN/END UNTRUSTED DATA envelope; retrieval.rs gained only a pub(crate) on order_candidates.'
  - 'The packet carries references and metadata only, so `--json` publishes the bare ContextPacket as the envelope`s `data`, satisfying CONTEXT_SPEC §7.6 without inventing a second shape. `--format prompt` is the only form that reads document bodies, and it reads them from the index at render time, so no document content is in the packet itself.'
  - 'Tests: 13 integration tests in the new crates/aicontext-cli/tests/context.rs drive the real binary in a fixture project (`--task`, `--query`, `--include`, `--exclude`, `--budget`, `--explain`, `--format json`, `--format prompt`, the CTX-030 exit 3, the cache-only write, the warm run under one second, and byte-identical output across two runs). Each spawns env!("CARGO_BIN_EXE_aicontext") in a TempDir and asserts the observable contract: the JSON envelope shape and values, prompt output wrapped in an untrusted-data envelope, and a failed assembly leaving stdout empty. 9 integration tests in the new crates/aicontext-context/tests/assemble.rs cover the assembler through its public API: policy and retrieved regions with tiers and provenance, a query seeding lexical retrieval, the ladder`s cross-tier conflict, a capped droppable tier cut and reported, a budget below the floor as CTX-030, exclusions that cannot touch AI.md/RULES.md, an include_kinds narrow, the untrusted prompt envelope, and determinism across two runs.'
  - 'Timing: a warm run measured 212 ms end to end through `cargo run` on this Windows host, well inside the one-second target. Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked green (620 passed, 0 failed). Scope is the declared crates/aicontext-cli/** and crates/aicontext-context/**; docs/CLI_SPEC.md (§3 folds show/explain into flags, §4 context prose, §5 exit 3) and docs/CONTEXT_SPEC.md (§7.6 note on the CLI views) were updated because the public command surface changed, the same widening TASK-036 recorded. The index still scans only root/.ai, so a task whose spec or touches point at crates/** becomes a dropped trace with reason `missing`; widening the index is not part of this task.'
```

### TASK-038 — Implement aicontext health

```yaml
id: TASK-038
title: Implement aicontext health
status: DONE
priority: MEDIUM
phase: 2
depends_on: [TASK-014, TASK-037]
spec: null
touches: ["crates/aicontext-cli/**"]
acceptance:
  - Reports context completeness, architecture consistency, documentation freshness, task
    consistency, decision coverage, and broken references
  - Every metric is a transparent count with the commands that reproduce it
  - No opaque composite score
done:
  - 'crates/aicontext-cli/src/health.rs is new: aicontext health reports the six CONTEXT_SPEC section 9 metrics this release can source - context completeness (documents present of the base documents init always writes), architecture consistency (CTX-013 findings), task consistency (CTX-009/010/011 findings, beside the register status breakdown), decision coverage (accepted of all decision records), broken references (CTX-007/008, broken down by code), and documentation freshness (documents whose updated date predates the newest commit touching their touches paths). Each metric is a count, never a composite score, and carries the exact command that reproduces it.'
  - 'The counts have one source, so a metric and its reproducing command cannot disagree. The check-derived metrics run the same aicontext_context::doctor::run the doctor command runs; templates::schema_sources and doctor::observed_languages were made pub(crate) so health reuses them rather than restating a rule. Decision coverage and freshness read the content index, built in memory and never saved: health is read-only and does not even create the index cache, which an integration test proves by snapshotting the tree before and after a run.'
  - 'A metric that cannot be computed says so instead of reporting a zero that would read as nothing wrong: documentation freshness is n/a with a reason outside a git repository, and a document with no touches is counted neither fresh nor stale. Two section 9 metrics, stale memory and plugin health, are deliberately not reported, because neither has a source to count and a number for either would be invented.'
  - 'Exit codes follow CLI_SPEC section 5: 0 for a report, 3 when the tree carries an error-level finding (the checks findings are published in the envelope, so the exit code is reproducible from the body), and 2 for an unusable root. --json publishes Health { metrics } as the envelope data, and the human view prints one indented block per metric: the value, the definition, reproduce:, and any notes (missing documents, the status breakdown, stale paths).'
  - 'Tests: 10 integration tests in the new crates/aicontext-cli/tests/health.rs drive the real binary against a skeleton written by init (the six-metric envelope, the human view, completeness after deleting a document, exit 3 for a missing AI.md, a broken reference counted by code, accepted-of-total decision coverage, freshness against a dated git commit, read-only behaviour, the PROJ-001 usage error, and byte-identical repeat runs), plus 4 unit tests for the renderer and touch_path in health.rs. cli.rs pending-command tests move from health (now built) to connect, and args.rs asserts health is no longer pending.'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked -- -D warnings clean; cargo test --workspace --locked green (635 passed, 0 failed, was 620). Scope widened from the declared crates/aicontext-cli/** to docs/CLI_SPEC.md section 4, docs/CONTEXT_SPEC.md section 9, and README.md, the same widening TASK-036 and TASK-037 recorded, because the implemented metric definitions and the command availability are public contract.'
```

---

## Phase 3 — AI provider layer

**Gate:** a provider can be configured from the environment or keychain, and one task can be planned
from assembled context. Plan mode only — no writes.

### TASK-050 — Define the AIProvider trait and registry

```yaml
id: TASK-050
title: Define the AIProvider trait and registry
status: DONE
priority: HIGH
phase: 3
depends_on: [TASK-037]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - CompletionRequest, CompletionResponse, ToolCall, and Usage are provider-neutral types
  - The trait has no provider-specific types in its signature
  - Adding an adapter touches no other crate
scope_notes:
  - 'Root Cargo.toml and .ai/ARCHITECTURE.md 2.2 are touched beside the declared
    crates/aicontext-providers/**. A new crate cannot be built otherwise: the workspace members
    list must name it, or cargo cannot see the directory and the boundary test fails, and the three
    direct dependencies need a written justification in 2.2 per RULES 6 (recorded there as three new
    rows, async-trait and futures-core in the table and futures-executor in the dev-only list).
    Cargo.lock records the seven transitive additions the three pull in. All three were named in the
    approved plan before the crate was written'
  - 'The crate deliberately does not depend on aicontext-core, though the boundary matrix permits
    it. Nothing here uses a core type: errors carry their own string codes, ids are a local
    ProviderId, and the registry keys on it. A dependency added for symmetry would be an unused one'
done:
  - 'crates/aicontext-providers is new, and aicontext-providers is added to the root members list
    (the boundary matrix already had its entry). Its public surface is exactly AI_PROVIDER_SPEC 2:
    the AIProvider trait (id, complete, stream, capabilities) as Send + Sync and dyn-compatible
    through async-trait, the neutral request and response types, and the registry. The signature
    names no provider, URL, header, or wire type - only CompletionRequest, CompletionResponse,
    StreamEvent, Capabilities, ProviderError, and ProviderId - so "no provider-specific types in its
    signature" holds by construction rather than by convention'
  - 'The neutral types are the spec 2 shapes: PolicyRegion (instruction + PolicyBlock list),
    DataBlock { source, path, tier, trust, content }, Message/Role, ToolSpec, TokenBudget,
    ModelOptions, CompletionRequest, CompletionResponse, ToolCall, FinishReason (Stop/ToolUse/
    Length/Refusal/Error plus a non_exhaustive Unknown(String), so T12 reports an unrecognised
    reason rather than guessing), Usage (reported and estimated constructors, with the estimated flag
    keeping a host estimate distinct from a provider report, section 7), StreamEvent, Capabilities,
    and RoutingRequest'
  - 'The two regions are two types, not one type with a flag. PolicyRegion and DataBlock are
    separate, and DataBlock.trust is a one-variant Trust::Untrusted with no other constructor, so a
    DataBlock cannot be read as an instruction and external content cannot enter the policy region
    (AI_PROVIDER_SPEC 2.1, SECURITY I1). A refusal is a FinishReason on a successful
    CompletionResponse, never an Err, and every ToolCall is a proposal the gateway verifies'
  - 'ProviderId validates a lowercase slug and returns ProviderError::InvalidId naming the offending
    value, so a typo in .ai/ai.yaml is caught where it is read. ProviderError carries one variant per
    failure mode - transport, three-phase timeout, 401/403/404/400/429/5xx, malformed body,
    unsupported capability, cancellation, invalid id - each with a stable PROVIDER_* code() to branch
    on and a remediation() hint, and transport errors keep their source rather than flattening it
    (RULES 4.4). No ErrorFamily or PREFIX-NNN code is added to aicontext-core: there is no external
    catalogue to number, and the mapping onto exit 6 belongs to the command that runs a provider'
  - 'ProviderRegistry holds a BTreeMap<ProviderId, Box<dyn AIProvider>>, so adapters of different
    concrete types live in one map and adding one changes no caller. register returns the provider it
    replaced rather than dropping it. route(&RoutingRequest) -> ProviderId is the seam AI_PROVIDER_SPEC
    8 asks for: in the MVP it returns the configured default and ignores the request, so inventing
    routing later changes no signature. Its Debug prints ids only, never a provider body, because a
    body could hold a credential (T10). The crate re-exports async_trait and Stream, and defines
    BoxStream<T>, so an adapter implements the trait and reads a stream with only this crate in scope'
  - 'Tests: 17 unit tests across types.rs, error.rs, and registry.rs (id acceptance and rejection,
    the code set being complete and distinct, a timeout naming its phase, a transport failure keeping
    its source, usage absent vs estimated, an unknown finish reason preserved, registry register/get/
    replace/default, and route ignoring its request) plus 6 integration tests in the new
    crates/aicontext-providers/tests/provider.rs that implement the trait using only the crate''s
    re-exports, drive complete and a hand-rolled stream through futures_executor::block_on with no
    async runtime, and assert a provider that cannot stream reports Unsupported rather than failing
    late, and 1 doctest on the crate docs.'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked --
    -D warnings clean; cargo test --workspace --locked green (659 passed, 0 failed, was 635 - the 24
    new tests are the crate''s own, the rest of the workspace is unchanged). Acceptance 3 ("adding an
    adapter touches no other crate") is met by construction and exercised by tests/provider.rs: the
    trait is implemented and the registry reached from outside the crate naming only its exports, with
    no other crate in the dependency graph. TASK-051 adds the first real adapter and is the
    integration proof'
```

### TASK-051 — Implement the OpenAI-compatible adapter

```yaml
id: TASK-051
title: Implement the OpenAI-compatible adapter
status: DONE
priority: HIGH
phase: 3
depends_on: [TASK-050]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - Works against OpenAI and against any base_url-compatible local server
  - Timeouts, retries with backoff, and cancellation are bounded and reported
  - A provider error becomes a typed error with a remediation hint, never a panic
  - Contract tests run against a local fake server
scope_notes:
  - 'Root Cargo.toml, Cargo.lock, and .ai/ARCHITECTURE.md 2.2 are touched beside the declared
    crates/aicontext-providers/**. The two new production dependencies (tokio, reqwest) and the
    already-listed tracing need a written justification in 2.2 per RULES 6, and their versions
    resolve into Cargo.lock. .ai/TASKS.md, .ai/MEMORY.md, and a new decisions/ADR-009 are the task
    bookkeeping. No crate outside aicontext-providers changed'
  - 'RULES 6 forbids a dependency that requires a C compiler, a system library, or a network install
    at runtime, and reqwest''s rustls-tls feature resolves to rustls with the ring provider, which
    compiles C and assembly through the cc crate at build time. This is recorded as a deviation in
    decisions/ADR-009: ring needs a C toolchain at build time but adds no system library and no
    runtime or network prerequisite, which is the part of the rule that protects the static binary.
    The alternative, rustls''s default aws-lc-rs, needs cmake as well and is worse'
  - 'crates/aicontext-context/src/discovery.rs is touched for a docs-only fix. The documentation gate
    CI runs (RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked) failed at HEAD, in a
    crate this task does not touch: the public discovery module document linked to the private
    constants MAX_ENTRIES_SCANNED and MAX_MANIFEST_BYTES. The defect predates this task and is filed
    as BUG-005; the two links became prose with the values spelled out. It is included here because a
    red documentation gate is a red build, and it was found only by running that gate'
done:
  - 'crates/aicontext-providers/src/openai is new: the OpenAI-compatible adapter AI_PROVIDER_SPEC 3
    names. One implementation over a configurable base_url reaches OpenAI and every server that
    speaks the same /chat/completions shape (Ollama, llama.cpp, vLLM, LM Studio, OpenRouter, Azure),
    which is acceptance 1. OpenAiCompatibleConfig carries the id, base_url, model, Auth, Timeouts,
    RetryPolicy, and a shared CancellationToken; Auth::Bearer holds a Secret, which renders
    [redacted] in every Debug and Display, so a credential cannot be printed by accident (T10)'
  - 'wire.rs is the mapping, over serde_json::Value by hand rather than derived types so no
    provider-shaped type enters the neutral API. PolicyRegion becomes system messages and DataBlocks
    become one enveloped user message (<untrusted-data ...>), so T9 is true by construction and a
    test asserts it on the bytes the server received. FinishReason maps stop/tool_calls/length/
    content_filter/error and preserves anything else as Unknown (T12). ToolCall arguments are parsed
    from the JSON-encoded arguments string, the provider payload is kept as raw for audit, usage maps
    prompt/completion/cached tokens (T4), and max_tokens is always sent (T11)'
  - 'Reliability is AI_PROVIDER_SPEC 6 and acceptance 2. Three timeouts - connect, request, total -
    are each configurable and each surfaces as ProviderError::Timeout carrying its phase. Retries are
    bounded by RetryPolicy.max_retries, apply only to an idempotent request (a non-empty
    idempotency_key), wait out a provider Retry-After when it sends one, use exponential backoff with
    deterministic jitter capped at max_delay, and log every attempt through tracing so a retry is
    never silent (RULES 6). A CancellationToken races both complete and stream, so cancellation ends
    the call or the stream promptly rather than waiting for a timeout'
  - 'Failures are typed and actionable and never a panic, which is acceptance 3: 400/401/403/404/429/
    5xx map onto the ProviderError variants TASK-050 defined, each with a PROVIDER_* code and a
    remediation() hint, and a body that is not JSON is MalformedResponse. The adapter writes nothing
    and never falls back to another provider or model on a failure (AI_PROVIDER_SPEC 6)'
  - 'stream returns a BoxStream<StreamEvent> with no error variant (AI_PROVIDER_SPEC 2), so a spawned
    reader task reads the response and translates it, ends the stream on a failure rather than
    emitting one, and stops on the shared cancellation token. It hands events to the host through a
    bounded tokio::sync::mpsc channel, which is why bytes is not a direct dependency'
  - 'Tests: the crate now runs 40 unit tests (cancel.rs, secret.rs, openai/retry.rs, openai/sse.rs,
    and openai/wire.rs - including the T9 body assertion and the T12 unknown reason - beside
    TASK-050''s types, error, and registry tests), 21 contract tests in the new
    crates/aicontext-providers/tests/openai_contract.rs covering T1-T12 plus streaming, 6 integration
    tests from TASK-050, and 2 doctests'
  - 'The contract tests run against crates/aicontext-providers/tests/support, a std-only fake HTTP
    server that binds an ephemeral localhost port and serves canned responses in order over HTTP/1.1
    keep-alive while recording every request, so no test touches the network and no dependency is
    added for the server (acceptance 4). T6 uses the RFC 5737 documentation range for the connect
    timeout, which is the only address a test needs that is not the fake server'
  - 'Gates: cargo fmt --all --check clean; cargo clippy --workspace --all-targets --locked --
    -D warnings clean; cargo test --workspace --locked green (704 passed, 0 failed, was 659 - all 45
    new tests are the crate''s own and the rest of the workspace is unchanged); RUSTDOCFLAGS=-D
    warnings cargo doc --workspace --no-deps --locked clean (it was not at HEAD - see BUG-005 and the
    scope note). Acceptance 3 of TASK-050, "adding an adapter touches no other crate", is proven by
    this task: the adapter is written entirely inside crates/aicontext-providers and neither the
    boundary matrix nor the crate-boundary test changed'
```

### TASK-052 — Implement the credential resolver

```yaml
id: TASK-052
title: Implement the credential resolver
status: DONE
priority: CRITICAL
phase: 3
depends_on: [TASK-050]
spec: docs/SECURITY.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - Resolution order: explicit flag, environment, OS keychain
  - Values are never written to .ai, stdout, logs, or the audit log
  - Redaction is applied at the logging boundary, not ad hoc
  - Unit tests assert no credential appears in any error or debug output
scope_notes:
  - 'Root Cargo.toml, Cargo.lock, and .ai/ARCHITECTURE.md 2.2 are touched beside the declared
    crates/aicontext-providers/**. The one new dependency, keyring, needs a written justification in
    2.2 per RULES 6, and its version resolves into Cargo.lock. .ai/TASKS.md and .ai/MEMORY.md are the
    task bookkeeping. No crate outside aicontext-providers changed'
  - 'RULES 6 forbids a dependency that requires a C compiler, a system library, or a network install
    at runtime. keyring''s backends are the OS credential store and nothing else - the Windows
    Credential Manager (windows-sys), the macOS Keychain (security-framework), the Linux kernel
    keyring (linux-keyutils) - so no C compiler and no third-party system library is required. The OS
    store is the dependency ADR-005 chose, and FR-16 allows exactly two homes for a credential; this
    differs from TASK-051''s rustls-and-ring, which needed ADR-009 for a build-time C toolchain, and
    needs no deviation'
done:
  - 'crates/aicontext-providers/src/credential.rs is new: CredentialSource (none, environment,
    keychain, or environment-or-keychain - the shapes AI_PROVIDER_SPEC 4 names), KeychainReference (a
    service-and-account key), the Keychain and Environment ports, CredentialResolver, CredentialError,
    Redactor, and KeyringKeychain. Resolution is explicit flag, then environment, then the OS keychain
    (acceptance 1); an empty explicit flag or an empty variable is treated as absent, and a named but
    unresolvable source is an error rather than a silent fallback (RULES 4.10) carrying a CREDENTIAL_*
    code and a remediation hint'
  - 'Environment and Keychain are traits, not direct calls, so no test reads the developer''s real
    environment or keychain (RULES 8) and no test calls std::env::set_var, which is unsafe in edition
    2024 and forbidden by RULES 3 - the fakes are injected instead'
  - 'Redaction (acceptance 3) has one type, Redactor, applied at one place: the adapter redacts a
    provider''s own error body before it becomes a ProviderError detail, because that is the one
    string in a call the provider, not we, built (wire::error_detail was the only such path).
    Redactor::redact masks a known value exactly and also masks values it was not told about by their
    well-known shape (sk-, ghp_, Bearer, ...); Redactor::redact_field drops a value whose field name is
    credential-shaped. It returns a borrow when nothing matched, so the common path allocates nothing'
  - 'Values reach no output (acceptance 2): the type-level half is Secret, which renders [redacted];
    CredentialError and KeychainError name only the source - a variable or a service and account -
    never a value. KeychainError carries no #[source] and KeyringKeychain maps keyring::Error onto
    those coarse variants, deliberately dropping the platform text, because a platform message can
    echo the entry it was reading and I5 makes "no secret in any output" absolute'
  - 'Tests (acceptance 4): 18 new unit tests in credential.rs. The resolution-order suite runs
    against fake ports for every path (explicit wins, empty explicit falls through, environment
    precedes keychain, keychain last, empty variable is absent, none needs no credential, and each
    failure code). no_credential_in_any_output asserts that no CredentialError, KeychainError, or
    Redactor Debug or Display carries a known credential, naming the SECURITY 3 I5 invariant'
  - 'One contract test is added: t10_redacts_a_credential_the_provider_echoes. The fake server answers
    400 with a body containing the exact credential that was sent, and both the Display and the Debug
    of the resulting ProviderError must show [redacted] and not the value. This is the adapter half of
    acceptance 2 and 4'
  - 'Gates: cargo fmt --all -- --check clean; cargo clippy --workspace --all-targets --locked -- -D
    warnings clean; cargo test --workspace --locked 723 passed, 0 failed (was 704; +18 unit and +1
    contract); RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked clean; aicontext
    doctor exit 0, the one CTX-012 warning about .ai/schemas being pre-existing and unrelated.
    keyring 3.6.3 was selected over 4.2.0 by MSRV-aware resolution: 4.2.0 requires Rust 1.88, above
    our 1.85 floor'
```

### TASK-053 — Implement aicontext connect

```yaml
id: TASK-053
title: Implement aicontext connect
status: DONE
priority: HIGH
phase: 3
depends_on: [TASK-052]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-cli/**"]
acceptance:
  - Stores a provider profile and a keychain reference, never a secret
  - Verifies connectivity with an explicit, minimal test call
  - disconnect removes the reference and leaves no residue
scope_notes:
  - 'Root Cargo.toml and Cargo.lock are touched beside the declared crates/aicontext-cli/**. The
    workspace tokio justification gains one clause: net is added per-crate by whoever builds a
    runtime of its own, so TASK-051''s row is not silently widened. .ai/ARCHITECTURE.md 2.2, the
    3.2 rule 7 clarification, .ai/MEMORY.md, .ai/TASKS.md, and docs/CLI_SPEC.md sections 3, 4, and 9
    are the bookkeeping. No crate outside aicontext-cli changed'
  - 'crates/aicontext-cli/Cargo.toml gains three already-justified workspace dependencies -
    aicontext-providers, tokio with the net feature, and yaml_serde - plus yaml_serde again as a
    dev-dependency, because an integration test is its own crate. aicontext-providers is an internal
    edge the matrix in crates/aicontext-testkit/tests/crate_boundaries.rs already permits and the
    3.2 diagram already draws; 2.2 records why yaml_serde is reached directly rather than through
    aicontext-context''s YamlCodec'
  - 'The credential lives in the OS keychain and only its reference reaches .ai, per SECURITY 6 and
    AI_PROVIDER_SPEC 4: --token writes the value under service aicontext and an entry named by
    --keychain-entry (defaulting to the provider id), and .ai/ai.yaml records only that name. --env
    records a variable''s name and never its value; --token wins over --env; --keychain-entry with no
    credential to put in it is a usage error'
  - 'An empty --token is refused in the command, never by a clap value_parser: clap''s own error
    quotes the value it rejected, and echoing the secret is the one thing this flag must never do.
    --token is therefore the only flag with no parser. The ones that carry no secret (provider,
    adapter, base-url, model, env, keychain-entry) reject a blank value at the parser, where a usage
    error belongs'
  - 'The document is a live yaml_serde::Value rather than a struct, so an upsert rewrites one
    provider entry and carries every key it does not understand through untouched. The CLI reads
    .ai/ai.yaml in its own module (connect/config.rs) rather than through aicontext-context''s
    private YamlCodec: it is a fixed-path config file, not front matter (CONTEXT_SPEC 1), and
    reaching through another crate''s internal trait would break ARCHITECTURE.md 3.2 rule 6'
  - 'Removing the last provider removes the file, decided with the user in review. providers is
    minItems 1 in schemas/ai-config.schema.json, so an emptied list is a document no reader can
    trust, and init never creates this file - a project with no providers has no .ai/ai.yaml. The
    two alternatives were refused: writing providers: [] would ship a file that fails its own schema,
    and relaxing minItems would edit a committed schema outside this task''s scope'
done:
  - 'crates/aicontext-cli/src/args.rs declares Connect and Disconnect with ConnectArgs and
    DisconnectArgs, and drops both words from PENDING, so --help moves them from the planned tree to
    the available one with their own connect/disconnect flags block. cli.rs''s four pending-command
    tests now use audit (TASK-075), since connect is no longer an example of a command that is not
    built, and help_lists_what_works_and_what_does_not asks for TASK-075 instead of TASK-053'
  - 'crates/aicontext-cli/src/exit.rs gains Exit::Provider (6, PROVIDER_ERROR) - already published in
    CLI_SPEC 5 and now produced by a real failure - and both of its tests carry the new variant'
  - 'crates/aicontext-cli/src/connect is new, in three modules. config.rs reads and writes .ai/ai.yaml
    as a live Value: load, upsert, remove, ensure_default, clear_default, discard, provider_ids,
    save, with one root() helper for the shape every mutation needs. error.rs is the typed
    ConnectError catalogue, CONNECT-001 to CONNECT-012, each variant carrying its code, its exit
    code, and a remediation (RULES 4.3). mod.rs holds the command, the three-port seam, and
    HttpVerifier'
  - 'Failures keep the vocabulary they already had rather than gaining a second opinion about them:
    PROVIDER_* from the provider crate, CREDENTIAL_* from the credential resolver, PROJ-* from the
    project root. The CLI only maps them onto exit codes - a rejected provider id is Usage, the
    verification call is the new Exit::Provider 6, a missing variable or keychain entry is Usage, a
    read or write failure is General, a parse or shape failure is Validation'
  - 'Order of operations in connect: root, id, adapter, and .ai checks, credential resolution, one
    minimal complete() call, in-memory upsert, ensure_default only when the file names no default,
    keychain, save. Nothing is written before the call answers, and if the save fails after the
    keychain entry was written the command deletes that entry again rather than stranding a secret
    nothing points at. The call is the smallest the contract allows: one message, max_tokens 8, no
    stream, no idempotency key, no retry (AI_PROVIDER_SPEC 11)'
  - 'crates/aicontext-cli/src/disconnect.rs removes the entry, clears a default_provider naming it,
    then saves or discards the file, then deletes the keychain entry - the file first, so a failure
    between the two is the recoverable direction: the reference still exists and connecting again
    restores a working profile. An id the file does not list exits 2 (CONNECT-009) naming the ids it
    does, and changes nothing (RULES 4.10)'
  - 'Seams: connect/mod.rs owns Ports and the Verifier trait, so every unit test injects a fake
    environment, keychain, and verifier and none reads the developer''s real one (RULES 8). The
    integration test needs no seam of ours - tests/connect.rs binds an ephemeral 127.0.0.1 port with
    std only and answers one request, exactly as AI_PROVIDER_SPEC 11 asks of the provider crate''s
    contract tests'
  - 'Tests: 12 new in connect/mod.rs, 10 in connect/config.rs, 4 in connect/error.rs, 8 in
    disconnect.rs, 2 in args.rs, both of exit.rs''s extended, and 9 end-to-end in
    crates/aicontext-cli/tests/connect.rs. The integration tests exercise only the auth: none path,
    which stores no credential, so no test can reach the developer''s OS keychain; the paths that
    would store one are covered behind the keychain port, the only place a keychain can be faked.
    Every success path validates the written .ai/ai.yaml against schemas/ai-config.schema.json
    rather than merely looking for substrings'
```

### TASK-054 — Implement the Anthropic and Gemini adapters

```yaml
id: TASK-054
title: Implement the Anthropic and Gemini adapters
status: DONE
priority: MEDIUM
phase: 3
depends_on: [TASK-051]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-providers/**"]
acceptance:
  - Each maps its tool-call and usage shapes onto the provider-neutral types
  - Both pass the shared contract test suite
scope_notes:
  - 'Scope extended by approval during review, in two directions. First, crates/aicontext-cli is
    touched outside the declared crates/aicontext-providers/**: connect builds whichever adapter
    --adapter names and verifies against it, so args.rs'' help, tests/connect.rs, and docs/CLI_SPEC.md
    4 move with it. Second, full streaming is in scope for both new adapters - stream() is implemented
    and capabilities().streaming is true for all three, not a documented future. Everything else is
    the declared crate plus the usual .ai/ bookkeeping'
  - 'No dependency is added. reqwest, tokio, serde_json, and tracing were already in
    aicontext-providers for the openai adapter, and the HTTP/TLS stack stays exactly what ADR-009
    recorded - this task builds adapters on the stack the workspace already commits to'
  - 'The shared transport is one extraction rather than three copies (RULES 2: shared code needs
    three concrete use cases, and Auth, timeouts, retries, SSE parsing, the stream pump, and error
    detail now have exactly three each). src/transport/{mod,retry,sse,stream,envelope}.rs lifts what
    the openai adapter already had out of it - RetryPolicy and SseParser moved verbatim - and the old
    tests/openai_contract.rs passed unchanged against the refactored openai adapter before that file
    was replaced, which is what makes "no behavior changed" a claim rather than a hope. What the
    adapters still own is what genuinely differs: URLs, headers, and the wire shape of a message, a
    tool call, a finish reason, and usage'
  - 'The credential is mapped once: Adapter::build turns the resolved secret into Auth::ApiKey, and
    each adapter''s auth_headers decides whether that becomes Authorization, x-api-key, or
    x-goog-api-key. The secret itself is only ever held by transport::Auth, whose Debug and Display
    redact it, so no adapter and no error message can render it (SPEC 11 T10)'
  - 'Two wire facts have no provider-neutral equivalent and are documented at the point of decision
    rather than papered over. Gemini assigns no id to a function call, so ToolCall.id is the empty
    string rather than an invented one, and a tool result arrives as a user text part wrapped in
    <tool-result call="..."> because a neutral Message carries a call id but not the tool name that
    functionResponse requires. Anthropic has no seed, so options.seed is dropped there. Gemini''s
    STOP with a functionCall in it maps to ToolUse - its own upgrade rule, since the wire spells a
    turn that proposed a function STOP'
done:
  - 'src/transport is new: Auth (None/Bearer/ApiKey over a Secret, with a header() and a redactor()),
    Timeouts, RetryPolicy and SseParser moved verbatim from the openai adapter, a stream pump that
    translates raw SSE lines through a closure the adapter supplies, error_detail() reading /error/
    message or /message from any provider''s body, and a dispatch() retry loop keyed on an idempotency
    seed. src/openai now sits on that transport and its own retry.rs and sse.rs are deleted'
  - 'src/anthropic is new: POST {base}/messages with x-api-key and anthropic-version always sent, the
    instruction and policy as a top-level system string, max_tokens always present, tool_use blocks
    mapped whole (their input is already an object) and tool_result correlated back to its tool_use_id
    in a user message, input/output/cache_read tokens onto Usage, and the stop_reason map
    end_turn|stop_sequence -> Stop, tool_use -> ToolUse, max_tokens -> Length, refusal -> Refusal,
    anything else -> Unknown(raw)'
  - 'src/gemini is new: the model in the path - {base}/models/{model}:generateContent, stream as
    :streamGenerateContent?alt=sse - with contents, systemInstruction, tools.functionDeclarations,
    and generationConfig carrying maxOutputTokens, temperature, stopSequences, and seed. functionCall
    parts map onto one ToolCallDelta with the arguments already serialized; finishReason maps
    STOP -> Stop (upgraded to ToolUse when a call is present), MAX_TOKENS -> Length, the safety family
    -> Refusal, MALFORMED_FUNCTION_CALL -> Error, anything else -> Unknown(raw); usageMetadata''s
    prompt/candidates/cached counts onto Usage; a response with no candidates or no function name is
    MalformedResponse rather than a guess'
  - 'src/adapter.rs is the factory the CLI needs: Adapter::{OpenAiCompatible, Anthropic, Gemini} with
    ALL, name(), from_name() and build(id, base_url, model, credential), so "which adapters exist" is
    one list in the provider crate that no other crate re-states. Both new adapters implement
    capabilities() with streaming true and stream() over their own wire''s event translation'
  - 'The contract suite became provider-neutral: tests/openai_contract.rs is deleted and
    tests/contract.rs runs every T1-T12 test plus both streaming tests in a loop over
    tests/support/fixture.rs''s three fixtures. Each fixture owns what differs - endpoint path, reply
    and tool-call and usage shapes, the finish-reason table, the header a credential lands in, where
    the policy and the data sit in the body, the ceiling, the stream''s canned bytes and their
    expected events - and the test file holds only the assertion, which is what the contract says is
    the same everywhere. Every assertion message names the adapter it came from, so one red test
    still says which wire format broke. 22 tests, each covering three adapters'
  - 'connect accepts all three: validate_adapter is Adapter::from_name, an unknown name is
    CONNECT-005 and exit 2 printing the list of names that do exist (the list comes from Adapter::ALL,
    so it cannot drift), Verifier::verify takes the parsed Adapter, and HttpVerifier hands it to
    Adapter::build. SCHEDULED_ADAPTERS and OPENAI_COMPATIBLE are gone - "not built yet" is no longer
    a state the CLI needs to know about'
  - 'Tests: 114 unit tests in the provider crate (including per-adapter tests for the system
    instruction, the tool shapes, the finish-reason maps, malformed responses, auth headers, endpoint
    building, config-Debug redaction, and every stream translation), 22 shared contract tests running
    against all three adapters, and 2 new end-to-end tests in aicontext-cli/tests/connect.rs - one
    proving --adapter anthropic reaches POST /messages and reads an Anthropic-shaped reply back, one
    proving an unknown name exits 2, names the three that exist, makes no call, and writes nothing.
    No test reads a real credential or reaches the network beyond 127.0.0.1'
  - 'Gates: cargo fmt --all --check, cargo clippy --workspace --all-targets -- -D warnings,
    cargo test --workspace (all suites green), and aicontext doctor (exit 0, only the pre-existing
    CTX-012 finding about .ai/schemas copies)'
```

### TASK-055 — Implement aicontext agent (plan mode only)

```yaml
id: TASK-055
title: Implement aicontext agent run in plan mode
status: DONE
priority: HIGH
phase: 3
depends_on: [TASK-051, TASK-053]
spec: docs/AI_PROVIDER_SPEC.md
touches: ["crates/aicontext-cli/**"]
scope_notes: >
  Shipped `agent run` only, in plan mode (`ADR-006` #1). `--task <ID>` is required and names the
  whole scope of a run; `--dry-run` assembles the context and reports it without calling anything or
  writing a plan. `--execute` and `--allow` are deliberately not in the parser, so passing either is
  clap's unknown-flag usage error (exit 2); an execution mode must not exist until plan mode has
  proved itself (`ADR-006` #7). `agent list` is likewise not built and exits 2. Cost in dollars is
  deferred to TASK-056: the ceiling is the constant `PLAN_MAX_OUTPUT_TOKENS` (4000), because the
  ai-config schema has no budget section to read one from, and the tokens a provider reports are the
  usage this task requires. The plan artifact is a structured rendering when the answer is a
  schema-valid `action_proposal`, and the verbatim prose otherwise. Besides the declared touches,
  `docs/CLI_SPEC.md` §4 was updated to describe plan mode and drop the not-yet-accepted
  `--execute`/`--allow` rows.
acceptance:
  - Assembles context, calls the provider, and writes a plan to .aicontext/plans
  - Modifies nothing in the working tree
  - Reports usage and cost, and honours the token budget
  - Unavailable provider produces a clear failure, not a silent degradation
```

### TASK-056 — Add cost and usage reporting

```yaml
id: TASK-056
title: Add cost and usage reporting
status: DONE
priority: MEDIUM
phase: 3
depends_on: [TASK-055]
spec: null
touches: ["crates/aicontext-providers/**", "crates/aicontext-cli/**"]
scope_notes:
  - 'Cost is configured, not compiled in, and the configuration lives in `.ai/ai.yaml` beside the
    providers it prices: an optional top-level `budget.max_run_usd` and a per-provider `pricing`
    block (`input_per_mtok`, `output_per_mtok`, optional `cached_input_per_mtok`, all USD per million
    tokens). Agreed with the user as option A. This extends the declared touches to
    `schemas/ai-config.schema.json`, `docs/AI_PROVIDER_SPEC.md` sections 5 and 7, and
    `docs/CLI_SPEC.md` sections 4 and 5, because the schema and both contracts publish the new keys
    and the exit-3 behaviour'
  - 'Four pre-existing rustdoc private-intra-doc-link errors in `crates/aicontext-providers/src`
    (`openai`, `anthropic`, `gemini`, and `transport`) were fixed in the same change. They are inside
    the declared crate but outside this task''s subject; the Documentation gate runs with
    `RUSTDOCFLAGS=-D warnings` and could not pass otherwise (`MEM-031`)'
acceptance:
  - Per-run token and cost totals, from provider-reported usage
  - Configurable budget ceiling that stops the run before overspend
done:
  - '`crates/aicontext-providers/src/cost.rs` is new: `Price` (micro-USD per million tokens), `Cost`
    (micro-USD, rendered with six fixed decimals) and `Pricing` (input, output, optional cached
    input). Money is integer micro-USD end to end — the product is `u128` before it saturates into
    `u64` — so no `f64` is on the path and the same usage always produces the same total. Each
    component rounds up, and `cost()` returns `None` unless both an input and an output count were
    reported: half a total is reported as unavailable, never as a total'
  - '`Usage.cached_input_tokens` is documented as a subset of `input_tokens`, and the Anthropic
    adapter normalises both paths to that shape: `input_tokens` becomes direct + cache reads + cache
    writes, `cached_input_tokens` carries only cache reads. Cache writes are charged at the input
    price, a documented limitation rather than an invented fourth price. The non-streamed and
    streamed tests and the shared contract fixture now use 8 direct + 2 cached = 10 input'
  - '`crates/aicontext-cli/src/connect/config.rs` reads `budget.max_run_usd` and each provider''s
    `pricing` exactly, from the YAML number''s decimal rendering rather than through `f64`; a value
    finer than a micro-dollar rounds up, so a ceiling is never looser than the number written. A
    malformed or incomplete block is `CONNECT-003` rather than a silently ignored price. `upsert`
    now merges the fields `connect` owns and preserves every other key, so reconnecting cannot
    delete a manually added `pricing` block or a field a future adapter needs'
  - '`crates/aicontext-cli/src/agent/mod.rs` enforces the ceiling twice. Before the call it
    estimates the request''s input from the JSON-escaped bytes of every string it carries plus a
    256-byte scaffolding allowance (over four) and charges the full 4,000-token output ceiling; a
    projection above the ceiling is `AGENT-012` and no call is made. After the call the
    provider-reported total is compared with the ceiling: an overage is an `AGENT-013` error finding,
    the plan is still written because the spend already happened, and the run exits 3; usage too
    incomplete to price is the `AGENT-014` warning. A ceiling on a provider with no `pricing` is
    `AGENT-011` and stops the run before any call, including a dry run'
  - 'Cost reaches every surface the run already had: the usage line carries in/out/cached counts,
    their origin (provider-reported or host estimate) and the cost, a dry run adds a projected line,
    and a configured ceiling adds a budget line. The JSON payload adds `cost_usd`,
    `projected_cost_usd` on a dry run, and `budget_usd`, all six-decimal strings, and the plan file
    records the cost of the run that wrote it. Exit 3 now comes from an error-severity finding as
    well as from a typed error'
  - 'Tests: 8 in `cost.rs`; 1 new error-catalogue test for `AGENT-011`/`AGENT-012`; 7 in
    `connect/config.rs` (exact and rounded-up ceilings, negative and malformed values, complete and
    incomplete pricing, merge-preserving upsert, decimal-to-micro conversion); the Anthropic
    cache-normalisation tests; 6 in `agent/mod.rs` (priced run, dry-run projection, missing prices,
    projected overage, actual overage, unverified usage); and 6 end-to-end in
    `crates/aicontext-cli/tests/agent.rs`. The refusal test asserts the socket saw no request, so
    "stops before overspend" is observed rather than inferred. No test reads a real keychain or
    reaches the network beyond `127.0.0.1`'
  - 'Gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D
    warnings`, `cargo test --workspace --locked` (all suites green),
    `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, and `aicontext doctor`
    (exit 0, only the pre-existing CTX-012 finding about the missing `.ai/schemas` copies)'
```

---

## Phase 4 — Plugin system

**Gate:** a plugin can be installed, its permissions granted, a tool called with approval, and the
call appears in a verifiable audit chain.

### TASK-070 — Define the plugin SDK

```yaml
id: TASK-070
title: Define the plugin SDK manifest and protocol types
status: DONE
priority: CRITICAL
phase: 4
depends_on: [TASK-011]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-sdk/**", "schemas/plugin-manifest.schema.json"]
scope_notes:
  - 'A workspace member cannot exist without a line in the root `Cargo.toml`, and the schema-set
    test in `crates/aicontext-cli/tests/schemas.rs` asserts the exact contents of `schemas/`, so
    both are extended. The plugin error family (`ErrorFamily::Plugin`, `PLG-004`, `PLG-005`, and a
    new `PLG-011` invalid manifest) is added to `crates/aicontext-core/src/error.rs`, because
    `RULES.md` 4.3 requires every error to carry a stable code and core is the common denominator.
    `PLG-011` is documented in `docs/PLUGIN_SPEC.md` 4 and `README.md` moves the SDK out of the
    not-yet-created crate list. Approved by the user before implementation'
acceptance:
  - Manifest validates against a Draft 2020-12 schema and declares plugin_api_version
  - Protocol types are JSON-RPC 2.0 and MCP-compatible
  - The SDK depends only on aicontext-core and never on the runtime or CLI
  - Version negotiation rejects an incompatible plugin with a clear message
done:
  - '`crates/aicontext-plugin-sdk/` is a new workspace member with five modules: `manifest.rs`,
    `protocol.rs`, `version.rs`, `error.rs`, and `lib.rs`. Its only internal dependency is
    `aicontext-core`; its external dependencies are `serde`, `serde_json`, and `thiserror`, all
    pre-justified in `ARCHITECTURE.md` 2.2. There is no runtime, CLI, process, pipe, timeout, or
    network code. `jsonschema` is a dev-dependency only, used to prove the published schema accepts
    the canonical manifest. The boundary test already listed `aicontext-plugin-sdk` as depending on
    `aicontext-core` only, so it needed no change'
  - '`manifest.rs` models `PLUGIN_SPEC` 2 field for field with `deny_unknown_fields`, a 1 MiB
    hostile-input cap, and checks for the plugin-name and three-segment tool patterns, `SemVer`,
    duplicate tools, empty auth references, and empty filesystem or network entries.
    `destructive: true` must accompany `effect: write`, and `Capability::permission_floor()` returns
    `PermissionMode::ExplicitApproval`, so a manifest cannot lower its own approval floor.
    `plugin_api_version` is parsed but deliberately not negotiated by `from_json`: a well-formed
    newer version must reach `negotiate` and receive `PLG-004`, while a malformed one is `PLG-011`'
  - '`schemas/plugin-manifest.schema.json` is new, self-contained Draft 2020-12 with an `$id`
    matching the file name, `additionalProperties: false` at every object level, the same name,
    tool, and `SemVer` patterns as the SDK, and an `allOf` rule that rejects `destructive: true` on
    a read capability. `plugin_api_version` is a four-digit decimal string rather than `const "1"`,
    so a plugin written for a future API version validates and is rejected at negotiation with both
    versions named. The canonical fixture `tests/fixtures/aws.json` is the `PLUGIN_SPEC` 2 manifest;
    `tests/manifest_schema.rs` proves it validates, proves the schema itself is meta-schema-valid,
    and proves individual mutations fail for the documented reasons'
  - '`protocol.rs` defines JSON-RPC 2.0 request, notification, success, and error envelopes with the
    six `PLUGIN_SPEC` 4 methods and a checked direction for each. `RequestId` accepts non-negative
    integers and strings; `FrameCodec` enforces one newline-terminated object per line and a
    caller-chosen byte cap. Tool descriptors accept MCP `name`, `description`, and `inputSchema` on
    input while emitting `tool`, `summary`, and `input_schema`; `CallToolResult` accepts `isError`
    as an alias for `is_error`; MCP content blocks, tool annotations, and unknown fields survive a
    round trip. `initialize`, `tools/list`, `tools/call`, `shutdown`, `log`, and
    `host/request_approval` all have typed params and results'
  - '`version.rs` parses a single decimal plugin API version, exposes `ApiVersion::CURRENT` and
    `SUPPORTED`, and implements `negotiate`. A mismatch returns
    `PluginError::ApiVersionMismatch`, which maps to `PLG-004` and names both the plugin''s version
    and the host''s. A malformed value returns `PLG-011`. A host that later supports more than one
    version only widens `SUPPORTED`; the plugin-facing call does not change'
  - '`crates/aicontext-core/src/error.rs` adds `ErrorFamily::Plugin` and `ErrorCode::plugin`, with
    only the three codes this task can produce: `PLG_004` mismatch, `PLG_005` invalid frame, and the
    additive `PLG_011` invalid manifest. TASK-073 adds the process-supervision codes with the
    runtime that can return them. The catalogue test is now 29 distinct codes, and a new test
    proves `PLG-004` and `CTX-004` share a number but not a rendered code'
  - '`docs/PLUGIN_SPEC.md` 4 lists `PLG-011` invalid manifest. `docs/PLUGIN_SPEC.md` 2 already
    requires negotiation at handshake; the schema and SDK now implement it.
    `crates/aicontext-cli/tests/schemas.rs` adds `plugin-manifest.schema.json` to
    `EXPECTED_SCHEMAS` and says why a published external contract belongs there. `README.md` lists
    the SDK, shows it depending on core in the diagram, and removes it from the not-yet-created list'
  - 'Tests: 47 unit tests in the SDK, 11 schema-contract tests, and 2 doctests. The schema tests
    cover the canonical manifest, a minimal manifest, a newer API version that must be schema-valid,
    malformed versions, destructive-read rejection, unknown fields, two-segment tool names, a
    missing filesystem block, and an unknown auth source. The workspace suite is 958 passed, 0
    failed'
  - 'Gates: `cargo fmt --all`, `cargo check -p aicontext-plugin-sdk --all-targets`, `cargo clippy
    --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` (958
    passed), `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked`, `aicontext
    doctor` (exit 0, only the pre-existing `CTX-012` warning about missing `.ai/schemas` copies),
    and `git diff --check`. No `unwrap` or `expect` outside tests, no `unsafe` outside the `forbid`
    attribute, and no secret values in the crate, tests, or fixtures'
  - 'Deliberately not done: no plugin runtime, process spawn, supervisor, install/list/remove
    command, approval broker, MCP bridge, or CLI wiring. Those are TASK-072 to TASK-078. The SDK
    intentionally does not validate the full schema at run time; install (TASK-077) does that. The
    plugin schema is not scaffolded into `.ai/schemas` because it governs an external plugin
    artefact, not an `.ai/` document, so `doctor` still reports 20 documents'
```

### TASK-071 — Implement the permission resolver

```yaml
id: TASK-071
title: Implement the permission model and resolver
status: DONE
priority: CRITICAL
phase: 4
depends_on: [TASK-070]
spec: docs/SECURITY.md
touches: ["crates/aicontext-permissions/**"]
scope_notes:
  - 'Root Cargo.toml, Cargo.lock, README.md, docs/PLUGIN_SPEC.md, and .ai/MEMORY.md are touched
    beside the declared crates/aicontext-permissions/**. A new workspace member cannot be built
    without the root member list, the lockfile records the package, the README layout must name the
    crate, PLUGIN_SPEC 5 has to describe the committed policy shape rather than the older
    `permissions:` example, and MEMORY records the default-mode and wildcard decisions.'
acceptance:
  - Pure: no I/O, no clock, no randomness, no environment access
  - Precedence is DENY over EXPLICIT_APPROVAL over APPROVAL over ALLOW, defaulting to DENY
  - 100 percent branch coverage over the full mode matrix, including unspecified capabilities
  - Property tests over the resolution order
done:
  - '`crates/aicontext-permissions/` is a new workspace member with `policy.rs`, `resolver.rs`,
    `error.rs`, and `lib.rs`. Its only internal dependency is `aicontext-core`, because
    `PermissionMode` has one definition; its only production external dependency is `thiserror`;
    `proptest` is dev-only. There is no loader, file access, clock, randomness, environment read,
    prompt, approval broker, or tool runtime in the crate'
  - '`Policy` owns the schema''s explicit `defaults.mode` plus validated rules. `Policy::default()`
    and `Policy::deny_all()` are deny-all, `Rule::exact` accepts local and dotted tool names, and
    `Rule::wildcard` accepts only `provider.service.*` with `deny` or `approval`. Empty patterns, a
    `*` in an exact rule, a top-level `*`, `*.action`, extra or empty segments, embedded `*`, a
    wildcard `allow` or `explicit_approval`, and duplicate patterns are refused with `PolicyError`
    carrying `CTX-003`'
  - '`Resolver::resolve` implements the eight PLUGIN_SPEC 5 steps literally: exact deny, wildcard
    deny, exact explicit approval, the destructive floor, exact approval, wildcard approval, exact
    allow, then the policy default. `Decision` carries both the mode and a `DecisionSource` naming
    the step that won. A wildcard covers exactly one action segment, so `aws.ec2.*` matches
    `aws.ec2.describe` but not `aws.ec2` or `aws.ec2.describe.extra`'
  - 'Tests: 28 unit tests, 7 proptest properties, and 1 doctest. The matrix test runs every one of
    the four default modes against every one of the four exact rule modes and both destructive
    states, then every default against both valid wildcard modes and both destructive states, plus
    an unmatched capability for each default. Direct helper tests cover every branch of the
    wildcard matcher. Properties cover determinism, reversed rule order, exact and wildcard deny
    dominance, the destructive floor, wildcard approval ahead of exact allow, and default fallback'
  - 'Branch coverage: every resolver return and every wildcard-matcher branch is asserted by the
    matrix and helper tests, including unspecified capabilities and both destructive states.
    `cargo-llvm-cov` and `cargo-tarpaulin` are not installed on this machine, so the 100 percent
    branch claim is demonstrated by exhaustive tests rather than a generated coverage report'
  - '`docs/PLUGIN_SPEC.md` 5 now uses the committed `version`/`defaults`/`tools` shape, states that
    file order never matters, records that a wildcard DENY beats an exact EXPLICIT_APPROVAL and a
    wildcard APPROVAL beats an exact ALLOW, limits wildcards to deny/approval and one action
    segment, and explains the explicit default versus an absent policy. `.ai/MEMORY.md` adds
    MEM-038; `README.md` adds the crate to the graph and layout and reduces the not-yet-created list
    to the runtime and audit crates'
  - 'Gates: `cargo fmt --all --check`; `cargo check --workspace --all-targets --locked`; `cargo
    clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`
    (994 passed, 0 failed, was 958); `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
    --locked`; `aicontext doctor` (exit 0, only the pre-existing CTX-012 warning about no
    `.ai/schemas` copies); and `git diff --check`. No `unwrap` or `expect` outside tests, no
    `unsafe` outside the `forbid` attribute, and no secret value in the crate, tests, or docs'
  - 'Deliberately not done: no policy-file loader, approval broker, tool gateway, plugin runtime,
    audit log, or CLI wiring. Those are TASK-072 to TASK-075. `scope` and `reason` remain approval
    metadata and are not resolution inputs'
```

### TASK-072 — Implement the approval broker

```yaml
id: TASK-072
title: Implement the approval broker
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-071]
spec: docs/SECURITY.md
touches: ["crates/aicontext-plugin-runtime/**", "crates/aicontext-cli/**"]
acceptance:
  - A TTY prompt shows the tool, target, arguments, and reason, and requires an explicit answer
  - A non-interactive session denies rather than assuming consent
  - EXPLICIT_APPROVAL cannot be granted for a session; it is per invocation
  - Every prompt and every answer is audited
```

### TASK-073 — Implement the plugin runtime supervisor

```yaml
id: TASK-073
title: Implement the plugin runtime supervisor
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-070]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-runtime/**"]
acceptance:
  - Spawns the plugin, performs the handshake, and negotiates the API version
  - Enforces a per-call timeout, an output size cap, and a clean kill on both
  - A crashing, hanging, or flooding plugin yields a typed error and never wedges the CLI
  - Covered by deliberately misbehaving fixture plugins
```

### TASK-074 — Implement the tool gateway

```yaml
id: TASK-074
title: Implement the tool gateway
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-071, TASK-072, TASK-073]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-runtime/**"]
acceptance:
  - Every call passes argument validation, permission resolution, approval, execution, and audit
  - There is exactly one code path from a tool call to a plugin, enforced by review and tests
  - Responses are normalised into an untrusted data envelope with size and time caps
  - No shell execution path exists
```

### TASK-075 — Implement the hash-chained audit log

```yaml
id: TASK-075
title: Implement the hash-chained audit log
status: TODO
priority: HIGH
phase: 4
depends_on: [TASK-073]
spec: docs/SECURITY.md
touches: ["crates/aicontext-audit/**"]
acceptance:
  - Append-only JSONL where each record includes the digest of the previous record
  - Verifies the chain on read and refuses to append to a broken chain
  - Stores argument digests, never raw secret-bearing values
  - aicontext audit show renders a readable timeline and --json a stable shape
```

### TASK-076 — Implement the sandbox

```yaml
id: TASK-076
title: Implement the plugin sandbox
status: TODO
priority: CRITICAL
phase: 4
depends_on: [TASK-073]
spec: docs/SECURITY.md
touches: ["crates/aicontext-plugin-runtime/**"]
acceptance:
  - Per-plugin working directory and an explicit filesystem path allowlist
  - Declared network egress allowlist; denied by default
  - No ambient access to the host home directory or unrelated environment variables
  - Escape attempts fail with a typed, audited error
```

### TASK-077 — Implement aicontext plugin

```yaml
id: TASK-077
title: Implement aicontext plugin list, install, and remove
status: TODO
priority: HIGH
phase: 4
depends_on: [TASK-070, TASK-071]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-cli/**"]
acceptance:
  - install validates the manifest, prints the permission diff, and requires confirmation
  - A plugin requesting an unknown permission is refused
  - remove revokes permissions and deletes the local state it owned
  - A lockfile-style record of installed plugins and their granted permissions
```

### TASK-078 — Verify MCP compatibility

```yaml
id: TASK-078
title: Verify MCP interoperability
status: TODO
priority: MEDIUM
phase: 4
depends_on: [TASK-073, TASK-074]
spec: docs/PLUGIN_SPEC.md
touches: ["crates/aicontext-plugin-runtime/**", "docs/PLUGIN_SPEC.md"]
acceptance:
  - A third-party MCP server can be registered as a plugin with a permission declaration
  - Tool discovery, invocation, and error shapes match the MCP specification
  - Documented limitations are listed explicitly
```

---

## Phase 5 — GitHub plugin (first real plugin)

```yaml
id: TASK-080
title: Implement the GitHub read-only plugin
status: TODO
priority: HIGH
phase: 5
depends_on: [TASK-077]
spec: docs/PLUGIN_SPEC.md
acceptance:
  - Tools github.repo.read, github.issue.list, github.issue.read, github.pr.read,
    github.branch.list, github.commit.list, github.file.read
  - All read tools resolve to ALLOW only after explicit grant; none may write
  - Issue and PR bodies are returned as untrusted data, never as instructions
  - Rate-limit and pagination errors are typed and actionable
```

```yaml
id: TASK-081
title: Add GitHub write tools behind approval
status: TODO
priority: MEDIUM
phase: 5
depends_on: [TASK-080]
spec: docs/PLUGIN_SPEC.md
acceptance:
  - github.issue.create and github.pr.create default to APPROVAL
  - The prompt shows the exact rendered body that will be sent
```

---

## Phase 6 — AWS plugin (read-only first)

```yaml
id: TASK-090
title: Implement the AWS read-only plugin
status: TODO
priority: MEDIUM
phase: 6
depends_on: [TASK-081]
spec: docs/PLUGIN_SPEC.md
acceptance:
  - Tools for EC2 describe/list, S3 list/head, RDS describe, CloudWatch read, Cost Explorer read
  - Least-privilege credentials only; no credential value is stored or logged
  - Every response is untrusted data with a size cap
```

```yaml
id: TASK-091
title: Add guarded AWS write operations
status: TODO
priority: LOW
phase: 6
depends_on: [TASK-090]
spec: docs/SECURITY.md
acceptance:
  - Start and stop resolve to APPROVAL; terminate to EXPLICIT_APPROVAL per invocation
  - Every call is audited with the approval decision and the acting agent
  - A dry-run mode shows the exact API call without sending it
```

---

## Phase 7 — Security hardening

```yaml
id: TASK-100
title: Prove the prompt-injection containment
status: TODO
priority: CRITICAL
phase: 7
depends_on: [TASK-074]
spec: docs/SECURITY.md
acceptance:
  - A named test proves a hostile instruction in a tool result cannot alter policy
  - A named test proves retrieved content never reaches an instruction role
  - A named test proves no model or tool output is ever passed to a shell
```

```yaml
id: TASK-101
title: Add the cloned-repository policy review
status: TODO
priority: HIGH
phase: 7
depends_on: [TASK-014]
spec: docs/SECURITY.md
acceptance:
  - doctor renders a digest and summary of the .ai policy files found in an unfamiliar repository
  - A repository that attempts to weaken permissions is flagged before any write action
```

---

## Phase 8 — Dashboard (deferred)

Read-only local UI over the same crates: overview, PRD, architecture, tasks, specs, memory,
decisions, bugs, changes, workflows, agents, plugins, permissions, audit, settings. No new write
paths. Blocked until Phase 4 exit criteria are met.

---

## Phase 9 — Multi-agent (deferred)

Planner, Architect, Developer, Tester, Reviewer, DevOps, composed from `.ai/agents/*.md` profiles
and the Phase 4 tool gateway. Explicitly blocked until the single-agent loop is reliable in
production use. No new primitive will be invented for it; it composes existing seams.

---

## Phase 10 — Marketplace and cloud (deferred)

Plugin registry, team collaboration, organisations, hosted context, SSO, RBAC. Requires a stable
plugin API (v1) and evidence that the core works without any of it.

---

## Definition of done

A task is `DONE` only when all of the following are true:

```text
[ ] Every acceptance criterion is demonstrably met
[ ] Tests exist and pass, including a failing-before/passing-after test for bug fixes
[ ] cargo fmt --check, cargo clippy -- -D warnings, cargo test are green
[ ] No new dependency without a recorded justification
[ ] Public API and CLI changes are documented in the same change
[ ] An ADR exists if an architectural decision was made
[ ] The task status here is updated in the same change
```
