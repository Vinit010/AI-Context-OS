//! The content-hash index, exercised through its public API.
//!
//! Mirrors the acceptance criteria of `TASK-031`:
//!
//! 1. **It indexes `.ai` documents** and respects `.gitignore`.
//! 2. **It persists** to `.aicontext/index.json`, keyed by content hash.
//! 3. **A second run over an unchanged tree re-reads nothing** — asserted deterministically through
//!    [`ScanReport::reads`], never through a wall-clock measurement (`RULES.md` §8).
//! 4. **A changed file invalidates only its own entry.**
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use std::fs;

use aicontext_context::{ContextIndex, DocumentKind, INDEX_PATH, IndexError, ScanReport, Value};
use aicontext_testkit::{TempProject, sample_ai_tree};

/// Builds the sample `.ai/` tree and indexes it into the cache at the documented path.
fn indexed(name: &str) -> (TempProject, ContextIndex, ScanReport) {
    let project = TempProject::new(name).expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the sample tree");
    let cache = project.join(INDEX_PATH);
    let (index, report) =
        ContextIndex::refresh(project.path(), &cache).expect("a fresh scan writes the cache");
    (project, index, report)
}

/// The entries of an index, cloned so two indexes can be compared without borrowing.
fn entries(index: &ContextIndex) -> Vec<aicontext_context::IndexEntry> {
    index.entries().cloned().collect()
}

#[test]
fn a_fresh_scan_indexes_every_markdown_document_under_ai() {
    let (_project, index, report) = indexed("fresh");

    assert_eq!(index.len(), 2, "the sample tree has AI.md and TASKS.md");
    assert_eq!(report.documents, 2);
    assert_eq!(report.reads(), 2, "the first scan reads both documents");
    assert!(index.get(".ai/AI.md").is_some());
    assert!(index.get(".ai/TASKS.md").is_some());
}

#[test]
fn a_document_outside_ai_is_not_indexed() {
    // The sample tree's `spec` points at `docs/CONTEXT_SPEC.md`, which is real but is not part of the
    // knowledge directory. The index describes `.ai/`, not the whole repository.
    let (_project, index, _) = indexed("outside");
    assert!(index.get("docs/CONTEXT_SPEC.md").is_none());
}

#[test]
fn an_empty_project_indexes_nothing() {
    let project = TempProject::new("empty").expect("a temporary project");
    let (index, report) = ContextIndex::refresh(project.path(), &project.join(INDEX_PATH))
        .expect("scans an empty root");

    assert!(index.is_empty());
    assert_eq!(report.documents, 0);
    assert_eq!(report.reads(), 0);
}

#[test]
fn a_gitignored_document_is_not_indexed() {
    let project = TempProject::new("ignored").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the sample tree");
    project
        .write(".ai/.gitignore", "scratch.md\n")
        .expect("writes");
    project
        .write(".ai/scratch.md", "# Scratch\n\nA local note.\n")
        .expect("writes");
    project.write(".ai/kept.md", "# Kept\n").expect("writes");

    let (index, _) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("scans");

    assert!(
        index.get(".ai/kept.md").is_some(),
        "a tracked neighbour is still indexed"
    );
    assert!(
        index.get(".ai/scratch.md").is_none(),
        "a gitignored file must not be indexed"
    );
}

#[test]
fn a_scan_persists_to_the_documented_path_and_reloads_identically() {
    let (project, index, _) = indexed("persist");
    let cache = project.join(INDEX_PATH);

    assert!(cache.is_file(), "the cache is written where the spec says");
    let text = project.read(INDEX_PATH).expect("reads the cache");
    assert!(
        text.contains("\"version\": 2"),
        "the cache names its format version"
    );

    let reloaded = ContextIndex::load(&cache).expect("a written cache loads");
    assert_eq!(entries(&reloaded), entries(&index));
}

#[test]
fn a_second_scan_of_an_unchanged_tree_reads_nothing() {
    let (project, _, first) = indexed("unchanged");
    assert!(first.reads() > 0, "the first scan must read the documents");

    let (_, second) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("rescans");

    assert_eq!(second.reads(), 0, "an unchanged tree is not re-read");
    assert_eq!(second.reused, second.documents);
    assert_eq!(second.parsed, 0);
}

#[test]
fn a_second_scan_writes_the_same_bytes() {
    let (project, _, _) = indexed("deterministic");
    let first = project.read(INDEX_PATH).expect("reads the cache");

    ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("rescans");

    assert_eq!(
        project.read(INDEX_PATH).expect("reads the cache"),
        first,
        "a no-op scan must not churn the cache file"
    );
}

#[test]
fn a_changed_file_invalidates_only_its_own_entry() {
    let (project, first, _) = indexed("changed");
    let before_ai = first.get(".ai/AI.md").expect("indexed").clone();
    let before_tasks = first.get(".ai/TASKS.md").expect("indexed").clone();

    project
        .write(".ai/AI.md", "# AI.md\n\nThis line changed.\n")
        .expect("writes");

    let (second, report) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("rescans");

    assert_eq!(report.parsed, 1, "one changed document is one parse");
    assert_eq!(report.reused, 1, "the untouched document is not re-read");

    let after_ai = second.get(".ai/AI.md").expect("still indexed");
    let after_tasks = second.get(".ai/TASKS.md").expect("still indexed");
    assert_ne!(
        after_ai.content_hash, before_ai.content_hash,
        "the changed document's hash moved"
    );
    assert_eq!(
        *after_tasks, before_tasks,
        "an untouched document keeps its entry verbatim"
    );
}

#[test]
fn a_removed_document_is_dropped_from_the_cache() {
    let (project, _, _) = indexed("removed");
    fs::remove_file(project.join(".ai/TASKS.md")).expect("removes the file");

    let (index, report) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("rescans");

    assert_eq!(report.removed, 1);
    assert!(index.get(".ai/TASKS.md").is_none());
    assert!(index.get(".ai/AI.md").is_some());
}

#[test]
fn a_corrupt_cache_is_rebuilt_rather_than_trusted() {
    let (project, _, _) = indexed("corrupt");
    let cache = project.join(INDEX_PATH);
    project
        .write(INDEX_PATH, "{ not json")
        .expect("writes garbage");

    assert!(
        ContextIndex::load(&cache).is_err(),
        "garbage does not parse"
    );

    let (index, report) = ContextIndex::refresh(project.path(), &cache).expect("rebuilds");

    assert_eq!(index.len(), 2, "the rebuild recovers every document");
    assert_eq!(report.parsed, 2);
}

#[test]
fn a_cache_from_another_version_is_rebuilt() {
    let (project, _, _) = indexed("version");
    let cache = project.join(INDEX_PATH);
    project
        .write(INDEX_PATH, "{\"version\": 999, \"entries\": {}}")
        .expect("writes a foreign cache");

    assert!(matches!(
        ContextIndex::load(&cache),
        Err(IndexError::Version { found: 999, .. })
    ));

    let (index, _) = ContextIndex::refresh(project.path(), &cache).expect("rebuilds");
    assert_eq!(index.len(), 2);
}

#[test]
fn document_kind_comes_from_the_location() {
    let project = TempProject::new("kinds").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the sample tree");
    project
        .write(".ai/specs/payments.md", "# Payments\n")
        .expect("writes");
    project
        .write(
            ".ai/decisions/ADR-001-storage.md",
            "---\nid: ADR-001\ntype: decision\ntitle: Storage\n---\n\n# Storage\n",
        )
        .expect("writes");

    let (index, _) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("scans");

    assert_eq!(
        index.get(".ai/AI.md").expect("indexed").kind,
        Some(DocumentKind::AgentEntryPoint)
    );
    assert_eq!(
        index.get(".ai/TASKS.md").expect("indexed").kind,
        Some(DocumentKind::TaskRegister)
    );
    assert_eq!(
        index.get(".ai/specs/payments.md").expect("indexed").kind,
        Some(DocumentKind::Specification)
    );
    assert_eq!(
        index
            .get(".ai/decisions/ADR-001-storage.md")
            .expect("indexed")
            .kind,
        Some(DocumentKind::Decision)
    );
}

#[test]
fn front_matter_title_headings_and_links_are_recorded() {
    let (_project, index, _) = indexed("metadata");
    let register = index.get(".ai/TASKS.md").expect("indexed");

    assert_eq!(register.id.as_deref(), Some("TASKS-001"));
    assert_eq!(register.title.as_deref(), Some("Task Register"));
    assert_eq!(
        register.front_matter.get("type").and_then(Value::as_str),
        Some("tasks")
    );
    assert!(
        register
            .headings
            .iter()
            .any(|heading| heading.text == "TASKS"),
        "the register's headings are recorded: {:?}",
        register.headings
    );
    assert!(
        register.links.iter().any(|link| link == "TASK-001"),
        "the inline task is a referenced ID: {:?}",
        register.links
    );
}

#[test]
fn a_document_without_front_matter_still_indexes_without_metadata() {
    let (_project, index, _) = indexed("plain");
    let ai = index.get(".ai/AI.md").expect("indexed");

    assert!(ai.front_matter.is_empty());
    assert_eq!(ai.id, None);
    assert_eq!(
        ai.title.as_deref(),
        Some("AI.md"),
        "the title falls back to the first heading"
    );
    assert!(!ai.headings.is_empty());
}

#[test]
fn an_oversize_document_is_counted_and_not_indexed() {
    let project = TempProject::new("oversize").expect("a temporary project");
    sample_ai_tree(project.path()).expect("writes the sample tree");
    let huge = "x".repeat(aicontext_context::DOCUMENT_MAX_BYTES + 1);
    project.write(".ai/huge.md", &huge).expect("writes");

    let (index, report) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("scans");

    assert_eq!(report.oversize, 1, "the oversize document is reported");
    assert!(index.get(".ai/huge.md").is_none());
    assert_eq!(index.len(), 2, "everything else is still indexed");
}

#[test]
fn a_document_kind_renders_with_the_same_name_serde_writes() {
    // `--explain` names a kind with `Display`; the cache names it with serde. The two must not
    // drift, so the multi-word variants are checked against the JSON spelling.
    for kind in [
        DocumentKind::AgentEntryPoint,
        DocumentKind::TaskRegister,
        DocumentKind::ContextNote,
        DocumentKind::Specification,
    ] {
        let json = serde_json::to_string(&kind).expect("a kind serialises");
        assert_eq!(
            format!("\"{kind}\""),
            json,
            "Display and serde disagree for {kind:?}"
        );
    }
}
