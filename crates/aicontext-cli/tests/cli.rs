//! The parts of the CLI that are about the binary rather than one command: `--version`, `--help`,
//! the refusal for a command that is not built yet, and the global flags.
//!
//! They live here rather than in `init.rs` because their subject is `main` and `args`, and a test
//! filed under one command's name while asserting another's behaviour misleads the next reader.

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

/// A temporary project directory, which a command that refuses never even has to look at.
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
fn version_works_without_a_subcommand() {
    // `--version` is answered from the raw arguments, so `aicontext --version` works even though a
    // subcommand is otherwise required.
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["--version"]);
    assert_eq!(run.code, 0);
}

#[test]
fn a_command_that_is_not_built_yet_exits_two_and_names_its_task() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["health"]);

    assert_eq!(run.code, 2, "an unimplemented command is a usage failure");
    assert!(run.stderr.contains("TASK-038"), "{}", run.stderr);
    assert!(
        run.stdout.is_empty(),
        "the refusal belongs on stderr: {}",
        run.stdout
    );
}

#[test]
fn an_unimplemented_command_answers_json_when_asked() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["health", "--json"]);

    let body: serde_json::Value = serde_json::from_str(&run.stdout).expect("one JSON object");
    assert_eq!(body["command"], "health");
    assert_eq!(body["exit_code"], 2);
    assert_eq!(body["ok"], false);
    assert_eq!(body["data"]["task"], "TASK-038");
}

#[test]
fn a_planned_command_is_refused_before_its_flags_are_validated() {
    // The developer is told what is missing before being told what else they typed, so a flag the
    // planned command may not even have still produces the named refusal rather than a usage error.
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["health", "--not-a-flag"]);
    assert_eq!(run.code, 2);
    assert!(run.stderr.contains("TASK-038"), "{}", run.stderr);
}

#[test]
fn help_lists_what_works_and_what_does_not() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["--help"]);

    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("init"), "{}", run.stdout);
    assert!(
        run.stdout.contains("doctor"),
        "a built command must be listed: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("status"),
        "a built command must be listed: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("TASK-038"),
        "a planned command must name its task"
    );
    assert!(
        !run.stdout.contains("TASK-013"),
        "status is built, so it no longer belongs to the planned list: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("TASK-014"),
        "doctor is built, so it no longer belongs to the planned list: {}",
        run.stdout
    );
}

#[test]
fn no_colour_reaches_the_output() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["init", "--color", "always", "--no-color"]);

    assert_eq!(run.code, 0, "the two flags together must not be an error");
    assert!(
        !run.stdout.contains('\u{1b}'),
        "colour must be off: {}",
        run.stdout
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
