# PLUGIN_SPEC — Plugins, tools, and the tool gateway

Normative. Implements `ADR-002` and `ADR-006`. Target plugin API version: **1**.

---

## 1. Model

```text
Agent ──ActionProposal──▶ Tool Gateway ──▶ PermissionResolver ──▶ ApprovalBroker
                                                                    │
                                                                    ▼
                                                             PluginRuntime
                                                                    │  JSON-RPC 2.0 / stdio
                                                                    ▼
                                                               Plugin process
```

A **plugin** is an executable that exposes **tools**. A **tool** is a named, schema-described
operation on a named provider's resources. A **capability** is the unit of permission and is named
identically to the tool.

There is exactly one path from an action to a plugin. There is no second, faster, or internal path.

---

## 2. Manifest

```json
{
  "manifest_version": 1,
  "plugin_api_version": "1",
  "name": "aws",
  "version": "1.0.0",
  "description": "Read-only AWS resource inspection",
  "vendor": "aicontext",
  "license": "Apache-2.0",
  "entry": { "command": "aicontext-plugin-aws", "args": ["serve"] },
  "minimum_aicontext_version": "0.5.0",
  "capabilities": [
    { "tool": "aws.ec2.describe", "effect": "read" },
    { "tool": "aws.s3.list", "effect": "read" },
    { "tool": "aws.ec2.start", "effect": "write", "destructive": false },
    { "tool": "aws.ec2.terminate", "effect": "write", "destructive": true }
  ],
  "auth": {
    "type": "env_or_credential_command",
    "references": [{ "name": "AWS_PROFILE", "source": "env" }],
    "instructions": "Use a read-only IAM policy. Never grant a wildcard resource."
  },
  "filesystem": { "read": [], "write": [] },
  "network": { "egress": ["api.ec2.us-east-1.amazonaws.com"] },
  "limits": { "call_timeout_ms": 15000, "max_output_bytes": 1048576 }
}
```

**Rules**

1. Validated against `plugin-manifest.schema.json` (JSON Schema Draft 2020-12) before anything is
   executed. An invalid manifest is never run.
2. `name` matches `^[a-z0-9][a-z0-9-]{1,63}$` and is globally unique.
3. Every tool name is `provider.service.action`, lowercase, dot-separated, 3 segments.
4. A plugin **may not** declare a capability it does not implement, and a request for an
   undeclared capability is refused by the gateway before the plugin is contacted.
5. `destructive: true` forces at least `EXPLICIT_APPROVAL` regardless of the policy file. A
   manifest cannot lower the floor that its own `destructive` flag sets.
6. `filesystem` and `network` allowlists are the **outer** jail. The union of the manifest and the
   user's policy must both permit an action; either may narrow it.
7. `minimum_aicontext_version` is a SemVer range. An unsatisfiable range is refused at install.
8. `plugin_api_version` is negotiated at handshake. A mismatch fails with `PLG-004` and states both
   versions.

---

## 3. Tool contract

```json
{
  "tool": "aws.ec2.stop",
  "summary": "Stop a running EC2 instance",
  "effect": "write",
  "destructive": false,
  "input_schema": {
    "type": "object",
    "additionalProperties": false,
    "required": ["instance_id"],
    "properties": {
      "instance_id": { "type": "string", "pattern": "^i-[0-9a-f]{8,17}$" },
      "reason": { "type": "string", "maxLength": 500 }
    }
  },
  "annotations": { "readOnlyHint": false, "idempotentHint": false }
}
```

- `input_schema` is Draft 2020-12 and is **enforced by the gateway before the call**, not only by
  the plugin. Unknown properties are rejected (`additionalProperties: false` is required).
- The gateway enforces a closed subset: `type` (one name, or a union carrying only `enum`/`const`),
  `enum`, `const`, `properties`, `required`, `additionalProperties: false`, `items`, `minItems`,
  `maxItems`, `minLength`, `maxLength`, `pattern`, `minimum`, and `maximum`. The annotations
  `$schema`, `$id`, `$comment`, `title`, `description`, `default`, `examples`, `deprecated`,
  `readOnly`, and `writeOnly` are ignored. Any other keyword (`oneOf`, `$ref`, `format`, and so on)
  is refused with `PLG-012` rather than silently ignored, and every object schema at every level
  must state `additionalProperties: false`. A schema and its arguments are each capped at 64 KiB and
  32 levels; `pattern` is compiled once with the linear-time `regex` engine under a 1 MiB size cap.
- `summary` is shown verbatim in the approval prompt. It is a safety surface, not decoration.
- MCP-compatible annotations are carried through unchanged.

---

## 4. Protocol

JSON-RPC 2.0 over stdin/stdout. Newline-delimited JSON. stdout carries protocol frames only;
anything a plugin wants to say to a human goes through the `log` notification, and the runtime
relays it to stderr.

| Direction | Method | Purpose |
|-----------|--------|---------|
| host → plugin | `initialize` | Handshake: `plugin_api_version`, host capabilities |
| host → plugin | `tools/list` | Tool descriptors, including input schemas |
| host → plugin | `tools/call` | `{ name, arguments }` |
| host → plugin | `shutdown` | Graceful stop |
| plugin → host | `log` | Diagnostics to the user's stderr |
| plugin → host | `host/request_approval` | Optional; the host is still the decision authority |

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
  "name":"aws.ec2.stop",
  "arguments":{"instance_id":"i-0abc1234","reason":"idle overnight"}}}
```

```json
{"jsonrpc":"2.0","id":1,"result":{
  "content":[{"type":"text","text":"{\"instance_id\":\"i-0abc1234\",\"previous\":\"running\"}"}],
  "is_error":false,
  "meta":{"request_id":"req-8f2c","duration_ms":412}}}
```

**Result handling.** Every result is untrusted data. The runtime:

- parses it into a bounded structure (max depth 32, max `max_output_bytes`),
- strips ANSI escapes and control characters, replaces bidi and zero-width format controls, and
  refuses the result if two keys become identical after sanitising,
- records `request_id` and `duration_ms` itself and drops any `meta` the plugin supplied,
- wraps the content and unknown fields in `{ trust: "untrusted", tool, plugin, content, extra,
  is_error, meta }` (`CONTEXT_SPEC.md` §7.6),
- carries a plugin's `is_error: true` as data, not as a runtime error to retry,
- measures the serialised envelope against `max_output_bytes` and returns `PLG-006` if the envelope
  is larger than the raw frame the supervisor already admitted,
- never passes it to a shell, an evaluator, or a template engine with code execution.

**Errors.** A plugin error is a typed runtime error with a code, never a raw string that the CLI
reformats optimistically. Codes: `PLG-001` spawn failure, `PLG-002` handshake failure, `PLG-003`
timeout, `PLG-004` API version mismatch, `PLG-005` invalid frame, `PLG-006` output limit exceeded,
`PLG-007` process exited unexpectedly, `PLG-008` capability not declared, `PLG-009` sandbox
violation, `PLG-010` approval required but unavailable, `PLG-011` invalid manifest, `PLG-012`
invalid tool arguments or an unenforceable input schema.

---

## 5. Permission model

Permission modes: `ALLOW`, `DENY`, `APPROVAL`, `EXPLICIT_APPROVAL`.

Resolution, in order — **the first match wins**:

```text
1  an explicit DENY for the exact tool                        → DENY
2  a DENY for the tool's provider.service wildcard             → DENY
3  EXPLICIT_APPROVAL for the exact tool                        → EXPLICIT_APPROVAL
4  the tool is declared destructive:true                      → EXPLICIT_APPROVAL
5  APPROVAL for the exact tool                                → APPROVAL
6  APPROVAL for a provider.service wildcard                   → APPROVAL
7  ALLOW for the exact tool                                  → ALLOW
8  otherwise                                                  → DENY   (default deny)
```

File order never affects the answer. Within a mode an exact rule is checked before its
`provider.service` wildcard, and between modes the order above is the only precedence: the mode
order is `DENY` > `EXPLICIT_APPROVAL` > `APPROVAL` > `ALLOW`. In particular a wildcard `DENY` beats
an exact `EXPLICIT_APPROVAL` (step 2 before step 3), and a wildcard `APPROVAL` beats an exact `ALLOW`
(step 6 before step 7).

Policy file `.ai/permissions/permissions.yaml`:

```yaml
version: 1

defaults:
  mode: deny

tools:
  "read_file":           { mode: allow }
  "aws.ec2.describe":    { mode: allow }
  "aws.ec2.start":       { mode: approval }
  "aws.ec2.stop":        { mode: approval }
  "aws.ec2.terminate":   { mode: explicit_approval }
  "github.repo.read":    { mode: allow }
  "github.issue.*":      { mode: approval }
  "aws.s3.*":            { mode: deny }
```

Every `tools` key is a capability name and every entry must carry its own `mode`; a missing `mode`
is a validation failure, never an assumed `deny`. `scope` and `reason`, when present, are approval
metadata rather than resolution inputs: they do not change the mode the resolver returns.

Wildcards are permitted at the `provider.service` level only, never at the top level, and never as
`*.action`. A wildcard covers exactly one action segment: `aws.ec2.*` covers `aws.ec2.describe` but
not `aws.ec2` or `aws.ec2.describe.extra`. A wildcard may carry only `deny` or `approval`; the
table above has no wildcard step for `allow` or `explicit_approval`, so a policy that gives one a
wildcard mode is rejected (`CTX-003`) rather than left silently inert.

`defaults.mode` is the policy's explicit mapping for every capability its `tools` table does not
name. The shipped template and `Policy::default()` both set it to `deny`; an absent or unreadable
policy is deny-all. A policy that fails schema or rule validation is an error (`CTX-003`), not a
default mode.

The resolver is a pure function: no I/O, no clock, no randomness, no environment. `TASK-071` in
`TASKS.md` requires 100 % branch coverage over the matrix above; the resolver's matrix test
enumerates every default mode, every rule mode, both destructive states, and an unspecified
capability, and its property tests exercise arbitrary rule sets in either file order.

---

## 6. Approval

`EXPLICIT_APPROVAL` is evaluated per invocation. It is never granted for a session, never stored as
a grant, and never satisfiable by a non-interactive flag.

`APPROVAL` may be satisfied per invocation: by an exact `--allow-once <tool>` grant for that run, or
by answering the prompt in an interactive terminal. There is no blanket `--yes`. A session grant is
not implemented yet; `TASK-072` implements the per-invocation decision only.

The prompt is line-based plain text on stderr, not a full-screen TUI. It shows, in this order and
verbatim: the tool name, the plugin, the target resource, the full argument set as compact JSON, the
reason supplied by the caller, and the permission mode. Caller-controlled text is JSON-quoted and
escaped, and the argument set is compact JSON, so no field can introduce a prompt line of its own.
Only the literal answer `yes`, after trimming surrounding whitespace, approves; Enter (an empty
line), `y`, `YES`, and every other answer deny. The terminal prompter caps an answer at 64 bytes and
installs no signal handler and no wall-clock timer; an interrupted read and a prompter-reported
timeout are denials. The default answer on Enter, timeout, EOF, non-TTY, or an over-long answer is
**deny**.

Every prompt is recorded before it is shown and every decision after it is reached. If either audit
record fails, the call is refused with `PLG-010` (`SECURITY.md` §9).

Approval is the host's decision and happens before the call. A plugin that sends
`host/request_approval` during a call is answered `approved: false`: the method cannot turn a denial
into consent, and the host does not re-enter the prompt from inside plugin code.

### Risk R-8 — approval fatigue

Default most tools to read-only `ALLOW`; prompt only where the consequence is real; group related
calls into one prompt; and measure prompt frequency per session. A prompt that appears on every
call trains the developer to say yes, which is worse than no prompt at all.

---

## 7. Sandbox

| Control | Behaviour |
|---------|-----------|
| Working directory | Set to a per-plugin empty `state/plugins/<name>/work`; the repository is not the cwd |
| Filesystem read | Host-mediated reads only at or below `manifest.filesystem.read`; relative paths resolve against the project root, and `..`, symlink, dangling-symlink, and non-portable-name escapes are refused |
| Filesystem write | Host-mediated writes only at or below `manifest.filesystem.write`; never `.ai/` unless explicitly listed |
| Network egress | Host-mediated requests only to hosts in `manifest.network.egress`; empty means no network |
| Environment | `env_clear()`, then a minimal base set plus `source: env` auth references. `HOME`, `USERPROFILE`, and the temporary variables point into the sandbox; a controlled name cannot be overridden |
| Timeouts | Per call, from the manifest; hard kill on expiry |
| Output | `max_output_bytes` cap; the process is killed if exceeded |
| Process | One plugin process per session, reaped on exit; a crash is an error, not a silent no-op |
| Refusal | Typed `PLG-009` or `PLG-011`; the gateway audits `SandboxRefused` before returning |

The filesystem and network rows describe the **host-mediated** boundary: what the runtime checks
when it performs an operation for the plugin. A plugin that opens a path or a socket directly is
constrained by the operating system, not by these checks, and isolation strength is therefore
**OS-dependent** (risk R-3). Where a control cannot be established structurally, the runtime
**fails closed** and states which control is unavailable on this platform. It never claims a
guarantee it does not provide, and it does not claim kernel-level parity across platforms.

---

## 8. Install, packaging, trust

```bash
aicontext plugin install <path-or-registry-ref>
aicontext plugin list
aicontext plugin remove <name>
```

`install` must, in order: validate the manifest; show the permission diff (requested vs already
granted); show the auth requirements; require confirmation unless `--yes` was passed; record the
result in the local lockfile.

A registry reference additionally shows the publisher, version, signature status, and the exact
permission set. **Signature verification is optional in the MVP but the status is always shown** —
an unsigned plugin is displayed as unsigned, never as trusted.

---

## 9. Authoring a plugin

```text
my-plugin/
├── manifest.json
├── README.md
├── src/                  # any language
└── tests/
```

Checklist for an author:

- [ ] `manifest.json` validates and declares every tool the plugin implements
- [ ] Permissions are declared at the narrowest scope that works
- [ ] `destructive: true` on anything that deletes, terminates, or overwrites
- [ ] `input_schema` has `additionalProperties: false` and real patterns
- [ ] `summary` is accurate — a human approves based on it
- [ ] No secret is read from a file the manifest did not declare
- [ ] No shell is spawned with caller-supplied text
- [ ] stdout carries protocol frames only
- [ ] Every external response is treated as untrusted input
- [ ] The plugin is tested against a misbehaving host: timeout, cancellation, no approval
- [ ] Documented limitations are stated in the README

The Rust SDK (`aicontext-plugin-sdk`) provides manifest and protocol types plus a server helper. It
is optional; the wire protocol is the contract.

---

## 10. MCP compatibility

The runtime speaks MCP-compatible tool definitions and transports, so an existing MCP server can be
registered by adding a manifest that declares its tools and permissions:

```bash
aicontext plugin install ./my-mcp-server --as-plugin
```

Compatibility is a stated goal, not a claim of full equivalence, and the following are **not**
provided in the MVP: MCP server-initiated sampling, MCP-provided resources, and elicitation
surfaces. Unknown U-2 in `ARCHITECTURE.md` §13 — whether the permission model maps cleanly onto
MCP — must be resolved in TASK-078 before Phase 5 depends on it.
