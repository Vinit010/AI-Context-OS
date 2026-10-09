//! `aicontext status` end to end, through the real binary.
//!
//! The contract is about the process a developer runs: what it prints, what it exits with, and — for
//! this command — that it does so *in a repository* and *outside one*. The repository fixtures come
//! from `aicontext-testkit`, so every test reads the same branch, sees the same commit, and inherits
//! none of the developer's Git configuration (`CONVENTIONS.md` §8).
//!
//! The child binary is spawned with the fixture's own environment, which is exactly what the
//! production `Git::new` will inherit: `PATH` points at the fixture's git and nowhere else, so the
//! status the binary reads is the status the fixture created.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use aicontext_testkit::{TempProject, TempRepository};
use serde_json::Value;
use tempfile::TempDir;

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A register with one current task and one done task, in the shape `TASKS.md` documents.
const REGISTER: &str = "\
---\n\
id: TASKS-001\n\
type: tasks\n\
title: Fixture Register\n\
status: active\n\
updated: 2026-10-09\n\
---\n\
\n\
# TASKS\n\
\n\
**Current phase:** Phase 1 — Context MVP\n\
**Current task:** TASK-013\n\
\n\
### TASK-013 — Implement aicontext status\n\
\n\
```yaml\n\
id: TASK-013\n\
title: Implement aicontext status\n\
status: TODO\n\
priority: HIGH\n\
```\n\
\n\
### TASK-017 — A task that is finished\n\
\n\
```yaml\n\
id: TASK-017\n\
title: A task that is finished\n\
status: DONE\n\
priority: LOW\n\
```\n";

/// Runs the binary in `root` with the process environment inherited.
fn run_in(root: &Path, args: &[&str]) -> Run {
    run_with(root, None, args)
}

/// Runs the binary in `root` with the fixture's hermetic environment, so git reads its config.
fn run_hermetic(repo: &TempRepository, args: &[&str]) -> Run {
    run_with(repo.path(), Some(repo.environment()), args)
}

/// Runs the binary and captures its three observable channels.
fn run_with(root: &Path, environment: Option<Vec<(OsString, OsString)>>, args: &[&str]) -> Run {
    let mut command = Command::new(env!("CARGO_BIN_EXE_aicontext"));
    command.current_dir(root).args(args);
    if let Some(environment) = environment {
        command.env_clear();
        for (name, value) in environment {
            command.env(name, value);
        }
    }
    let output = command.output().expect("the binary runs");
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

#[test]
fn status_reports_the_project_branch_phase_current_task_and_pending_tasks() {
    let repo = TempRepository::new("ledger").expect("a repository");
    repo.write(".ai/TASKS.md", REGISTER).expect("writes");
    repo.commit_all("add the register").expect("commits");

    let run = run_hermetic(&repo, &["status"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let out = &run.stdout;
    for expected in [
        "ledger",
        "branch main",
        "Phase 1 — Context MVP",
        "TASK-013",
        "Implement aicontext status",
        "1 pending",
        "1 done",
        "clean",
    ] {
        assert!(out.contains(expected), "missing {expected:?} in:\n{out}");
    }
}

#[test]
fn status_json_exposes_the_current_task_id_for_scripts() {
    // `docs/CLI_SPEC.md` §8 names this exact path: `jq -r '.data.current_task.id'`.
    let repo = TempRepository::new("ledger").expect("a repository");
    repo.write(".ai/TASKS.md", REGISTER).expect("writes");

    let run = run_hermetic(&repo, &["status", "--json"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let body = envelope(&run);
    assert_eq!(body["command"], "status");
    assert_eq!(body["ok"], true);
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["data"]["project"], "ledger");
    assert_eq!(body["data"]["vcs"]["kind"], "git");
    assert_eq!(body["data"]["vcs"]["branch"], "main");
    assert_eq!(body["data"]["current_phase"], "Phase 1 — Context MVP");
    assert_eq!(body["data"]["current_task"]["id"], "TASK-013");
    assert_eq!(body["data"]["current_task"]["status"], "TODO");
    assert_eq!(body["data"]["current_task"]["priority"], "HIGH");
    assert_eq!(body["data"]["tasks"]["total"], 2);
    assert_eq!(body["data"]["tasks"]["pending"], 1);
    assert_eq!(body["data"]["tasks"]["done"], 1);
    assert_eq!(body["data"]["tasks"]["pending_ids"][0], "TASK-013");
}

#[test]
fn status_degrades_gracefully_outside_a_repository() {
    let project = TempProject::new("plain").expect("a project");
    project
        .write(".ai/TASKS.md", REGISTER)
        .expect("the register fixture");

    let run = run_in(project.path(), &["status"]);

    assert_eq!(run.code, 0, "outside a repository is not a failure");
    assert!(
        run.stdout.contains("no repository"),
        "the report must say there is no repository: {}",
        run.stdout
    );
    assert!(
        run.stdout.contains("TASK-013"),
        "the register is still reported outside a repository: {}",
        run.stdout
    );

    let body = envelope(&run_in(project.path(), &["status", "--json"]));
    assert_eq!(body["data"]["vcs"]["kind"], "none");
    assert_eq!(body["data"]["current_task"]["id"], "TASK-013");
}

#[test]
fn status_reports_modified_and_untracked_files() {
    let repo = TempRepository::new("changes").expect("a repository");
    repo.write(".ai/TASKS.md", REGISTER).expect("writes");
    repo.commit_all("add the register").expect("commits");
    repo.write("README.md", "# Fixture repository, edited\n")
        .expect("edits a tracked file");
    repo.write("scratch.txt", "new\n").expect("adds a file");

    let run = run_hermetic(&repo, &["status"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    for expected in ["README.md", "modified", "scratch.txt", "untracked"] {
        assert!(
            run.stdout.contains(expected),
            "missing {expected:?} in:\n{}",
            run.stdout
        );
    }

    let body = envelope(&run_hermetic(&repo, &["status", "--json"]));
    let changes = body["data"]["vcs"]["changes"]
        .as_array()
        .expect("a list of changes");
    assert_eq!(changes.len(), 2, "two files changed: {changes:?}");
    assert!(
        changes
            .iter()
            .any(|change| change["path"] == "scratch.txt" && change["state"] == "untracked"),
        "{changes:?}"
    );
}

#[test]
fn status_reports_a_branch_in_a_repository_with_no_commits() {
    // A fresh `git init` is a normal state, not an error: the branch has a name and no head commit.
    let repo = TempRepository::init_empty("fresh").expect("a repository");
    repo.write(".ai/TASKS.md", REGISTER).expect("writes");

    let run = run_hermetic(&repo, &["status", "--json"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let body = envelope(&run);
    assert_eq!(body["data"]["vcs"]["kind"], "git");
    assert_eq!(body["data"]["vcs"]["branch"], "main");
    assert!(
        body["data"]["vcs"]["head"].is_null(),
        "there is no commit to name yet: {body}"
    );
}

#[test]
fn status_points_at_the_next_command_and_shows_its_exit_code() {
    let repo = TempRepository::new("ledger").expect("a repository");
    repo.write(".ai/TASKS.md", REGISTER).expect("writes");

    let run = run_hermetic(&repo, &["status"]);

    let out = &run.stdout;
    assert!(out.contains("exit 0"), "{out}");
    assert!(out.contains("aicontext doctor"), "{out}");
}

#[test]
fn a_root_that_cannot_be_used_is_a_usage_error() {
    let outer = TempDir::new().expect("temp dir");

    // Run from the existing parent and point `--cwd` at a sibling that is not there: the binary has
    // to start in order to refuse, which running with a missing working directory could not do.
    let run = run_in(outer.path(), &["status", "--cwd", "does-not-exist"]);

    assert_eq!(run.code, 2, "an unusable root is a usage failure");
    assert!(run.stderr.contains("PROJ-001"), "{}", run.stderr);
}
