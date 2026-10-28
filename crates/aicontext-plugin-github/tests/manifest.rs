//! The manifest and tool descriptors are one published contract.
//!
//! The host validates `manifest.json` against `schemas/plugin-manifest.schema.json` at install and
//! compares every descriptor from `tools/list` with the manifest's capabilities at start. These
//! tests check the same three things before a host ever sees the plugin: the schema accepts the
//! manifest, the SDK accepts it, and the seven descriptors agree with it.

use std::fs;
use std::path::{Path, PathBuf};

use aicontext_plugin_github::{
    MANIFEST_JSON, PLUGIN_API_VERSION, PLUGIN_NAME, PLUGIN_VERSION, tools,
};
use aicontext_plugin_sdk::{ApiVersion, Effect, PluginManifest, PluginProtocol};
use serde_json::{Value, json};

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

fn validator() -> jsonschema::Validator {
    jsonschema::validator_for(&schema()).expect("the manifest schema compiles")
}

fn manifest_value() -> Value {
    serde_json::from_str(MANIFEST_JSON).expect("manifest.json is valid JSON")
}

#[test]
fn the_manifest_validates_against_the_published_schema() {
    let instance = manifest_value();
    if let Err(error) = validator().validate(&instance) {
        panic!("crates/aicontext-plugin-github/manifest.json does not validate: {error}");
    }
}

#[test]
fn the_published_schema_is_still_valid_draft_2020_12() {
    if let Err(error) = jsonschema::meta::validate(&schema()) {
        panic!("schemas/plugin-manifest.schema.json is not valid Draft 2020-12: {error}");
    }
}

#[test]
fn the_sdk_accepts_the_manifest_and_negotiates_version_one() {
    let manifest = PluginManifest::from_json(MANIFEST_JSON).expect("the SDK accepts the manifest");
    assert_eq!(manifest.name, PLUGIN_NAME);
    assert_eq!(manifest.version, PLUGIN_VERSION);
    assert_eq!(manifest.plugin_api_version, PLUGIN_API_VERSION);
    assert_eq!(manifest.protocol, PluginProtocol::AiPlugin);
    assert_eq!(
        manifest
            .negotiate(ApiVersion::CURRENT)
            .expect("version 1 is supported"),
        ApiVersion::CURRENT
    );
}

#[test]
fn the_manifest_declares_exactly_the_seven_read_only_tools() {
    let manifest = PluginManifest::from_json(MANIFEST_JSON).expect("the SDK accepts the manifest");
    let names: Vec<&str> = manifest
        .capabilities
        .iter()
        .map(|capability| capability.tool.as_str())
        .collect();

    assert_eq!(names, tools::TOOL_NAMES);
    for capability in &manifest.capabilities {
        assert_eq!(
            capability.effect,
            Effect::Read,
            "{} must be read-only",
            capability.tool
        );
        assert!(
            !capability.destructive,
            "{} must not be destructive",
            capability.tool
        );
        assert!(
            capability.mcp_tool.is_none(),
            "{} is a native tool, not an MCP translation",
            capability.tool
        );
        assert_eq!(
            capability.permission_floor(),
            None,
            "{} must not impose a manifest floor above the policy",
            capability.tool
        );
    }
}

#[test]
fn the_descriptors_match_the_manifest_in_name_effect_and_destructive_flag() {
    let manifest = PluginManifest::from_json(MANIFEST_JSON).expect("the SDK accepts the manifest");
    let definitions = tools::tool_definitions();
    let names: Vec<&str> = definitions.iter().map(|tool| tool.tool.as_str()).collect();

    assert_eq!(names, tools::TOOL_NAMES);
    for definition in &definitions {
        let capability = manifest
            .capabilities
            .iter()
            .find(|capability| capability.tool == definition.tool)
            .unwrap_or_else(|| panic!("{} is not declared in the manifest", definition.tool));
        assert_eq!(definition.effect, capability.effect);
        assert_eq!(definition.destructive, capability.destructive);
        assert!(!definition.summary.trim().is_empty());
    }
}

#[test]
fn every_input_schema_is_draft_2020_12_and_closed() {
    for definition in tools::tool_definitions() {
        if let Err(error) = jsonschema::meta::validate(&definition.input_schema) {
            panic!(
                "the input schema for {} is not valid Draft 2020-12: {error}",
                definition.tool
            );
        }
        assert_eq!(
            definition.input_schema["type"], "object",
            "{} must take an object",
            definition.tool
        );
        assert_eq!(
            definition.input_schema["additionalProperties"], false,
            "{} must reject undeclared properties",
            definition.tool
        );
        assert!(
            definition.input_schema["required"]
                .as_array()
                .is_some_and(|required| !required.is_empty()),
            "{} must require at least one argument",
            definition.tool
        );
    }
}

#[test]
fn the_manifest_declares_the_token_source_and_egress_without_a_value() {
    let manifest = PluginManifest::from_json(MANIFEST_JSON).expect("the SDK accepts the manifest");
    let auth = manifest.auth.as_ref().expect("the manifest declares auth");
    assert_eq!(auth.references.len(), 1);
    assert_eq!(auth.references[0].name, "GITHUB_TOKEN");
    assert_eq!(
        auth.references[0].source,
        aicontext_plugin_sdk::AuthSource::Env
    );
    assert!(
        auth.instructions
            .as_deref()
            .is_some_and(|text| text.contains("read-only")),
        "the install instructions must tell the developer to use a read-only token"
    );
    assert_eq!(manifest.network.egress, ["api.github.com"]);
    assert!(manifest.filesystem.read.is_empty());
    assert!(manifest.filesystem.write.is_empty());
    assert_eq!(manifest.limits.call_timeout_ms, 30_000);
    assert_eq!(manifest.limits.max_output_bytes, 1_048_576);
    assert_eq!(manifest.entry.command, "aicontext-plugin-github");
    assert!(manifest.entry.args.is_empty());
}

#[test]
fn the_manifest_has_no_write_capability_even_if_a_field_is_added_later() {
    let instance = manifest_value();
    let capabilities = instance["capabilities"]
        .as_array()
        .expect("capabilities is an array");
    for capability in capabilities {
        assert_eq!(
            capability["effect"], "read",
            "a write capability would need TASK-081's approval path"
        );
        assert_ne!(
            capability["destructive"],
            json!(true),
            "a destructive capability would need TASK-081's approval path"
        );
    }
}
