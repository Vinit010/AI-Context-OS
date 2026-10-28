# CLI_SPEC — The `aicontext` command line

Normative. Exit codes and the `--json` envelope are a **stable contract**; changing either is a
breaking change requiring a major version bump.

---

## 1. Principles

1. **Scriptable first.** Every command works without a TTY, has a documented exit code, and has
   `--json`.
2. **stdout is data, stderr is human.** Human output goes to stdout only when there is no `--json`.
   With `--json`, stdout is a single JSON object and everything else goes to stderr.
3. **Read commands are offline.** Anything documented as offline never touches the network.
4. **No surprise writes.** A command that writes says so in `--help` and supports `--dry-run`.
5. **Say what you did not do.** Every report ends with counts, the exit code, and the next command.

---

## 2. Global flags

| Flag | Effect |
|------|--------|
| `--json` | Emit a single JSON object to stdout (stable shape per command) |
| `--quiet` | Errors only |
| `--verbose` | Diagnostics to stderr; repeatable (`-vv` adds trace) |
| `--no-color` | Disable colour (also honours `NO_COLOR` and non-TTY) |
| `--color <auto\|always\|never>` | Force colour behaviour |
| `--cwd <path>` | Operate as if started in `<path>` |
| `--config <path>` | Use a specific config file |
| `--offline` | Fail rather than make any network call |
| `--yes` | Confirm non-destructive prompts only; never satisfies `EXPLICIT_APPROVAL` |
| `--allow-once <tool>` | Approve one exact tool for this run without prompting; repeatable. Ignored for `EXPLICIT_APPROVAL` |
| `--version` | Print version, plugin API version, and build profile |
| `--help` | Usage, including what the command changes and what it requires |

---

## 3. Command tree

```text
aicontext
├── init                     Create the .ai skeleton
├── status                   Project, branch, phase, current task, changes
├── doctor                   Validate and diagnose context
├── health                   Transparent context metrics
├── context                  Assemble a context packet
├── plan                     Produce an implementation plan for a task
├── task                     Task management
│   ├── list | show | create | set | link | done | next
├── memory                   Memory register
│   ├── list | show | add | supersede
├── decision                 Architecture decision records
│   ├── list | show | new | accept | deprecate
├── bug                      Bug memory
│   ├── list | show | new | resolve
├── change                   Change history
│   ├── list | new
├── workflow                 Run a documented workflow
│   ├── list | show | start
├── agent                    Agent profiles and runs
│   ├── list | run | plan
├── ai                       Provider configuration
│   ├── list | test | route
├── plugin                   Plugin management
│   ├── list | info | install | remove | enable | disable | permissions
├── connect                  Prove a provider answers, then store it
├── disconnect               Remove a stored provider reference
├── export                   Export .ai as a portable archive
├── import                   Import and verify an archive
└── audit                    Audit log
    ├── show | verify | tail
```

Phase availability is printed by `aicontext --help`; a command not yet implemented exits 2 with
`E_NOT_IMPLEMENTED` and names the task that will provide it. It is never a silent no-op.

---

## 4. Command contracts

### `aicontext init`

Creates the `.ai/` skeleton, copies the schema set into `.ai/schemas/`, and adds `.aicontext/` to
`.gitignore`. It never creates `.aicontext/` itself: the entry exists so the directory is ignored
once another command creates it.

| Flag | Effect |
|------|--------|
| `--dry-run` | Print the plan; write nothing |
| `--force` | Overwrite existing documents that differ from the template. Requires an interactive terminal; refuses with exit 5 otherwise |
| `--no-detect` | Skip project discovery |
| `--template <name>` | Start from a template (`default`, `rust`, `node`, `python`, `blank`) |

Guarantees: idempotent; never overwrites an edited document without `--force`; never modifies
anything outside `.ai/` and `.gitignore`; validates its own output and reports the next commands.

`--force` is deliberately not a second confirmation prompt. Approval requires a human, and a human
who typed `--force` at a terminal is the approval; the approval broker's line-based prompt
(`SECURITY.md` §9) belongs to tool calls, so `init` does not invent its own. `--yes` never satisfies
this check.

Schemas are materialised into `.ai/schemas/` by `TASK-015`: all 20 files, copied from the `schemas/`
source at compile time, so a scaffolded project receives exactly the bytes CI validated against the
Draft 2020-12 meta-schema. They are ordinary documents, so all the guarantees above apply to them —
in particular an edited schema is preserved and reported as CTX-017. `doctor` compares the copy
against the source (CTX-012): a missing or edited copy is a warning, and a project with no copies at
all is one finding against the directory rather than one per schema, because that is one problem
with one fix.

### `aicontext status`

Reports project name, branch, current phase, current task, modified files, and pending tasks.
Degrades gracefully outside a Git repository.

### `aicontext doctor`

| Flag | Effect |
|------|--------|
| `--strict` | Promote warnings to errors |
| `--explain <code>` | Explain a check and how to fix it |
| `--rebuild-index` | Discard the cache and re-index |
| `--only <code-prefix>` | Run a subset, e.g. `--only CTX-01` |

`--only` and `--explain` match a code prefix without regard to case, and an empty value is a usage
error rather than a silent run of everything. `--strict` raises warnings to errors and informational
findings to warnings, so the exit code is unchanged by findings that are not errors to begin with.
`--rebuild-index` is accepted and reports that there is no cache yet, because the index arrives with
TASK-031; it is not silently ignored. A document that produced a finding is never also listed as
checked, including when the finding comes from a check that reads the document as part of the tree.

### `aicontext health`

Counts and rates only. Every metric is accompanied by the command that reproduces it, and there is no
composite score: each number is a count a developer can reproduce by hand. The metrics are the six
`docs/CONTEXT_SPEC.md` §9 defines that this release can source — context completeness, architecture
consistency, task consistency, decision coverage, broken references, and documentation freshness. The
command takes no flags of its own beyond the global ones and writes nothing: it does not even rebuild
the index cache. It exits 0 for a report and 3 when the tree carries an error-level finding (§5). A
metric that cannot be computed is printed as `n/a` with its reason rather than as a zero that would
read as "nothing wrong" — documentation freshness, for example, is unavailable outside a Git
repository. The two remaining §9 metrics, stale memory and plugin health, are not reported: the first
has no memory check to count and the second has no CLI path that executes a plugin — the plugin
runtime library exists, but plugin execution is not wired to a command — so a number for either would
be invented.

### `aicontext context`

| Flag | Effect |
|------|--------|
| `--task <ID>` | Seed the plan from a task |
| `--query <text>` | Free-text seed |
| `--include <path>` | Force-include a document |
| `--exclude <ID|path>` | Force-exclude |
| `--budget <tokens>` | Override the token budget |
| `--explain` | Show the plan, per-document scores and reasons, conflicts, and drops |
| `--format <text\|json\|prompt>` | `prompt` renders the packet as provider input |

The two views a plain tree would have split into `show` and `explain` are one command: the default
writes the packet, and `--explain` adds the resolved plan, per-document scores and reasons, the
conflicts, and every drop. `--format json` is an alias for the global `--json` and yields to it when
both are given; under `--json` the envelope's `data` is the `ContextPacket` of `CONTEXT_SPEC.md`
§7.6 — references and metadata only. `--format prompt` reads the bodies from the index at render time
and is the only form that writes document content, wrapping every retrieved document in a data
envelope marked untrusted. A budget that cannot hold tiers 1–5 exits 3 naming `CTX-030`.

### `aicontext plan --task <ID>`

Produces a plan listing files to modify, create, and delete; dependencies affected; schema and data
impact; and security impact. Writes to `.aicontext/plans/`. **Modifies nothing.**

### `aicontext agent run`

Plan mode only for now (`ADR-006` #1): the run assembles the task's context, asks the configured
provider for a plan, and writes it to `.aicontext/plans/<task>.md`. It modifies nothing outside
`.aicontext/`, and never commits, pushes, branches, or rewrites history.

| Flag | Effect |
|------|--------|
| `--task <ID>` | The task in scope. Required: it names the whole scope of the run |
| `--dry-run` | Assemble the context and report what would be sent; no call, nothing written |

`--execute` and `--allow` are not accepted yet: an execution mode must not exist until plan mode
has proved itself (`ADR-006` #7), so passing either is a usage error rather than a flag that
quietly does nothing.

The run reports what it cost. With `pricing` configured for the provider (`AI_PROVIDER_SPEC.md` §5),
the human report and the plan file carry the provider-reported token counts and a six-decimal
`cost_usd`; without prices the cost is reported as unavailable, never as zero. The JSON payload adds
`cost_usd`, `projected_cost_usd` (a dry run), and `budget_usd` when a ceiling is configured.

`budget.max_run_usd` is checked before and after the call (`AI_PROVIDER_SPEC.md` §7): a projected
worst case above the ceiling exits 3 with `AGENT-012` and makes no call; an actual cost above it is
an `AGENT-013` error finding after the plan is written; usage too incomplete to price is the
`AGENT-014` warning; a ceiling with no prices is `AGENT-011` and stops before any call.

### `aicontext plugin list|install|remove`

`install <path>` validates a local manifest, prints the permission diff against what is already
granted and the auth requirements, confirms unless `--yes` was passed, then records the result.
`--yes` is accepted here (install is not itself a tool call) but the recorded grant is still subject
to `permissions.yaml`. Only the literal answer `yes` confirms; a run with no interactive terminal and
no `--yes` exits 5 with `PLUGIN-004`. A relative `<path>` resolves against the project root and may
name a directory containing `manifest.json` or a manifest file. Installing
`crates/aicontext-plugin-github`, for example, records its seven read capabilities; the base policy
grants none of them, so each tool still needs an explicit rule in `permissions.yaml` before the
gateway will call it.

`--as-plugin` registers a third-party MCP server (`PLUGIN_SPEC.md` §10). It is required when the
manifest declares `protocol: "mcp"` and refused when it does not; either mismatch is `PLUGIN-012`
and exits 2. The preview and the record name the protocol, so a native plugin and an MCP server are
never confused after install.

This build installs local paths only: a registry reference is refused with `PLUGIN-003`, and every
install is recorded and displayed as `unsigned` because no signature verifier exists yet.

The record is `.aicontext/plugins.lock.json` (format version 1, plugins sorted by name, permissions
sorted by kind and identity). A record written before MCP support omits `protocol` and is read as
`aiplugin`. `list` reads the record and prints every installed plugin with its version, protocol,
signature status, and permission count. `remove <name>` revokes the recorded permissions, deletes
`.aicontext/plugins/<name>/`, and deletes the lockfile when the last plugin is removed; it does not
prompt. `install` requires an existing `.ai/` tree; `list` and `remove` do not.

`--json` publishes one envelope with `command: "plugin"` and `data.action` equal to `list`,
`install`, or `remove`. The payload carries `lockfile` and `plugins` for `list`; `lockfile`, `plugin`,
and `diff` for `install`; and `lockfile`, `plugin`, `revoked`, `state_removed`, and `remaining` for
`remove`. An MCP plugin's `plugin` object includes `protocol: "mcp"`; a native plugin omits it.
Failures use `PLUGIN-001`…`PLUGIN-012`; a manifest failure keeps the SDK's `PLG-011` or `PLG-004`.
A bad reference, a protocol mismatch, or an unknown plugin name exits 2, an invalid manifest or
malformed lockfile exits 3, an unconfirmed install exits 5, and a filesystem or report failure exits
1.

### `aicontext export <archive>` / `aicontext import <archive>`

`export` writes a deterministic archive of `.ai/`; `import` verifies it and writes it back. The
format is one pretty-printed JSON document (`format` `aicontext-archive`, `version` 1): a manifest —
project name, file count, total bytes, one digest row per file sorted by path, and a SHA-256 digest
over the manifest — followed by the contents verbatim, in the same order (`ADR-008`). No timestamps,
no absolute paths, no host names, so exporting the same tree twice produces the same bytes and an
archive can be diffed and committed.

| Flag | Effect |
|------|--------|
| `--dry-run` | Run every check and every non-write guard — including the interactive-terminal requirement for `--force` — report the outcome, and write or replace nothing |
| `--force` | `export`: replace an existing archive. `import`: overwrite a conflicting file. Approval requires an interactive terminal; an absent human is a deny (exit 5). `--yes` never satisfies it |

Guarantees:

- **Verify everything before writing anything.** `import` checks the format, version, counts, paths,
  sizes, and every digest before planning a single write; an archive that fails any check exits 3 and
  leaves `.ai/` exactly as it was.
- **A relative `<archive>` resolves against the project root**, matching how `doctor` and `status`
  see the project, not against the working directory.
- **Bounded reads.** `export` caps the tree at 512 files, 8 MiB per file, and 16 path components
  (the same order of bounds `doctor` applies); a file breaking a cap is a finding and the run refuses
  whole. `import` caps the archive file at 64 MiB before it is read (`RULES.md` §11).
- **An archive is a merge, not a mirror.** `import` never deletes a file the archive does not
  mention, and leaves an existing file alone — reporting it `unchanged` — when it already holds the
  archived bytes.
- **Conflicts are reported, not resolved.** An existing file that differs from the archive is a
  conflict: without `--force` the run exits 5 and touches nothing; with `--force` it is overwritten.
- **A round trip on an unedited tree is byte-identical** (PRD F10): content is UTF-8 text stored
  verbatim, and a fresh same-named root re-exports the same bytes.

Failure codes: fatal failures use `EXP-001`…`EXP-014` for `export` and `IMP-001`…`IMP-015` for
`import`. Any usable-root, missing `.ai/`, archive-unreadable, or not-one-of-ours failure is a usage
error (exit 2); an existing output without `--force`, or `--force` without a terminal, exits 5; a
tree or an archive that fails validation exits 3; a filesystem failure exits 1. Exactly one code per
run (§5).

### `aicontext connect` / `aicontext disconnect`

`connect` describes one provider, **proves it answers one real call**, and only then commits
(`AI_PROVIDER_SPEC.md` §4): the profile goes to `.ai/ai.yaml`, and a credential — if one was given —
goes to the OS keychain. `disconnect` takes all of it back out.

| Flag | Effect |
|------|--------|
| `--provider <ID>` | Required on both. The id the provider is registered under, or removed by |
| `--base-url <URL>` | Required on `connect`. The server root, e.g. `http://localhost:11434/v1` |
| `--model <NAME>` | Required on `connect`. The model the verification call asks for |
| `--adapter <NAME>` | `openai-compatible`, `anthropic`, or `gemini`. Defaults to `openai-compatible`; an unknown name is a usage error that prints the list |
| `--env <VAR>` | Record that the credential comes from this variable. Names the variable; never its value |
| `--token <VALUE>` | The credential. Written to the OS keychain; never to `.ai/` |
| `--keychain-entry <NAME>` | The keychain entry to write. Defaults to `--provider`; requires `--token` or `--env` |

Guarantees:

- **Nothing is stored until it answers.** The whole flow is: checks, credential, one minimal
  `complete()` call, then the file. A rejected or unreachable provider exits 6 and writes nothing —
  not the profile, not the keychain entry.
- **The call is the smallest one the contract allows:** one message, `max_tokens` 8, no stream, no
  idempotency key, no retry (`AI_PROVIDER_SPEC.md` §11).
- **`.ai/ai.yaml` records sources, never values.** `auth.type` is `keychain` (a `--token`),
  `env_or_keychain` (an `--env`), or `none`, and only the *names* `env` / `keychain_entry` are
  written. `schemas/ai-config.schema.json` has no `token` property at all, so a value cannot be
  smuggled in even by hand-editing.
- **The credential's value is never echoed.** `--token` is the one flag with no parser: `clap`'s own
  error quotes the value it rejected, and an empty one is refused by the command instead, by name.
  `--token` wins over `--env`.
- **Blank values are refused at the edge.** `--provider ""` and `--base-url ""` are usage errors
  (exit 2), not a profile nothing can look up or a transport error that looks like the server's fault.
- **Unknown keys survive.** An upsert rewrites the one entry it matches; keys in the file the command
  does not understand are carried through untouched.
- **A default is only set when there is none.** `default_provider` defaults to the first provider
  connected and is never overwritten by a later one.
- **`--offline` refuses `connect`** with `CONNECT-008`. There is deliberately no `--no-verify`: the
  call *is* the command's meaning.
- **`disconnect` removes all three copies** — the entry, a `default_provider` left dangling on it,
  and the keychain entry it named — and does the file first, so a failure in between is the
  recoverable direction: the reference still exists and reconnecting restores it. An id the file does
  not list exits 2 naming the ids it does, and changes nothing.
- **Removing the last provider removes the file.** `providers` is `minItems: 1` in the schema, so an
  emptied list would be a document no reader can trust, and `init` never creates this file — a project
  with no providers has no `.ai/ai.yaml`. Only the file goes; the `.ai/` tree is `init`'s to manage.

| Outcome | Exit |
|---------|------|
| Stored / removed | 0 |
| Filesystem (`CONNECT-001`, `CONNECT-004`, `CONNECT-012`), keychain (`CONNECT-010`), unwritable JSON report (`CONNECT-011`) | 1 |
| Blank flag, unsupported adapter, `--offline`, `--keychain-entry` with no credential, an unknown `--provider`, a rejected provider id, a missing `.ai/` (`PROJ-001`) | 2 |
| Unparseable or wrong-shaped `.ai/ai.yaml` (`CONNECT-002`, `CONNECT-003`) | 3 |
| The verification call failed (`PROVIDER_*`) | 6 |

`disconnect`'s `--json` payload reports `provider`, `config_path`, `config_deleted`,
`keychain_entry`, `keychain_deleted`, and `remaining_providers`; `connect`'s reports `provider`,
`adapter`, `base_url`, `model`, `auth`, `env`, `keychain_entry`, `keychain_stored`, `verified`, and
`config_path`. `keychain_stored` and `keychain_deleted` say whether an entry was actually touched, so
a script can tell "there was nothing there" from "it worked".

### `aicontext audit show|verify|tail`

Reads the hash-chained log at `.aicontext/audit.jsonl` (`SECURITY.md` §8). No view writes, repairs,
or truncates the log; the first command that executes a plugin becomes the writer.

| View | Effect |
|------|--------|
| `show` | Render every verified record as a timeline |
| `verify` | Report whether the chain links and print the head digest for an external witness |
| `tail --limit <N>` | Render the newest N verified records; the default is 20, and zero is a usage error |

Guarantees:

- **Every view verifies the whole chain first.** A record is shown only when it and every record
  before it link from genesis. On a break, every view reports the verified prefix, the first bad
  line, and `AUD-002`/`AUD-003`, and exits 3.
- **A missing log is an empty valid chain.** "Nothing has been audited yet" is not a damaged log;
  it exits 0 and creates nothing.
- **`verify` is the witness.** The human report prints the head digest and the JSON `head` is the same
  value, so a reviewer can record it somewhere the project cannot write. A later rewrite then
  disagrees with it.
- **Reads are bounded.** A log above 64 MiB is refused rather than parsed (`AUD-004`), and an
  unreadable file is `AUD-001` (exit 1), never an empty log.
- **Display is sanitised.** Control and bidi/zero-width characters in `resource` are replaced before
  a terminal sees them; `--json` publishes the record exactly as stored.

The `--json` payload has the same shape for all three views: `view` (`show`, `verify`, or `tail`),
`log` (`.aicontext/audit.jsonl`), `chain_valid`, `verified_records`, `returned_records`, `head`,
`limit` (the requested tail limit, or null), `records` (what the view returned; empty for `verify`
and the verified prefix on a break), and `problem` (`code`, `line`, `message`, `remediation`, or
null). A break also adds one error finding with the same code.

| Outcome | Exit |
|---------|------|
| Valid chain, including an absent log | 0 |
| Chain does not verify (`AUD-002`, `AUD-003`, `AUD-004`) | 3 |
| Log cannot be read (`AUD-001`) | 1 |
| Bad view, non-positive `--limit`, or an unusable root (`PROJ-001`) | 2 |

---

## 5. Exit codes

| Code | Name | Meaning |
|------|------|---------|
| 0 | `OK` | Success |
| 1 | `GENERAL` | Unclassified failure |
| 2 | `USAGE` | Bad arguments, unknown command, or not yet implemented |
| 3 | `VALIDATION` | `doctor`/`health` found at least one error-level finding, or a command's request cannot be satisfied (`context` names `CTX-030` when tiers 1–5 do not fit the budget; `agent run` names `AGENT-011`/`AGENT-012` before a call, or `AGENT-013` after one that exceeded the configured ceiling; `audit` names `AUD-002`/`AUD-003`/`AUD-004` when the log does not verify) |
| 4 | `PERMISSION_DENIED` | The permission resolver denied the action |
| 5 | `APPROVAL_REQUIRED` | Approval was required and not granted (including every non-interactive case) |
| 6 | `PROVIDER_ERROR` | The AI provider failed, timed out, or was unreachable |
| 7 | `PLUGIN_ERROR` | A plugin failed to load, crashed, or violated a limit |
| 8 | `NETWORK_UNAVAILABLE` | A network operation was required and unavailable |
| 9 | `INTERRUPTED` | Cancelled by the user or by a signal |
| 10 | `LOCKED` | Another `aicontext` process holds the project lock |
| 70 | `INTERNAL` | A bug. Includes the bug reference when one exists |

Rules: exactly one code per run. A command that both validates and acts exits with the most severe
applicable code. Scripts branch on the code, so a new failure mode maps onto an existing code
wherever honest.

---

## 6. JSON envelope

```json
{
  "schema_version": 1,
  "command": "doctor",
  "ok": false,
  "exit_code": 3,
  "data": { "…": "command specific" },
  "findings": [
    { "code": "CTX-007", "severity": "error", "path": ".ai/TASKS.md",
      "message": "TASK-042 references SPEC-auth-missing, which does not exist",
      "remediation": "create .ai/specs/auth-missing.md or fix the reference" }
  ],
  "warnings": [],
  "summary": "2 errors, 1 warning"
}
```

`schema_version` increments on any breaking change to the shape; a snapshot test fails CI otherwise.
`findings[].code` is stable and safe to branch on. `message` is for humans and may be reworded;
`code` and `path` are not.

`data` is command-specific. A command with subcommands that read the same subject (`audit`)
publishes one shape across all of them, so a script reads `data.view` rather than assuming which
subcommand produced the report.

---

## 7. Output style

Follows `DESIGN.md` §6.

```text
$ aicontext doctor

AI Context OS  ·  my-project  ·  branch feature/context-engine

  ✗ error    TASKS.md: TASK-042 references SPEC-auth-missing, which does not exist
  ✗ error    ARCHITECTURE.md names MongoDB; project config indicates PostgreSQL
  ⚠ warn     ADR-004 is deprecated with no successor
  ✓ ok       RULES.md valid

  2 errors, 1 warning  ·  exit 3
  next: aicontext doctor --explain CTX-007
```

- Identity line first, then findings sorted by severity, then a summary.
- Colour is redundant with the glyph and the severity word.
- Symbols degrade to `+` / `x` / `!` on a non-UTF-8 terminal.
- Truncation is always visible; `--verbose` prints the full value.
- No spinner under 100 ms.

---

## 8. Scripting

```bash
# Fail CI on any context error
aicontext doctor --strict --json > doctor.json || exit $?

# Block a commit while the current task has no acceptance criteria
jq -e '.findings[] | select(.code=="CTX-009")' doctor.json && exit 1 || true

# Show the current task
aicontext status --json | jq -r '.data.current_task.id'

# Read the manifest digest of an export
aicontext export backup.aix --json | jq -r '.data.digest'

# Assemble context for a prompt
aicontext context --task TASK-037 --format prompt > ctx.md

# Read the audit trail
aicontext audit verify --json | jq '.data.chain_valid'
```

Every example above is covered by a test in `crates/aicontext-cli/tests/`, so the documented
interface cannot silently drift.

---

## 9. Environment variables

| Variable | Purpose |
|----------|---------|
| `AICON_TEXT_CONFIG` | Config file path |
| `AICON_TEXT_NO_COLOR` | Disable colour |
| `AICON_TEXT_LOG` | Log filter directive |
| `AICON_TEXT_OFFLINE` | Force offline mode |
| `NO_COLOR` | Standard colour opt-out, honoured |

Credentials are **not** configured by `AICON_TEXT_*` variables. They resolve per `ADR-005`.

`aicontext connect --env <VAR>` names a developer-chosen variable — `OPENAI_API_KEY`, say — and writes
that name into `.ai/ai.yaml`. The value is read once, at connect time, to prove the credential works;
only the name is kept, and `--env` may name any variable, not only an `AICON_TEXT_*` one.
