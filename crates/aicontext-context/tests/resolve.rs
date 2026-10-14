//! The priority ladder and its conflict rule, exercised through the public API.
//!
//! Mirrors `TASK-035`: every document kind sits on the rung `ARCHITECTURE.md` §6 gives it; a
//! lower-tier front-matter assertion that disagrees with a higher-tier one loses and is reported; and
//! the higher-tier document is never rewritten (`docs/CONTEXT_SPEC.md` §7.4).
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use std::fs;

use aicontext_context::doctor::{self, Inputs};
use aicontext_context::{ContextIndex, DocumentKind, INDEX_PATH, PriorityResolver, Tier};
use aicontext_testkit::TempProject;

/// A temporary project holding `files` and the index they scan into.
fn indexed(name: &str, files: &[(&str, &str)]) -> (TempProject, ContextIndex) {
    let project = TempProject::new(name).expect("a temporary project");
    for (relative, contents) in files {
        project
            .write(relative, contents)
            .expect("writes a document");
    }
    let (index, _) =
        ContextIndex::refresh(project.path(), &project.join(INDEX_PATH)).expect("scans the tree");
    (project, index)
}

#[test]
fn each_kind_maps_to_the_rung_the_architecture_names() {
    // One path per kind that the ladder distinguishes. The mapping is derived from each document's
    // location, exactly as a retrieval candidate's tier is, so this exercises the real classification
    // rather than a second hand-written table.
    let cases: &[(&str, DocumentKind, Tier)] = &[
        (
            ".ai/AI.md",
            DocumentKind::AgentEntryPoint,
            Tier::INSTRUCTION,
        ),
        (
            ".ai/agents/reviewer.md",
            DocumentKind::AgentProfile,
            Tier::INSTRUCTION,
        ),
        (".ai/RULES.md", DocumentKind::Rules, Tier::POLICY),
        (
            ".ai/CONVENTIONS.md",
            DocumentKind::Conventions,
            Tier::POLICY,
        ),
        (
            ".ai/ARCHITECTURE.md",
            DocumentKind::Architecture,
            Tier::ARCHITECTURE,
        ),
        (".ai/DESIGN.md", DocumentKind::Design, Tier::ARCHITECTURE),
        (".ai/PRD.md", DocumentKind::Product, Tier::SPECIFICATION),
        (
            ".ai/specs/context.md",
            DocumentKind::Specification,
            Tier::SPECIFICATION,
        ),
        (".ai/TASKS.md", DocumentKind::TaskRegister, Tier::TASK),
        (".ai/tasks/TASK-001-one.md", DocumentKind::Task, Tier::TASK),
        (
            ".ai/decisions/ADR-001-x.md",
            DocumentKind::Decision,
            Tier::DECISION,
        ),
        (".ai/MEMORY.md", DocumentKind::MemoryRegister, Tier::MEMORY),
        (
            ".ai/context/note.md",
            DocumentKind::ContextNote,
            Tier::SOURCE,
        ),
        (
            ".ai/workflows/flow.md",
            DocumentKind::Workflow,
            Tier::SOURCE,
        ),
        (".ai/bugs/BUG-001-x.md", DocumentKind::Bug, Tier::HISTORY),
        (
            ".ai/changes/CHG-001-x.md",
            DocumentKind::Change,
            Tier::HISTORY,
        ),
    ];

    let files: Vec<(&str, &str)> = cases
        .iter()
        .map(|(path, _, _)| (*path, "---\nid: X-001\ntitle: T\n---\n\nBody.\n"))
        .collect();
    let (_project, index) = indexed("resolve-ladder", &files);

    for (path, kind, tier) in cases {
        let entry = index
            .get(path)
            .unwrap_or_else(|| panic!("{path} is indexed"));
        assert_eq!(entry.kind, Some(*kind), "kind of {path}");
        assert_eq!(
            PriorityResolver::tier_of(entry),
            *tier,
            "tier of {path} ({kind})"
        );
    }
}

#[test]
fn an_unclassified_document_sits_on_the_source_rung() {
    let (_project, index) = indexed(
        "resolve-unclassified",
        &[(".ai/notes.md", "No front matter, no location convention.\n")],
    );

    let entry = index.get(".ai/notes.md").expect("notes are indexed");
    assert_eq!(entry.kind, None);
    assert_eq!(PriorityResolver::tier_of(entry), Tier::SOURCE);
}

/// The §7.6 example: tier 3 asserts `mongodb`, tier 9 asserts `postgresql`.
fn database_disagreement(name: &str) -> (TempProject, ContextIndex) {
    indexed(
        name,
        &[
            (
                ".ai/ARCHITECTURE.md",
                "---\nid: ARCH-001\ntype: architecture\ntitle: A\ndatabase: mongodb\n---\n\nBody.\n",
            ),
            (
                ".ai/changes/CHG-004-x.md",
                "---\nid: CHG-004\ntype: change\ntitle: C\ndatabase: postgresql\n---\n\nBody.\n",
            ),
        ],
    )
}

#[test]
fn a_lower_tier_assertion_loses_to_a_higher_tier_one() {
    let (project, index) = database_disagreement("resolve-conflict");
    let before = fs::read_to_string(project.path().join(".ai/ARCHITECTURE.md")).expect("reads");

    let conflicts = PriorityResolver::conflicts(&index);

    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    let conflict = &conflicts[0];
    assert_eq!(conflict.key, "database");
    assert_eq!(conflict.asserted_by.tier, Tier::ARCHITECTURE);
    assert_eq!(conflict.asserted_by.id, "ARCH-001");
    assert_eq!(conflict.asserted_by.value.to_string(), "mongodb");
    assert_eq!(
        conflict.asserted_by.path,
        std::path::Path::new(".ai/ARCHITECTURE.md")
    );
    assert_eq!(conflict.conflicts_with.tier, Tier::HISTORY);
    assert_eq!(conflict.conflicts_with.id, "CHG-004");
    assert_eq!(conflict.conflicts_with.value.to_string(), "postgresql");
    // Acceptance 3: the winner is never rewritten. The resolver is pure; the file is untouched.
    let after = fs::read_to_string(project.path().join(".ai/ARCHITECTURE.md")).expect("reads");
    assert_eq!(before, after);
}

#[test]
fn documents_that_agree_about_a_key_are_not_in_conflict() {
    let (_project, index) = indexed(
        "resolve-agree",
        &[
            (
                ".ai/ARCHITECTURE.md",
                "---\nid: ARCH-001\ndatabase: mongodb\n---\n\nBody.\n",
            ),
            (
                ".ai/changes/CHG-004-x.md",
                "---\nid: CHG-004\ndatabase: mongodb\n---\n\nBody.\n",
            ),
        ],
    );

    assert!(PriorityResolver::conflicts(&index).is_empty());
}

#[test]
fn two_documents_on_the_same_rung_are_not_a_lower_versus_higher_conflict() {
    let (_project, index) = indexed(
        "resolve-same-tier",
        &[
            (
                ".ai/changes/CHG-001-x.md",
                "---\nid: CHG-001\ndatabase: mongodb\n---\n\nBody.\n",
            ),
            (
                ".ai/changes/CHG-002-x.md",
                "---\nid: CHG-002\ndatabase: postgresql\n---\n\nBody.\n",
            ),
        ],
    );

    assert!(
        PriorityResolver::conflicts(&index).is_empty(),
        "the rule is about a lower tier contradicting a higher one"
    );
}

#[test]
fn a_list_assertion_is_not_a_single_comparable_value() {
    let (_project, index) = indexed(
        "resolve-list",
        &[
            (
                ".ai/ARCHITECTURE.md",
                "---\nid: ARCH-001\ndatabase: [mongodb, postgres]\n---\n\nBody.\n",
            ),
            (
                ".ai/changes/CHG-004-x.md",
                "---\nid: CHG-004\ndatabase: [sqlite]\n---\n\nBody.\n",
            ),
        ],
    );

    assert!(PriorityResolver::conflicts(&index).is_empty());
}

#[test]
fn conflict_detection_is_deterministic() {
    let (_project, index) = database_disagreement("resolve-determinism");

    let first = PriorityResolver::conflicts(&index);
    let second = PriorityResolver::conflicts(&index);

    assert_eq!(first, second);
}

#[test]
fn a_conflict_serialises_to_the_packet_shape_without_the_path() {
    let (_project, index) = database_disagreement("resolve-json");
    let conflict = PriorityResolver::conflicts(&index).remove(0);

    let json = serde_json::to_value(&conflict).expect("a conflict serialises");

    assert_eq!(json["key"].as_str(), Some("database"));
    assert_eq!(json["asserted_by"]["tier"].as_i64(), Some(3));
    assert_eq!(json["asserted_by"]["id"].as_str(), Some("ARCH-001"));
    assert_eq!(json["asserted_by"]["value"].as_str(), Some("mongodb"));
    assert_eq!(json["conflicts_with"]["tier"].as_i64(), Some(9));
    assert_eq!(json["resolution"].as_str(), Some("higher-tier-wins"));
    assert!(
        json["asserted_by"].get("path").is_none(),
        "the §7.6 packet shape names only tier, id, and value"
    );
}

#[test]
fn doctor_reports_a_lower_tier_conflict_as_ctx_013() {
    let (project, _index) = database_disagreement("resolve-doctor");
    // The schema directory keeps the report focused on the conflict: without it every run also
    // carries a CTX-012 about the copies `init` would have made.
    fs::create_dir_all(project.path().join(".ai/schemas")).expect("schema dir");
    project
        .write(".ai/AI.md", "# AI.md\n\nEntry point.\n")
        .expect("writes");

    let report = doctor::run(&Inputs {
        root: project.path(),
        observed_languages: &[],
        schema_source: &[],
    });

    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == "CTX-013")
        .unwrap_or_else(|| panic!("expected CTX-013 in {:?}", report.findings));
    assert_eq!(finding.path, ".ai/changes/CHG-004-x.md");
    assert!(finding.message.contains("database"), "{finding:?}");
    assert!(finding.message.contains("ARCH-001"), "{finding:?}");
    assert!(finding.message.contains("mongodb"), "{finding:?}");
}
