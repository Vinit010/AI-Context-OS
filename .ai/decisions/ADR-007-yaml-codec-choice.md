---
id: ADR-007
type: decision
title: YAML front matter is parsed through yaml_serde behind a YamlCodec trait
status: accepted
date: 2026-09-27
deciders: founder
supersedes: null
superseded_by: null
---

# ADR-007 - YAML front matter is parsed through `yaml_serde` behind a `YamlCodec` trait

Resolves open question `Q-1` in `MEMORY.md`. Closes the owner task for risk `R-1` in
`ARCHITECTURE.md` §13.

## Context

`docs/CONTEXT_SPEC.md` §2 makes a leading YAML block the machine-readable record for every document
in `.ai/`. That block is a storage contract: it is what `doctor` validates, what `status` reports,
and what an external tool reads without running our binary. It therefore has to be real YAML, not
a hand-rolled subset that happens to suit our own files.

The mainstream crate for this, `serde_yaml`, is archived. Its successor ecosystem is fragmented,
and the choice was recorded as `R-1`: "a bad choice forces a late swap" with the impact
"parser rewrite across the storage contract". The concrete fear is picking a single-maintainer
crate that is itself archived in two years, which would mean a forced migration of every document
format we ship.

Candidates considered, with the state of each verified on 2026-09-27 against crates.io and the
upstream repositories:

| Candidate | State | Verdict |
|-----------|-------|---------|
| `serde_yaml` 0.9 | Archived by dtolnay. The reference implementation, no longer maintained. | Rejected: unmaintained, and it is the crate everything else forked from. |
| `serde_yml` 0.0.13 | Archived and explicitly deprecated. Carries `RUSTSEC-2025-0068` (unsound `libyaml` C-FFI) for every version, and `cargo audit` keeps flagging it even where the surface is structurally gone. | Rejected: deprecated, and a permanent advisory in a project that treats `cargo audit` as a merge gate. |
| `serde_norway` 0.9.42 | Hard fork of `serde_yaml`, widely used. Last release 2024-12-21. | Rejected: plausible, but ~21 months without a release at the time of writing, and a fork of an archived crate inherits its ceiling. |
| `noyalib` 0.0 | Modern pure-Rust backend with a `serde_yaml`-shaped compatibility layer. | Rejected: at version 0.0, single-maintainer, and an unusual position in the ecosystem for a storage contract. |
| `serde-saphyr` 0.0 | Maintained, but deliberately exposes no `Value` DOM. | Rejected: we must hold front-matter values dynamically, because which keys are valid depends on the document's schema (`docs/CONTEXT_SPEC.md` §3), which arrives in `TASK-015`. |
| `yaml-rust2` 0.9 | Mature low-level parser, not serde-integrated. | Rejected: no `Value` DOM and no serde support either, so we would own the serde bridge as well as the format. |
| **`yaml_serde` 0.10** | **Actively maintained fork of `serde_yaml`, published by the official YAML organisation (`github.com/yaml`), 977 commits, MIT OR Apache-2.0, MSRV 1.82, pure Rust, depends only on `serde` and `indexmap`.** | **Chosen.** |

## Decision

Use **`yaml_serde` 0.10** for YAML, reached only through a private `YamlCodec` trait.

No YAML type from `yaml_serde` appears in a public signature. The public surface is our own
`Value` enum and the parsed document types, so a future swap is a change to one module plus the
implementations of one trait, not a rewrite of the storage contract. This is the mitigation
`ARCHITECTURE.md` §13 already committed to for `R-1`, implemented rather than noted.

`serde` is **not** a direct dependency. The codec parses into `yaml_serde`'s own value tree and
walks it into our `Value` rather than deriving `Deserialize`, because a derived mapping resolves a
repeated key by a rule of its own - last one wins, or a generic syntax error - and `rule 5` of the
format says nothing written by hand may be silently lost. Keeping both copies long enough to name
the offending key costs one recursive function.

## Reason

1. **A named steward beats a named individual.** The whole failure mode of `R-1` is a crate losing
   its maintainer. `yaml_serde` is published by the YAML organisation, which exists to maintain
   the format's reference tooling, and it continues `serde_yaml`'s line rather than diverging from
   it. That is a materially different bet from any individual fork.
2. **Full compatibility, so the swap cost is bounded.** It is API-compatible with `serde_yaml`,
   including through Cargo's package renaming. If it is ever abandoned, the migration is a
   one-line `Cargo.toml` change and an import rename, not a redesign.
3. **It satisfies our security rules.** Pure Rust, so no C toolchain at build time and no FFI
   surface; no `unsafe` we have to audit; no network, no home-directory access, no telemetry. It
   clears the bar that eliminated `serde_yml`.
4. **A dynamic `Value` is required, not optional.** Because key validity depends on the document
   schema, the parser must hold values it cannot type statically today. Both candidates that drop
   the DOM are disqualified by the contract, not by taste.
5. **Licence compatible.** MIT OR Apache-2.0, matching this project's own licence.

## Consequences

- The `Cargo.lock` must be committed and CI builds with `--locked`, so a transitive change to the
  YAML backend is a visible diff rather than a silent one.
- `RUSTSEC-2025-0068` does not apply: it is specific to the `serde_yml` C-FFI chain, which is not
  in our graph. `cargo audit` in CI remains the check that keeps this true.
- `YamlCodec` earns its keep only if it stays thin. If it grows into a general YAML abstraction
  layer, it has become the risk rather than the mitigation, and the right move is to delete it and
  call `yaml_serde` directly. This is stated in its documentation so the next agent does not
  inherit a layer that grew for its own sake.
- `R-1` is downgraded from *open* to *mitigated*. The residual risk is that `yaml_serde` changes
  its API incompatibly in a 1.0 release. Cargo's package renaming is the planned escape, and
  `YamlCodec` is the containment boundary.
