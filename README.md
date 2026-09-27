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

**There is no usable binary yet.** The `aicontext` command currently exits with code 2. The task
register in [`.ai/TASKS.md`](.ai/TASKS.md) is the authoritative list of what is being built, in
what order, and what "done" means for each item.

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

- Rust 1.85 or newer. The exact toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml),
  so `rustup` installs it for you.
- `git` on `PATH`.

There are no third-party Rust dependencies yet, and none will be added without a written
justification in [`.ai/ARCHITECTURE.md`](.ai/ARCHITECTURE.md).

## Build and test

```sh
cargo build --workspace           # build everything
cargo test --workspace            # run the test suite
cargo fmt --all -- --check        # formatting must be clean
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps   # documentation must build without warnings
```

`cargo run -p aicontext-cli` runs the stub binary. The first real commands arrive with Phase 1.

---

## Using the tool

Planned command surface, from [`docs/CLI_SPEC.md`](docs/CLI_SPEC.md). Exit codes and the `--json`
envelope are a stable contract.

```text
aicontext
├─ init                     create the .ai skeleton
├─ status                   project, branch, phase, current task, changes
├─ doctor                   validate and diagnose context
├─ health                   transparent context metrics
├─ context                  assemble a context packet
│   ├─ show                 render the packet
│   └─ explain              show the plan, scores, and drops
├─ plan                     produce an implementation plan for a task
├─ task | memory | decision | bug | change | workflow | agent
├─ ai | plugin | connect | disconnect
├─ export | import
└─ audit
    ├─ show | verify | tail
```

Every command works without a TTY, has a documented exit code, and supports `--json`. Read
commands never touch the network, and anything that writes supports `--dry-run`.

---

## Contributing

Read [`.ai/AI.md`](.ai/AI.md) first; it defines the agent workflow. Then
[`.ai/RULES.md`](.ai/RULES.md), which states the engineering rules and the reason each one exists.

The short version:

1. Pick one task from [`.ai/TASKS.md`](.ai/TASKS.md). Do not start a task whose dependencies are
   not done. One task at a time.
2. If a task changes the architecture, write an ADR in `.ai/decisions/` first, with the
   alternatives you rejected.
3. Keep the workspace building: `fmt`, `clippy -D warnings`, and `test` all pass before review.
4. Explain the change in the pull request: what changed, why, and what you verified.

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
