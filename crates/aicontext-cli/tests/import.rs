//! `aicontext import` end to end, through the real binary.
//!
//! Import writes documents a developer did not compose, so its contract (`docs/CLI_SPEC.md` §4,
//! `TASK-020`) is the strict one: verify *everything* before writing *anything*, never touch a file
//! that already holds the archived content, refuse a conflict without approval, and never delete a
//! file the archive does not mention. Every test here asserts through the process a developer runs:
//! the exit code, the files on disk, and the bytes of a file that should have been left alone.
//!
//! The archives under test are produced by the binary's own `export`, then tampered where a test
//! needs an archive that must not be trusted.

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

/// Produces an archive `name` inside `source`'s root and returns its path.
fn exported(source: &TempProject, name: &str) -> String {
    sample_ai_tree(source.path()).expect("the sample tree");
    let run = run_in(source.path(), &["export", name]);
    assert_eq!(run.code, 0, "the fixture export failed: {}", run.stderr);
    source.join(name).to_string_lossy().replace('\\', "/")
}

/// The archive text with one entry's content altered in place, so its digest no longer matches.
fn tampered_archive_text(path: &Path) -> String {
    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("one document");
    let content = doc["entries"][0]["content"].as_str().expect("content");
    doc["entries"][0]["content"] = format!("{content}x").into();
    let mut text = serde_json::to_string(&doc).expect("serialises");
    text.push('\n');
    text
}

#[test]
fn import_writes_the_archive_into_a_fresh_root() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");

    let run = run_in(target.path(), &["import", &archive]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(target.join(".ai/AI.md").is_file());
    assert!(target.join(".ai/TASKS.md").is_file());
    assert_eq!(
        fs::read(target.join(".ai/AI.md")).expect("read"),
        fs::read(source.join(".ai/AI.md")).expect("read"),
        "imported content must be byte-identical to the export's"
    );
    assert!(run.stdout.contains("no vcs"), "{}", run.stdout);

    let probe = TempProject::new("probe").expect("a fresh root");
    let parsed = envelope(&run_in(probe.path(), &["import", &archive, "--json"]));
    assert_eq!(parsed["command"], "import");
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["exit_code"], 0);
    assert_eq!(parsed["data"]["project"], "probe");
    assert_eq!(parsed["data"]["dry_run"], false);
    assert_eq!(parsed["data"]["file_count"], 2);
    assert!(parsed["data"]["digest"].as_str().unwrap().len() == 64);
    assert_eq!(
        parsed["data"]["created"],
        json_array(&[".ai/AI.md", ".ai/TASKS.md"])
    );
    assert_eq!(
        parsed["data"]["unchanged"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(
        parsed["data"]["conflicts"].as_array().map(Vec::len),
        Some(0)
    );
}

/// The documented list-shaped field as a JSON array, escaping nothing and adding nothing.
fn json_array(items: &[&str]) -> Value {
    serde_json::Value::Array(
        items
            .iter()
            .map(|item| Value::String((*item).to_string()))
            .collect(),
    )
}

#[test]
fn a_second_import_changes_nothing_and_reports_unchanged() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    assert_eq!(run_in(target.path(), &["import", &archive]).code, 0);

    let first = fs::read(target.join(".ai/TASKS.md")).expect("read");
    let run = run_in(target.path(), &["import", &archive]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert_eq!(fs::read(target.join(".ai/TASKS.md")).expect("read"), first);
    let parsed = envelope(&run_in(target.path(), &["import", &archive, "--json"]));
    assert_eq!(parsed["data"]["created"].as_array().map(Vec::len), Some(0));
    assert_eq!(
        parsed["data"]["unchanged"],
        json_array(&[".ai/AI.md", ".ai/TASKS.md"])
    );
}

#[test]
fn the_round_trip_is_byte_identical() {
    // The acceptance criterion of TASK-020: export a tree, import it into a fresh same-named root,
    // and the re-export must be byte-for-byte the archive the first export produced.
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let round = TempProject::new("pipeline").expect("a same-named root");
    assert_eq!(run_in(round.path(), &["import", &archive]).code, 0);

    for file in [".ai/AI.md", ".ai/TASKS.md"] {
        assert_eq!(
            fs::read(round.join(file)).expect("read"),
            fs::read(source.join(file)).expect("read"),
            "{file} must survive the round trip byte for byte"
        );
    }

    assert_eq!(run_in(round.path(), &["export", "again.aix"]).code, 0);
    let reexported = fs::read(round.join("again.aix")).expect("read");
    let original = fs::read(source.join("backup.aix")).expect("read");
    assert_eq!(
        reexported, original,
        "re-exporting the imported tree must reproduce the archive exactly"
    );
}

#[test]
fn import_ignores_files_the_archive_does_not_mention() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    target.write(".ai/local.md", "mine\n").expect("local file");
    target.write("README.md", "also mine\n").expect("unrelated");

    let run = run_in(target.path(), &["import", &archive]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert_eq!(target.read(".ai/local.md").expect("read"), "mine\n");
    assert_eq!(target.read("README.md").expect("read"), "also mine\n");
}

#[test]
fn conflicts_are_refused_without_force_and_left_untouched() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    target
        .write(".ai/AI.md", "my own\n")
        .expect("the existing edit");

    let run = run_in(target.path(), &["import", &archive]);

    assert_eq!(
        run.code, 5,
        "a conflict needs approval. stderr: {}",
        run.stderr
    );
    assert!(run.stdout.contains("IMP-020"), "{}", run.stdout);
    let parsed = envelope(&run_in(target.path(), &["import", &archive, "--json"]));
    assert_eq!(parsed["exit_code"], 5);
    assert_eq!(parsed["findings"][0]["code"], "IMP-020");
    assert_eq!(parsed["data"]["conflicts"], json_array(&[".ai/AI.md"]));
    assert_eq!(target.read(".ai/AI.md").expect("read"), "my own\n");
    assert!(
        !target.join(".ai/TASKS.md").exists(),
        "the whole tree stays untouched while a conflict is unresolved"
    );
}

#[test]
fn force_in_a_pipe_refuses_conflicts_rather_than_overwriting() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    target.write(".ai/AI.md", "my own\n").expect("the edit");

    // A test harness captures stdout, so this run is not a terminal: the approval cannot be given.
    let run = run_in(target.path(), &["import", &archive, "--force"]);
    assert_eq!(run.code, 5, "APPROVAL_REQUIRED. stderr was: {}", run.stderr);
    assert!(run.stderr.contains("IMP-004"), "{}", run.stderr);
    assert_eq!(target.read(".ai/AI.md").expect("read"), "my own\n");
}

#[test]
fn force_dry_run_writes_nothing_but_reports_would_replace() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    target.write(".ai/AI.md", "my own\n").expect("the edit");

    let run = run_in(target.path(), &["import", &archive, "--force", "--dry-run"]);
    assert_eq!(
        run.code, 0,
        "a dry run is not a write. stderr: {}",
        run.stderr
    );
    assert!(run.stdout.contains("would replace"), "{}", run.stdout);
    let parsed = envelope(&run_in(
        target.path(),
        &["import", &archive, "--force", "--dry-run", "--json"],
    ));
    assert_eq!(parsed["data"]["dry_run"], true);
    assert_eq!(parsed["data"]["conflicts"], json_array(&[".ai/AI.md"]));
    assert_eq!(target.read(".ai/AI.md").expect("read"), "my own\n");
}

#[test]
fn a_tampered_archive_is_refused_whole_and_writes_nothing() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    let hostile = target.join("packed.aix");
    fs::write(&hostile, tampered_archive_text(Path::new(&archive))).expect("write");

    let run = run_in(target.path(), &["import", hostile.to_str().unwrap()]);

    assert_eq!(
        run.code, 3,
        "a tampered archive is a validation failure. stderr: {}",
        run.stderr
    );
    assert!(run.stdout.contains("IMP-011"), "{}", run.stdout);
    assert!(
        !target.join(".ai").exists(),
        "nothing may be written from an archive that does not verify"
    );
    let parsed = envelope(&run_in(
        target.path(),
        &["import", hostile.to_str().unwrap(), "--json"],
    ));
    assert_eq!(parsed["exit_code"], 3);
    assert!(
        parsed["findings"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["code"] == "IMP-011")),
        "{}",
        parsed["findings"]
    );
}

#[test]
fn a_wrong_format_file_is_a_usage_error_and_a_missing_one_is_too() {
    let project = TempProject::new("mirror").expect("a project");
    fs::write(project.join("not-an-archive.json"), "hello").expect("write");

    let run = run_in(project.path(), &["import", "not-an-archive.json"]);
    assert_eq!(run.code, 2, "unreadable content is a usage failure");
    assert!(run.stderr.contains("IMP-003"), "{}", run.stderr);

    let run = run_in(project.path(), &["import", "nowhere.aix"]);
    assert_eq!(run.code, 2, "a missing archive is a usage failure");
    assert!(run.stderr.contains("IMP-002"), "{}", run.stderr);
}

#[test]
fn a_traversal_path_in_the_archive_is_refused_and_writes_nowhere() {
    // A hostile archive must not be able to name a file outside `.ai/`: the path check runs before
    // planning, so even a hand-made document cannot place a file outside the context directory.
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");

    let mut doc: Value =
        serde_json::from_str(&fs::read_to_string(&archive).expect("read")).expect("one document");
    doc["manifest"]["files"][0]["path"] = Value::String("../../escape.md".to_string());
    doc["entries"][0]["path"] = Value::String("../../escape.md".to_string());
    let hostile = target.join("packed.aix");
    let mut text = serde_json::to_string(&doc).expect("serialises");
    text.push('\n');
    fs::write(&hostile, text).expect("write");

    let run = run_in(target.path(), &["import", hostile.to_str().unwrap()]);

    assert_eq!(run.code, 3, "a traversal path must be refused");
    assert!(run.stdout.contains("IMP-013"), "{}", run.stdout);
    assert!(
        !target.temp_path().join("escape.md").exists(),
        "nothing may be written outside .ai/ (../../ from .ai/ lands in the fixture's temp directory)"
    );
    assert!(!target.join(".ai").exists());
}

#[test]
fn a_directory_where_a_file_must_go_blocks_the_whole_import() {
    let source = TempProject::new("pipeline").expect("a project");
    let archive = exported(&source, "backup.aix");
    let target = TempProject::new("mirror").expect("a project");
    target.create_dir(".ai/AI.md").expect("the obstruction");

    let run = run_in(target.path(), &["import", &archive]);
    assert_eq!(
        run.code, 1,
        "a blocked path is a general failure. stderr: {}",
        run.stderr
    );
    assert!(run.stderr.contains("IMP-005"), "{}", run.stderr);
    assert!(
        !target.join(".ai/TASKS.md").exists(),
        "the whole import is refused before any write"
    );
}

#[test]
fn an_empty_archive_imports_as_no_files() {
    let source = TempProject::new("bare").expect("a project");
    source.create_dir(".ai").expect("the context directory");
    assert_eq!(run_in(source.path(), &["export", "empty.aix"]).code, 0);
    let target = TempProject::new("mirror").expect("a project");

    let run = run_in(
        target.path(),
        &["import", source.join("empty.aix").to_str().unwrap()],
    );

    assert_eq!(run.code, 0, "an empty archive is not an error");
    assert!(
        run.stdout.contains("holds no files"),
        "the report says so: {}",
        run.stdout
    );
    assert!(!target.join(".ai").exists());
}
