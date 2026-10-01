//! `aicontext doctor` end to end, through the real binary.
//!
//! These run the compiled binary rather than calling into the crate, because the contract
//! `docs/CLI_SPEC.md` states is about the process a developer invokes: the words on stdout, the code
//! it exits with, and the fact that it writes nothing at all. A unit test can agree with itself and
//! still be wrong about all three.
//!
//! Two shapes of project are used. A freshly `init`-ed directory is clean by construction, which is
//! what makes it the right baseline for exit 0; a fixture written by hand with one thing broken in it
//! is what the findings are asserted against.

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

/// Writes a file, creating its parent directories.
fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("has a parent")).expect("creates");
    fs::write(path, contents).expect("writes");
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

/// A project `init` has just scaffolded, which is clean by construction.
fn initialised(name: &str) -> (TempDir, PathBuf) {
    let (outer, root) = project(name);
    let run = run_in(&root, &["init"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    (outer, root)
}

/// Parses stdout as the one JSON object the envelope contract promises.
fn json(run: &Run) -> serde_json::Value {
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|error| {
        panic!("stdout is one JSON object ({error}); it was: {}", run.stdout)
    })
}

#[test]
fn a_freshly_initialised_project_has_no_findings_and_exits_zero() {
    let (_outer, root) = initialised("payments-ledger");
    let run = run_in(&root, &["doctor"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(
        !run.stdout.contains("CTX-0"),
        "a scaffolded project must be clean: {}",
        run.stdout
    );
    assert!(run.stdout.contains("no findings"), "{}", run.stdout);
    assert!(run.stdout.contains("payments-ledger"), "{}", run.stdout);
}

#[test]
fn doctor_writes_nothing_at_all() {
    // The only command allowed to write is `init`. A check that repairs itself cannot be trusted to
    // report the damage, so this is asserted by comparing the whole tree rather than one file.
    let (_outer, root) = initialised("payments-ledger");
    let before = snapshot(&root);
    let run = run_in(&root, &["doctor", "--strict", "--rebuild-index"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert_eq!(snapshot(&root), before, "doctor changed the tree");
}

#[test]
fn a_missing_entry_point_is_an_error_and_exits_three() {
    let (_outer, root) = initialised("payments-ledger");
    fs::remove_file(root.join(".ai/AI.md")).expect("remove");

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 3, "stderr was: {}", run.stderr);
    assert!(run.stdout.contains("CTX-001"), "{}", run.stdout);
    assert!(run.stdout.contains(".ai/AI.md"), "{}", run.stdout);
    assert!(
        run.stdout.contains("init"),
        "the fix must be on screen: {}",
        run.stdout
    );
}

#[test]
fn a_document_without_front_matter_is_an_error_and_names_the_file() {
    let (_outer, root) = initialised("payments-ledger");
    write(&root, ".ai/CONVENTIONS.md", "# Conventions\n\nNothing declared.\n");

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 3);
    assert!(run.stdout.contains("CTX-002"), "{}", run.stdout);
    assert!(run.stdout.contains("CONVENTIONS.md"), "{}", run.stdout);
}

#[test]
fn a_front_matter_document_missing_a_required_key_names_the_key() {
    let (_outer, root) = initialised("payments-ledger");
    // `title` is the key left out. §2 rule 1 requires `id`, `type`, and `title` of every document at a
    // catalogue location, so the report has to say which one is absent rather than "invalid".
    write(
        &root,
        ".ai/CONVENTIONS.md",
        "---\nid: CONV-001\ntype: conventions\n---\n\n# Conventions\n",
    );

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 3, "stderr was: {}", run.stderr);
    assert!(run.stdout.contains("CTX-002"), "{}", run.stdout);
    assert!(run.stdout.contains("title"), "{}", run.stdout);
}

#[test]
fn a_free_form_note_needs_no_front_matter() {
    // `.ai/AI.md` and anything outside the catalogue are prose. Requiring structure of a note would
    // be inventing a rule the spec does not have.
    let (_outer, root) = initialised("payments-ledger");
    write(&root, ".ai/notes/scratch.md", "# Scratch\n\nNot an entity.\n");

    let run = run_in(&root, &["doctor"]);
    assert_eq!(
        run.code, 0,
        "a free-form note is not a finding: {}",
        run.stdout
    );
    assert!(!run.stdout.contains("CTX-002"), "{}", run.stdout);
}

#[test]
fn a_reference_to_a_task_that_does_not_exist_is_an_error() {
    let (_outer, root) = initialised("payments-ledger");
    // A task block appended to the register, declaring a dependency on a task nobody created.
    let extra = "### TASK-900 — *ghost dependency*\n\n```yaml\nid: TASK-900\ntitle: \"Ghost dependency\"\nstatus: TODO\npriority: HIGH\nphase: 1\ndepends_on:\n  - TASK-901\ntouches: []\nacceptance:\n  - \"never happens\"\n```\n";
    let tasks = read(&root, ".ai/TASKS.md");
    write(&root, ".ai/TASKS.md", &format!("{tasks}\n{extra}"));

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 3, "stderr was: {}", run.stderr);
    assert!(run.stdout.contains("CTX-007"), "{}", run.stdout);
    assert!(run.stdout.contains("TASK-901"), "{}", run.stdout);
}

#[test]
fn a_rewritten_schema_is_a_warning_and_leaves_the_exit_code_at_zero() {
    let (_outer, root) = initialised("payments-ledger");
    let schema = read(&root, ".ai/schemas/task.schema.json");
    write(
        &root,
        ".ai/schemas/task.schema.json",
        &format!("{schema}\n"),
    );

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 0, "a warning is not a failure");
    assert!(run.stdout.contains("CTX-012"), "{}", run.stdout);
}

#[test]
fn strict_promotes_a_warning_to_an_error_and_therefore_exits_three() {
    let (_outer, root) = initialised("payments-ledger");
    let schema = read(&root, ".ai/schemas/task.schema.json");
    write(
        &root,
        ".ai/schemas/task.schema.json",
        &format!("{schema}\n"),
    );

    let run = run_in(&root, &["doctor", "--strict"]);
    assert_eq!(run.code, 3, "stderr was: {}", run.stderr);
    assert!(run.stdout.contains("CTX-012"), "{}", run.stdout);
}

#[test]
fn a_declared_language_discovery_cannot_see_is_a_warning() {
    let (_outer, root) = initialised("payments-ledger");
    // `Cargo.toml` makes discovery observe rust; the document then claims go, which it cannot. `go` is
    // in the vocabulary the check compares, which is what makes the mismatch reportable at all.
    write(
        &root,
        "Cargo.toml",
        "[package]\nname = \"ledger\"\nversion = \"0.1.0\"\n",
    );
    write(
        &root,
        ".ai/ARCHITECTURE.md",
        "---\nid: ARCH-001\ntype: architecture\ntitle: Architecture\nstatus: active\nstack:\n  - go\n---\n\n# ARCHITECTURE\n",
    );

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 0, "a mismatch is a warning: {}", run.stdout);
    assert!(run.stdout.contains("CTX-013"), "{}", run.stdout);
    assert!(run.stdout.contains("go"), "{}", run.stdout);
    assert!(run.stdout.contains("rust"), "what was observed: {}", run.stdout);
}

#[test]
fn a_deprecated_decision_with_no_successor_is_a_warning() {
    let (_outer, root) = initialised("payments-ledger");
    // `superseded_by: null` is an explicit "nothing replaced this", which §2 rule 9 says to read as
    // no successor rather than as a reference that failed to resolve.
    write(
        &root,
        ".ai/decisions/ADR-001-old.md",
        "---\nid: ADR-001\ntype: decision\ntitle: Old choice\nstatus: deprecated\nsuperseded_by: null\n---\n\n# Old choice\n\nKept for the record.\n",
    );

    let run = run_in(&root, &["doctor"]);
    assert_eq!(run.code, 0, "a warning does not fail the run: {}", run.stdout);
    assert!(run.stdout.contains("CTX-014"), "{}", run.stdout);
    assert!(run.stdout.contains("ADR-001"), "{}", run.stdout);
}

#[test]
fn only_narrows_the_run_to_the_codes_that_were_asked_for() {
    let (_outer, root) = initialised("payments-ledger");
    fs::remove_file(root.join(".ai/AI.md")).expect("remove");
    write(&root, ".ai/CONVENTIONS.md", "# Conventions\n\nNothing declared.\n");

    let run = run_in(&root, &["doctor", "--only", "CTX-001"]);
    assert!(run.stdout.contains("CTX-001"), "{}", run.stdout);
    assert!(
        !run.stdout.contains("CTX-002"),
        "the filter must narrow the run: {}",
        run.stdout
    );
    assert_eq!(run.code, 3, "CTX-001 is still an error");
}

#[test]
fn rebuild_index_is_accepted_and_says_out_loud_that_it_did_nothing() {
    let (_outer, root) = initialised("payments-ledger");
    let run = run_in(&root, &["doctor", "--rebuild-index"]);

    assert_eq!(run.code, 0);
    assert!(
        run.stderr.contains("rebuild-index") && run.stderr.contains("TASK-031"),
        "a flag that does nothing must say so: {}",
        run.stderr
    );
}

#[test]
fn explaining_a_code_needs_no_project_at_all() {
    // Asked before anything reads the filesystem, so the answer does not depend on the directory the
    // developer happens to be standing in.
    let (_outer, root) = project("no-project-here");
    let run = run_in(&root, &["doctor", "--explain", "CTX-007"]);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(run.stdout.contains("CTX-007"), "{}", run.stdout);
    assert!(run.stdout.contains("does not exist"), "{}", run.stdout);
}

#[test]
fn explaining_a_deferred_code_lists_the_checks_that_do_run() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["doctor", "--explain", "CTX-010"]);

    assert_eq!(run.code, 0, "a deferred code is a real question");
    assert!(run.stdout.contains("CTX-001"), "{}", run.stdout);
}

#[test]
fn explaining_a_code_that_is_not_a_check_is_a_usage_error() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["doctor", "--explain", "CTX-999"]);

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(run.stderr.contains("CTX-999"), "{}", run.stderr);
}

#[test]
fn json_is_one_object_whose_exit_code_matches_the_process() {
    let (_outer, root) = initialised("payments-ledger");
    fs::remove_file(root.join(".ai/AI.md")).expect("remove");

    let run = run_in(&root, &["doctor", "--json"]);
    assert_eq!(run.code, 3);
    let body = json(&run);

    assert_eq!(body["command"], "doctor");
    assert_eq!(body["exit_code"], 3);
    assert_eq!(body["ok"], false);
    let root_path = json(&run)["data"]["root"]
        .as_str()
        .expect("the root is a string")
        .to_string();
    assert!(root_path.ends_with("payments-ledger"), "{root_path}");
    assert!(
        !root_path.starts_with("//?/"),
        "a reported path is spelled the way a person would type it: {root_path}"
    );

    let findings = body["findings"].as_array().expect("findings array");
    let codes: Vec<&str> = findings
        .iter()
        .map(|finding| finding["code"].as_str().expect("a code"))
        .collect();
    assert!(codes.contains(&"CTX-001"), "{codes:?}");
    assert_eq!(findings[0]["severity"], "error");
    assert_eq!(findings[0]["path"], ".ai/AI.md");
    assert!(
        !findings[0]["remediation"].as_str().unwrap_or_default().is_empty(),
        "every finding carries its fix"
    );
}

#[test]
fn json_explanation_is_a_success_envelope_carrying_the_severity() {
    let (_outer, root) = project("ledger");
    let run = run_in(&root, &["doctor", "--explain", "CTX-012", "--strict", "--json"]);

    assert_eq!(run.code, 0);
    let body = json(&run);
    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["code"], "CTX-012");
    assert_eq!(body["data"]["implemented"], true);
    assert_eq!(body["data"]["severity"], "error");
}

#[test]
fn json_names_the_checks_that_ran_and_the_ones_that_are_deferred() {
    let (_outer, root) = initialised("payments-ledger");
    let run = run_in(&root, &["doctor", "--json"]);

    assert_eq!(run.code, 0);
    let body = json(&run);
    let checks: Vec<&str> = body["data"]["checks"]
        .as_array()
        .expect("checks array")
        .iter()
        .map(|code| code.as_str().expect("a code"))
        .collect();
    assert!(checks.contains(&"CTX-001"), "{checks:?}");
    assert!(checks.contains(&"CTX-018"), "{checks:?}");

    let deferred = body["data"]["unimplemented"].as_array().expect("an array");
    assert!(
        deferred.iter().any(|code| code == "CTX-003"),
        "runtime validation is still deferred"
    );
}

#[test]
fn json_rebuild_index_records_that_nothing_was_rebuilt() {
    let (_outer, root) = initialised("payments-ledger");
    let run = run_in(&root, &["doctor", "--rebuild-index", "--json"]);
    let body = json(&run);

    assert_eq!(body["data"]["rebuild_index"]["requested"], true);
    assert_eq!(body["data"]["rebuild_index"]["performed"], false);
    assert!(
        body["data"]["rebuild_index"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("TASK-031"),
        "the reason is in the payload as well as on stderr"
    );
}

#[test]
fn json_leaves_the_human_report_off_stdout() {
    let (_outer, root) = initialised("payments-ledger");
    fs::remove_file(root.join(".ai/AI.md")).expect("remove");
    let run = run_in(&root, &["doctor", "--json"]);

    assert_eq!(run.code, 3);
    assert!(
        !run.stdout.contains("fix:"),
        "stdout is the envelope alone: {}",
        run.stdout
    );
    assert!(
        run.stdout.trim().starts_with('{') && run.stdout.trim().ends_with('}'),
        "one object, no commentary: {}",
        run.stdout
    );
}

#[test]
fn a_root_that_does_not_exist_is_a_usage_failure() {
    let (_outer, root) = project("ledger");
    let missing = root.join("nowhere");
    let run = run_in(&root, &["doctor", "--cwd", missing.to_str().expect("utf-8")]);

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(run.stderr.contains("PROJ-001"), "{}", run.stderr);
}

#[test]
fn an_oversized_document_is_reported_rather_than_read() {
    let (_outer, root) = initialised("payments-ledger");
    let huge = format!(
        "---\nkind: context-note\nid: NOTE-001\ntitle: Big\nstatus: active\n---\n\n{}\n",
        "# Padding\n\n".repeat(140_000)
    );
    assert!(huge.len() > 1024 * 1024, "the fixture must exceed 1 MiB");
    write(&root, ".ai/notes/huge.md", &huge);

    let run = run_in(&root, &["doctor"]);
    assert!(run.stdout.contains("CTX-018"), "{}", run.stdout);
    assert_eq!(
        run.code, 0,
        "too large is a warning, not a failure"
    );
}

/// Reads a file, or fails with the path so the assertion says what was missing.
fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap_or_else(|error| panic!("reading {path}: {error}"))
}

/// Every file below `root`, with its contents, so "wrote nothing" is checked rather than assumed.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let (Ok(name), Ok(bytes)) = (path.strip_prefix(root), fs::read(&path)) {
                files.push((name.to_string_lossy().replace('\\', "/"), bytes));
            }
        }
    }
    files.sort();
    files
}