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
│   ├── show                 Render the packet
│   └── explain              Show the plan, scores, and drops
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
├── connect                  Configure a provider or integration
├── disconnect               Remove a stored reference
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

Creates the `.ai/` skeleton, creates `.ai/schemas/`, and adds `.aicontext/` to `.gitignore`. It never
creates `.aicontext/` itself: the entry exists so the directory is ignored once another command
creates it.

| Flag | Effect |
|------|--------|
| `--dry-run` | Print the plan; write nothing |
| `--force` | Overwrite existing documents that differ from the template. Requires an interactive terminal; refuses with exit 5 otherwise |
| `--no-detect` | Skip project discovery |
| `--template <name>` | Start from a template (`default`, `rust`, `node`, `python`, `blank`) |

Guarantees: idempotent; never overwrites an edited document without `--force`; never modifies
anything outside `.ai/` and `.gitignore`; validates its own output and reports the next commands.

`--force` is deliberately not a second confirmation prompt. Approval requires a human, and a human
who typed `--force` at a terminal is the approval; `Q-4` leaves the prompt *mechanism* undecided
(`TASK-072`), so `init` does not invent one. `--yes` never satisfies this check.

Schemas are not yet materialised into `.ai/schemas/`; the directory is created empty. No schema
exists to copy until `TASK-015` defines the set, so the copy step is deferred rather than faked.

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

### `aicontext health`

Counts and rates only. Every metric is accompanied by the command that reproduces it. No composite
score.

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

### `aicontext plan --task <ID>`

Produces a plan listing files to modify, create, and delete; dependencies affected; schema and data
impact; and security impact. Writes to `.aicontext/plans/`. **Modifies nothing.**

### `aicontext agent run`

| Flag | Effect |
|------|--------|
| `--task <ID>` | The task in scope |
| `--execute` | Allow writes, confined to the task's `touches` globs (per `ADR-006`) |
| `--allow <path>` | Extend the write scope explicitly |
| `--dry-run` | Show actions without calling any tool |

Never commits, pushes, branches, or rewrites history.

### `aicontext plugin install <ref>`

Validates the manifest, prints the permission diff and auth requirements, confirms, and records the
result. `--yes` is accepted here (install is not itself a tool call) but the recorded grant is still
subject to `permissions.yaml`. `--as-plugin` registers a third-party MCP server as a plugin
(`PLUGIN_SPEC.md` §10).

### `aicontext export` / `aicontext import`

`export` writes a deterministic archive of `.ai/` with a manifest and per-file digests. `import`
verifies digests, reports conflicts, and requires `--force` to overwrite. A round trip on an
unedited tree is byte-identical.

---

## 5. Exit codes

| Code | Name | Meaning |
|------|------|---------|
| 0 | `OK` | Success |
| 1 | `GENERAL` | Unclassified failure |
| 2 | `USAGE` | Bad arguments, unknown command, or not yet implemented |
| 3 | `VALIDATION` | `doctor`/`health` found at least one error-level finding |
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
      "message": "TASK-014 references SPEC-auth-missing, which does not exist",
      "remediation": "create .ai/specs/auth-missing.md or fix the reference" }
  ],
  "warnings": [],
  "summary": "2 errors, 1 warning"
}
```

`schema_version` increments on any breaking change to the shape; a snapshot test fails CI otherwise.
`findings[].code` is stable and safe to branch on. `message` is for humans and may be reworded;
`code` and `path` are not.

---

## 7. Output style

Follows `DESIGN.md` §6.

```text
$ aicontext doctor

AI Context OS  ·  my-project  ·  branch feature/context-engine

  ✗ error    TASKS.md: TASK-014 references SPEC-auth-missing, which does not exist
  ✗ error    ARCHITECTURE.md names MongoDB; project config indicates PostgreSQL
  ⚠ warn     ADR-004 is deprecated with no successor
  ✓ ok       RULES.md valid
  ✓ ok       TASKS.md valid  (24 tasks, 3 in progress)

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
