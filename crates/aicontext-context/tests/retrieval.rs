//! The retrieval plan, exercised through its public API.
//!
//! Mirrors `TASK-032`: a plan derives the mandatory documents, graph links, filters, and globs from
//! the task; it is inspectable through [`RetrievalPlan::explain`]; and it is fully serialisable.
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use std::path::PathBuf;

use aicontext_context::{
    ContextBudget, ContextIndex, DEFAULT_MAX_TOKENS, Document, DocumentKind, FrontMatterPredicate,
    INDEX_PATH, LexicalRetriever, MANDATORY_ENTRY_POINT, MANDATORY_RULES, PlanOptions, PredicateOp,
    Register, RetrievalPlan, Retriever, TaskEntry, Tier,
};
use aicontext_testkit::TempProject;

/// A task shaped like the register's own fenced blocks.
fn task(id: &str) -> TaskEntry {
    TaskEntry {
        id: id.to_string(),
        title: Some("Build the content-hash index".to_string()),
        status: Some("IN_PROGRESS".to_string()),
        priority: Some("CRITICAL".to_string()),
        phase: Some(2),
        epic: None,
        spec: Some("docs/CONTEXT_SPEC.md".to_string()),
        depends_on: vec!["TASK-031".to_string()],
        touches: vec!["crates/aicontext-context/**".to_string()],
        relates_to: vec!["BUG-001".to_string()],
        tags: vec!["retrieval".to_string()],
    }
}

/// A register holding `tasks`, with the first named as current.
fn register(tasks: Vec<TaskEntry>) -> Register {
    Register {
        current_phase: Some("Phase 2".to_string()),
        current_task: tasks.first().map(|task| task.id.clone()),
        tasks,
    }
}

/// The document the predicates are tested against.
fn document(source: &str) -> Document {
    Document::parse(source).expect("a well-formed document")
}

#[test]
fn a_plan_from_a_task_carries_the_mandatory_set_and_the_graph() {
    let register = register(vec![task("TASK-032"), task("TASK-031")]);
    let plan = RetrievalPlan::for_task(&register, "TASK-032", &PlanOptions::default());

    assert_eq!(plan.task.as_deref(), Some("TASK-032"));
    assert_eq!(
        plan.mandatory,
        vec![
            PathBuf::from(MANDATORY_ENTRY_POINT),
            PathBuf::from(MANDATORY_RULES)
        ]
    );
    assert_eq!(
        plan.linked_ids,
        vec!["docs/CONTEXT_SPEC.md", "TASK-031", "BUG-001"]
    );
    assert_eq!(plan.globs, vec!["crates/aicontext-context/**"]);
    assert_eq!(
        plan.filters,
        vec![FrontMatterPredicate::equals("phase", "2")]
    );
    assert_eq!(plan.budget, ContextBudget::default());
}

#[test]
fn an_unknown_task_is_still_named_but_opens_no_edges() {
    let register = register(vec![task("TASK-032")]);
    let plan = RetrievalPlan::for_task(&register, "TASK-999", &PlanOptions::default());

    assert_eq!(plan.task.as_deref(), Some("TASK-999"));
    assert!(plan.linked_ids.is_empty());
    assert!(plan.globs.is_empty());
    assert!(plan.filters.is_empty());
    assert_eq!(plan.mandatory.len(), 2);
}

#[test]
fn a_plan_without_a_task_is_just_the_mandatory_set() {
    let plan = RetrievalPlan::new(&PlanOptions::default());

    assert_eq!(plan.task, None);
    assert_eq!(plan.mandatory.len(), 2);
    assert!(plan.linked_ids.is_empty());
    assert!(plan.globs.is_empty());
}

#[test]
fn options_force_include_exclude_and_add_globs() {
    let register = register(vec![task("TASK-032")]);
    let options = PlanOptions {
        query: Some("token budget".to_string()),
        include: vec![
            PathBuf::from("docs/CONTEXT_SPEC.md"),
            PathBuf::from(MANDATORY_ENTRY_POINT),
        ],
        exclude: vec!["ADR-003".to_string()],
        globs: vec!["docs/**".to_string()],
        filters: vec![FrontMatterPredicate::exists("status")],
        include_kinds: vec![DocumentKind::Specification],
        budget: Some(ContextBudget::default().with_max_tokens(4000)),
    };
    let plan = RetrievalPlan::for_task(&register, "TASK-032", &options);

    // The duplicate AI.md in --include is not repeated, and order is preserved.
    assert_eq!(plan.mandatory.len(), 3);
    assert_eq!(plan.mandatory[2], PathBuf::from("docs/CONTEXT_SPEC.md"));
    assert_eq!(plan.globs, vec!["docs/**", "crates/aicontext-context/**"]);
    assert_eq!(plan.exclude_ids, vec!["ADR-003"]);
    assert_eq!(plan.query.as_deref(), Some("token budget"));
    assert_eq!(plan.include_kinds, vec![DocumentKind::Specification]);
    assert_eq!(plan.budget.max_tokens, 4000);
    // The caller's filter precedes the task-derived one.
    assert_eq!(
        plan.filters,
        vec![
            FrontMatterPredicate::exists("status"),
            FrontMatterPredicate::equals("phase", "2"),
        ]
    );
}

#[test]
fn a_plan_round_trips_through_json() {
    let register = register(vec![task("TASK-032")]);
    let options = PlanOptions {
        include: vec![PathBuf::from("docs/CONTEXT_SPEC.md")],
        include_kinds: vec![DocumentKind::Specification],
        budget: Some(ContextBudget {
            max_tokens: 8000,
            per_tier: [(Tier::SPECIFICATION, 2000u32)].into_iter().collect(),
            reserved_policy_tokens: 500,
        }),
        ..PlanOptions::default()
    };
    let plan = RetrievalPlan::for_task(&register, "TASK-032", &options);

    let json = serde_json::to_string(&plan).expect("a plan serialises");
    let back: RetrievalPlan = serde_json::from_str(&json).expect("and deserialises");

    assert_eq!(back, plan);
}

#[test]
fn the_budget_defaults_to_the_documented_ceiling() {
    let budget = ContextBudget::default();

    assert_eq!(budget.max_tokens, DEFAULT_MAX_TOKENS);
    assert!(budget.per_tier.is_empty());
    assert_eq!(budget.reserved_policy_tokens, 0);
    assert_eq!(budget.with_max_tokens(1).max_tokens, 1);
}

#[test]
fn explain_prints_every_field() {
    let register = register(vec![task("TASK-032")]);
    let plan = RetrievalPlan::for_task(&register, "TASK-032", &PlanOptions::default());
    let text = plan.explain();

    assert!(text.starts_with("retrieval plan\n"));
    assert!(text.contains("  task:      TASK-032\n"));
    assert!(text.contains("  query:     (none)\n"));
    assert!(text.contains("  mandatory: "));
    assert!(text.contains(&PathBuf::from(MANDATORY_ENTRY_POINT).display().to_string()));
    assert!(text.contains(&PathBuf::from(MANDATORY_RULES).display().to_string()));
    assert!(text.contains("  linked:    docs/CONTEXT_SPEC.md, TASK-031, BUG-001\n"));
    assert!(text.contains("  globs:     crates/aicontext-context/**\n"));
    assert!(text.contains("  filters:   phase = 2\n"));
    assert!(text.contains("  kinds:     (none)\n"));
    assert!(text.contains("  excluded:  (none)\n"));
    assert!(text.contains(&format!("  budget:    {DEFAULT_MAX_TOKENS} tokens")));
}

#[test]
fn equals_matches_a_string_value() {
    let document = document("---\nepic: context-engine\n---\n\nBody.\n");

    assert!(FrontMatterPredicate::equals("epic", "context-engine").matches_document(&document));
    assert!(!FrontMatterPredicate::equals("epic", "other").matches_document(&document));
    assert!(!FrontMatterPredicate::equals("missing", "x").matches_document(&document));
}

#[test]
fn equals_matches_an_integer_value_by_its_decimal() {
    let document = document("---\nphase: 2\n---\n\nBody.\n");

    assert!(FrontMatterPredicate::equals("phase", "2").matches_document(&document));
    assert!(!FrontMatterPredicate::equals("phase", "3").matches_document(&document));
}

#[test]
fn contains_matches_a_list_item_or_a_substring() {
    let document = document("---\nid: TASK-032\ntags: [retrieval, context]\n---\n\nBody.\n");

    assert!(FrontMatterPredicate::contains("tags", "retrieval").matches_document(&document));
    assert!(!FrontMatterPredicate::contains("tags", "budget").matches_document(&document));
    // A string value is searched as a substring.
    assert!(FrontMatterPredicate::contains("id", "TASK").matches_document(&document));
}

#[test]
fn exists_and_not_exists_test_presence() {
    let document = document("---\nstatus: active\n---\n\nBody.\n");

    assert!(FrontMatterPredicate::exists("status").matches_document(&document));
    assert!(!FrontMatterPredicate::exists("tags").matches_document(&document));
    assert!(FrontMatterPredicate::not_exists("tags").matches_document(&document));
    assert!(!FrontMatterPredicate::not_exists("status").matches_document(&document));
}

#[test]
fn not_equals_and_not_contains_negate_their_operators() {
    let doc = document("---\nstatus: active\ntags: [a, b]\n---\n\nBody.\n");
    let not_equals = FrontMatterPredicate {
        key: "status".to_string(),
        op: PredicateOp::NotEquals,
        value: Some("done".to_string()),
    };
    let not_contains = FrontMatterPredicate {
        key: "tags".to_string(),
        op: PredicateOp::NotContains,
        value: Some("c".to_string()),
    };

    assert!(not_equals.matches_document(&doc));
    assert!(!not_equals.matches_document(&document("---\nstatus: done\n---\n\nBody.\n")));
    assert!(not_contains.matches_document(&doc));
}

#[test]
fn tiers_are_numbered_one_through_nine() {
    assert_eq!(Tier::INSTRUCTION.number(), 1);
    assert_eq!(Tier::HISTORY.number(), 9);
    assert_eq!(Tier::new(5), Some(Tier::TASK));
    assert_eq!(Tier::new(0), None);
    assert_eq!(Tier::new(10), None);
}

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

/// A query plan with no task, seeded only by free text.
fn query(text: &str) -> RetrievalPlan {
    RetrievalPlan::new(&PlanOptions {
        query: Some(text.to_string()),
        ..PlanOptions::default()
    })
}

#[test]
fn the_lexical_retriever_prefers_a_title_match_to_a_body_match() {
    let (_project, index) = indexed(
        "lexical-rank",
        &[
            (
                ".ai/specs/title.md",
                "---\ntitle: Retrieval budget\n---\n\nUnrelated prose.\n",
            ),
            (
                ".ai/specs/body.md",
                "---\ntitle: Something else\n---\n\nThe body mentions retrieval once.\n",
            ),
            (
                ".ai/specs/absent.md",
                "---\ntitle: Nothing relevant\n---\n\nNo matching token here.\n",
            ),
        ],
    );

    let results = LexicalRetriever::new().retrieve(&query("retrieval"), &index);

    assert_eq!(
        results.len(),
        2,
        "a document with no matching term is dropped"
    );
    assert_eq!(
        results[0].path,
        PathBuf::from(".ai/specs/title.md"),
        "the title's 3.0 weight outranks the body's 1.0"
    );
    assert!(
        results[0].score > results[1].score,
        "the title match scores strictly higher"
    );
    for result in &results {
        assert!(result.score > 0.0, "every result carries a positive score");
        assert!(
            result.reason.contains("'retrieval'"),
            "the reason names the matched term, got {:?}",
            result.reason
        );
        assert_eq!(result.tier, Tier::SPECIFICATION);
        assert_eq!(result.trace.source_path, result.path);
    }
}

#[test]
fn the_lexical_retriever_breaks_ties_by_identifier_then_path() {
    let (_project, index) = indexed(
        "lexical-tie",
        &[
            (".ai/specs/b.md", "---\ntitle: Budget too\n---\n\nx\n"),
            (".ai/specs/a.md", "---\ntitle: Budget matters\n---\n\nx\n"),
        ],
    );

    let results = LexicalRetriever::new().retrieve(&query("budget"), &index);

    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].score.to_bits(),
        results[1].score.to_bits(),
        "the two documents tie"
    );
    assert_eq!(results[0].path, PathBuf::from(".ai/specs/a.md"));
    assert_eq!(results[1].path, PathBuf::from(".ai/specs/b.md"));
}

#[test]
fn the_lexical_retriever_is_deterministic() {
    let (_project, index) = indexed(
        "lexical-determinism",
        &[
            (
                ".ai/specs/one.md",
                "---\ntitle: Retrieval plan\n---\n\nBody about context.\n",
            ),
            (
                ".ai/specs/two.md",
                "---\ntitle: Context index\n---\n\nBody about retrieval.\n",
            ),
        ],
    );

    let first = LexicalRetriever::new().retrieve(&query("retrieval context"), &index);
    let second = LexicalRetriever::new().retrieve(&query("retrieval context"), &index);

    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(&second) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.score.to_bits(), b.score.to_bits());
        assert_eq!(a.reason, b.reason);
    }
}

#[test]
fn the_lexical_retriever_returns_nothing_without_a_query() {
    let (_project, index) = indexed(
        "lexical-no-query",
        &[(
            ".ai/specs/one.md",
            "---\ntitle: Retrieval plan\n---\n\nBody.\n",
        )],
    );

    assert!(
        LexicalRetriever::new()
            .retrieve(&RetrievalPlan::default(), &index)
            .is_empty()
    );
}
