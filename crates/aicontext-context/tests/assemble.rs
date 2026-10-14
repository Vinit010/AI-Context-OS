//! The assembler, exercised through the public API.
//!
//! Mirrors `TASK-037`: a plan and an index become one `ContextPacket` whose `policy` region is tiers
//! 1-2 and whose `retrieved` region is untrusted data, carrying the ladder's conflicts, the budget's
//! report, and a provenance trace for every candidate considered. Tests live out here rather than in
//! the module so they can only reach the public surface.
//!
//! The fixture is a small but complete `.ai/` tree: the always-on entry point and rules, a spec a
//! task links, an architecture document that agrees with the rules, and a change note that does not.
//! That one disagreement is the conflict the ladder has to resolve, and the change note is the
//! droppable candidate a budget has to cut.

use aicontext_context::{
    ContextAssembler, ContextBudget, ContextIndex, DEFAULT_MAX_TOKENS, DocumentKind,
    PACKET_VERSION, PlanOptions, RetrievalPlan, Tier, Trust,
};
use aicontext_testkit::TempProject;

/// The agent entry point, tier 1. No front-matter id, so it is named by its path.
const AI: &str =
    "---\ntype: agent-entry-point\ntitle: AI entry point\n---\n\n# AI\n\nStart here.\n";

/// Project policy, tier 2.
const RULES: &str = "---\nid: RULES-001\ntype: rules\ntitle: Rules\nstore: postgres\n---\n\n\
    # RULES\n\nNever commit without approval.\n";

/// A specification, tier 4.
const SPEC: &str = "---\nid: SPEC-context\ntype: spec\ntitle: Context spec\n---\n\n\
    # Context\n\nThe retrieval budget degrades from the lowest tier upward, so policy is never cut.\n";

/// Architecture, tier 3. It agrees with the rules about `store`, so it is not a conflict.
const ARCHITECTURE: &str = "---\nid: ARCH-001\ntype: architecture\ntitle: Architecture\nstore: postgres\n---\n\n\
    # Architecture\n\nThe context engine is deterministic and offline.\n";

/// History, tier 9. It disagrees with the rules about `store`, which is the conflict.
const CHANGE: &str = "---\nid: CHG-001\ntype: change\ntitle: Change one\nstore: sqlite\n---\n\n\
    # Change one\n\nA history note long enough to be a real candidate on the lowest rung.\n";

/// The task register, tier 5, with one task that links the spec, the change, and an `.ai` glob.
const TASKS: &str = "---\nid: TASKS-001\ntype: tasks\ntitle: Tasks\nstatus: active\n---\n\n\
    **Current phase:** Phase 1\n**Current task:** TASK-001\n\n\
    ### TASK-001 — One\n\n```yaml\nid: TASK-001\ntitle: One\nstatus: TODO\nphase: 1\n\
    spec: .ai/specs/context.md\nrelates_to: [CHG-001]\ntouches: [\".ai/specs/*.md\"]\n```\n";

/// A temporary project holding the whole fixture tree.
fn project() -> TempProject {
    let project = TempProject::new("context-assemble").expect("a temporary project");
    for (relative, contents) in [
        (".ai/AI.md", AI),
        (".ai/RULES.md", RULES),
        (".ai/specs/context.md", SPEC),
        (".ai/architecture/ARCHITECTURE.md", ARCHITECTURE),
        (".ai/changes/CHG-001.md", CHANGE),
        (".ai/TASKS.md", TASKS),
    ] {
        project
            .write(relative, contents)
            .expect("writes a document");
    }
    project
}

/// The index of a fixture, built the way the command builds its first run.
fn index(project: &TempProject) -> ContextIndex {
    let (index, _report) = ContextIndex::build(project.path(), None);
    index
}

/// The plan `--task TASK-001` produces.
fn task_plan(project: &TempProject, options: &PlanOptions) -> RetrievalPlan {
    let register = aicontext_context::register::read(project.path());
    RetrievalPlan::for_task(&register, "TASK-001", options)
}

/// A packet's retrieved ids, for the many assertions that ask "is this document in?".
fn retrieved_ids(packet: &aicontext_context::ContextPacket) -> Vec<&str> {
    packet
        .retrieved
        .iter()
        .map(|item| item.id.as_str())
        .collect()
}

#[test]
fn a_task_seeds_policy_and_its_linked_documents_as_untrusted_data() {
    let project = project();
    let index = index(&project);
    let packet = ContextAssembler::new()
        .assemble(&task_plan(&project, &PlanOptions::default()), &index)
        .expect("assembles");

    assert_eq!(packet.version, PACKET_VERSION);
    assert_eq!(packet.request.task.as_deref(), Some("TASK-001"));
    assert_eq!(packet.request.instruction, None);

    let policy: Vec<&str> = packet.policy.iter().map(|item| item.id.as_str()).collect();
    assert!(packet.policy.iter().all(|item| item.tier.number() <= 2));
    assert!(policy.contains(&".ai/AI.md"), "{policy:?}");
    assert!(policy.contains(&"RULES-001"), "{policy:?}");

    let spec = packet
        .retrieved
        .iter()
        .find(|item| item.id == "SPEC-context")
        .expect("the linked spec is retrieved");
    assert_eq!(spec.tier, Tier::SPECIFICATION);
    assert_eq!(spec.trust, Trust::Untrusted);
    assert_eq!(spec.content_ref.to_string_lossy(), ".ai/specs/context.md");

    assert!(
        packet
            .untrusted_content_ids
            .contains(&"SPEC-context".to_string())
    );
    assert!(
        packet
            .untrusted_content_ids
            .contains(&"CHG-001".to_string())
    );
    assert!(
        packet
            .provenance
            .iter()
            .any(|trace| trace.id == "SPEC-context" && trace.included),
        "every included document has a provenance trace"
    );
}

#[test]
fn a_query_seeds_lexical_retrieval_and_is_echoed_as_the_instruction() {
    let project = project();
    let index = index(&project);
    let options = PlanOptions {
        query: Some("retrieval budget".to_string()),
        ..PlanOptions::default()
    };
    let packet = ContextAssembler::new()
        .assemble(&RetrievalPlan::new(&options), &index)
        .expect("assembles");

    assert_eq!(
        packet.request.instruction.as_deref(),
        Some("retrieval budget")
    );
    assert!(
        retrieved_ids(&packet).contains(&"SPEC-context"),
        "the spec's body mentions the query: {:?}",
        retrieved_ids(&packet)
    );
}

#[test]
fn the_packet_carries_the_ladder_conflicts_and_the_budget_report() {
    let project = project();
    let index = index(&project);
    let packet = ContextAssembler::new()
        .assemble(&task_plan(&project, &PlanOptions::default()), &index)
        .expect("assembles");

    assert_eq!(packet.conflicts.len(), 1, "{:?}", packet.conflicts);
    let conflict = &packet.conflicts[0];
    assert_eq!(conflict.key, "store");
    assert_eq!(conflict.asserted_by.id, "RULES-001");
    assert_eq!(conflict.asserted_by.tier, Tier::POLICY);
    assert_eq!(conflict.conflicts_with.id, "CHG-001");
    assert_eq!(conflict.conflicts_with.tier, Tier::HISTORY);

    assert_eq!(packet.budget.requested, DEFAULT_MAX_TOKENS);
    assert!(packet.budget.used > 0);
    assert!(packet.budget.dropped.is_empty());
}

#[test]
fn a_capped_droppable_tier_is_cut_and_reported() {
    let project = project();
    let index = index(&project);
    let options = PlanOptions {
        budget: Some(ContextBudget {
            max_tokens: DEFAULT_MAX_TOKENS,
            per_tier: [(Tier::HISTORY, 0)].into_iter().collect(),
            reserved_policy_tokens: 0,
        }),
        ..PlanOptions::default()
    };
    let packet = ContextAssembler::new()
        .assemble(&task_plan(&project, &options), &index)
        .expect("assembles");

    assert!(
        packet
            .retrieved
            .iter()
            .all(|item| item.tier != Tier::HISTORY),
        "a tier capped at zero contributes nothing: {:?}",
        retrieved_ids(&packet)
    );
    assert!(
        packet
            .budget
            .dropped
            .iter()
            .any(|dropped| dropped.id == "CHG-001" && dropped.reason == "budget"),
        "{:?}",
        packet.budget.dropped
    );
    let trace = packet
        .provenance
        .iter()
        .find(|trace| trace.id == "CHG-001")
        .expect("the change is still traced");
    assert!(!trace.included);
    assert_eq!(trace.dropped_reason.as_deref(), Some("budget"));
}

#[test]
fn a_budget_too_small_for_the_protected_tiers_is_an_error() {
    let project = project();
    let index = index(&project);
    let options = PlanOptions {
        budget: Some(ContextBudget::default().with_max_tokens(1)),
        ..PlanOptions::default()
    };
    let error = ContextAssembler::new()
        .assemble(&task_plan(&project, &options), &index)
        .expect_err("policy cannot fit one token");

    assert_eq!(error.code().to_string(), "CTX-030");
}

#[test]
fn an_excluded_document_is_dropped_with_a_trace_and_policy_is_never_excluded() {
    let project = project();
    let index = index(&project);
    let options = PlanOptions {
        exclude: vec!["SPEC-context".to_string(), "RULES-001".to_string()],
        ..PlanOptions::default()
    };
    let packet = ContextAssembler::new()
        .assemble(&task_plan(&project, &options), &index)
        .expect("assembles");

    assert!(
        !retrieved_ids(&packet).contains(&"SPEC-context"),
        "an excluded document is not retrieved"
    );
    assert!(
        packet.policy.iter().any(|item| item.id == "RULES-001"),
        "the always-on policy cannot be excluded"
    );
    let trace = packet
        .provenance
        .iter()
        .find(|trace| trace.id == "SPEC-context")
        .expect("the exclusion is traced");
    assert!(!trace.included);
    assert_eq!(trace.dropped_reason.as_deref(), Some("excluded"));
}

#[test]
fn include_kinds_narrows_the_retrieved_set_but_not_the_policy_region() {
    let project = project();
    let index = index(&project);
    let options = PlanOptions {
        include_kinds: vec![DocumentKind::Decision],
        ..PlanOptions::default()
    };
    let packet = ContextAssembler::new()
        .assemble(&task_plan(&project, &options), &index)
        .expect("assembles");

    assert!(
        packet
            .retrieved
            .iter()
            .all(|item| item.tier != Tier::SPECIFICATION),
        "a kind filter drops the spec: {:?}",
        retrieved_ids(&packet)
    );
    assert!(
        !packet.policy.is_empty(),
        "tier 1-2 documents are not filtered by kind"
    );
}

#[test]
fn the_prompt_wraps_retrieved_content_in_an_untrusted_data_envelope() {
    let project = project();
    let index = index(&project);
    let packet = ContextAssembler::new()
        .assemble(&task_plan(&project, &PlanOptions::default()), &index)
        .expect("assembles");
    let prompt = packet.render_prompt(&index);

    assert!(prompt.contains("# policy (trusted, tiers 1-2)"), "{prompt}");
    assert!(
        prompt.contains("----- BEGIN UNTRUSTED DATA | id=SPEC-context"),
        "{prompt}"
    );
    assert!(
        prompt.contains("----- END UNTRUSTED DATA | id=SPEC-context -----"),
        "{prompt}"
    );
    let begin = prompt
        .find("----- BEGIN UNTRUSTED DATA | id=SPEC-context")
        .expect("a begin marker");
    let body = prompt
        .find("The retrieval budget degrades")
        .expect("the spec body is rendered");
    assert!(begin < body, "the content must sit inside the envelope");
}

#[test]
fn assembly_is_deterministic() {
    let project = project();
    let index = index(&project);
    let plan = task_plan(&project, &PlanOptions::default());
    let once = ContextAssembler::new()
        .assemble(&plan, &index)
        .expect("assembles");
    let again = ContextAssembler::new()
        .assemble(&plan, &index)
        .expect("assembles");

    assert_eq!(
        once, again,
        "the same plan and index must produce one packet"
    );
}
