//! The retrieval plan, exercised through its public API.
//!
//! Mirrors `TASK-032`: a plan derives the mandatory documents, graph links, filters, and globs from
//! the task; it is inspectable through [`RetrievalPlan::explain`]; and it is fully serialisable.
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use std::path::PathBuf;

use aicontext_context::{
    ContextBudget, ContextIndex, DEFAULT_MAX_TOKENS, Document, DocumentKind, FilterRetriever,
    FrontMatterPredicate, GlobRetriever, INDEX_PATH, LexicalRetriever, LinkedId, LinkedRetriever,
    MANDATORY_ENTRY_POINT, MANDATORY_RULES, PlanOptions, PredicateOp, Register, RetrievalPlan,
    Retriever, TaskEntry, Tier,
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
        plan.linked,
        vec![
            LinkedId::spec("docs/CONTEXT_SPEC.md"),
            LinkedId::depends_on("TASK-031"),
            LinkedId::relates_to("BUG-001"),
        ]
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
    assert!(plan.linked.is_empty());
    assert!(plan.globs.is_empty());
    assert!(plan.filters.is_empty());
    assert_eq!(plan.mandatory.len(), 2);
}

#[test]
fn a_plan_without_a_task_is_just_the_mandatory_set() {
    let plan = RetrievalPlan::new(&PlanOptions::default());

    assert_eq!(plan.task, None);
    assert_eq!(plan.mandatory.len(), 2);
    assert!(plan.linked.is_empty());
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
    assert!(text.contains(
        "  linked:    docs/CONTEXT_SPEC.md (spec), TASK-031 (depends_on), BUG-001 (relates_to)\n"
    ));
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

/// A plan seeded for a task that opens the given graph edges and globs.
fn linked_plan(task: &str, linked: Vec<LinkedId>, globs: Vec<&str>) -> RetrievalPlan {
    RetrievalPlan {
        task: Some(task.to_string()),
        linked,
        globs: globs.into_iter().map(str::to_string).collect(),
        ..RetrievalPlan::default()
    }
}

#[test]
fn the_linked_retriever_follows_spec_dependencies_and_bugs_to_documents() {
    let (_project, index) = indexed(
        "linked-follows",
        &[
            (
                ".ai/specs/context.md",
                "---\nid: SPEC-context\n---\n\nOutlines retrieval.\n",
            ),
            (
                ".ai/decisions/ADR-004-deterministic.md",
                "---\nid: ADR-004\n---\n\nDeterministic retrieval.\n",
            ),
            (
                ".ai/tasks/TASK-031-index.md",
                "---\nid: TASK-031\n---\n\nBuilds the index.\n",
            ),
            (
                ".ai/bugs/BUG-001-crash.md",
                "---\nid: BUG-001\n---\n\nThe old reader crashed.\n",
            ),
        ],
    );
    let plan = linked_plan(
        "TASK-034",
        vec![
            LinkedId::spec("SPEC-context"),
            LinkedId::depends_on("TASK-031"),
            LinkedId::relates_to("BUG-001"),
        ],
        vec![],
    );

    let results = LinkedRetriever.retrieve(&plan, &index);

    assert_eq!(results.len(), 3, "every edge resolves to its document");
    // Equal scores are broken by identifier then path, so find each edge by its reason.
    let spec = results
        .iter()
        .find(|result| result.reason.ends_with("(spec)"))
        .expect("the spec edge resolves");
    let depends = results
        .iter()
        .find(|result| result.reason.ends_with("(depends_on)"))
        .expect("the dependency edge resolves");
    let relates = results
        .iter()
        .find(|result| result.reason.ends_with("(relates_to)"))
        .expect("the bug edge resolves");
    assert_eq!(spec.reason, "linked from TASK-034 (spec)");
    assert_eq!(depends.reason, "linked from TASK-034 (depends_on)");
    assert_eq!(relates.reason, "linked from TASK-034 (relates_to)");
    assert_eq!(spec.path, PathBuf::from(".ai/specs/context.md"));
    assert_eq!(depends.path, PathBuf::from(".ai/tasks/TASK-031-index.md"));
    assert_eq!(relates.path, PathBuf::from(".ai/bugs/BUG-001-crash.md"));
    for result in &results {
        assert!(result.trace.included);
        assert_eq!(result.trace.source_path, result.path);
        assert_eq!(result.score.to_bits(), 1.0f64.to_bits());
    }
    // The tier comes from the document's kind, exactly like the lexical retriever.
    assert_eq!(spec.tier, Tier::SPECIFICATION);
    assert_eq!(depends.tier, Tier::TASK);
    assert_eq!(relates.tier, Tier::HISTORY);
}

#[test]
fn an_inline_entity_resolves_to_the_register_that_defines_it() {
    let (_project, index) = indexed(
        "linked-inline",
        &[
            (
                ".ai/TASKS.md",
                "---\nid: TASKS-001\n---\n\n### TASK-032 — Build the plan\n\nWords.\n\
                 ### TASK-031 — Build the index\n\nWords.\n",
            ),
            (
                ".ai/MEMORY.md",
                "---\nid: MEMORY-001\n---\n\n## MEM-021 — The index is a cache\n\nWords.\n",
            ),
        ],
    );
    let plan = linked_plan(
        "TASK-034",
        vec![
            LinkedId::depends_on("TASK-032"),
            LinkedId::relates_to("MEM-021"),
        ],
        vec![],
    );

    let results = LinkedRetriever.retrieve(&plan, &index);

    assert_eq!(
        results.len(),
        2,
        "a register heading resolves the inline entity to its register"
    );
    assert_eq!(
        results
            .iter()
            .find(|result| result.path == std::path::Path::new(".ai/TASKS.md"))
            .expect("the task register resolves")
            .reason,
        "linked from TASK-034 (depends_on)"
    );
    assert_eq!(
        results
            .iter()
            .find(|result| result.path == std::path::Path::new(".ai/MEMORY.md"))
            .expect("the memory register resolves")
            .reason,
        "linked from TASK-034 (relates_to)"
    );
    assert!(results.iter().all(|result| result.trace.included));
}

#[test]
fn a_broken_link_is_reported_as_a_dropped_candidate() {
    let (_project, index) = indexed(
        "linked-broken",
        &[(
            ".ai/TASKS.md",
            "---\nid: TASKS-001\n---\n\n### TASK-032 — Build the plan\n\nWords.\n",
        )],
    );
    let plan = linked_plan(
        "TASK-034",
        vec![
            LinkedId::spec("docs/CONTEXT_SPEC.md"),
            LinkedId::depends_on("TASK-999"),
            LinkedId::depends_on("TASK-032"),
        ],
        vec![],
    );

    let results = LinkedRetriever.retrieve(&plan, &index);

    assert_eq!(
        results.len(),
        3,
        "unresolved edges are reported, not skipped"
    );
    assert_eq!(
        results[0].path,
        PathBuf::from(".ai/TASKS.md"),
        "the resolved edge scores 1.0 and sorts above the broken ones"
    );
    assert!(results[0].trace.included);

    // The two broken links both score 0.0 and are ordered by identifier, so find each by name.
    let spec_broken = results
        .iter()
        .find(|result| result.reason.contains("docs/CONTEXT_SPEC.md"))
        .expect("the spec edge is reported");
    assert_eq!(
        spec_broken.trace.dropped_reason.as_deref(),
        Some("broken link"),
        "the reason bucket is explicit"
    );
    assert_eq!(spec_broken.path, PathBuf::new(), "nothing to read");
    assert_eq!(
        spec_broken.tier,
        Tier::SPECIFICATION,
        "a spec edge sits at tier 4"
    );
    assert_eq!(spec_broken.score.to_bits(), 0.0f64.to_bits());
    assert!(!spec_broken.trace.included);

    let missing_task = results
        .iter()
        .find(|result| result.reason.contains("TASK-999"))
        .expect("the missing dependency is reported");
    assert_eq!(
        missing_task.trace.dropped_reason.as_deref(),
        Some("broken link")
    );
    assert_eq!(missing_task.tier, Tier::TASK, "a dependency sits at tier 5");
}

#[test]
fn the_filter_retriever_keeps_every_document_that_matches_all_predicates() {
    let (_project, index) = indexed(
        "filter-matches",
        &[
            (
                ".ai/specs/a.md",
                "---\nphase: 2\nstatus: active\n---\n\nA.\n",
            ),
            (
                ".ai/specs/b.md",
                "---\nphase: 2\nfeature: filter\n---\n\nB.\n",
            ),
            (
                ".ai/specs/c.md",
                "---\nphase: 1\nstatus: active\n---\n\nC.\n",
            ),
        ],
    );
    let plan = RetrievalPlan::new(&PlanOptions {
        filters: vec![
            FrontMatterPredicate::equals("phase", "2"),
            FrontMatterPredicate::exists("status"),
        ],
        ..PlanOptions::default()
    });

    let results = FilterRetriever.retrieve(&plan, &index);

    assert_eq!(
        results.len(),
        1,
        "the conjunction keeps only a.md, which satisfies both predicates"
    );
    assert_eq!(results[0].path, PathBuf::from(".ai/specs/a.md"));
    assert!(results[0].trace.included);
    assert_eq!(results[0].reason, "matches phase = 2, status exists");
    assert_eq!(results[0].score.to_bits(), 1.0f64.to_bits());
    assert_eq!(results[0].tier, Tier::SPECIFICATION);
}

#[test]
fn a_retriever_with_nothing_to_match_returns_nothing() {
    let (_project, index) = indexed(
        "retriever-empty",
        &[(".ai/specs/a.md", "---\nid: SPEC-a\n---\n\nA.\n")],
    );
    let plan = RetrievalPlan::default();

    assert!(FilterRetriever.retrieve(&plan, &index).is_empty());
    assert!(GlobRetriever.retrieve(&plan, &index).is_empty());
    assert!(LinkedRetriever.retrieve(&plan, &index).is_empty());
}

#[test]
fn the_glob_retriever_matches_repository_paths() {
    let (_project, index) = indexed(
        "glob-matches",
        &[
            (".ai/specs/a.md", "---\nid: SPEC-a\n---\n\nA.\n"),
            (".ai/specs/b.md", "---\nid: SPEC-b\n---\n\nB.\n"),
            (".ai/decisions/c.md", "---\nid: ADR-001\n---\n\nC.\n"),
        ],
    );
    let plan = RetrievalPlan::new(&PlanOptions {
        globs: vec![".ai/specs/**".to_string()],
        ..PlanOptions::default()
    });

    let results = GlobRetriever.retrieve(&plan, &index);

    assert_eq!(results.len(), 2, "the decision is outside the glob");
    let paths: Vec<PathBuf> = results.iter().map(|result| result.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            PathBuf::from(".ai/specs/a.md"),
            PathBuf::from(".ai/specs/b.md")
        ]
    );
    assert_eq!(results[0].reason, "matches glob '.ai/specs/**'");
    assert_eq!(results[0].score.to_bits(), 1.0f64.to_bits());
    assert!(results.iter().all(|result| result.trace.included));
}

#[test]
fn glob_star_and_question_do_not_cross_path_segments() {
    let (_project, index) = indexed(
        "glob-syntax",
        &[
            (".ai/AI.md", "---\nid: AI-001\n---\n\nAgent entry point.\n"),
            (".ai/specs/a.md", "---\nid: SPEC-a\n---\n\nA.\n"),
            (".ai/specs/bb.md", "---\nid: SPEC-b\n---\n\nB.\n"),
        ],
    );

    let star = RetrievalPlan::new(&PlanOptions {
        globs: vec!["*.md".to_string()],
        ..PlanOptions::default()
    });
    assert!(
        GlobRetriever.retrieve(&star, &index).is_empty(),
        "`*` stays inside one segment, so it cannot reach a path below .ai/"
    );

    let globstar = RetrievalPlan::new(&PlanOptions {
        globs: vec!["**/*.md".to_string()],
        ..PlanOptions::default()
    });
    assert_eq!(GlobRetriever.retrieve(&globstar, &index).len(), 3);

    let single = RetrievalPlan::new(&PlanOptions {
        globs: vec![".ai/specs/?.md".to_string()],
        ..PlanOptions::default()
    });
    let results = GlobRetriever.retrieve(&single, &index);
    assert_eq!(results.len(), 1, "`?` is exactly one character");
    assert_eq!(results[0].path, PathBuf::from(".ai/specs/a.md"));
}
