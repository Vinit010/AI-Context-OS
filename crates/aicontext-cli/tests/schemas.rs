//! The `schemas/` set is a contract other tools read, so it is checked rather than assumed.
//!
//! `docs/CONTEXT_SPEC.md` 1 requires a schema for every document kind and every fixed-path
//! config file, and TASK-015 requires CI to assert they are valid Draft 2020-12 schemas. Both
//! halves of that are here: the expected set is listed literally, so a new document kind without
//! a schema fails the build, and each file is validated against the 2020-12 meta-schema, so a
//! typo in a keyword fails the build instead of the reader.
//!
//! The schemas are self-contained by design — see `schema_set_is_self_contained` — which is what
//! lets this test check them one at a time with no network and no `$id` resolution.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The only schema dialect in this repository. Draft 2020-12, per `CONTEXT_SPEC` 1.
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// Every schema the specification commits to, listed rather than counted.
///
/// A count would survive a schema being renamed and fail only when someone noticed the gap. The
/// names are the contract: `CONTEXT_SPEC` 1 enumerates the document kinds and fixed-path configs,
/// and a kind appearing in that table without appearing here is a missing schema.
const EXPECTED_SCHEMAS: &[&str] = &[
    "action-proposal.schema.json",
    "agent.schema.json",
    "ai-config.schema.json",
    "ai-entrypoint.schema.json",
    "architecture.schema.json",
    "bug.schema.json",
    "change.schema.json",
    "context-note.schema.json",
    "conventions.schema.json",
    "decision.schema.json",
    "design.schema.json",
    "memory.schema.json",
    "permission-policy.schema.json",
    "prd.schema.json",
    "project-profile.schema.json",
    "rules.schema.json",
    "spec.schema.json",
    "task.schema.json",
    "tasks.schema.json",
    "workflow.schema.json",
];

/// The repository's `schemas/` directory, found relative to this crate rather than the working
/// directory, so the test does not depend on where `cargo test` was invoked from.
fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

/// Every schema in the directory, as `(file name, parsed value)`, sorted by file name so a
/// failure names the same schema on every platform.
fn load_schemas() -> Vec<(String, Value)> {
    let dir = schemas_dir();
    let mut loaded: Vec<(String, Value)> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| {
            let path = entry.expect("directory entry must be readable").path();
            let name = path
                .file_name()
                .expect("directory entry must have a file name")
                .to_string_lossy()
                .into_owned();
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let value: Value = serde_json::from_str(&source)
                .unwrap_or_else(|error| panic!("{name} is not valid JSON: {error}"));
            (name, value)
        })
        .filter(|(name, _)| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect();
    loaded.sort_by(|left, right| left.0.cmp(&right.0));
    loaded
}

/// The `$ref` values anywhere in a schema, at any depth.
fn collect_refs(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
                found.push(reference.to_owned());
            }
            for child in map.values() {
                collect_refs(child, found);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_refs(child, found);
            }
        }
        _ => {}
    }
}

#[test]
fn schema_set_is_exactly_the_expected_files() {
    let actual: BTreeSet<String> = load_schemas().into_iter().map(|(name, _)| name).collect();
    let expected: BTreeSet<String> = EXPECTED_SCHEMAS
        .iter()
        .map(|name| (*name).to_owned())
        .collect();

    let missing: Vec<&String> = expected.difference(&actual).collect();
    let unexpected: Vec<&String> = actual.difference(&expected).collect();

    assert!(
        missing.is_empty(),
        "schemas missing from the set, one per document kind or config file in CONTEXT_SPEC 1: {missing:?}"
    );
    assert!(
        unexpected.is_empty(),
        "files in schemas/ that no document kind or config file calls for: {unexpected:?}"
    );
}

#[test]
fn every_schema_declares_draft_2020_12() {
    for (name, schema) in load_schemas() {
        assert_eq!(
            schema.get("$schema").and_then(Value::as_str),
            Some(DRAFT_2020_12),
            "{name} must declare Draft 2020-12, and nothing else"
        );
    }
}

#[test]
fn every_schema_is_valid_against_the_meta_schema() {
    for (name, schema) in load_schemas() {
        // `jsonschema` performs the check; this is the assertion that a schema which fails it
        // does not ship. The error is rendered as its own `Display`, which names the failing
        // instance path, so the message says which keyword is wrong rather than just that one is.
        if let Err(error) = jsonschema::meta::validate(&schema) {
            panic!("{name} is not a valid Draft 2020-12 schema: {error}");
        }
    }
}

#[test]
fn schema_set_is_self_contained() {
    for (name, schema) in load_schemas() {
        let mut found = Vec::new();
        collect_refs(&schema, &mut found);
        for reference in found {
            assert!(
                reference.starts_with('#'),
                "{name} refers to {reference}, which is not inside itself. No file-fetching \
                 resolver is enabled in this project, so a cross-file $ref would validate in the \
                 author's editor and fail for every reader."
            );
        }
    }
}

#[test]
fn schema_ids_are_unique_and_match_their_file_names() {
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for (name, schema) in load_schemas() {
        let id = schema
            .get("$id")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name} has no $id"));

        assert!(
            id.ends_with(&format!("/{name}")),
            "{name} declares $id {id}, which does not end in its own file name. An $id that \
             disagrees with the path is how a document ends up validated against a schema that \
             was not the one next to it."
        );
        assert!(
            seen.insert(id.to_owned()),
            "$id {id} is declared twice, so one of the two schemas would win for every reader"
        );
    }
}
