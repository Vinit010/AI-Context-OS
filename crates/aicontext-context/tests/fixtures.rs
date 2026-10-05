//! Proves the shared fixtures are what they claim to be.
//!
//! `aicontext-testkit` cannot check this itself: it may not depend on `aicontext-context`
//! (`ARCHITECTURE.md` §3.2), so the crate that owns the rules is the only one that can prove a
//! fixture satisfies them. Without this file, `sample_ai_tree()` could drift — a new rule, or a
//! changed one — and every test that used it would quietly inherit the new finding. A fixture that
//! is wrong is worse than no fixture, because it looks like a clean baseline.
//!
//! The guarantees checked here are the ones a test *relies* on rather than the ones the fixture
//! documents:
//!
//! 1. `sample_ai_tree` produces a tree `doctor` reports no findings for.
//! 2. It stays that way, with the `checked` list showing what was examined rather than an empty tree
//!    being mistaken for a passing run over a real one.
//! 3. Breaking one thing produces exactly the finding the fixture is used to test against, which is
//!    what proves the first test is not vacuous.
//! 4. `TempRepository` is a repository whose history and status mean what they say, on every machine.

use aicontext_context::doctor::{self, Inputs};
use aicontext_testkit::{TempProject, TempRepository, sample_ai_tree};

/// The schema source set used for the run.
///
/// Empty is correct and is not a shortcut: the sample tree ships the `.ai/schemas` *directory*
/// precisely so that `CTX-012` compares each schema against this list and finds none missing. Passing
/// a fabricated list here would instead assert nothing.
const NO_SCHEMAS: &[(&str, &str)] = &[];

/// Runs `doctor` over `root` with no discovered languages, so no stack claim is contradicted.
fn report(root: &std::path::Path) -> aicontext_context::Report {
    doctor::run(&Inputs {
        root,
        observed_languages: &[],
        schema_source: NO_SCHEMAS,
    })
}

#[test]
fn the_sample_ai_tree_produces_no_findings() {
    let project = TempProject::new("payments-ledger").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the tree");

    let report = report(project.path());

    assert!(
        report.findings.is_empty(),
        "the shared sample tree must be a clean baseline; every test that uses it inherits these \
         findings. Got: {:#?}",
        report.findings
    );
    assert!(
        !report.has_errors(),
        "and therefore no error-level finding either: {:#?}",
        report.findings
    );
}

#[test]
fn the_clean_run_examined_the_tree_rather_than_finding_nothing_to_examine() {
    // A tree that is empty produces no findings either. Without this, the test above would pass for a
    // fixture that silently wrote nothing, which is the failure mode a `write` helper has when it
    // writes to the wrong path.
    let project = TempProject::new("examined").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the tree");

    let report = report(project.path());
    let examined: Vec<&str> = report.checked.iter().map(|row| row.path.as_str()).collect();

    assert!(
        examined.contains(&".ai/AI.md"),
        "the entry point must be listed as checked, not merely absent from the findings: {examined:?}"
    );
    assert!(
        examined.contains(&".ai/TASKS.md"),
        "the register must be listed as checked: {examined:?}"
    );
    assert!(
        report
            .checked
            .iter()
            .any(|row| row.path == ".ai/TASKS.md" && row.note == "1 tasks"),
        "the register holds one inline task, and the note must count it: {:#?}",
        report.checked
    );
}

#[test]
fn removing_the_entry_point_produces_exactly_the_finding_its_absent_file_names() {
    // The first test would pass just as happily against a fixture doctor cannot see into, so this
    // proves the guard has teeth: a tree with one defect must produce exactly that one defect's code.
    let project = TempProject::new("broken").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the tree");
    std::fs::remove_file(project.join(".ai/AI.md")).expect("removes the entry point");

    let report = report(project.path());
    let codes: Vec<&str> = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();

    assert_eq!(
        codes,
        vec!["CTX-001"],
        "expected only the missing entry point: {:#?}",
        report.findings
    );
    assert!(report.has_errors());
    assert!(
        report.checked.iter().all(|row| row.path != ".ai/AI.md"),
        "a document that produced a finding is never also listed as checked"
    );
}

#[test]
fn a_dangling_spec_reference_in_the_sample_tree_would_be_reported() {
    // The sample task points `spec` at a file the fixture creates. If that file were ever dropped
    // from the tree, every test using the fixture would inherit a CTX-007. This asserts the
    // reference resolves *and* that the check that would catch it is actually running.
    let project = TempProject::new("spec-resolves").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the tree");

    assert!(
        report(project.path())
            .findings
            .iter()
            .all(|finding| finding.code != "CTX-007"),
        "the sample tree's own references must resolve"
    );

    std::fs::remove_file(project.join("docs/CONTEXT_SPEC.md")).expect("removes the spec");
    let after = report(project.path());
    assert!(
        after
            .findings
            .iter()
            .any(|finding| finding.code == "CTX-007"),
        "and the check must fire once it does not: {:#?}",
        after.findings
    );
}

#[test]
fn a_temporary_repository_is_a_clean_repository_on_the_documented_branch() {
    // `aicontext status` (TASK-013) is the first consumer of this fixture, and "degrades gracefully
    // outside a Git repository" is one of its acceptance criteria. A fixture that only ever exists
    // inside a repository could not tell the two cases apart.
    let repo = TempRepository::new("payments-ledger").expect("a repository");

    assert!(TempRepository::is_repository(repo.path()));
    assert!(repo.has_commits(), "the fixture has its first commit");
    assert_eq!(repo.branch().expect("a branch"), "main");
    assert!(
        repo.status().expect("a status").is_empty(),
        "a fresh fixture repository is clean, so a change reported later came from the test"
    );
    assert_eq!(
        repo.commits(10).expect("a history").len(),
        1,
        "exactly one commit: the fixture's own"
    );
}

#[test]
fn a_directory_that_is_not_a_repository_is_distinguishable_from_one_that_is() {
    let project = TempProject::new("plain").expect("a project");

    assert!(
        !TempRepository::is_repository(project.path()),
        "a bare temporary directory must not be mistaken for a repository"
    );
}

#[test]
fn the_fixture_tree_survives_being_written_into_a_repository() {
    // `status` reports a project *and* a branch, so the natural fixture is a repository carrying a
    // `.ai/` tree. This asserts the two compose, which is the shape TASK-013 needs.
    let repo = TempRepository::new("payments-ledger").expect("a repository");
    sample_ai_tree(repo.path()).expect("writes the tree");

    assert!(report(repo.path()).findings.is_empty());
    let changes = repo.status().expect("a status");
    assert!(
        !changes.is_empty(),
        "a fresh `.ai/` tree is untracked, which is what a status report would show"
    );
}
