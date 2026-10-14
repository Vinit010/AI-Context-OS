//! The token budget and its degradation order, exercised through the public API.
//!
//! Mirrors `TASK-036`: tiers 1–5 are never dropped, a budget too small for them fails with `CTX-030`,
//! and everything a budget must cut comes from the bottom of the ladder upwards — tier 9 first, then
//! 8, 7, 6 — lowest-scoring document first within a tier (`docs/CONTEXT_SPEC.md` §7.5). Candidates
//! are priced from the same index a retrieval reads, so this exercises `ContextBudget::apply` rather
//! than only its pure core.
//!
//! Tests live out here rather than in the module so that they can only reach the public surface. If
//! something in this file compiles, it is API; if it does not, the API is incomplete.

use aicontext_context::{
    ContextBudget, ContextIndex, DEFAULT_MAX_TOKENS, INDEX_PATH, PriorityResolver, RetrievalTrace,
    ScoredDocument, Tier,
};
use aicontext_testkit::TempProject;

/// A change note long enough that the index prices it clearly above a one-liner.
const LONG_CHANGE: &str = "---\nid: CHG-001\ntype: change\ntitle: Change one\n---\n\n# Change one\n\n\
    Connectors were reworked so a task reads the register before it plans, which means an agent that\n\
    previously guessed at dependencies now receives them. The body is deliberately wordy so the\n\
    heuristic token estimate is more than a couple of tokens, which keeps the budget arithmetic in\n\
    these tests about whole documents rather than rounding noise.\n";

/// A change note short enough to sit well below [`LONG_CHANGE`] on the same rung.
const SHORT_CHANGE: &str =
    "---\nid: CHG-002\ntype: change\ntitle: Change two\n---\n\nA brief note.\n";

/// An architecture document on tier 3.
const ARCHITECTURE: &str = "---\nid: ARCH-001\ntype: architecture\ntitle: Architecture\n---\n\n\
    # Architecture\n\nThe service indexes a project's `.ai` directory and assembles a context packet\n\
    for a task. It makes no network calls and no model calls, so the same repository state always\n\
    produces the same packet.\n";

/// A decision record on tier 6.
const DECISION: &str = "---\nid: ADR-001\ntype: decision\ntitle: Determinism\n---\n\n\
    # Determinism\n\nRetrieval is lexical and rule-based, with a fixed ladder and a total tie-break.\n";

/// A memory register on tier 7.
const MEMORY: &str = "---\nid: MEMORY-001\ntitle: Memory\n---\n\n\
    # Memory\n\nA durable note that changes how the next agent reads the register, kept long enough to\n\
    weigh more than the shortest documents in these fixtures.\n";

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

/// The index's heuristic token estimate for `path`, the cost `apply` charges for it.
fn tokens(index: &ContextIndex, path: &str) -> u32 {
    index
        .get(path)
        .unwrap_or_else(|| panic!("{path} is indexed"))
        .token_estimate
}

/// A candidate on the rung the ladder gives `path`, scored `score`.
fn candidate(index: &ContextIndex, path: &str, score: f64) -> ScoredDocument {
    let entry = index
        .get(path)
        .unwrap_or_else(|| panic!("{path} is indexed"));
    let tier = PriorityResolver::tier_of(entry);
    let id = entry.id.clone().unwrap_or_else(|| path.to_string());
    ScoredDocument {
        id: entry.id.clone(),
        path: entry.path.clone(),
        tier,
        score,
        reason: "test".to_string(),
        trace: RetrievalTrace {
            id,
            tier,
            score,
            reason: "test".to_string(),
            source_path: entry.path.clone(),
            included: true,
            dropped_reason: None,
        },
    }
}

#[test]
fn protected_tiers_survive_a_budget_that_cannot_hold_history() {
    let (_project, index) = indexed(
        "budget-protected",
        &[
            (".ai/ARCHITECTURE.md", ARCHITECTURE),
            (".ai/changes/CHG-001-a.md", LONG_CHANGE),
            (".ai/changes/CHG-002-b.md", SHORT_CHANGE),
        ],
    );
    let arch = tokens(&index, ".ai/ARCHITECTURE.md");
    let mut candidates = vec![
        candidate(&index, ".ai/ARCHITECTURE.md", 1.0),
        candidate(&index, ".ai/changes/CHG-001-a.md", 5.0),
        candidate(&index, ".ai/changes/CHG-002-b.md", 1.0),
    ];
    let budget = ContextBudget::default().with_max_tokens(arch);

    let report = budget
        .apply(&mut candidates, &index)
        .expect("the protected tiers fit");

    // The whole budget is spent on the never-dropped tier, so both history documents go.
    assert_eq!(report.requested, arch);
    assert_eq!(report.used, arch);
    assert_eq!(
        report
            .dropped
            .iter()
            .map(|dropped| (dropped.id.as_str(), dropped.tier))
            .collect::<Vec<_>>(),
        vec![("CHG-002", Tier::HISTORY), ("CHG-001", Tier::HISTORY)],
        "same tier, so the lower score is reported first"
    );
    assert!(
        report
            .dropped
            .iter()
            .all(|dropped| dropped.reason == "budget")
    );

    assert!(candidates[0].trace.included, "tier 3 may never be dropped");
    assert!(!candidates[1].trace.included);
    assert_eq!(
        candidates[1].trace.dropped_reason.as_deref(),
        Some("budget")
    );
}

#[test]
fn a_budget_below_the_protected_tiers_is_ctx_030() {
    let (_project, index) = indexed("budget-too-small", &[(".ai/ARCHITECTURE.md", ARCHITECTURE)]);
    let mut candidates = vec![candidate(&index, ".ai/ARCHITECTURE.md", 1.0)];
    let budget = ContextBudget::default().with_max_tokens(1);

    let error = budget
        .apply(&mut candidates, &index)
        .expect_err("tier 3 may never be dropped");

    assert_eq!(error.code().to_string(), "CTX-030");
    assert!(error.to_string().contains("tiers 1-5"), "{error}");
    assert!(
        candidates[0].trace.included,
        "an impossible request is an error, not a silent truncation"
    );
    assert_eq!(candidates[0].trace.dropped_reason, None);
}

#[test]
fn a_budget_cuts_from_the_bottom_of_the_ladder_upwards() {
    let (_project, index) = indexed(
        "budget-order",
        &[
            (".ai/ARCHITECTURE.md", ARCHITECTURE),
            (".ai/decisions/ADR-001-x.md", DECISION),
            (".ai/MEMORY.md", MEMORY),
            (".ai/changes/CHG-001-a.md", LONG_CHANGE),
        ],
    );
    let max = tokens(&index, ".ai/ARCHITECTURE.md") + tokens(&index, ".ai/decisions/ADR-001-x.md");
    let mut candidates = vec![
        candidate(&index, ".ai/ARCHITECTURE.md", 1.0),
        candidate(&index, ".ai/MEMORY.md", 1.0),
        candidate(&index, ".ai/decisions/ADR-001-x.md", 1.0),
        candidate(&index, ".ai/changes/CHG-001-a.md", 1.0),
    ];
    let budget = ContextBudget::default().with_max_tokens(max);

    let report = budget
        .apply(&mut candidates, &index)
        .expect("the protected tiers fit");

    // Tier 6 still fits. Tier 7 and tier 9 do not, and are reported tier 9 first.
    assert_eq!(report.used, max);
    assert_eq!(
        report
            .dropped
            .iter()
            .map(|dropped| dropped.tier)
            .collect::<Vec<_>>(),
        vec![Tier::HISTORY, Tier::MEMORY]
    );
    assert!(candidates[0].trace.included, "tier 3 stays");
    assert!(
        candidates[2].trace.included,
        "tier 6 stays over tier 7 and 9"
    );
    assert!(!candidates[1].trace.included, "tier 7 goes before tier 6");
    assert!(!candidates[3].trace.included, "tier 9 goes first");
}

#[test]
fn a_candidate_is_priced_from_the_index() {
    let (_project, index) = indexed(
        "budget-cost",
        &[
            (".ai/changes/CHG-001-a.md", LONG_CHANGE),
            (".ai/changes/CHG-002-b.md", SHORT_CHANGE),
        ],
    );
    let long = tokens(&index, ".ai/changes/CHG-001-a.md");
    let short = tokens(&index, ".ai/changes/CHG-002-b.md");
    assert!(long > short, "the fixture must have distinct weights");
    let mut candidates = vec![
        candidate(&index, ".ai/changes/CHG-001-a.md", 1.0),
        candidate(&index, ".ai/changes/CHG-002-b.md", 1.0),
    ];
    let budget = ContextBudget::default().with_max_tokens(short);

    let report = budget
        .apply(&mut candidates, &index)
        .expect("the cheaper document fits");

    // Only the index can say the kept document costs `short`, so `used` proves the lookup ran.
    assert_eq!(report.used, short);
    assert_eq!(report.dropped.len(), 1);
    assert_eq!(
        report.dropped[0].id, "CHG-001",
        "the heavier document is cut"
    );
}

#[test]
fn a_per_tier_cap_bounds_a_droppable_tier() {
    let (_project, index) = indexed(
        "budget-cap",
        &[
            (".ai/changes/CHG-001-a.md", LONG_CHANGE),
            (".ai/changes/CHG-002-b.md", SHORT_CHANGE),
        ],
    );
    let cap = tokens(&index, ".ai/changes/CHG-002-b.md");
    let mut candidates = vec![
        candidate(&index, ".ai/changes/CHG-001-a.md", 5.0),
        candidate(&index, ".ai/changes/CHG-002-b.md", 1.0),
    ];
    let budget = ContextBudget {
        max_tokens: DEFAULT_MAX_TOKENS,
        per_tier: [(Tier::HISTORY, cap)].into_iter().collect(),
        reserved_policy_tokens: 0,
    };

    let report = budget
        .apply(&mut candidates, &index)
        .expect("the cap can hold one");

    assert_eq!(report.used, cap);
    assert_eq!(report.dropped.len(), 1);
    assert_eq!(
        report.dropped[0].id, "CHG-001",
        "the score winner is cut because the cap cannot hold it"
    );
}

#[test]
fn a_protected_tier_over_its_cap_is_an_error() {
    let (_project, index) = indexed(
        "budget-cap-protected",
        &[(".ai/ARCHITECTURE.md", ARCHITECTURE)],
    );
    let arch = tokens(&index, ".ai/ARCHITECTURE.md");
    let mut candidates = vec![candidate(&index, ".ai/ARCHITECTURE.md", 1.0)];
    let budget = ContextBudget {
        max_tokens: DEFAULT_MAX_TOKENS,
        per_tier: [(Tier::ARCHITECTURE, arch - 1)].into_iter().collect(),
        reserved_policy_tokens: 0,
    };

    let error = budget
        .apply(&mut candidates, &index)
        .expect_err("a protected tier cannot lose documents to a cap");

    assert_eq!(error.code().to_string(), "CTX-030");
    assert!(candidates[0].trace.included);
}

#[test]
fn applying_a_budget_is_deterministic() {
    let (_project, index) = indexed(
        "budget-determinism",
        &[
            (".ai/ARCHITECTURE.md", ARCHITECTURE),
            (".ai/changes/CHG-001-a.md", LONG_CHANGE),
            (".ai/changes/CHG-002-b.md", SHORT_CHANGE),
        ],
    );
    let budget = ContextBudget::default().with_max_tokens(tokens(&index, ".ai/ARCHITECTURE.md"));

    let mut first_candidates = vec![
        candidate(&index, ".ai/ARCHITECTURE.md", 1.0),
        candidate(&index, ".ai/changes/CHG-001-a.md", 5.0),
        candidate(&index, ".ai/changes/CHG-002-b.md", 1.0),
    ];
    let mut second_candidates = vec![
        candidate(&index, ".ai/ARCHITECTURE.md", 1.0),
        candidate(&index, ".ai/changes/CHG-001-a.md", 5.0),
        candidate(&index, ".ai/changes/CHG-002-b.md", 1.0),
    ];

    let first = budget.apply(&mut first_candidates, &index).expect("fits");
    let second = budget.apply(&mut second_candidates, &index).expect("fits");

    assert_eq!(first, second);
}

#[test]
fn the_report_serialises_to_the_packet_shape() {
    let (_project, index) = indexed(
        "budget-json",
        &[
            (".ai/ARCHITECTURE.md", ARCHITECTURE),
            (".ai/changes/CHG-001-a.md", LONG_CHANGE),
        ],
    );
    let arch = tokens(&index, ".ai/ARCHITECTURE.md");
    let mut candidates = vec![
        candidate(&index, ".ai/ARCHITECTURE.md", 1.0),
        candidate(&index, ".ai/changes/CHG-001-a.md", 1.0),
    ];
    let report = ContextBudget::default()
        .with_max_tokens(arch)
        .apply(&mut candidates, &index)
        .expect("fits");

    let json = serde_json::to_value(&report).expect("a report serialises");

    assert_eq!(json["requested"].as_u64(), Some(u64::from(arch)));
    assert_eq!(json["used"].as_u64(), Some(u64::from(arch)));
    assert_eq!(json["dropped"][0]["id"].as_str(), Some("CHG-001"));
    assert_eq!(json["dropped"][0]["tier"].as_i64(), Some(9));
    assert_eq!(json["dropped"][0]["reason"].as_str(), Some("budget"));
}
