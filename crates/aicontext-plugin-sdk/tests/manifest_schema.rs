//! The manifest contract is a published schema, not only a Rust type.
//!
//! These tests are the reason `schemas/plugin-manifest.schema.json` can be trusted by a plugin
//! author writing in another language: the canonical manifest from `docs/PLUGIN_SPEC.md` §2 is
//! validated against the schema with the same Draft 2020-12 engine CI uses, and the schema itself
//! is checked against the meta-schema.

use std::fs;
use std::path::{Path, PathBuf};

use aicontext_plugin_sdk::PluginManifest;
use jsonschema::Validator;
use serde_json::{Value, json};

const CANONICAL: &str = include_str!("fixtures/aws.json");

fn schema_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/plugin-manifest.schema.json")
}

fn schema() -> Value {
    let path = schema_path();
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

fn validator() -> Validator {
    jsonschema::validator_for(&schema()).expect("the manifest schema compiles")
}

fn canonical() -> Value {
    serde_json::from_str(CANONICAL).expect("the fixture is valid JSON")
}

#[test]
fn the_schema_is_a_valid_draft_2020_12_schema() {
    let schema = schema();
    if let Err(error) = jsonschema::meta::validate(&schema) {
        panic!("schemas/plugin-manifest.schema.json is not valid Draft 2020-12: {error}");
    }
}

#[test]
fn the_schema_id_matches_its_file_name() {
    let schema = schema();
    let id = schema["$id"].as_str().expect("the schema has an $id");
    assert!(
        id.ends_with("/plugin-manifest.schema.json"),
        "$id {id} must end in the file name, or a reader validates against the wrong document"
    );
}

#[test]
fn the_canonical_manifest_validates_and_parses() {
    let instance = canonical();
    let validator = validator();
    if let Err(error) = validator.validate(&instance) {
        panic!("the canonical manifest does not validate: {error}");
    }

    let manifest = PluginManifest::from_json(CANONICAL).expect("the SDK accepts the same manifest");
    assert_eq!(manifest.name, "aws");
    assert_eq!(manifest.capabilities.len(), 4);
}

#[test]
fn a_minimal_manifest_validates() {
    let instance = json!({
        "manifest_version": 1,
        "plugin_api_version": "1",
        "name": "demo",
        "version": "0.1.0",
        "entry": {"command": "demo-plugin"},
        "capabilities": [{"tool": "demo.items.read", "effect": "read"}],
        "filesystem": {"read": [], "write": []},
        "network": {"egress": []},
        "limits": {"call_timeout_ms": 1000, "max_output_bytes": 1024}
    });
    let validator = validator();
    if let Err(error) = validator.validate(&instance) {
        panic!("the minimal manifest does not validate: {error}");
    }
    assert!(
        PluginManifest::from_json(&instance.to_string()).is_ok(),
        "the SDK must accept what the schema accepts"
    );
}

#[test]
fn a_newer_plugin_api_version_is_schema_valid_so_negotiation_can_explain_it() {
    // If the schema pinned "1", a plugin built for version 2 would fail with a schema error that
    // never mentions the host's version. PLUGIN_SPEC 2 rule 8 requires PLG-004 to state both.
    let mut instance = canonical();
    instance["plugin_api_version"] = json!("2");
    let validator = validator();
    assert!(validator.is_valid(&instance));
}

#[test]
fn a_malformed_plugin_api_version_is_not_schema_valid() {
    for version in ["0", "01", "", "1.0", "one"] {
        let mut instance = canonical();
        instance["plugin_api_version"] = json!(version);
        let validator = validator();
        assert!(
            !validator.is_valid(&instance),
            "plugin_api_version {version:?} must fail the schema"
        );
    }
}

#[test]
fn a_capability_that_is_destructive_and_read_is_rejected() {
    let mut instance = canonical();
    instance["capabilities"][0]["destructive"] = json!(true);
    let validator = validator();
    assert!(
        !validator.is_valid(&instance),
        "a destructive read capability must fail the schema, not only the SDK"
    );
}

#[test]
fn an_unknown_top_level_field_is_rejected() {
    let mut instance = canonical();
    instance["surprise"] = json!(true);
    let validator = validator();
    assert!(
        !validator.is_valid(&instance),
        "a manifest typo must not be silently ignored"
    );
}

#[test]
fn a_tool_name_with_two_segments_is_rejected() {
    let mut instance = canonical();
    instance["capabilities"][0]["tool"] = json!("aws.ec2");
    let validator = validator();
    assert!(!validator.is_valid(&instance));
}

#[test]
fn a_missing_filesystem_block_is_rejected() {
    let mut instance = canonical();
    instance
        .as_object_mut()
        .expect("the fixture is an object")
        .remove("filesystem");
    let validator = validator();
    assert!(
        !validator.is_valid(&instance),
        "an omitted jail must fail closed rather than mean access everywhere"
    );
}

#[test]
fn an_unknown_auth_source_is_rejected() {
    let mut instance = canonical();
    instance["auth"]["references"][0]["source"] = json!("plaintext");
    let validator = validator();
    assert!(!validator.is_valid(&instance));
}
