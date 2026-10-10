//! `aicontext export` end to end, through the real binary.
//!
//! The contract `docs/CLI_SPEC.md` §4 and `TASK-020` state is about the process a developer actually
//! invokes: the archive bytes written, the code it exits with, and the files left alone. A unit test
//! can agree with itself about `walk` and still be wrong about the process, so these tests run the
//! compiled binary.
//!
//! The fixture trees come from `aicontext-testkit`'s `sample_ai_tree`, so an export carries exactly
//! the two `.ai/` documents and nothing else — a tree that grows between release notes keeps this
//! suite from noticing, but the counts below would catch it loudly.

use std::fs;
use std::path::Path;
use std::process::Command;

use aicontext_testkit::{TempProject, sample_ai_tree};
use serde_json::Value;

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Runs the binary in `root`.
fn run_in(root: &Path, args: &[&str]) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_aicontext"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("the binary runs");
    Run {
        code: output.status.code().expect("the process exited normally"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// The JSON envelope one run printed, parsed.
fn envelope(run: &Run) -> Value {
    serde_json::from_str(run.stdout.trim())
        .unwrap_or_else(|error| panic!("stdout was not one JSON object ({error}): {}", run.stdout))
}

/// The archive `export` wrote, parsed as its JSON document.
fn archive_document(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("the archive is readable"))
        .expect("the archive is one JSON document")
}

#[test]
fn export_writes_the_documented_archive_and_a_matching_envelope() {
    let source = TempProject::new("pipeline").expect("a project");
    source
        .write("README.md", "not context\n")
        .expect("unrelated");
    sample_ai_tree(source.path()).expect("the sample tree");

    let run = run_in(source.path(), &["export", "backup.aix"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(
        source.join("backup.aix").is_file(),
        "the archive is written"
    );
    let doc = archive_document(&source.join("backup.aix"));
    assert_eq!(doc["format"], "aicontext-archive");
    assert_eq!(doc["version"], 1);
    assert_eq!(doc["manifest"]["project"], "pipeline");
    assert_eq!(doc["manifest"]["file_count"], 2);
    assert_eq!(doc["manifest"]["files"].as_array().map(Vec::len), Some(2));
    assert_eq!(doc["entries"].as_array().map(Vec::len), Some(2));
    let rows = doc["entries"].as_array().expect("entries");
    assert_eq!(rows[0]["path"], "AI.md");
    assert_eq!(rows[1]["path"], "TASKS.md");

    let parsed = envelope(&run_in(source.path(), &["export", "backup2.aix", "--json"]));
    assert_eq!(parsed["command"], "export");
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["exit_code"], 0);
    assert_eq!(parsed["data"]["project"], "pipeline");
    assert_eq!(parsed["data"]["dry_run"], false);
    assert_eq!(parsed["data"]["replacing"], false);
    assert_eq!(parsed["data"]["file_count"], 2);
    assert!(parsed["data"]["total_bytes"].as_u64().unwrap() > 0);
    let digest = parsed["data"]["digest"].as_str().expect("a digest");
    assert!(digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(parsed["data"]["files"].as_array().map(Vec::len), Some(2));
    assert_eq!(parsed["data"]["files"][0]["path"], "AI.md");
    assert_eq!(parsed["findings"].as_array().map(Vec::len), Some(0));
}

#[test]
fn export_is_deterministic_byte_for_byte() {
    let first = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(first.path()).expect("tree");
    let second = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(second.path()).expect("tree");

    assert_eq!(run_in(first.path(), &["export", "one.aix"]).code, 0);
    assert_eq!(run_in(first.path(), &["export", "two.aix"]).code, 0);
    assert_eq!(run_in(second.path(), &["export", "three.aix"]).code, 0);

    let one = fs::read(first.join("one.aix")).expect("read");
    let two = fs::read(first.join("two.aix")).expect("read");
    let three = fs::read(second.join("three.aix")).expect("read");
    assert_eq!(
        one, two,
        "the same tree must export the same bytes however many times"
    );
    assert_eq!(
        one, three,
        "a fresh same-named root must export the same bytes, without timestamps or paths in the document"
    );
}

#[test]
fn a_relative_output_is_resolved_against_the_project_root() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    source.create_dir("nested").expect("the output directory");

    let run = run_in(source.path(), &["export", "nested/backup.aix"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(source.join("nested/backup.aix").is_file());
    let parsed = envelope(&run_in(
        source.path(),
        &["export", "nested/backup.aix", "--json"],
    ));
    let archive = parsed["data"]["archive"].as_str().expect("archive path");
    assert!(archive.ends_with("/nested/backup.aix"), "{archive}");
}

#[test]
fn export_without_an_ai_directory_is_a_usage_error() {
    let project = TempProject::new("empty").expect("a project");

    let run = run_in(project.path(), &["export", "backup.aix"]);

    assert_eq!(
        run.code, 2,
        "no .ai/ is a usage failure. stderr: {}",
        run.stderr
    );
    assert!(run.stderr.contains("EXP-002"), "{}", run.stderr);
    assert!(!project.join("backup.aix").exists());
}

#[test]
fn an_existing_archive_is_refused_without_force() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    assert_eq!(run_in(source.path(), &["export", "backup.aix"]).code, 0);
    let before = fs::read(source.join("backup.aix")).expect("read");

    let run = run_in(source.path(), &["export", "backup.aix"]);
    assert_eq!(
        run.code, 5,
        "an existing archive needs approval. stderr: {}",
        run.stderr
    );
    assert!(run.stdout.contains("EXP-020"), "{}", run.stdout);
    assert_eq!(
        fs::read(source.join("backup.aix")).expect("read"),
        before,
        "the refusal must not touch the archive"
    );
}

#[test]
fn force_in_a_pipe_refuses_rather_than_overwriting() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    assert_eq!(run_in(source.path(), &["export", "backup.aix"]).code, 0);
    let before = fs::read(source.join("backup.aix")).expect("read");

    // A test harness captures stdout, so this run is not a terminal: the approval cannot be given.
    let run = run_in(source.path(), &["export", "backup.aix", "--force"]);
    assert_eq!(run.code, 5, "APPROVAL_REQUIRED. stderr was: {}", run.stderr);
    assert!(run.stderr.contains("EXP-006"), "{}", run.stderr);
    assert_eq!(fs::read(source.join("backup.aix")).expect("read"), before);
}

#[test]
fn force_dry_run_in_a_pipe_writes_nothing_but_says_it_would() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    assert_eq!(run_in(source.path(), &["export", "backup.aix"]).code, 0);
    let before = fs::read(source.join("backup.aix")).expect("read");

    let run = run_in(
        source.path(),
        &["export", "backup.aix", "--force", "--dry-run"],
    );
    assert_eq!(
        run.code, 0,
        "a dry run is not a write. stderr: {}",
        run.stderr
    );
    assert!(run.stdout.contains("would write"), "{}", run.stdout);
    assert_eq!(
        fs::read(source.join("backup.aix")).expect("read"),
        before,
        "a dry run must not write"
    );
    let parsed = envelope(&run_in(
        source.path(),
        &["export", "backup.aix", "--force", "--dry-run", "--json"],
    ));
    assert_eq!(parsed["data"]["dry_run"], true);
    assert_eq!(parsed["data"]["replacing"], true);
    assert_eq!(parsed["data"]["file_count"], 2);
}

#[test]
fn a_non_utf8_file_is_refused_and_nothing_is_written() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    fs::write(source.join(".ai/binary.md"), [0xff, 0xfe, 0x41, 0x00]).expect("write");

    let run = run_in(source.path(), &["export", "backup.aix"]);

    assert_eq!(run.code, 3, "a binary file in .ai/ is a validation failure");
    assert!(run.stdout.contains("EXP-010"), "{}", run.stdout);
    assert!(
        !source.join("backup.aix").exists(),
        "refused, so no archive"
    );

    let parsed = envelope(&run_in(source.path(), &["export", "backup.aix", "--json"]));
    assert_eq!(parsed["exit_code"], 3);
    assert!(parsed["data"]["digest"].is_null());
    assert_eq!(parsed["data"]["file_count"], 0);
    assert_eq!(parsed["findings"][0]["code"], "EXP-010");
    assert_eq!(parsed["findings"][0]["path"], ".ai/binary.md");
}

#[test]
fn an_entry_deeper_than_the_limit_is_refused_rather_than_archived() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    let deep = "a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/f.md";
    source
        .write(&format!(".ai/{deep}"), "# deep\n")
        .expect("write");

    let run = run_in(source.path(), &["export", "backup.aix"]);

    assert_eq!(
        run.code, 3,
        "a path past the depth cap is a validation failure"
    );
    assert!(run.stdout.contains("EXP-014"), "{}", run.stdout);
    assert!(!source.join("backup.aix").exists());
}

#[test]
fn exporting_into_a_directory_is_a_usage_error() {
    let source = TempProject::new("pipeline").expect("a project");
    sample_ai_tree(source.path()).expect("tree");
    source.create_dir("out").expect("a directory");

    let run = run_in(source.path(), &["export", "out"]);

    assert_eq!(
        run.code, 2,
        "a directory is not an archive. stderr: {}",
        run.stderr
    );
    assert!(run.stderr.contains("EXP-008"), "{}", run.stderr);
}

#[test]
fn export_prints_the_vcs_label_and_declares_an_empty_ai_directory_cleanly() {
    let source = TempProject::new("bare").expect("a project");
    source.create_dir(".ai").expect("the context directory");

    let run = run_in(source.path(), &["export", "backup.aix"]);

    assert_eq!(run.code, 0, "an empty .ai/ exports a zero-file archive");
    assert!(run.stdout.contains("no vcs"), "{}", run.stdout);
    assert!(source.join("backup.aix").is_file());
    let doc = archive_document(&source.join("backup.aix"));
    assert_eq!(doc["entries"].as_array().map(Vec::len), Some(0));
    assert_eq!(doc["manifest"]["file_count"], 0);
}
