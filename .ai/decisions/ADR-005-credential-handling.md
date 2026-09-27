---
id: ADR-005
type: decision
title: Secrets never enter the repository; credentials resolve from environment or OS keychain
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-005 — Credential handling

## Context

The product connects to GitHub, AWS, databases, and chat platforms. Each needs a credential. The
product's central promise is that project knowledge is portable, Git-native files — which is
exactly the property that makes a committed credential a disaster, because Git preserves it
forever, in every clone, in every fork, in every CI cache.

The brief is unambiguous ("never store passwords, API keys, cloud secret keys, OAuth secrets,
database credentials, or private tokens inside `.ai`") but leaves the mechanism open: environment
variables, a file outside the repo, the OS keychain, or an external secret manager.

## Decision

1. **No secret is ever written to `.ai/`, source code, fixtures, snapshots, logs, the audit log, or
   an error message.** This is a test-enforced invariant, not a convention.
2. **Credentials resolve in this order**, first hit wins:
   1. An explicit flag or environment override for a single invocation
   2. A process environment variable
   3. The **OS keychain** via a reference stored in the local, git-ignored config directory
3. **`.ai/integrations/` may record *where* a credential comes from** — the variable name, the
   keychain service and account, or the command to run — **never the value.**
4. **Redaction happens at the logging boundary**, driven by key names and value patterns, not by
   remembering to be careful at each call site.
5. **The audit log records argument digests, not raw argument values**, for any tool that can carry
   a secret.
6. **Least privilege is the documented default.** A read-only cloud credential is what
   `aicontext connect` instructs the user to create; write scopes are opt-in and separately
   permissioned.
7. **`doctor` scans `.ai/` for credential-shaped keys and values** and reports the path, line, and
   rule — never the matched secret.

## Reason

- **Git is forever.** A secret in `.ai/` is copied to every clone and cannot be un-committed from
  history. The only safe time to prevent it is before it is written.
- **Portability must not mean transporting secrets.** A context directory that travels between
  machines and contributors should carry *references*, not authority.
- **The keychain is the only option that is both secure and frictionless.** The user never types a
  token into a file, and the credential is protected by OS-level access control rather than by
  file permissions we would have to get right ourselves.
- **Redaction at the boundary is testable.** Ad-hoc redaction at each log call is a bug factory;
  one chokepoint with named rules can be exhaustively tested.

## Alternatives considered

- **A `.env` file inside the project, git-ignored.** Rejected: it is a plaintext secret on disk,
  it is trivially copied by a well-meaning "let me zip the repo", and it teaches the developer a
  pattern that eventually leaks.
- **Credentials in `.aicontext/` (git-ignored, file-permission protected).** Rejected as the
  default: acceptable as a fallback, weaker than the keychain, and on some systems the project
  directory is on a shared or backed-up volume. Permitted as an explicitly-configured opt-in for
  headless CI, where a keychain is often unavailable.
- **A cloud secret manager (AWS Secrets Manager, Vault).** Rejected as a *requirement* (it would
  break the offline, no-account, local-first promise) but supported as a **credential source** for
  teams that already run one.
- **OAuth device flow for every integration.** Rejected as the universal mechanism: it is excellent
  for SaaS providers and impossible for an arbitrary database. Planned per-plugin, not as a core
  feature.

## Rejected alternatives and why

- **A built-in credential vault encrypted with a project-derived key.** Rejected: the key would
  have to live near the data, and a "encrypted at rest" claim built on a key derived from the
  repository is security theatre with extra steps. Use the OS keychain, which already has the
  right primitives and the right threat model.
- **Prompting for the credential on every tool call.** Rejected: unusable, and it trains the
  developer to type secrets into anything that asks.

## Consequences

**Positive** — the repository is safe to push, fork, and archive; no secret-handling code to get
wrong beyond one redaction chokepoint; works offline; least privilege is the documented default.

**Negative** — a headless CI runner without a keychain needs the explicit environment-variable
path; keychain access prompts the user on some platforms (macOS in particular); rotating a
credential is the user's job, since the tool stores only a reference; some deployments have no
keychain at all and must be told so explicitly.

**Follow-up** — TASK-052 (credential resolver, with tests asserting no credential appears in any
error or debug output), TASK-019 (secret scan in `doctor`).

## Related tasks

TASK-019, TASK-052, TASK-053, TASK-075, TASK-090
