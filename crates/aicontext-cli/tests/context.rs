//! `aicontext context` end to end, through the real binary.
//!
//! These run the compiled binary rather than calling into the crate, because the contract
//! `docs/CLI_SPEC.md` §4 states is about the process a developer invokes: the words on stdout, the
//! code it exits with, and the files it leaves behind. A unit test can agree with itself and still be
//! wrong about all three.
//!
//! The fixture is a hand-written `.ai/` tree so the packet's contents are known exactly: the
//! always-on entry point and rules, a spec a task links, and a change note that contradicts the
//! rules on `store`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use tempfile::TempDir;

/// The agent entry point, tier 1.
const AI: &str = "---\ntype: agent-entry-point\ntitle: AI entry point\n---\n\n# AI\n\nStart here.\n";

/// Project policy, tier 2.
const RULES: &str = "---\nid: RULES-001\ntype: rules\ntitle: Rules\nstore: postgres\n---\n\n\
    # RULES\n\nNever commit without approval.\n";

/// A specification, tier 4.
const SPEC: &str = "---\nid: SPEC-context\ntype: spec\ntitle: Context spec\n---\n\n\
    # Context\n\nThe retrieval budget degrades from the lowest tier upward, so policy is never cut.\n";

/// A change note that contradicts the rules on `store`, tier 9.
const CHANGE: &str = "---\nid: CHG-001\ntype: change\ntitle: Change one\nstore: sqlite\n---\n\n\
    # Change one\n\nA history note long enough to be a real candidate on the lowest rung.\n";

/// The task register, tier 5, with one task that links the spec and the change.
const TASKS: &str = "---\nid: TASKS-001\ntype: tasks\ntitle: Tasks\nstatus: active\n---\n\n\
    **Current phase:** Phase 1\n**Current task:** TASK-001\n\n\
    ### TASK-001 — One\n\n```yaml\nid: TASK-001\ntitle: One\nstatus: TODO\nphase: 1\n\
    spec: .ai/specs/context.md\nrelates_to: [CHG-001]\ntouches: [\".ai/specs/*.md\"]\n```\n";

/// One invocation's observable result.
struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

/// A temporary project with a name a developer would recognise.
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

/// Reads a file, so an assertion can prove it was left untouched.
fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).expect("reads")
}

/// A project holding the fixture `.ai/` tree.
fn fixture(name: &str) -> (TempDir, PathBuf) {
    let (outer, root) = project(name);
    for (relative, contents) in [
        (".ai/AI.md", AI),
        (".ai/RULES.md", RULES),
        (".ai/specs/context.md", SPEC),
        (".ai/changes/CHG-001.md", CHANGE),
        (".ai/TASKS.md", TASKS),
    ] {
        write(&root, relative, contents);
    }
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

/// Parses stdout as the one JSON object the envelope contract promises.
fn json(run: &Run) -> serde_json::Value {
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "stdout is one JSON object ({error}); it was: {}",
            run.stdout
        )
    })
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

#[test]
fn json_output_is_the_context_packet_envelope() {
    // `docs/CONTEXT_SPEC.md` §10 requires `context --json` to be snapshotted so an accidental shape
    // change fails: the keys and the values the packet contract names are asserted here.
    let (_outer, root) = fixture("payments-ledger");
    let run = run_in(&root, &["context", "--task", "TASK-001", "--json"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let body = json(&run);
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["command"], "context");
    assert_eq!(body["ok"], true);
    assert_eq!(body["exit_code"], 0);

    let data = &body["data"];
    assert_eq!(data["version"], 1);
    assert_eq!(data["request"]["task"], "TASK-001");

    let policy = data["policy"].as_array().expect("policy is an array");
    assert!(
        policy
            .iter()
            .any(|item| item["id"] == ".ai/AI.md" && item["tier"] == 1),
        "the entry point is policy: {policy:?}"
    );
    assert!(
        policy
            .iter()
            .any(|item| item["id"] == "RULES-001" && item["tier"] == 2),
        "the rules are policy: {policy:?}"
    );

    let retrieved = data["retrieved"].as_array().expect("retrieved is an array");
    let spec = retrieved
        .iter()
        .find(|item| item["id"] == "SPEC-context")
        .expect("the linked spec is retrieved");
    assert_eq!(spec["tier"], 4);
    assert_eq!(spec["trust"], "untrusted");
    assert_eq!(spec["content_ref"], ".ai/specs/context.md");

    assert_eq!(data["budget"]["requested"], 32000);
    assert_eq!(data["conflicts"][0]["key"], "store");
    assert!(
        data["provenance"]
            .as_array()
            .expect("provenance is an array")
            .iter()
            .any(|trace| trace["id"] == "SPEC-context" && trace["included"] == true),
        "every included document is traced"
    );
    assert!(
        data["untrusted_content_ids"]
            .as_array()
            .expect("the untrusted ids are an array")
            .iter()
            .any(|id| id == "SPEC-context"),
        "the packet names what a caller must treat as data: {data:?}"
    );
}

#[test]
fn format_json_is_the_same_envelope_as_the_global_flag() {
    let (_outer, root) = fixture("payments-ledger");
    let global = run_in(&root, &["context", "--task", "TASK-001", "--json"]);
    let alias = run_in(&root, &["context", "--task", "TASK-001", "--format", "json"]);

    assert_eq!(alias.code, 0, "stderr was: {}", alias.stderr);
    assert_eq!(
        alias.stdout, global.stdout,
        "`--format json` is an alias for the global `--json`"
    );
}

#[test]
fn format_prompt_puts_retrieved_content_in_an_untrusted_envelope() {
    let (_outer, root) = fixture("payments-ledger");
    let run = run_in(&root, &["context", "--task", "TASK-001", "--format", "prompt"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    assert!(
        run.stdout.contains("# policy (trusted, tiers 1-2)"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("----- BEGIN UNTRUSTED DATA | id=SPEC-context"),
        "{}",
        run.stdout
    );
    assert!(
        run.stdout
            .contains("----- END UNTRUSTED DATA | id=SPEC-context -----"),
        "{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("schema_version"),
        "prompt output is provider input, not the JSON envelope: {}",
        run.stdout
    );

    let begin = run
        .stdout
        .find("----- BEGIN UNTRUSTED DATA | id=SPEC-context")
        .expect("a begin marker");
    let body = run
        .stdout
        .find("The retrieval budget degrades")
        .expect("the spec body is rendered");
    assert!(begin < body, "the content must sit inside the envelope");
}

#[test]
fn a_budget_too_small_for_policy_exits_three_and_names_ctx_030() {
    let (_outer, root) = fixture("payments-ledger");
    let run = run_in(&root, &["context", "--task", "TASK-001", "--budget", "1"]);

    assert_eq!(
        run.code, 3,
        "stdout was: {}\nstderr was: {}",
        run.stdout, run.stderr
    );
    assert!(run.stderr.contains("CTX-030"), "{}", run.stderr);
    assert!(
        run.stdout.trim().is_empty(),
        "a failed assembly writes no packet: {}",
        run.stdout
    );
}

#[test]
fn context_adds_only_the_index_cache_and_rewrites_nothing() {
    let (_outer, root) = fixture("payments-ledger");
    let before = snapshot(&root);

    let run = run_in(&root, &["context", "--task", "TASK-001"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let mut after = snapshot(&root);
    let cache = after
        .iter()
        .position(|(name, _)| name == ".aicontext/index.json")
        .expect("the index cache is written");
    after.remove(cache);

    assert_eq!(
        after, before,
        "context may add only the rebuildable cache; it rewrote a document"
    );
    assert_eq!(read(&root, ".ai/TASKS.md"), TASKS);
    assert_eq!(read(&root, ".ai/specs/context.md"), SPEC);
}

#[test]
fn a_warm_run_is_under_a_second() {
    let (_outer, root) = fixture("payments-ledger");
    let cold = run_in(&root, &["context", "--task", "TASK-001"]);
    assert_eq!(cold.code, 0, "stderr was: {}", cold.stderr);

    let started = Instant::now();
    let warm = run_in(&root, &["context", "--task", "TASK-001"]);
    let elapsed = started.elapsed();

    assert_eq!(warm.code, 0, "stderr was: {}", warm.stderr);
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "a warm run must stay under the one-second target; it took {elapsed:?}"
    );
}

#[test]
fn excluding_policy_by_id_leaves_it_in_the_packet() {
    let (_outer, root) = fixture("payments-ledger");
    let run = run_in(
        &root,
        &[
            "context",
            "--task",
            "TASK-001",
            "--exclude",
            "RULES-001",
            "--json",
        ],
    );
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let body = json(&run);
    let policy = body["data"]["policy"].as_array().expect("policy is an array");
    assert!(
        policy.iter().any(|item| item["id"] == "RULES-001"),
        "the always-on policy cannot be excluded: {policy:?}"
    );
}

#[test]
fn excluding_a_retrieved_document_removes_it() {
    let (_outer, root) = fixture("payments-ledger");
    let run = run_in(
        &root,
        &[
            "context",
            "--task",
            "TASK-001",
            "--exclude",
            "SPEC-context",
            "--json",
        ],
    );
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let body = json(&run);
    let retrieved = body["data"]["retrieved"].as_array().expect("retrieved");
    assert!(
        retrieved.iter().all(|item| item["id"] != "SPEC-context"),
        "an excluded document is not retrieved: {retrieved:?}"
    );
}

#[test]
fn a_missing_root_is_reported_as_a_usage_error() {
    let outer = TempDir::new().expect("temp dir");
    let missing = outer.path().join("not-here");
    let run = run_in(
        outer.path(),
        &["context", "--cwd", missing.to_str().expect("utf-8")],
    );

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(run.stderr.contains("PROJ-001"), "{}", run.stderr);
}

#[test]
fn the_same_tree_produces_a_byte_identical_packet() {
    // Determinism end to end: the same tree and the same task produce byte-identical output, which is
    // the property a caller caching a packet relies on.
    let (_outer, root) = fixture("payments-ledger");
    let first = run_in(&root, &["context", "--task", "TASK-001", "--json"]);
    let second = run_in(&root, &["context", "--task", "TASK-001", "--json"]);
    assert_eq!(first.code, 0, "stderr was: {}", first.stderr);
    assert_eq!(second.code, 0, "stderr was: {}", second.stderr);
    assert_eq!(first.stdout, second.stdout);
}
