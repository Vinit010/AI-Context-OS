# AI Context OS

> **Git-native context, memory, governance, and tool integration for AI-assisted development.**

[![CI](https://github.com/Vinit010/AI-Context-OS/actions/workflows/ci.yml/badge.svg)](https://github.com/Vinit010/AI-Context-OS/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV: Rust 1.85](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](rust-toolchain.toml)
[![Status: Phase 4 of 11](https://img.shields.io/badge/status-phase%204%20of%2011-yellow.svg)](#status)

AI coding agents are individually capable and collectively unreliable, because they are *stateless
with respect to the project*. AI Context OS gives them a persistent, structured, reviewable
understanding of a repository, and gives you a control plane over what they are allowed to do.

The deliverable is a **local-first CLI named `aicontext`**, written in Rust, shipping a single static
binary. Project knowledge lives in the repository as plain Markdown, so it is versioned, reviewable,
and portable between machines and between AI tools.

| Promise | What it means |
|---------|---------------|
| **AI models change. Project knowledge persists.** | The knowledge lives in the repository, not in a vendor's chat history. |
| **Code changes often. Decisions stay traceable.** | Every architectural choice and fixed bug is a versioned artifact with a recorded reason. |
| **AI assists; the developer decides.** | Dangerous actions require explicit human approval, and every consequential action is auditable. |

---

## Contents

- [Status](#status)
- [Quick start](#quick-start)
- [How it works](#how-it-works)
- [Features](#features)
- [Repository layout](#repository-layout)
- [Requirements](#requirements)
- [Build and test](#build-and-test)
- [Commands](#commands)
- [Contributing](#contributing)
- [Documentation](#documentation)
- [License](#license)

---

## Status

**Specification complete. Implementation in progress (Phase 4 of 11).**

| Phase | Scope | State |
|-------|-------|-------|
| 0 | Specification and architecture | Done |
| 1 | Context MVP: `init`, `status`, `doctor`, `export`, `import` | Done |
| 2 | Discovery, indexing, retrieval, budgets, `health`, `context` | Done |
| 3 | AI provider layer: adapters, credentials, `connect`, `agent run` (plan mode) | Done |
| 4 | Plugin system, permissions, audit | In progress |
| 5-7 | GitHub plugin, AWS plugin, hardening | Not started |
| 8-10 | Dashboard, multi-agent, marketplace | Deferred |

**Eleven commands ship today: `init`, `status`, `doctor`, `health`, `context`, `export`, `import`,
`connect`, `disconnect`, `agent run`, and `audit`.**

`init` creates the `.ai` skeleton, `status` reports where the project and its task register stand,
`doctor` validates the `.ai/` tree and explains what is wrong with it, `health` reports transparent
context metrics, `context` assembles a context packet for a task or query, `export` writes a
deterministic archive of `.ai/` with a digest manifest, and `import` verifies such an archive and
restores it without ever overwriting a file without `--force`. `connect` proves a provider answers
and records only the credential's source — the value goes to the OS keychain, never to `.ai/`;
`disconnect` removes that reference. `agent run --task <id>` assembles context, asks the configured
provider for a plan, and writes it under `.aicontext/plans/`; it modifies nothing in the working
tree. `audit show|verify|tail` reads the hash-chained log: every record is verified from genesis
before it is rendered, a break is reported with the verified prefix, and `verify` prints the head
digest a reviewer can witness.

Every other command in the plan refuses to run and names the task that will build it. Phase 3 is
complete: provider-neutral request/response types, the `AIProvider` trait, the registry, three
adapters, credential resolution, and a plan-mode agent loop all exist. Phase 4 is complete: the
plugin SDK, the pure permission resolver, the approval broker, the plugin supervisor, the tool
gateway, the plugin sandbox, an MCP adapter, the hash-chained audit log, and
`plugin install|list|remove`. The gateway is a library edge and nothing appends to the
audit log yet: the CLI reads it, and the first command that executes a plugin becomes its writer.
`plugin install` records a grant; it does not execute the plugin.
The task register in [`.ai/TASKS.md`](.ai/TASKS.md) is the authoritative list of what is
being built, in what order, and what "done" means for each item.

```sh
cargo run -p aicontext-cli -- init --dry-run   # show the plan, write nothing
cargo run -p aicontext-cli -- init             # create .ai/
```

`init` writes the skeleton, appends one `.gitignore` entry, and never touches anything else. A second
run reports every document as unchanged; an edited document is preserved rather than overwritten.

---

## Quick start

```sh
# 1. Build the binary from source.
cargo build --release --locked -p aicontext-cli

# 2. Scaffold a project (write nothing yet).
cargo run -p aicontext-cli -- init --dry-run

# 3. Create the .ai/ knowledge tree.
cargo run -p aicontext-cli -- init

# 4. Ask where the project stands.
cargo run -p aicontext-cli -- status

# 5. Check the tree is healthy.
cargo run -p aicontext-cli -- doctor

# 6. Assemble the context a model would be given for a task.
cargo run -p aicontext-cli -- context --task TASK-001
```

Every command has a documented exit code and a machine-readable `--json` mode. Read commands never
touch the network. Commands that modify the knowledge tree support `--dry-run`; `connect` makes its
verification call before writing anything, and `agent run` is plan-only.

---

## How it works

Project knowledge is committed to the repository under `.ai/`. The CLI reads it, assembles a
**context packet** for a specific task, and hands that packet to a model as *plan mode only*: the
model proposes, the human approves, and only then does anything with write access run. The context
engine, provider layer, plan-mode agent loop, approval broker, plugin supervisor, tool gateway,
plugin sandbox, MCP adapter, and local plugin registry exist today. The gateway is not yet wired to a
CLI command, so no plugin can be executed from `aicontext` itself; `plugin install` records the
permission grant, and execution arrives with the Phase 5 loop.

```mermaid
flowchart LR
    K[".ai/ knowledge<br/>(committed Markdown)"]
    C["aicontext-context<br/>assemble a ContextPacket"]
    P["aicontext-providers<br/>AIProvider trait + registry"]
    M["Model<br/>(hosted or local)"]
    A{"Human approval"}
    W["Tool gateway<br/>(the only writer)"]
    N["Nothing is written"]

    K --> C --> P --> M
    M -->|action proposal| A
    A -->|approved| W
    A -->|denied| N
```

Everything the agent knows is plain files in the repository:

```text
.ai/
  AI.md              # agent entry point and the mandatory workflow
  PRD.md             # what the product is and why
  ARCHITECTURE.md    # crate graph, storage model, runtime flows
  RULES.md           # engineering rules with justifications
  CONVENTIONS.md     # naming, layout, error handling
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

## Features

| Area | Capability |
|------|------------|
| **Durable project memory** | Requirements, architecture, rules, decisions, bugs, and task state as versioned Markdown under `.ai/`. |
| **Assembled context** | A bounded, provenance-carrying `ContextPacket` per task, with a transparent retrieval plan and budget report. |
| **Validation you can trust** | `doctor` reports stable `CTX-*` codes with the exact fix, and never silently repairs the tree. |
| **Provider-neutral by design** | `CompletionRequest`/`CompletionResponse` are provider-agnostic; OpenAI-compatible, Anthropic, and Gemini adapters, the credential resolver, and plan-mode `agent run` share one interface. |
| **Credentials stay out of Git** | `connect` proves a provider answers, stores the value in the OS keychain, and commits only the credential's source. |
| **Honest metrics** | `health` reports counts it can source, each with the command that reproduces it, and no composite score. |
| **Portable archives** | `export`/`import` round-trip `.ai/` deterministically, with a SHA-256 digest per file. |
| **Local-first** | Single static binary, no runtime prerequisite, no telemetry, and read commands never touch the network. |

---

## Repository layout

```mermaid
flowchart TD
    CLI["aicontext-cli<br/>the `aicontext` binary"]
    CTX["aicontext-context<br/>discovery, parsing, retrieval, validation"]
    GIT["aicontext-git<br/>thin wrapper over the git binary"]
    CORE["aicontext-core<br/>domain types and the error model"]
    SDK["aicontext-plugin-sdk<br/>manifest and protocol types for plugin authors"]
    PERM["aicontext-permissions<br/>policy model and pure resolver"]
    PROV["aicontext-providers<br/>AIProvider trait and adapters"]
    RT["aicontext-plugin-runtime<br/>approval broker, supervisor, sandbox, tool gateway"]
    AUD["aicontext-audit<br/>hash-chained append-only log"]
    TK["aicontext-testkit<br/>fixtures and workspace boundary checks"]

    CLI --> CTX
    CLI --> GIT
    CLI --> CORE
    CLI --> PROV
    CLI --> RT
    CLI --> AUD
    CTX --> CORE
    GIT --> CORE
    SDK --> CORE
    PERM --> CORE
    RT --> CORE
    RT --> PERM
    RT --> SDK
    AUD --> CORE
```

`aicontext-plugin-sdk` is a library for plugin authors and depends on `aicontext-core` only.
`aicontext-permissions` is pure policy data and resolution: no loader, no prompt, no audit entry,
and no I/O of any kind. `aicontext-plugin-runtime` owns the approval decision, the plugin process,
and the one gateway that reaches it. It has no terminal and no clock of its own: the CLI supplies
the terminal when a command eventually wires it. Source-scan tests enforce one production call site
to the supervisor and no process construction in the gateway.

`aicontext-audit` owns the durable log: the record shape, the SHA-256 chain, verification on every
read, and refusal to append to a chain that does not verify. It depends on `aicontext-core` only,
and the CLI is its sole reader today. The plugin runtime writes through a narrow audit sink and does
not depend on the audit crate, so a plugin author never links the log.

`aicontext-providers` is wired to the CLI: `connect` uses it for one verification call, and
`agent run` uses it to plan. `aicontext-testkit` has no internal crate dependency — it reads every
other crate's manifest to enforce the boundary policy in CI.

```text
crates/
  aicontext-core           # domain types and the error model. Depends on nothing internal.
  aicontext-context        # document discovery, parsing, retrieval, validation
  aicontext-git            # thin wrapper over the git binary
  aicontext-permissions    # policy model and pure permission resolver
  aicontext-audit          # hash-chained, append-only audit log
  aicontext-plugin-sdk     # manifest and protocol types for plugin authors
  aicontext-plugin-runtime # approval broker, supervisor, sandbox, tool gateway
  aicontext-providers      # AIProvider trait, neutral types, and the registry
  aicontext-cli            # the `aicontext` binary
  aicontext-testkit        # test support: manifest reader, boundary policy
.ai/                       # the project's own context, dogfooding the product
docs/                      # normative specifications
.github/workflows/         # CI
```

The plugin SDK and permissions crate depend on `aicontext-core` only; the audit crate does too. The
runtime also uses the permissions resolver and the SDK, and remains a library with no terminal, no
durable log, and no CLI wiring, so a plugin author never links the runtime or the CLI.

The dependency graph is a DAG pointing inward toward `aicontext-core`, and it is checked by
`crates/aicontext-testkit/tests/crate_boundaries.rs` on every CI run. Adding a workspace member
without declaring what it may depend on fails the build.

---

## Requirements

- **Rust 1.85 or newer.** That is the **MSRV**, declared as `rust-version` in the workspace
  `Cargo.toml` and proved real by a dedicated CI job, not by assertion.
- [`rust-toolchain.toml`](rust-toolchain.toml) pins the toolchain that local builds, CI, and release
  artefacts all use — currently 1.95.0 — so `rustup` installs it for you, and a contributor is never
  forced off their own stable to match. Update the pin deliberately, in its own commit.
- `git` on `PATH`.
- **A C compiler available at build time.** `reqwest` uses `rustls`, whose `ring` provider compiles a
  small amount of C. This is the one accepted deviation from the pure-Rust dependency rule, recorded
  in [`ADR-009`](.ai/decisions/ADR-009-http-tls-and-the-c-compiler-deviation.md). No OpenSSL or other
  third-party system TLS library is needed, and the built binary has no compiler or library
  prerequisite at runtime.

Third-party crates are added only with a written justification in
[`.ai/ARCHITECTURE.md`](.ai/ARCHITECTURE.md) §2.2 or a decision record. Production code currently
uses 13 direct dependencies: `thiserror` for typed errors, `yaml_serde` for YAML, `clap` for the
command tree, `serde` with `serde_json` for the published `--json` envelope, `sha2` for content and
archive digests, `ignore` for `.gitignore`-aware discovery, `async-trait` with `futures-core` for the
dyn-safe `AIProvider` interface, `tokio` as the async runtime, `reqwest` over `rustls` for provider
HTTP, `tracing` for structured diagnostics, and `keyring` for the OS credential store. Tests add
`proptest`, `tempfile`, `jsonschema`, and `futures-executor`; `tempfile` also backs the testkit. The
YAML library is reached only through a private two-method trait, so no YAML type appears in a public
signature and a future swap touches one module; see
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

Parser properties run 1,000 cases by default; raise the count for a longer soak:

```sh
PROPTEST_CASES=10000 cargo test -p aicontext-context --locked
```

---

## Commands

```text
aicontext
├─ init                     create the .ai skeleton                          available
├─ status                   project, branch, phase, current task, changes    available
├─ doctor                   validate and diagnose context                    available
├─ health                   transparent context metrics                       available
├─ context                  assemble a context packet                         available
│                            (--task, --include, --exclude, --budget, --explain, --json)
├─ export | import          portable .ai archives                            available
├─ connect | disconnect     configure or remove a provider                   available
├─ agent run                plan a task with the configured provider         available
├─ audit                    read the hash-chained log (show, verify, tail)    available
├─ plugin                   install, list, remove local plugins               available
├─ plan | task | memory | decision | bug | change | workflow                 planned
└─ ai                       provider configuration                           planned
```

Global flags apply to every command: `--json`, `--quiet`, `-v`, `--color`, `--no-color`, `--cwd`,
`--config`, `--offline`, `--yes`, and `--version`.

### `init`

```sh
aicontext init [--template default|rust|node|python|blank] [--dry-run] [--force] [--no-detect]
```

Creates the skeleton under `.ai/`, appends `.aicontext/` to `.gitignore` if it is not listed already,
and writes nothing outside those two places. It never creates `.aicontext/` itself.

Every template except `blank` also copies the 20 JSON Schemas into `.ai/schemas/`, so a scaffolded
project can validate its own documents. They are the same files CI checks against the Draft 2020-12
meta-schema, and they behave like any other document: an edited schema is preserved, not replaced.

| Flag | Effect |
|------|--------|
| `--template <name>` | `default` is the base skeleton; `rust`, `node`, and `python` add stack-specific conventions and architecture; `blank` writes only `AI.md` and `RULES.md`. |
| `--dry-run` | Print the plan and write nothing. |
| `--force` | Replace documents that differ from the template. Refuses with exit 5 when stdout is not a terminal, because an absent human is a deny. |
| `--no-detect` | Skip discovery, so no stack hints are reported. |

Discovery is advisory. `init` reports the files it saw at the top level — `Cargo.toml`, `package.json`,
`pyproject.toml`, Terraform — as hints, records them in `.ai/context/stack.md`, and chooses no template
for you. `AI.md` states the obligations it cannot enforce; everything else in `.ai/` is a starting
point, and the first useful edit is `.ai/RULES.md`.

### `status`

```sh
aicontext status [--json]
```

Reports where the project stands without writing anything: the project name and root, the current
branch and working-tree changes, and the register's current phase, current task, and pending tasks. The
register is read from `.ai/TASKS.md`; a project that has not run `init` is described as having no
register rather than failing. Outside a Git repository it says so and still prints the register. A warm
run over this repository stays inside the 300 ms budget.

### `doctor`

```sh
aicontext doctor [--strict] [--only <prefix>] [--explain <code>] [--rebuild-index] [--json]
```

Validates the `.ai/` tree and prints one line per finding — a stable code from
`docs/CONTEXT_SPEC.md` §8, the file, the message, and the fix — followed by the files it read and found
clean. A clean tree exits 0; any error-level finding exits 3, and `--strict` promotes warnings. The
checks that run today are `CTX-001`, `CTX-002`, `CTX-007`, `CTX-012`, `CTX-013`, `CTX-014`, `CTX-016`,
and `CTX-018`; the rest of the catalogue is listed by `doctor --json` as deferred with the task that
owns it. `CTX-016` scans `.ai/` for credential-shaped content and reports only the path, line, and rule,
never the match. `--explain <code>` answers what a check means without reading a project, `--only`
narrows the run to matching codes, and `--rebuild-index` is accepted and says out loud that this
release has no persistent index cache to rebuild.

### `health`

```sh
aicontext health [--json]
```

Reports the `docs/CONTEXT_SPEC.md` §9 metrics as transparent counts, each with the exact command that
reproduces it and no opaque composite score: context completeness, architecture consistency, task
consistency, decision coverage, broken references, and documentation freshness. A metric it cannot
source is reported as `n/a` with a reason rather than as a misleading zero. `health` is read-only: it
builds the index in memory and never creates `.aicontext/`.

### `context`

```sh
aicontext context [--task <id>] [--include <glob>] [--exclude <glob>] [--budget <tokens>] [--explain] [--json]
```

Assembles a context packet for a task or query: the trusted policy region, the retrieved items with
their provenance and scores, any detected conflicts, and the budget report. External content is wrapped
in a data envelope marked untrusted and is never placed in an instruction role. `--explain` shows the
full retrieval plan and every drop, and `--format prompt` renders the exact provider input.

### `connect` and `disconnect`

```sh
aicontext connect --provider <id> --adapter <name> --base-url <url> --model <name>
                  [--env <var> | --token <value>] [--keychain-entry <name>]
aicontext disconnect --provider <id> [--json]
```

`connect` resolves the credential, makes one minimal verification call, and only then writes the
profile to `.ai/ai.yaml`. A failed call leaves both the repository and the keychain unchanged. The
file records the credential's **source**, never its value; `--token` stores the value in the OS
keychain. `disconnect` removes the provider's reference and its keychain entry. The adapters are
`openai-compatible`, `anthropic`, and `gemini`.

### `agent run`

```sh
aicontext agent run --task <id> [--dry-run] [--json]
```

Assembles the task's context packet, asks the configured default provider for a plan, validates any
structured proposal, and writes the result to `.aicontext/plans/<task>.md`. The plan records the
provider, the context sources, and the token usage. Working-tree files are not modified, and there is
no execution mode: `--execute` is deliberately not accepted (`ADR-006`). `--dry-run` assembles and
reports without making a provider call.

### `export` and `import`

```sh
aicontext export <archive> [--dry-run] [--force] [--json]
aicontext import <archive> [--dry-run] [--force] [--json]
```

`export` writes a deterministic archive of `.ai/` — one pretty-printed JSON document with a manifest,
a SHA-256 digest per file, and a digest over the whole manifest. `import` verifies every digest before
planning a single write, then restores the tree. No timestamps, absolute paths, or host names leak
into the archive, so exporting the same tree twice produces identical bytes and a round trip on an
unedited tree is byte-identical. A relative `<archive>` path resolves against the project root.

| Flag | Effect |
|------|--------|
| `--dry-run` | Run every check (including the terminal requirement for `--force`) and write or replace nothing. |
| `--force` | `export`: replace an existing archive. `import`: overwrite a conflicting file. Requires an interactive terminal; an absent human is a deny (exit 5). |

`import` is a merge, not a mirror: it never deletes a file the archive does not mention, and an
existing file that already holds the archived bytes is reported unchanged rather than rewritten.
Reads are bounded — 512 entries, 8 MiB per file, 16 path components, and 64 MiB for the archive — and
a violation refuses the run instead of truncating. Full contract in
[`docs/CLI_SPEC.md`](docs/CLI_SPEC.md) §4; format and rationale in
[`ADR-008`](.ai/decisions/ADR-008-portable-archive-format.md).

### `plugin list|install|remove`

```sh
aicontext plugin install <path> [--as-plugin] [--yes] [--json]
aicontext plugin list [--json]
aicontext plugin remove <name> [--json]
```

`install` validates a local manifest, prints the permission diff against what is already granted and
the auth requirements, and asks for confirmation. Only the literal answer `yes` approves; `--yes` is
the non-interactive confirmation, and a run with no terminal and no `--yes` exits 5. The confirmed
grant is recorded in `.aicontext/plugins.lock.json`, and the validated manifest is copied to
`.aicontext/plugins/<name>/manifest.json`. The record is local state: every tool call is still
resolved against `.ai/permissions/permissions.yaml`, which the record cannot widen.

`--as-plugin` registers a third-party MCP server. The manifest declares `protocol: "mcp"`, and each
capability maps the name the model sees onto the server's tool with `mcp_tool`. The flag and the
manifest must agree; a mismatch is `PLUGIN-012`. The server's annotations are untrusted, and its
input schemas must fit the gateway's closed subset, so a schema without
`additionalProperties: false` is refused rather than widened
([`docs/PLUGIN_SPEC.md`](docs/PLUGIN_SPEC.md) §10).

`list` shows the installed plugins, their versions, protocol, and signature status. `remove` revokes
the recorded permissions, deletes the plugin's state tree, and removes the lockfile when the last
plugin goes. This build installs local paths only and has no signature verifier, so every install is
shown as `unsigned`; a registry reference is refused rather than guessed at.

---

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) is the practical guide: setup, the checks your change must pass,
how to add a dependency, and what needs a decision record. Read [`.ai/AI.md`](.ai/AI.md) first; it
defines the agent workflow. Then [`.ai/RULES.md`](.ai/RULES.md), which states the engineering rules
and the reason each one exists.

The short version:

1. Pick one task from [`.ai/TASKS.md`](.ai/TASKS.md). Do not start a task whose dependencies are
   not done. One task at a time.
2. If a task changes the architecture, write an ADR in `.ai/decisions/` first, with the reason and
   the alternatives considered.
3. Keep the workspace green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
   --locked -- -D warnings`, and `cargo test --workspace --locked` must all pass.
4. Open a pull request using the template. It asks for the task, what changed, how it was verified,
   and anything you deliberately left out.

---

## Documentation

| Document | Purpose |
|----------|---------|
| [`.ai/AI.md`](.ai/AI.md) | Agent entry point and mandatory workflow |
| [`.ai/PRD.md`](.ai/PRD.md) | Product requirements and scope |
| [`.ai/ARCHITECTURE.md`](.ai/ARCHITECTURE.md) | Crate graph, storage model, runtime flows |
| [`.ai/RULES.md`](.ai/RULES.md) | Engineering rules with justifications |
| [`.ai/CONVENTIONS.md`](.ai/CONVENTIONS.md) | Naming, layout, error handling |
| [`.ai/DESIGN.md`](.ai/DESIGN.md) | Index into the specifications |
| [`.ai/TASKS.md`](.ai/TASKS.md) | Task register: order, dependencies, acceptance |
| [`.ai/MEMORY.md`](.ai/MEMORY.md) | Durable decisions and open questions |
| [`docs/CONTEXT_SPEC.md`](docs/CONTEXT_SPEC.md) | Context file formats and schema |
| [`docs/PLUGIN_SPEC.md`](docs/PLUGIN_SPEC.md) | Plugin protocol and lifecycle |
| [`docs/AI_PROVIDER_SPEC.md`](docs/AI_PROVIDER_SPEC.md) | Provider abstraction and routing |
| [`docs/SECURITY.md`](docs/SECURITY.md) | Threat model and security controls |
| [`docs/CLI_SPEC.md`](docs/CLI_SPEC.md) | Commands, flags, exit codes |

---

## License

[Apache License 2.0](LICENSE).
