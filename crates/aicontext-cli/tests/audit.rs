//! `aicontext audit` end to end, through the real binary.
//!
//! `docs/SECURITY.md` §8 defines the record and `docs/CLI_SPEC.md` §4 the command. These tests
//! assert what a reviewer or a script relies on:
//!
//! - a missing log is an empty, valid chain, not an error;
//! - the three views read one verification, and `verify` publishes the head a reviewer can witness;
//! - a broken chain renders the verified prefix and exits 3, while an unreadable file exits 1;
//! - `--json` publishes the same shape for every view;
//! - reading never writes.
//!
//! Records are produced through the real `AuditLog::append_at` in a temporary project, so the bytes
//! under test are the bytes the library seals — not a hand-written fixture that could drift from it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aicontext_audit::{
    Action, Actor, Approval, AuditEntry, AuditLog, GENESIS, MAX_LOG_BYTES, ResultSummary, Timestamp,
};
use aicontext_core::PermissionMode;
use serde_json::{Value, json};
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

/// A temporary project directory. `audit` needs no `.ai/` tree: it reads one file.
fn project(name: &str) -> (TempDir, PathBuf) {
    let outer = TempDir::new().expect("temp dir");
    let root = outer.path().join(name);
    fs::create_dir_all(&root).expect("project root");
    (outer, root)
}

/// The project-relative path the command publishes.
const LOG: &str = ".aicontext/audit.jsonl";

/// The digest of the empty input, used as a well-formed argument digest.
const EMPTY_DIGEST: &str =
    "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// A complete record: every optional field present, so the views have something to render.
fn append(root: &Path, tool: &str, millis: i64) -> String {
    let entry = AuditEntry::new(
        "completed",
        Actor::agent("developer")
            .with_provider("openai-compatible")
            .with_model("test-model"),
        Action::new(tool, "aws", "write"),
    )
    .with_resource("i-0abc1234")
    .with_arguments_digest(EMPTY_DIGEST)
    .with_approval(
        Approval::new(PermissionMode::Approval)
            .with_granted_by("developer")
            .with_mode("interactive"),
    )
    .with_result(ResultSummary::new("success").with_duration_ms(412));
    AuditLog::in_project(root)
        .append_at(&entry, Timestamp::from_unix_millis(millis))
        .expect("the record is appended")
        .digest()
        .to_string()
}

/// Parses a `--json` run, which must be exactly one object.
fn body(run: &Run) -> Value {
    serde_json::from_str(&run.stdout).unwrap_or_else(|error| {
        panic!(
            "expected one JSON object, got {error}; stdout was: {}; stderr was: {}",
            run.stdout, run.stderr
        )
    })
}

/// The lines of the log as the file holds them.
fn lines(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join(LOG))
        .expect("the log exists")
        .lines()
        .map(str::to_string)
        .collect()
}

/// Replaces the lines of the log.
fn write_lines(root: &Path, lines: &[String]) {
    fs::write(root.join(LOG), format!("{}\n", lines.join("\n"))).expect("the log is written");
}

/// Changes one record's tool without resealing it, which breaks `self`.
fn tamper_tool(root: &Path, index: usize, tool: &str) {
    let mut all = lines(root);
    let mut value: Value = serde_json::from_str(&all[index]).expect("a printed record parses");
    value["action"]["tool"] = Value::String(tool.to_string());
    all[index] = serde_json::to_string(&value).expect("renders");
    write_lines(root, &all);
}

/// Every file under `root` with its bytes, for proving a command wrote nothing.
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .expect("a path under the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((relative, fs::read(&path).expect("the file is readable")));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn a_missing_log_is_an_empty_valid_chain_and_creates_nothing() {
    let (_outer, root) = project("audit-empty");
    let run = run_in(&root, &["audit", "show", "--json"]);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let body = body(&run);
    assert_eq!(body["command"], "audit");
    assert_eq!(body["ok"], true);
    assert_eq!(body["exit_code"], 0);
    assert_eq!(body["data"]["view"], "show");
    assert_eq!(body["data"]["log"], LOG);
    assert_eq!(body["data"]["chain_valid"], true);
    assert_eq!(body["data"]["verified_records"], 0);
    assert_eq!(body["data"]["returned_records"], 0);
    assert_eq!(body["data"]["head"], GENESIS);
    assert_eq!(body["data"]["limit"], Value::Null);
    assert_eq!(body["data"]["records"], json!([]));
    assert_eq!(body["data"]["problem"], Value::Null);
    assert_eq!(body["findings"], json!([]));
    assert!(
        !root.join(".aicontext").exists(),
        "reading must not create the directory"
    );
}

#[test]
fn a_valid_chain_renders_every_view_and_verify_publishes_the_head() {
    let (_outer, root) = project("audit-valid");
    let first = append(&root, "aws.ec2.describe", 0);
    let second = append(&root, "aws.ec2.stop", 1_000);
    let third = append(&root, "aws.ec2.start", 2_000);

    let show = run_in(&root, &["audit", "show", "--json"]);
    assert_eq!(show.code, 0, "stderr was: {}", show.stderr);
    let shown = body(&show);
    assert_eq!(shown["data"]["view"], "show");
    assert_eq!(shown["data"]["chain_valid"], true);
    assert_eq!(shown["data"]["verified_records"], 3);
    assert_eq!(shown["data"]["returned_records"], 3);
    assert_eq!(shown["data"]["head"], third);
    assert_eq!(shown["data"]["limit"], Value::Null);
    let records = shown["data"]["records"]
        .as_array()
        .expect("the records are an array");
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["seq"], 1);
    assert_eq!(records[1]["seq"], 2);
    assert_eq!(records[2]["seq"], 3);
    assert_eq!(records[0]["prev"], GENESIS);
    assert_eq!(records[1]["prev"], first);
    assert_eq!(records[2]["prev"], second);
    assert_eq!(records[2]["self"], third);
    assert_eq!(records[1]["ts"], "1970-01-01T00:00:01.000Z");
    assert_eq!(records[1]["action"]["tool"], "aws.ec2.stop");
    assert_eq!(records[1]["approval"]["required"], "approval");

    let verify = run_in(&root, &["audit", "verify", "--json"]);
    assert_eq!(verify.code, 0, "stderr was: {}", verify.stderr);
    let verified = body(&verify);
    assert_eq!(verified["data"]["view"], "verify");
    assert_eq!(verified["data"]["chain_valid"], true);
    assert_eq!(verified["data"]["verified_records"], 3);
    assert_eq!(verified["data"]["returned_records"], 0);
    assert_eq!(verified["data"]["head"], third);
    assert_eq!(verified["data"]["records"], json!([]));
    assert_eq!(verified["data"]["problem"], Value::Null);

    let tail = run_in(&root, &["audit", "tail", "--limit", "2", "--json"]);
    assert_eq!(tail.code, 0, "stderr was: {}", tail.stderr);
    let tailed = body(&tail);
    assert_eq!(tailed["data"]["view"], "tail");
    assert_eq!(tailed["data"]["chain_valid"], true);
    assert_eq!(tailed["data"]["verified_records"], 3);
    assert_eq!(tailed["data"]["returned_records"], 2);
    assert_eq!(tailed["data"]["limit"], 2);
    let newest = tailed["data"]["records"]
        .as_array()
        .expect("the records are an array");
    assert_eq!(newest.len(), 2);
    assert_eq!(newest[0]["seq"], 2);
    assert_eq!(newest[1]["seq"], 3);
    assert_eq!(newest[1]["self"], third);

    // Every view publishes the same keys, so a script reads one shape whatever it asked for.
    let keys = |value: &Value| {
        let mut keys: Vec<String> = value["data"]
            .as_object()
            .expect("the payload is an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&shown), keys(&verified));
    assert_eq!(keys(&shown), keys(&tailed));
}

#[test]
fn the_human_views_render_a_timeline_and_the_head() {
    let (_outer, root) = project("audit-human");
    let _first = append(&root, "aws.ec2.describe", 0);
    let _second = append(&root, "aws.ec2.stop", 1_000);
    let third = append(&root, "aws.ec2.start", 2_000);

    let show = run_in(&root, &["audit", "show"]);
    assert_eq!(show.code, 0, "stderr was: {}", show.stderr);
    for expected in [
        "1970-01-01T00:00:00.000Z",
        "1970-01-01T00:00:01.000Z",
        "aws.ec2.describe [aws]",
        "aws.ec2.stop [aws]",
        "on i-0abc1234",
        "by agent:developer",
        "approval approval by developer (interactive)",
        "result success in 412 ms",
        "3 records",
    ] {
        assert!(
            show.stdout.contains(expected),
            "missing `{expected}`:\n{}",
            show.stdout
        );
    }
    let first_row = show.stdout.find("#1").expect("the first row is numbered");
    let second_row = show.stdout.find("#2").expect("the second row is numbered");
    let third_row = show.stdout.find("#3").expect("the third row is numbered");
    assert!(
        first_row < second_row && second_row < third_row,
        "rows stay in chain order"
    );

    let verify = run_in(&root, &["audit", "verify"]);
    assert_eq!(verify.code, 0, "stderr was: {}", verify.stderr);
    assert!(verify.stdout.contains("valid"), "{}", verify.stdout);
    assert!(
        verify.stdout.contains(&third),
        "verify must publish the head a reviewer witnesses:\n{}",
        verify.stdout
    );
    assert!(
        !verify.stdout.contains("#1"),
        "verify is not a timeline:\n{}",
        verify.stdout
    );
}

#[test]
fn a_broken_chain_exits_three_and_renders_the_verified_prefix() {
    let (_outer, root) = project("audit-broken");
    let first = append(&root, "aws.ec2.describe", 0);
    let _second = append(&root, "aws.ec2.stop", 1_000);
    let _third = append(&root, "aws.ec2.start", 2_000);
    tamper_tool(&root, 1, "aws.ec2.terminate");

    let show = run_in(&root, &["audit", "show", "--json"]);
    assert_eq!(show.code, 3, "a broken chain is a validation failure");
    let shown = body(&show);
    assert_eq!(shown["ok"], false);
    assert_eq!(shown["exit_code"], 3);
    assert_eq!(shown["data"]["chain_valid"], false);
    assert_eq!(shown["data"]["verified_records"], 1);
    assert_eq!(shown["data"]["returned_records"], 1);
    assert_eq!(shown["data"]["head"], first);
    let records = shown["data"]["records"]
        .as_array()
        .expect("the verified prefix is an array");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["seq"], 1);
    let problem = &shown["data"]["problem"];
    assert_eq!(problem["code"], "AUD-003");
    assert_eq!(problem["line"], 2);
    assert!(
        problem["message"]
            .as_str()
            .expect("a message")
            .contains("self"),
        "the tampered record's own digest no longer matches: {problem}"
    );
    let finding = &shown["findings"][0];
    assert_eq!(finding["code"], "AUD-003");
    assert_eq!(finding["severity"], "error");
    assert_eq!(finding["path"], LOG);
    assert!(
        shown["summary"]
            .as_str()
            .expect("a summary")
            .contains("broken"),
        "the summary names the break: {}",
        shown["summary"]
    );

    let verify = run_in(&root, &["audit", "verify", "--json"]);
    assert_eq!(verify.code, 3);
    let verified = body(&verify);
    assert_eq!(verified["data"]["chain_valid"], false);
    assert_eq!(verified["data"]["verified_records"], 1);
    assert_eq!(verified["data"]["head"], first);
    assert_eq!(verified["data"]["records"], json!([]));

    let tail = run_in(&root, &["audit", "tail", "--limit", "1", "--json"]);
    assert_eq!(tail.code, 3);
    let tailed = body(&tail);
    assert_eq!(tailed["data"]["returned_records"], 1);
    assert_eq!(tailed["data"]["records"][0]["seq"], 1);

    let human = run_in(&root, &["audit", "show"]);
    assert_eq!(human.code, 3);
    assert!(
        human.stdout.contains("chain broken at line 2"),
        "the human view names the break:\n{}",
        human.stdout
    );
    assert!(
        human.stdout.contains("#1"),
        "the verified prefix is still rendered:\n{}",
        human.stdout
    );
    assert!(
        !human.stdout.contains("#2"),
        "the broken record is not rendered"
    );
}

#[test]
fn a_malformed_line_is_aud_002_and_keeps_the_records_before_it() {
    let (_outer, root) = project("audit-malformed");
    let first = append(&root, "aws.ec2.describe", 0);
    let mut all = lines(&root);
    all.push("{\"not\":\"a record\"}".to_string());
    write_lines(&root, &all);

    let run = run_in(&root, &["audit", "show", "--json"]);
    assert_eq!(run.code, 3);
    let body = body(&run);
    assert_eq!(body["data"]["chain_valid"], false);
    assert_eq!(body["data"]["verified_records"], 1);
    assert_eq!(body["data"]["head"], first);
    assert_eq!(body["data"]["problem"]["code"], "AUD-002");
    assert_eq!(body["data"]["problem"]["line"], 2);
    assert_eq!(body["data"]["records"][0]["seq"], 1);
}

#[test]
fn a_log_over_the_read_cap_is_aud_004() {
    let (_outer, root) = project("audit-large");
    fs::create_dir_all(root.join(".aicontext")).expect("the directory is created");
    let file = fs::File::create(root.join(LOG)).expect("the log is created");
    file.set_len(MAX_LOG_BYTES + 1).expect("the length is set");

    let run = run_in(&root, &["audit", "show", "--json"]);
    assert_eq!(run.code, 3);
    let body = body(&run);
    assert_eq!(body["data"]["chain_valid"], false);
    assert_eq!(body["data"]["verified_records"], 0);
    assert_eq!(body["data"]["head"], GENESIS);
    assert_eq!(body["data"]["problem"]["code"], "AUD-004");
}

#[test]
fn an_unreadable_log_is_a_general_failure_not_an_empty_chain() {
    let (_outer, root) = project("audit-unreadable");
    fs::create_dir_all(root.join(LOG)).expect("a directory stands where the log should be");

    let run = run_in(&root, &["audit", "show", "--json"]);
    assert_eq!(run.code, 1, "stderr was: {}", run.stderr);
    let body = body(&run);
    assert_eq!(body["exit_code"], 1);
    assert_eq!(body["data"]["chain_valid"], false);
    assert_eq!(body["data"]["problem"]["code"], "AUD-001");
}

#[test]
fn a_root_that_cannot_be_used_exits_two_before_anything_is_read() {
    let (_outer, root) = project("audit-root");
    let missing = root.join("does-not-exist");
    let run = run_in(
        &root,
        &[
            "audit",
            "show",
            "--cwd",
            &missing.to_string_lossy(),
            "--json",
        ],
    );
    assert_eq!(run.code, 2);
    assert!(
        run.stderr.contains("PROJ-001"),
        "the root failure names its code: {}",
        run.stderr
    );
    assert!(
        run.stdout.is_empty(),
        "a root failure happens before a report exists: {}",
        run.stdout
    );
}

#[test]
fn the_view_and_its_limit_are_usage_errors_when_missing_or_zero() {
    let (_outer, root) = project("audit-usage");
    for args in [
        vec!["audit"],
        vec!["audit", "tail", "--limit", "0"],
        vec!["audit", "tail", "--limit", "many"],
        vec!["audit", "publish"],
    ] {
        let run = run_in(&root, &args);
        assert_eq!(run.code, 2, "{args:?} must be a usage error");
        assert!(!run.stderr.is_empty(), "{args:?} must explain itself");
    }
}

#[test]
fn reading_the_log_never_writes_to_the_project() {
    let (_outer, root) = project("audit-read-only");
    append(&root, "aws.ec2.describe", 0);
    append(&root, "aws.ec2.stop", 1_000);
    let before = snapshot(&root);

    for args in [
        vec!["audit", "show"],
        vec!["audit", "verify"],
        vec!["audit", "tail", "--limit", "1"],
        vec!["audit", "show", "--json"],
    ] {
        let run = run_in(&root, &args);
        assert_eq!(run.code, 0, "{args:?} failed: {}", run.stderr);
    }

    assert_eq!(
        before,
        snapshot(&root),
        "an audit view is a read and must not change a byte"
    );
}
