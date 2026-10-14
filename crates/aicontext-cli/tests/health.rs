//! `aicontext health` end to end, through the real binary.
//!
//! `docs/CONTEXT_SPEC.md` §9 defines the metrics and `docs/CLI_SPEC.md` §4 the command; these tests
//! assert the contract a developer relies on: every metric is a count with the command that
//! reproduces it, a count that cannot be computed says so instead of reporting zero, and the run
//! writes nothing.
//!
//! The fixture is a real skeleton written by `aicontext init`, so the expected-document count and the
//! register breakdown are the ones a fresh project actually has, not a hand-written approximation.

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

/// A skeleton written by `init`, which is what a real project starts with.
fn skeleton(name: &str) -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().join(name);
    fs::create_dir_all(&root).expect("project root");
    let run = run_in(&root, &["init", "--template", "default"]);
    assert_eq!(run.code, 0, "init failed; stderr was: {}", run.stderr);
    (outer, root)
}

/// Writes a file, creating its parent directory.
fn write(root: &Path, relative: &str, text: &str) {
    let full = root.join(relative);
    fs::create_dir_all(full.parent().expect("a parent")).expect("parent directories");
    fs::write(&full, text).expect("the file is written");
}

/// Every file under `root` with its bytes, for proving what a run wrote.
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

/// Parses stdout as the one JSON object the envelope contract promises.
fn json(run: &Run) -> serde_json::Value {
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "stdout is one JSON object ({error}); it was: {}",
            run.stdout
        )
    })
}

/// The metric with `id`, from a parsed envelope.
fn metric<'a>(body: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    body["data"]["metrics"]
        .as_array()
        .expect("metrics is an array")
        .iter()
        .find(|metric| metric["id"] == id)
        .unwrap_or_else(|| panic!("the {id} metric is reported"))
}

/// Runs git in `root` with a fixed identity, so a commit is reproducible.
fn git(root: &Path, args: &[&str], date: Option<&str>) {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "aicontext tests")
        .env("GIT_AUTHOR_EMAIL", "tests@example.com")
        .env("GIT_COMMITTER_NAME", "aicontext tests")
        .env("GIT_COMMITTER_EMAIL", "tests@example.com");
    if let Some(date) = date {
        command
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
    }
    let output = command.output().expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn json_output_is_the_health_envelope() {
    let (_outer, root) = skeleton("payments-ledger");
    let run = run_in(&root, &["health", "--json"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let body = json(&run);
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["command"], "health");
    assert_eq!(body["ok"], true);
    assert_eq!(body["exit_code"], 0);

    let metrics = body["data"]["metrics"]
        .as_array()
        .expect("metrics is an array");
    assert_eq!(
        metrics.len(),
        6,
        "the six implemented §9 metrics: {metrics:?}"
    );
    for id in [
        "context-completeness",
        "architecture-consistency",
        "task-consistency",
        "decision-coverage",
        "broken-references",
        "documentation-freshness",
    ] {
        let entry = metric(&body, id);
        assert!(
            entry["reproduce"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "{id} names the command that reproduces it: {entry}"
        );
    }
    assert!(
        body["summary"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "the envelope carries a one-line summary"
    );
}

#[test]
fn a_skeleton_reports_every_metric_in_the_human_view() {
    let (_outer, root) = skeleton("payments-ledger");
    let run = run_in(&root, &["health"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    for name in [
        "context completeness",
        "architecture consistency",
        "task consistency",
        "decision coverage",
        "broken references",
        "documentation freshness",
    ] {
        assert!(run.stdout.contains(name), "the {name} metric is shown");
    }
    assert!(
        run.stdout.contains("reproduce:"),
        "every metric names its command: {}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("score"),
        "health publishes no composite score: {}",
        run.stdout
    );
}

#[test]
fn completeness_counts_the_documents_a_skeleton_omits() {
    let (_outer, root) = skeleton("payments-ledger");
    let before = json(&run_in(&root, &["health", "--json"]));
    let full = metric(&before, "context-completeness");
    assert_eq!(
        full["count"], full["total"],
        "a fresh skeleton has every expected document: {full}"
    );

    fs::remove_file(root.join(".ai/DESIGN.md")).expect("the design document is removed");
    let after = json(&run_in(&root, &["health", "--json"]));
    let short = metric(&after, "context-completeness");
    assert_eq!(short["total"], full["total"]);
    assert_eq!(
        short["count"].as_u64(),
        full["count"].as_u64().map(|count| count - 1),
        "one document fewer is counted"
    );
    assert!(
        short["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|note| note.as_str() == Some("missing: .ai/DESIGN.md")),
        "the missing document is named: {short}"
    );
}

#[test]
fn a_missing_entry_point_exits_three() {
    let (_outer, root) = skeleton("payments-ledger");
    fs::remove_file(root.join(".ai/AI.md")).expect("the entry point is removed");

    let run = run_in(&root, &["health", "--json"]);
    assert_eq!(run.code, 3, "stderr was: {}", run.stderr);

    let body = json(&run);
    assert_eq!(body["ok"], false);
    assert_eq!(body["exit_code"], 3);
    assert!(
        body["findings"]
            .as_array()
            .expect("findings")
            .iter()
            .any(|finding| finding["code"] == "CTX-001"),
        "the finding behind the exit code is published: {}",
        body["findings"]
    );
}

#[test]
fn broken_references_are_counted_by_check_code() {
    let (_outer, root) = skeleton("payments-ledger");
    write(
        &root,
        ".ai/decisions/ADR-100-broken-link.md",
        "---\nid: ADR-100\ntype: decision\ntitle: Broken link\nstatus: accepted\n\
         date: 2026-01-01\ndeciders: tests\nsupersedes: ADR-404\nsuperseded_by: null\n---\n\n\
         # ADR-100\n\nSupersedes a decision that was never recorded.\n",
    );

    let run = run_in(&root, &["health", "--json"]);
    assert_eq!(
        run.code, 3,
        "a broken reference is an error-level finding; stderr: {}",
        run.stderr
    );

    let body = json(&run);
    let entry = metric(&body, "broken-references");
    assert_eq!(entry["count"], 1, "{entry}");
    assert!(
        entry["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|note| note.as_str() == Some("CTX-007: 1")),
        "the count is broken down by check code: {entry}"
    );
}

#[test]
fn decision_coverage_counts_accepted_records() {
    let (_outer, root) = skeleton("payments-ledger");
    for (name, id, status) in [
        ("accepted", "ADR-201", "accepted"),
        ("proposed", "ADR-202", "proposed"),
    ] {
        write(
            &root,
            &format!(".ai/decisions/{id}-{name}.md"),
            &format!(
                "---\nid: {id}\ntype: decision\ntitle: {name}\nstatus: {status}\n\
                 date: 2026-01-01\ndeciders: tests\nsupersedes: null\nsuperseded_by: null\n---\n\n\
                 # {id}\n\nA decision.\n"
            ),
        );
    }

    let body = json(&run_in(&root, &["health", "--json"]));
    let entry = metric(&body, "decision-coverage");
    assert_eq!(entry["count"], 1, "one accepted of two: {entry}");
    assert_eq!(entry["total"], 2, "{entry}");
    assert!(
        entry["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|note| note.as_str() == Some("proposed: 1")),
        "the status breakdown is shown: {entry}"
    );
}

#[test]
fn documentation_freshness_compares_updated_against_the_git_log() {
    let (_outer, root) = skeleton("payments-ledger");
    write(
        &root,
        ".ai/specs/ledger.md",
        "---\nid: SPEC-ledger\ntype: spec\ntitle: Ledger\nupdated: 2020-01-01\n\
         touches: [\"crates/ledger/**\"]\n---\n\n# Ledger\n\nThe ledger spec.\n",
    );
    write(
        &root,
        "crates/ledger/src/lib.rs",
        "// the code the spec describes\n",
    );

    git(&root, &["init", "-q"], None);
    git(&root, &["add", "."], None);
    git(
        &root,
        &["commit", "-q", "-m", "add the ledger"],
        Some("2026-06-01T00:00:00"),
    );

    let run = run_in(&root, &["health", "--json"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let body = json(&run);
    let entry = metric(&body, "documentation-freshness");
    assert_eq!(
        entry["count"], 1,
        "the spec is older than the code: {entry}"
    );
    assert_eq!(
        entry["total"], 1,
        "one document declares `updated` and `touches`"
    );
    assert!(
        entry["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .any(|note| note.as_str() == Some("stale: .ai/specs/ledger.md")),
        "the stale document is named: {entry}"
    );
}

#[test]
fn health_writes_nothing() {
    let (_outer, root) = skeleton("payments-ledger");
    let before = snapshot(&root);

    let run = run_in(&root, &["health"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    assert_eq!(
        snapshot(&root),
        before,
        "health is read-only; it must not even write the index cache"
    );
}

#[test]
fn a_missing_root_is_reported_as_a_usage_error() {
    let outer = TempDir::new().expect("temp dir");
    let missing = outer.path().join("not-here");
    let run = run_in(
        outer.path(),
        &["health", "--cwd", missing.to_str().expect("utf-8")],
    );

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(run.stderr.contains("PROJ-001"), "{}", run.stderr);
}

#[test]
fn the_same_tree_produces_a_byte_identical_report() {
    let (_outer, root) = skeleton("payments-ledger");
    let first = run_in(&root, &["health", "--json"]);
    let second = run_in(&root, &["health", "--json"]);
    assert_eq!(first.code, 0, "stderr was: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr was: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout);
}
