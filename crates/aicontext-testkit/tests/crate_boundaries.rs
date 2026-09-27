//! Enforces the crate dependency direction defined in `ARCHITECTURE.md` §3.2.
//!
//! Architectural boundaries that are only written down are boundaries that erode. This check runs
//! inside `cargo test`, so a violation fails the same command that runs the tests, on every
//! platform, with no extra tooling to remember.
//!
//! What it asserts:
//!
//! 1. Every workspace member has a rule here, so a new crate cannot be added without a decision.
//! 2. Every rule names only crates that the rules themselves know about (catches typos).
//! 3. Every declared internal dependency is permitted by that crate's rule.
//! 4. `aicontext-core` depends on no internal crate.
//! 5. `aicontext-context` never depends on `aicontext-providers` — the context engine must not know
//!    that models exist.
//! 6. `aicontext-testkit` is never a production dependency.
//! 7. The permitted graph has no cycles, so "dependencies point inward" actually terminates.
//! 8. The workspace member list matches the crates that exist on disk, in both directions.
//! 9. Every crate root carries `#![forbid(unsafe_code)]`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use aicontext_testkit::Manifest;

/// Permitted **production** internal dependencies of each crate.
///
/// A declared production dependency must be a subset of the entry for its crate, so a crate may
/// depend on fewer things than it is allowed to. Crates appear here before they exist, so adding
/// one requires no change to this file.
///
/// `aicontext-testkit` is deliberately absent from every entry: it is test support, and is permitted
/// only in a `dev-dependencies` table. See `DEV_ONLY`.
const ALLOWED: &[(&str, &[&str])] = &[
    ("aicontext-audit", &["aicontext-core"]),
    (
        "aicontext-cli",
        &[
            "aicontext-audit",
            "aicontext-context",
            "aicontext-core",
            "aicontext-git",
            "aicontext-permissions",
            "aicontext-plugin-runtime",
            "aicontext-plugin-sdk",
            "aicontext-providers",
        ],
    ),
    ("aicontext-context", &["aicontext-core"]),
    ("aicontext-core", &[]),
    ("aicontext-git", &["aicontext-core"]),
    ("aicontext-permissions", &["aicontext-core"]),
    (
        "aicontext-plugin-runtime",
        &[
            "aicontext-audit",
            "aicontext-core",
            "aicontext-permissions",
            "aicontext-plugin-sdk",
        ],
    ),
    ("aicontext-plugin-sdk", &["aicontext-core"]),
    ("aicontext-providers", &["aicontext-core"]),
    ("aicontext-testkit", &[]),
];

/// Crates that any crate may depend on, but only for its own tests.
const DEV_ONLY: &[&str] = &["aicontext-testkit"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("testkit is at <root>/crates/aicontext-testkit")
        .to_path_buf()
}

fn read_manifest(path: &Path) -> Manifest {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    Manifest::parse(&source).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn member_names() -> Vec<String> {
    let root = workspace_root();
    read_manifest(&root.join("Cargo.toml"))
        .workspace_members()
        .iter()
        .map(|member| {
            Path::new(member)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_else(|| panic!("member path has no final segment: {member}"))
                .to_owned()
        })
        .collect()
}

fn crate_manifests() -> BTreeMap<String, Manifest> {
    member_names()
        .into_iter()
        .map(|name| {
            let manifest = read_manifest(
                &workspace_root()
                    .join("crates")
                    .join(&name)
                    .join("Cargo.toml"),
            );
            (name, manifest)
        })
        .collect()
}

fn known_crates() -> BTreeSet<String> {
    ALLOWED.iter().map(|(name, _)| (*name).to_owned()).collect()
}

fn allowed_for(crate_name: &str) -> Option<&'static [&'static str]> {
    ALLOWED
        .iter()
        .find(|(name, _)| *name == crate_name)
        .map(|(_, deps)| *deps)
}

#[test]
fn every_member_has_a_policy_entry() {
    for name in member_names() {
        assert!(
            allowed_for(&name).is_some(),
            "crate `{name}` is a workspace member but has no entry in the boundary matrix in \
             tests/crate_boundaries.rs. Add it, and decide what it may depend on, in the same \
             commit (RULES.md 5)."
        );
    }
}

#[test]
fn every_crate_manifest_names_its_directory() {
    for (directory, manifest) in crate_manifests() {
        assert_eq!(
            manifest.require_name().unwrap_or_else(|error| {
                panic!("crates/{directory}/Cargo.toml has no [package] name: {error}")
            }),
            directory,
            "crates/{directory}/Cargo.toml declares a different package name, so dependency \
             entries would not match the directory the crate lives in."
        );
    }
}

#[test]
fn policy_entries_reference_only_known_crates() {
    let known = known_crates();
    for (name, deps) in ALLOWED {
        for dep in *deps {
            assert!(
                known.contains(*dep),
                "`{name}` is allowed to depend on `{dep}`, which has no entry in the matrix. \
                 Likely a typo, or a crate that must be added."
            );
        }
    }
}

#[test]
fn declared_internal_dependencies_are_permitted() {
    let known = known_crates();
    for (name, manifest) in crate_manifests() {
        let permitted: BTreeSet<String> = allowed_for(&name)
            .expect("every member has an entry, checked above")
            .iter()
            .chain(DEV_ONLY.iter())
            .map(|dep| (*dep).to_owned())
            .collect();
        for declared in manifest.internal_dependencies(&known) {
            assert!(
                permitted.contains(&declared),
                "`{name}` depends on `{declared}`, which ARCHITECTURE.md 3.2 does not permit. \
                 Either the dependency is wrong, or the architecture must change with an ADR."
            );
        }
    }
}

#[test]
fn core_depends_on_no_internal_crate() {
    let known = known_crates();
    let manifest = read_manifest(&workspace_root().join("crates/aicontext-core/Cargo.toml"));
    assert_eq!(
        manifest
            .internal_dependencies(&known)
            .into_iter()
            .collect::<Vec<_>>(),
        Vec::<String>::new(),
        "aicontext-core is the dependency floor; it must depend on nothing internal"
    );
}

#[test]
fn context_never_depends_on_providers() {
    let known = known_crates();
    let manifest = read_manifest(&workspace_root().join("crates/aicontext-context/Cargo.toml"));
    assert!(
        !manifest
            .internal_dependencies(&known)
            .contains("aicontext-providers"),
        "aicontext-context must not depend on aicontext-providers: the context engine assembles \
         a ContextPacket and must not know that models exist (docs/AI_PROVIDER_SPEC.md 1)."
    );
}

#[test]
fn testkit_is_never_a_production_dependency() {
    let known = known_crates();
    for (name, manifest) in crate_manifests() {
        let leaked: Vec<&str> = manifest
            .dependencies()
            .iter()
            .chain(manifest.build_dependencies().iter())
            .filter(|dep| known.contains(*dep) && DEV_ONLY.contains(&dep.as_str()))
            .map(String::as_str)
            .collect();
        assert!(
            leaked.is_empty(),
            "`{name}` lists {} as a production dependency. Dev-only crates may appear only in a \
             dev-dependencies table.",
            leaked.join(", ")
        );
    }
}

#[test]
fn the_permitted_graph_is_acyclic() {
    let graph: BTreeMap<&str, BTreeSet<&str>> = ALLOWED
        .iter()
        .map(|(name, deps)| (*name, deps.iter().copied().collect()))
        .collect();
    assert!(
        !has_cycle(&graph),
        "the permitted dependency graph contains a cycle, so \"dependencies point inward\" does \
         not terminate. ARCHITECTURE.md 3.2 requires a DAG."
    );
}

#[test]
fn the_member_list_matches_the_crates_directory() {
    let crates_dir = workspace_root().join("crates");
    let on_disk: BTreeSet<String> = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", crates_dir.display()))
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .collect();

    let declared: BTreeSet<String> = member_names().into_iter().collect();
    assert_eq!(
        declared, on_disk,
        "the [workspace] members list and the crates/ directory disagree. A crate directory that \
         is not a member would escape this check entirely."
    );
}

#[test]
fn every_crate_forbids_unsafe_code() {
    for name in member_names() {
        let crate_dir = workspace_root().join("crates").join(&name);
        let mut found = false;
        for entry in ["src/lib.rs", "src/main.rs"] {
            let path = crate_dir.join(entry);
            if !path.is_file() {
                continue;
            }
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            assert!(
                source.contains("#![forbid(unsafe_code)]"),
                "{} is missing `#![forbid(unsafe_code)]`. `unsafe` is forbidden in this \
                 repository without exception (RULES.md 3).",
                path.display()
            );
            found = true;
        }
        assert!(
            found,
            "`{name}` has no src/lib.rs or src/main.rs, so it cannot be a Rust crate"
        );
    }
}

/// Kahn's algorithm over `crate -> permitted dependencies`. Any node that cannot be removed to
/// zero in-degree sits on, or is reachable from, a cycle.
fn has_cycle(graph: &BTreeMap<&str, BTreeSet<&str>>) -> bool {
    let mut in_degree: BTreeMap<&str, usize> = graph.keys().map(|node| (*node, 0usize)).collect();
    for deps in graph.values() {
        for dep in deps {
            if let Some(count) = in_degree.get_mut(dep) {
                *count += 1;
            }
        }
    }

    let mut ready: Vec<&str> = in_degree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| *node)
        .collect();

    let mut removed = 0;
    while let Some(node) = ready.pop() {
        removed += 1;
        for dep in &graph[node] {
            if let Some(count) = in_degree.get_mut(dep) {
                *count -= 1;
                if *count == 0 {
                    ready.push(dep);
                }
            }
        }
    }

    removed != graph.len()
}
