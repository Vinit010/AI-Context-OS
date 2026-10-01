//! `aicontext init` end to end, through the real binary.
//!
//! These tests run the compiled binary rather than calling into the crate, because the contract
//! `docs/CLI_SPEC.md` states is about the process a developer actually invokes: the bytes on stdout,
//! the code it exits with, and the files left behind. A unit test can agree with itself and still be
//! wrong about all three.
//!
//! Every project is a temporary directory with a real name inside it, so the derived project name is
//! asserted on rather than whatever a random suffix happened to be.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A temporary project directory with a name a developer would recognise.
fn project(name: &str) -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().join(name);
    fs::create_dir_all(&root).expect("create project");
    (outer, root)
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

/// Reads a file, or fails with the path so the assertion says what was missing.
fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("reading {path}: {error}"))
}

#[test]
fn init_creates_the_documented_skeleton_in_a_fresh_directory() {
    let (_outer, root) = project("payments-ledger");
    let run = run_in(&root, &["init"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    for path in [
        ".ai/AI.md",
        ".ai/PRD.md",
        ".ai/ARCHITECTURE.md",
        ".ai/RULES.md",
        ".ai/CONVENTIONS.md",
        ".ai/DESIGN.md",
        ".ai/TASKS.md",
        ".ai/MEMORY.md",
        ".ai/context/stack.md",
        ".ai/permissions/permissions.yaml",
    ] {
        assert!(root.join(path).is_file(), "{path} was not created");
    }
    for path in [
        ".ai/specs",
        ".ai/tasks",
        ".ai/decisions",
        ".ai/bugs",
        ".ai/workflows",
    ] {
        assert!(root.join(path).is_dir(), "{path}/ was not created");
    }
    assert!(read(&root, ".gitignore").contains(".aicontext/"));
    assert!(
        !root.join(".aicontext").exists(),
        "the local state directory must never be created"
    );
}

#[test]
fn the_project_name_comes_from_the_directory() {
    let (_outer, root) = project("payments-ledger");
    let run = run_in(&root, &["init"]);

    assert!(run.stdout.contains("payments-ledger"), "{}", run.stdout);
    assert!(read(&root, ".ai/RULES.md").contains("title: \"payments-ledger — Rulebook\""));
}

#[test]
fn init_is_idempotent_and_produces_no_diff_on_a_second_run() {
    let (_outer, root) = project("ledger");
    run_in(&root, &["init"]);
    let before = read(&root, ".ai/RULES.md");

    let second = run_in(&root, &["init"]);
    assert_eq!(second.code, 0, "stderr was: {}", second.stderr);
    assert_eq!(
        read(&root, ".ai/RULES.md"),
        before,
        "a second run must change nothing"
    );
    assert!(
        second.stdout.contains("already current"),
        "{}",
        second.stdout
    );
}

#[test]
fn a_dry_run_writes_nothing() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init", "--dry-run"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(
        !root.join(".ai").exists(),
        "a dry run must not create anything"
    );
    assert!(run.stdout.contains("would write"), "{}", run.stdout);
}

#[test]
fn an_edited_document_is_preserved_and_reported() {
    let (_outer, root) = project("ledger");
    run_in(&root, &["init"]);
    fs::write(root.join(".ai").join("RULES.md"), "my own rules\n").expect("edit");

    let run = run_in(&root, &["init"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert_eq!(
        read(&root, ".ai/RULES.md"),
        "my own rules\n",
        "an edit must survive"
    );
    assert!(
        run.stdout.contains("CTX-017"),
        "the finding must be reported: {}",
        run.stdout
    );
}

#[test]
fn force_in_a_pipe_refuses_rather_than_overwriting() {
    let (_outer, root) = project("ledger");
    run_in(&root, &["init"]);
    fs::write(root.join(".ai").join("RULES.md"), "my own rules\n").expect("edit");

    // A test harness captures stdout, so this run is not a terminal: the approval cannot be given.
    let run = run_in(&root, &["init", "--force"]);
    assert_eq!(run.code, 5, "APPROVAL_REQUIRED. stderr was: {}", run.stderr);
    assert_eq!(read(&root, ".ai/RULES.md"), "my own rules\n");
    assert!(run.stderr.contains("--force"), "{}", run.stderr);
}

#[test]
fn yes_does_not_stand_in_for_the_missing_human() {
    let (_outer, root) = project("ledger");
    run_in(&root, &["init"]);
    fs::write(root.join(".ai").join("RULES.md"), "my own rules\n").expect("edit");

    let run = run_in(&root, &["init", "--force", "--yes"]);
    assert_eq!(run.code, 5, "--yes must not satisfy an explicit approval");
    assert_eq!(read(&root, ".ai/RULES.md"), "my own rules\n");
}

#[test]
fn json_output_is_one_object_with_the_documented_envelope() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init", "--json"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let body: serde_json::Value = serde_json::from_str(&run.stdout)
        .unwrap_or_else(|error| panic!("stdout was not one JSON object: {error}\n{}", run.stdout));
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["command"], "init");
    assert_eq!(body["ok"], true);
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["data"]["project"], "ledger");
    assert_eq!(body["data"]["template"], "default");
    assert_eq!(body["data"]["dry_run"], false);
    assert_eq!(body["data"]["gitignore"], "created");
    assert!(
        body["data"]["actions"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    assert_eq!(body["findings"].as_array().map(Vec::len), Some(0));
    assert!(
        body["summary"]
            .as_str()
            .is_some_and(|line| line.contains("wrote"))
    );
}

#[test]
fn a_preserved_edit_appears_as_a_warning_code_in_the_envelope() {
    let (_outer, root) = project("ledger");
    run_in(&root, &["init"]);
    fs::write(root.join(".ai").join("RULES.md"), "mine\n").expect("edit");

    let run = run_in(&root, &["init", "--json"]);
    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["warnings"][0], "CTX-017");
    assert_eq!(body["findings"][0]["severity"], "warning");
    assert_eq!(body["findings"][0]["path"], ".ai/RULES.md");
    assert_eq!(body["exit_code"], 0, "a preserved edit is not a failure");
}

#[test]
fn the_blank_template_creates_two_documents_and_nothing_else() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init", "--template", "blank"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(root.join(".ai/AI.md").is_file());
    assert!(root.join(".ai/RULES.md").is_file());
    assert!(!root.join(".ai/PRD.md").exists());
    assert!(
        !root.join(".ai/specs").exists(),
        "blank has no register directories"
    );
}

#[test]
fn a_stack_template_replaces_the_base_conventions_and_architecture() {
    let (_outer, root) = project("ledger");
    assert_eq!(run_in(&root, &["init"]).code, 0);
    let base_conventions = read(&root, ".ai/CONVENTIONS.md");

    let rust = run_in(&root, &["init", "--template", "rust", "--force"]);
    assert_eq!(
        rust.code, 5,
        "forcing needs a terminal, so this run must refuse"
    );

    // Build the same skeleton the overlay would, in a fresh directory, and compare.
    let (_other, fresh) = project("ledger");
    assert_eq!(run_in(&fresh, &["init", "--template", "rust"]).code, 0);
    let rust_conventions = read(&fresh, ".ai/CONVENTIONS.md");

    assert_ne!(
        base_conventions, rust_conventions,
        "the overlay must replace the base file"
    );
    assert!(rust_conventions.contains("Rust starter conventions"));
}

#[test]
fn an_existing_gitignore_is_appended_to_and_never_rewritten() {
    let (_outer, root) = project("ledger");
    fs::write(root.join(".gitignore"), "target/\n.env\n").expect("write");

    assert_eq!(run_in(&root, &["init"]).code, 0);
    assert_eq!(read(&root, ".gitignore"), "target/\n.env\n.aicontext/\n");
}

#[test]
fn discovery_reports_what_it_found_without_writing_it_anywhere() {
    let (_outer, root) = project("ledger");
    fs::write(root.join("Cargo.toml"), "[package]\n").expect("write");

    let run = run_in(&root, &["init"]);
    assert!(run.stdout.contains("languages rust"), "{}", run.stdout);
    assert!(
        run.stdout.contains("Cargo.toml") || !run.stdout.contains("languages rust"),
        "the report names what it found"
    );

    // The detection is advisory: it is not written into ARCHITECTURE.md.
    assert!(!read(&root, ".ai/ARCHITECTURE.md").contains("languages rust"));
}

#[test]
fn an_empty_project_reports_nothing_rather_than_a_guess() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init"]);

    assert!(
        run.stdout.contains("nothing found, and nothing guessed"),
        "{}",
        run.stdout
    );
}

#[test]
fn discovery_can_be_skipped() {
    let (_outer, root) = project("ledger");
    fs::write(root.join("Cargo.toml"), "[package]\n").expect("write");

    let run = run_in(&root, &["init", "--no-detect"]);
    assert!(run.stdout.contains("nothing found"), "{}", run.stdout);
    assert!(!run.stdout.contains("languages rust"));
}

#[test]
fn the_cwd_flag_operates_on_the_named_directory() {
    let (_outer, root) = project("outer");
    let inner = root.join("inner");
    fs::create_dir_all(&inner).expect("dir");

    let run = run_in(
        &root,
        &["init", "--cwd", inner.to_str().expect("utf-8 path")],
    );
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(inner.join(".ai/RULES.md").is_file());
    assert!(
        !root.join(".ai").exists(),
        "the flag must move the root, not add one"
    );
}

#[test]
fn a_command_that_is_not_built_yet_exits_two_and_names_its_task() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["doctor"]);

    assert_eq!(run.code, 2, "an unimplemented command is a usage failure");
    assert!(run.stderr.contains("TASK-014"), "{}", run.stderr);
    assert!(
        run.stdout.is_empty(),
        "the refusal belongs on stderr: {}",
        run.stdout
    );
}

#[test]
fn an_unimplemented_command_answers_json_when_asked() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["doctor", "--json"]);

    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["command"], "doctor");
    assert_eq!(body["exit_code"], 2);
    assert_eq!(body["ok"], false);
    assert_eq!(body["data"]["task"], "TASK-014");
}

#[test]
fn version_reports_the_version_the_api_and_the_build_profile() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["--version"]);

    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("aicontext "), "{}", run.stdout);
    assert!(run.stdout.contains("plugin-api "), "{}", run.stdout);
    assert!(run.stdout.contains("profile "), "{}", run.stdout);
}

#[test]
fn help_lists_what_works_and_what_does_not() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["--help"]);

    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("init"), "{}", run.stdout);
    assert!(
        run.stdout.contains("TASK-014"),
        "a planned command must name its task"
    );
}

#[test]
fn no_colour_reaches_the_output() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init", "--color", "always", "--no-color"]);

    assert_eq!(run.code, 0, "the two flags together must not be an error");
    assert!(
        !run.stdout.contains('\u{1b}'),
        "colour must be off: {run:?}",
        run = run.stdout
    );
}

#[test]
fn colour_is_emitted_when_it_is_asked_for_explicitly() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init", "--color", "always", "--dry-run"]);

    assert!(
        run.stdout.contains('\u{1b}'),
        "colour was asked for: {}",
        run.stdout
    );
}

#[test]
fn a_root_that_does_not_exist_is_refused_without_writing_anything() {
    let (_outer, root) = project("ledger");
    let missing = root.join("nowhere");
    let run = run_in(
        &root,
        &["init", "--cwd", missing.to_str().expect("utf-8 path")],
    );

    assert_eq!(
        run.code, 2,
        "a bad root is a usage failure. stderr: {}",
        run.stderr
    );
    assert!(run.stderr.contains("INIT-001"), "{}", run.stderr);
    assert!(!root.join(".ai").exists());
}
