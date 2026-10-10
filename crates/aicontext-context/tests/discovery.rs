//! The discovery engine, exercised through its public API.
//!
//! Mirrors the acceptance criteria of `TASK-030`:
//!
//! 1. **It detects** language, framework, package manager, tests, CI, containers, `IaC`, and cloud
//!    configuration.
//! 2. **It never mutates** the project: the tree is byte-for-byte the same after a run.
//! 3. **It never guesses**: a project that matches nothing is `unrecognised`.
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use std::fs;
use std::path::Path;

use aicontext_context::{Confidence, ProjectProfile, discover};

/// A recursive snapshot of every file below `root`, so a run that wrote something shows up.
///
/// Names are sorted and contents are read as text: enough to prove discovery is read-only without
/// hashing, because the comparison is against the same bytes that were just written.
fn snapshot(root: &Path) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    collect(root, root, &mut entries);
    entries.sort();
    entries
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("readable directory") {
        let entry = entry.expect("readable entry");
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("inside the root")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, fs::read_to_string(&path).unwrap_or_default()));
        }
    }
}

#[test]
fn a_rust_service_is_fully_described() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let path = root.path();

    fs::write(path.join("Cargo.toml"), "[dependencies]\naxum = \"0.7\"\n").expect("write");
    fs::write(path.join("Cargo.lock"), "").expect("write");
    fs::create_dir(path.join(".git")).expect("dir");
    fs::create_dir_all(path.join(".github").join("workflows")).expect("dir");
    fs::write(path.join("Dockerfile"), "FROM rust\n").expect("write");
    fs::create_dir(path.join("tests")).expect("dir");
    fs::write(path.join("main.tf"), "").expect("write");
    fs::create_dir(path.join("k8s")).expect("dir");

    let profile = discover(path);

    assert_eq!(profile.languages, ["rust"]);
    assert_eq!(profile.frameworks, ["axum"]);
    assert_eq!(profile.package_manager.as_deref(), Some("cargo"));
    assert_eq!(profile.vcs.as_deref(), Some("git"));
    assert!(profile.testing);
    assert_eq!(profile.ci, ["github-actions"]);
    assert_eq!(profile.containers, ["docker"]);
    assert_eq!(profile.iac, ["kubernetes", "terraform"]);
    assert!(profile.cloud_hints.is_empty());
    assert!(!profile.unrecognised);
    assert_eq!(
        profile.confidence,
        Some(Confidence::Medium),
        "the tests directory is the weakest signal"
    );
}

#[test]
fn an_unknown_project_yields_an_empty_profile_rather_than_a_guess() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let profile = discover(root.path());

    assert!(profile.unrecognised);
    assert!(profile.languages.is_empty());
    assert!(profile.frameworks.is_empty());
    assert!(profile.signals.is_empty());
    assert_eq!(profile.confidence, None);
}

#[test]
fn discovery_never_mutates_the_project() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let path = root.path();

    fs::write(
        path.join("package.json"),
        "{\"dependencies\":{\"react\":\"18\"}}",
    )
    .expect("write");
    fs::write(path.join("tsconfig.json"), "{}").expect("write");
    fs::write(path.join("package-lock.json"), "{}").expect("write");
    fs::create_dir(path.join("src")).expect("dir");

    let before = snapshot(path);
    let _ = discover(path);
    let after = snapshot(path);

    assert_eq!(before, after, "discovery must not write anything");
    assert!(
        !before.is_empty(),
        "the snapshot must actually contain the fixture"
    );
}

#[test]
fn a_node_project_reports_type_script_and_its_frameworks() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let path = root.path();

    fs::write(
        path.join("package.json"),
        "{\"dependencies\":{\"react\":\"18\",\"next\":\"14\"},\"devDependencies\":{\"jest\":\"29\"}}",
    )
    .expect("write");
    fs::write(path.join("tsconfig.json"), "{}").expect("write");
    fs::write(path.join("package-lock.json"), "{}").expect("write");
    fs::write(path.join("jest.config.js"), "module.exports = {};\n").expect("write");

    let profile = discover(path);

    assert_eq!(profile.languages, ["javascript", "typescript"]);
    assert_eq!(profile.frameworks, ["nextjs", "react"]);
    assert_eq!(profile.package_manager.as_deref(), Some("npm"));
    assert!(profile.testing);
    assert_eq!(profile.confidence, Some(Confidence::High));
}

#[test]
fn a_python_project_is_described() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let path = root.path();

    fs::write(
        path.join("pyproject.toml"),
        "[project]\ndependencies = [\"fastapi\"]\n",
    )
    .expect("write");
    fs::write(path.join("poetry.lock"), "").expect("write");

    let profile = discover(path);

    assert_eq!(profile.languages, ["python"]);
    assert_eq!(profile.frameworks, ["fastapi"]);
    assert_eq!(profile.package_manager.as_deref(), Some("poetry"));
}

#[test]
fn every_field_value_keeps_its_evidence() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let path = root.path();

    fs::write(path.join("Cargo.toml"), "[dependencies]\naxum = \"0.7\"\n").expect("write");
    fs::write(path.join("Cargo.lock"), "").expect("write");
    fs::create_dir(path.join(".git")).expect("dir");
    fs::write(path.join("Dockerfile"), "FROM rust\n").expect("write");

    let profile = discover(path);
    let kinds: Vec<&str> = profile.signals.iter().map(|signal| signal.kind).collect();

    for kind in [
        "languages",
        "frameworks",
        "package_manager",
        "vcs",
        "containers",
    ] {
        assert!(kinds.contains(&kind), "no signal recorded for {kind}");
        assert!(
            profile
                .signals
                .iter()
                .filter(|signal| signal.kind == kind)
                .all(|signal| !signal.evidence.is_empty()),
            "a {kind} signal without evidence is a claim with no file behind it"
        );
    }
}

#[test]
fn two_runs_over_an_unchanged_tree_agree() {
    let root = tempfile::TempDir::new().expect("temp dir");
    let path = root.path();
    fs::write(path.join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("write");
    fs::write(path.join("main.tf"), "").expect("write");

    let first = discover(path);
    let second: ProjectProfile = discover(path);
    assert_eq!(first, second);
}
