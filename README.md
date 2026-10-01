# AI Context OS

**Git-native context, memory, governance, and tool integration for AI-assisted development.**

AI coding agents are individually capable and collectively unreliable, because they are *stateless
with respect to the project*. AI Context OS gives them a persistent, structured, reviewable
understanding of a repository, and gives you a control plane over what they are allowed to do.

Three promises:

1. **AI models change. Project knowledge persists.** The knowledge lives in the repository, in
   plain Markdown, not in a vendor's chat history.
2. **Code changes often. Decisions stay traceable.** Every architectural choice and every fixed
   bug is a versioned artifact with a recorded reason.
3. **AI assists; the developer decides.** Dangerous actions require explicit human approval, and
   every consequential action is auditable.

The deliverable is a local-first CLI named `aicontext`, written in Rust, shipping a single static
binary.

---

## Status

**Specification complete. Implementation in progress (Phase 1 of 11).**

| Phase | Scope | State |
|-------|-------|-------|
| 0 | Specification and architecture | Done |
| 1 | Context MVP: `init`, `status`, `doctor` | In progress |
| 2 | Discovery, indexing, retrieval, budgets | Not started |
| 3 | AI provider layer (plan mode only) | Not started |
| 4 | Plugin system, permissions, audit | Not started |
| 5-7 | GitHub plugin, AWS plugin, hardening | Not started |
| 8-10 | Dashboard, multi-agent, marketplace | Deferred |

**One real command ships today.** `aicontext init` creates the `.ai` skeleton; every other command in
the plan refuses to run and names the task that will build it. The task register in
[`.ai/TASKS.md`](.ai/TASKS.md) is the authoritative list of what is being built, in what order, and
what "done" means for each item.

```sh
cargo run -p aicontext-cli -- init --dry-run   # show the plan, write nothing
cargo run -p aicontext-cli -- init             # create .ai/
```

`init` writes the skeleton, appends one `.gitignore` entry, and never touches anything else. A second
run reports every document as unchanged; an edited document is preserved rather than overwritten. The
foundation underneath it is complete and tested: `aicontext-context` splits a document, reads its
front matter into typed values, and renders it back so a rewrite preserves the author's meaning rather
than the author's formatting, covered by unit tests, hand-written cases, and property tests over
documents nobody wrote by hand.

---

## How it works

Project knowledge is committed to the repository under `.ai/`, so it is versioned, reviewable, and
portable between machines and between AI tools.

```text
.ai/
  AI.md              # agent entry point and the mandatory workflow
  PRD.md             # what the product is and why
  ARCHITECTURE.md    # crate graph, storage model, runtime flows
  RULES.md           # engineering rules with justifications
  CONVENTIONS.md    # naming, layout, error handling
  DESIGN.md          # index into specs
  TASKS.md           # the task register
  MEMORY.md          # durable decisions and open questions
  specs/             # feature specifications
  context/           # domain knowledge, terminology, environments
  tasks/             # one file per task
  decisions/         # ADRs: one decision, one file, one reason
  bugs/              # bug memory: what broke and why
  changes/           # change history
  workflows/         # recurring procedures
  agents/            # agent role profiles
  integrations/      # external system notes
  permissions/       # permissions.yaml
  schemas/           # JSON Schema, materialised by the CLI
```

Working state that should never be committed lives in `.aicontext/`, which is git-ignored.

Two properties are enforced in CI and by review, not by convention alone:

- **`.ai/` is committed, `.aicontext/` is not.** Knowledge persists; scratch state does not.
- **Agents cannot write outside a plan.** A change is proposed, reviewed, and only then applied.

---

## Repository layout

```text
crates/
  aicontext-core           # domain types and the error model. Depends on nothing internal.
  aicontext-context        # document discovery, parsing, assembly
  aicontext-git            # thin wrapper over the git binary
  aicontext-cli            # the `aicontext` binary
  aicontext-testkit        # test support: manifest reader, boundary policy
.ai/                       # the project's own context, dogfooding the product
docs/                      # normative specifications
.github/workflows/         # CI
```

Five more crates are specified but do not exist yet: `aicontext-providers`,
`aicontext-plugin-sdk`, `aicontext-plugin-runtime`, `aicontext-permissions`, and
`aicontext-audit`. They are added by the task that needs them, and never speculatively.

The dependency graph is a DAG pointing inward toward `aicontext-core`, and it is checked by
`crates/aicontext-testkit/tests/crate_boundaries.rs` on every CI run. Adding a workspace member
without declaring what it may depend on fails the build.

---

## Requirements

- Rust 1.85 or newer. That is the **MSRV**, declared as `rust-version` in the workspace
  `Cargo.toml` and proved real by a dedicated CI job, not by assertion.
- [`rust-toolchain.toml`](rust-toolchain.toml) pins the toolchain that local builds, CI, and
  release artefacts all use — currently 1.95.0 — so `rustup` installs it for you, and a
  contributor is never forced off their own stable to match. Update the pin deliberately, in its
  own commit.
- `git` on `PATH`.

No C compiler and no system library are needed: every dependency is pure Rust.

Third-party crates are added only with a written justification in
[`.ai/ARCHITECTURE.md`](.ai/ARCHITECTURE.md) §2.2 or a decision record. The tree currently carries five
in production — `thiserror` for typed errors, `yaml_serde` for YAML, `clap` for the command tree, and
`serde` with `serde_json` for the published `--json` envelope — plus `proptest` and `tempfile` for
tests. The YAML library is reached only through a private two-method trait, so no YAML type appears in
a public signature and a future swap touches one module; see
[`ADR-007`](.ai/decisions/ADR-007-yaml-codec-choice.md).

## Build and test

```sh
cargo build --workspace --locked                       # build everything
cargo test --workspace --locked                        # run the test suite
cargo fmt --all --check                                # formatting must be clean
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo audit                                            # no known vulnerabilities
```

Parser properties run 1,000 cases by default. To run them hard:

```sh
PROPTEST_CASES=50000 cargo test -p aicontext-context --test frontmatter
```

The suite is expected to pass on the pinned toolchain and on the MSRV; CI checks both.
`cargo run -p aicontext-cli -- --help` lists what is implemented and what is not.

---

## Using the tool

`init` is the only command that runs today. The rest are listed so the shape is on the record; each one
exits with code 2 and names the task that will build it, rather than pretending to work. Exit codes
and the `--json` envelope are a stable contract, in
[`docs/CLI_SPEC.md`](docs/CLI_SPEC.md).

```text
aicontext
├─ init                     create the .ai skeleton              available
├─ status                   project, branch, phase, current task, changes   TASK-013
├─ doctor                   validate and diagnose context                  TASK-014
├─ health                   transparent context metrics                     TASK-038
├─ context                  assemble a context packet                       TASK-037
│   ├─ show                 render the packet
│   └─ explain              show the plan, scores, and drops
├─ plan                     produce an implementation plan for a task
├─ task | memory | decision | bug | change | workflow | agent               TASK-055
├─ ai | plugin              plugin management                              TASK-077
├─ connect | disconnect     configure or remove an integration              TASK-053
├─ export | import          portable .ai archives                         TASK-020
└─ audit                    audit log (show, verify, tail)                  TASK-075
```

### `init`

```sh
aicontext init [--template default|rust|node|python|blank] [--dry-run] [--force] [--no-detect]
```

Creates the skeleton under `.ai/`, appends `.aicontext/` to `.gitignore` if it is not listed already,
and writes nothing outside those two places. It never creates `.aicontext/` itself.

| Flag | Effect |
|------|--------|
| `--template <name>` | `default` is the base skeleton; `rust`, `node`, and `python` add stack-specific conventions and architecture; `blank` writes only `AI.md` and `RULES.md`. |
| `--dry-run` | Print the plan and write nothing. |
| `--force` | Replace documents that differ from the template. Refuses with exit 5 when stdout is not a terminal, because an absent human is a deny. |
| `--no-detect` | Skip discovery, so no stack hints are reported. |

Global flags apply: `--json`, `--quiet`, `-v`, `--color`, `--no-color`, `--cwd`, `--config`,
`--offline`, `--yes`, and `--version`.

Discovery is advisory. `init` reports the files it saw at the top level — `Cargo.toml`, `package.json`,
`pyproject.toml`, Terraform — as hints, records them in `.ai/context/stack.md`, and chooses no template
for you. `AI.md` states the obligations it cannot enforce; everything else in `.ai/` is a starting
point, and the first useful edit is `.ai/RULES.md`.

Every command works without a TTY, has a documented exit code, and supports `--json`. Read
commands never touch the network, and anything that writes supports `--dry-run`.

---

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) is the practical guide: setup, the checks your change must pass,
how to add a dependency, and what needs a decision record. Read [`.ai/AI.md`](.ai/AI.md) first; it
defines the agent workflow. Then [`.ai/RULES.md`](.ai/RULES.md), which states the engineering rules
and the reason each one exists.

The short version:

1. Pick one task from [`.ai/TASKS.md`](.ai/TASKS.md). Do not start a task whose dependencies are
   not done. One task at a time.
2. If a task changes the architecture, write an ADR in `.ai/decisions/` first, with the
   alternatives you rejected.
3. Keep the workspace building: `fmt`, `clippy -D warnings`, and `test` all pass before review.
4. Explain the change in the pull request: what changed, why, what you verified, and what you
   deliberately did not do. The template asks for all four.

## Documentation

| Document | Authority |
|----------|-----------|
| [`.ai/AI.md`](.ai/AI.md) | Agent workflow, trust model, task loop |
| [`.ai/PRD.md`](.ai/PRD.md) | Product requirements and scope |
| [`.ai/ARCHITECTURE.md`](.ai/ARCHITECTURE.md) | Crate graph, storage, runtime, risks |
| [`.ai/RULES.md`](.ai/RULES.md) | Engineering rules and their justifications |
| [`.ai/CONVENTIONS.md`](.ai/CONVENTIONS.md) | Naming, layout, error handling |
| [`.ai/DESIGN.md`](.ai/DESIGN.md) | Index into the specifications |
| [`.ai/TASKS.md`](.ai/TASKS.md) | Task register: order, dependencies, acceptance |
| [`.ai/MEMORY.md`](.ai/MEMORY.md) | Durable decisions and open questions |
| [`docs/CONTEXT_SPEC.md`](docs/CONTEXT_SPEC.md) | Context file formats and schema |
| [`docs/PLUGIN_SPEC.md`](docs/PLUGIN_SPEC.md) | Plugin protocol and lifecycle |
| [`docs/AI_PROVIDER_SPEC.md`](docs/AI_PROVIDER_SPEC.md) | Provider abstraction and routing |
| [`docs/SECURITY.md`](docs/SECURITY.md) | Threat model and security controls |
| [`docs/CLI_SPEC.md`](docs/CLI_SPEC.md) | Commands, flags, exit codes |

## License

[Apache License 2.0](LICENSE).
