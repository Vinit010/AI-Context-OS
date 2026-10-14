---
id: ADR-009
type: decision
title: HTTP and TLS for the provider layer use reqwest over rustls/ring, accepting a build-time C compiler
status: accepted
date: 2026-10-14
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-009 — HTTP and TLS for the provider layer use `reqwest` over `rustls`/`ring`, accepting a build-time C compiler

Implements the HTTP transport `docs/AI_PROVIDER_SPEC.md` §6 assumes. Closes `TASK-051`.

## Context

`TASK-051` is the first crate in the workspace to make a network call. It needs an HTTP client that
can reach `https://api.openai.com/v1` and an `http://localhost:11434/v1`, with a connection pool, a
connect timeout, per-request timeouts, and a streaming response body. `AI_PROVIDER_SPEC.md` §11
requires the contract tests to run against a local fake server with no network, so the client must
speak plain HTTP to a localhost port as well.

`ARCHITECTURE.md` §2.2 pre-justifies `reqwest` + `rustls` for exactly this ("Provider calls and plugin
transports. `rustls` avoids a system OpenSSL dependency"), and RISK `R-2` names the tree that must
stay out of the workspace. So the direct dependency itself is not in question. What had not been
checked until this task is what `reqwest`'s TLS feature actually resolves to.

`RULES.md` §6 says: "No dependency may be added that requires a C compiler, a system library, or a
network install at runtime. `rustls` over `openssl` for this reason." The rule names `rustls` as the
compliant choice. The problem is that `rustls` is not itself the layer that talks to the CPU: it
delegates hashing, AEAD, and key agreement to a *crypto provider*, and every RustCrypto-adjacent
provider of that role compiles C or assembly.

`cargo tree -p aicontext-providers` after the first build resolves the TLS stack as
`reqwest 0.12` → `rustls 0.23.45` → `ring 0.17.14`, with `cc` 1.8.0 and `find-msvc-tools` in the build
graph. `ring` is not pure Rust: it compiles a small amount of C and a large amount of pre-generated
assembly through the `cc` crate. So the build needs a C toolchain, and `RULES.md` §6 as written is
violated by the very crate the rule names.

## Decision

The provider layer uses `reqwest 0.12` with `default-features = false` and exactly the features
`rustls-tls` and `stream`. `rustls-tls` resolves to `rustls` with the `ring` crypto provider, so a C
compiler (and, on Windows, the MSVC build tools already required by the `x86_64-pc-windows-msvc`
toolchain) is required **at build time**.

This is a deliberate, recorded deviation from the literal wording of `RULES.md` §6, and it is bounded
to the build:

- **No system library at runtime.** `ring`'s C and assembly are compiled and linked into the binary.
  There is no `libssl`, no `libcrypto`, and no DLL to install or ship.
- **No network install.** Nothing is fetched at build or run time beyond the crates.io registry the
  workspace already uses.
- **No new runtime prerequisite.** The single static binary the product promises is unchanged.

The dependencies are pinned by `Cargo.lock` and built with `--locked` in CI.

## Reason

1. **The intent of RULES 6 is preserved; only its proxy is violated.** The rule's stated reason is
   "`rustls` over `openssl`… for this reason" - avoiding a runtime system library and a
   per-platform install step. `ring` honors that intent: it is compiled in, not linked against. The
   clause about a C compiler is a proxy for portability, and the real portability cost here is one
   tool the Rust toolchain already assumes on every supported platform.
2. **Every alternative is worse.** `native-tls` reintroduces the system OpenSSL the rule exists to
   avoid, which is a *runtime* dependency and therefore a strictly worse violation. `rustls` with its
   default `aws-lc-rs` provider additionally requires `cmake` and has a larger C surface than `ring`.
   A hand-rolled HTTP/1.1 client over `std::net` would have to reimplement TLS for `https://`, which
   defeats the point; `reqwest` with no TLS feature would forbid the OpenAI endpoint that is the
   adapter's primary target, so accepting the C compiler is the smallest possible cost.
3. **`ring` is the conservative choice.** It is the provider `rustls` shipped for years, it is widely
   audited, and it needs only the C compiler the platform already has, not `cmake` and not a
   toolchain fetch.
4. **The deviation is visible, not silent.** Recording it here means the next task that adds a
   dependency, and any reviewer reading `RULES.md` §6, can see why one crate appears to break a rule
   that the others keep. The alternative - an unstated exception - is the failure mode the rule's own
   justification requirement exists to prevent.

## Consequences

- **The workspace now requires a C toolchain to build.** On Windows this is the MSVC build tools that
  the pinned `x86_64-pc-windows-msvc` toolchain needs anyway; on Linux and macOS it is `cc`, which the
  Rust toolchain's default linker also assumes. A pure-Rust build is no longer possible while an HTTP
  client is in the tree.
- **`RULES.md` §6's wording is narrower than its intent.** This ADR is the reference for the next
  reader who notices the contradiction; if the rule is ever reworded, it should distinguish a
  *runtime* system dependency (still forbidden) from a *build-time* C compiler (now accepted once,
  and only where no pure-Rust alternative exists).
- **The transitive tree grows by the `reqwest`/`hyper`/`rustls`/`ring` stack.** It is justified in
  `ARCHITECTURE.md` §2.2 as a whole, and `Cargo.lock` records every addition. The `cc` and
  `find-msvc-tools` crates are build-dependencies of `ring` and do not ship.
- **CI's MSRV job builds the same C.** MSRV 1.85 must be able to compile `ring` 0.17; if a future
  `ring` raises its own MSRV above 1.85, this decision is the one to revisit, because a dependency
  that cannot build on the compatibility floor is a different problem from one that needs a C
  compiler.
- **If a pure-Rust TLS provider becomes viable**, the swap is contained: TLS is reached only through
  `reqwest`, which the adapter wraps in one module (`src/openai/mod.rs`). Changing the TLS backend is
  a feature flag on one dependency, not a code change.
