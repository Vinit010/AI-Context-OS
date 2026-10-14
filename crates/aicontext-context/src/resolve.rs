//! The priority ladder and its conflict rule (`docs/CONTEXT_SPEC.md` §7.4, §7.6,
//! `ARCHITECTURE.md` §6).
//!
//! The ladder is fixed: nine rungs, numbered from the caller's instruction (1, the highest priority)
//! to history (9, the lowest). Every document sits on the rung its kind names, and when two documents
//! disagree about a value the smaller tier number wins. This module is the single home for the
//! kind-to-rung mapping; the retrievers stamp their candidates with it rather than carrying a second
//! copy (`RULES.md` §2).
//!
//! Conflict detection covers front-matter assertions: when two documents assert different scalar
//! values for the same project-wide key, the lower tier loses. The higher-tier document is never
//! rewritten (`docs/CONTEXT_SPEC.md` §7.4): the conflict is a value returned here, recorded in the
//! packet by `TASK-037`, and reported by `doctor` as `CTX-013`.
//!
//! Detection is pure and deterministic: it reads only the [`ContextIndex`], orders its output by a
//! total order, and performs no I/O (`ADR-004`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::Value;
use crate::index::{ContextIndex, DocumentKind, IndexEntry};
use crate::retrieval::Tier;

/// The front-matter keys whose value is a claim about the project as a whole, rather than about the
/// document that carries it (`docs/CONTEXT_SPEC.md` §7.4, §7.6).
///
/// Only these keys are compared across documents. A document's own keys - `status`, `phase`,
/// `depends_on`, `spec`, `feature`, and the rest - describe that document and are expected to
/// differ, so comparing every shared key would report a conflict for each pair of documents that do
/// not happen to agree. The list is the one place this policy lives; a key joins it when the
/// project gains a field that genuinely has one value for the whole repository. The spec's own
/// example is `database` (§7.6), and `store` is its structural twin, so those two name the policy
/// today.
pub const ASSERTION_KEYS: &[&str] = &["store", "database"];

/// One document's claim about an asserted key (`docs/CONTEXT_SPEC.md` §7.6).
///
/// The packet carries `{tier, id, value}`; [`path`](Self::path) is held out of band for `doctor`,
/// which reports against a path rather than an id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assertion {
    /// The rung the asserting document sits on.
    pub tier: Tier,
    /// The asserting document's front-matter id, or its path when it has none.
    pub id: String,
    /// The value the document asserts.
    pub value: Value,
    /// The asserting document's repository-relative path.
    ///
    /// Not serialised: the §7.6 packet shape names only `{tier, id, value}`, and the path is here
    /// for the one caller that needs to point at a file.
    #[serde(skip)]
    pub path: PathBuf,
}

/// How a conflict was resolved.
///
/// The ladder defines exactly one rule, so this is a one-variant enum rather than a boolean: the
/// packet names the resolution, and a reader should see the rule the system applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Resolution {
    /// The higher-tier assertion stands. The lower-tier document is recorded and never written
    /// over (`docs/CONTEXT_SPEC.md` §7.4).
    HigherTierWins,
}

/// Two documents asserting different values for the same key (`docs/CONTEXT_SPEC.md` §7.6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    /// The asserted key the two documents disagree about.
    pub key: String,
    /// The winning assertion: the one at the higher tier, meaning the smaller tier number.
    pub asserted_by: Assertion,
    /// The losing assertion: the one at the lower tier, meaning a larger tier number.
    pub conflicts_with: Assertion,
    /// How the ladder resolved it.
    pub resolution: Resolution,
}

/// The fixed ladder and its one conflict rule (`ARCHITECTURE.md` §6).
#[derive(Clone, Copy, Debug, Default)]
pub struct PriorityResolver;

impl PriorityResolver {
    /// The rung a document kind sits on (`docs/CONTEXT_SPEC.md` §7.4).
    #[must_use]
    pub fn tier_for_kind(kind: DocumentKind) -> Tier {
        match kind {
            DocumentKind::MemoryRegister => Tier::MEMORY,
            DocumentKind::TaskRegister | DocumentKind::Task => Tier::TASK,
            DocumentKind::Decision => Tier::DECISION,
            DocumentKind::Specification | DocumentKind::Product => Tier::SPECIFICATION,
            DocumentKind::Architecture | DocumentKind::Design => Tier::ARCHITECTURE,
            DocumentKind::Rules | DocumentKind::Conventions => Tier::POLICY,
            DocumentKind::Bug | DocumentKind::Change => Tier::HISTORY,
            DocumentKind::AgentEntryPoint | DocumentKind::AgentProfile => Tier::INSTRUCTION,
            DocumentKind::ContextNote | DocumentKind::Workflow => Tier::SOURCE,
        }
    }

    /// The rung an indexed entry sits on: its kind's rung, or the source-code rung when the index
    /// could not classify it.
    #[must_use]
    pub fn tier_of(entry: &IndexEntry) -> Tier {
        entry.kind.map_or(Tier::SOURCE, Self::tier_for_kind)
    }

    /// Every front-matter conflict among the indexed documents, over [`ASSERTION_KEYS`]
    /// (`docs/CONTEXT_SPEC.md` §7.4).
    ///
    /// The result is a total order - key, then the winning tier, then both ids - so the same index
    /// always yields the same list (`ADR-004`). A key asserted once, or asserted with the same
    /// value by every document, is not a conflict.
    #[must_use]
    pub fn conflicts(index: &ContextIndex) -> Vec<Conflict> {
        Self::conflicts_on(index, ASSERTION_KEYS)
    }

    /// Conflicts over an explicit key set.
    ///
    /// This is the seam the public default is built on; a caller with a project-specific asserted
    /// key can ask about it directly. The comparison is identical to [`conflicts`](Self::conflicts).
    #[must_use]
    pub fn conflicts_on(index: &ContextIndex, keys: &[&str]) -> Vec<Conflict> {
        let mut grouped: BTreeMap<&str, Vec<Assertion>> = BTreeMap::new();
        for entry in index.entries() {
            let tier = Self::tier_of(entry);
            let id = entry
                .id
                .clone()
                .unwrap_or_else(|| entry.path.display().to_string());
            for key in keys {
                // A list or a map is a collection of claims, not one value, so it does not name a
                // single assertion the ladder can compare. `null` is the absence of a claim
                // (`docs/CONTEXT_SPEC.md` §2 rule 9), not a value two documents can disagree on.
                let Some(value) = entry
                    .front_matter
                    .get(*key)
                    .filter(|value| is_scalar(value))
                else {
                    continue;
                };
                grouped.entry(*key).or_default().push(Assertion {
                    tier,
                    id: id.clone(),
                    value: value.clone(),
                    path: entry.path.clone(),
                });
            }
        }

        let mut conflicts = Vec::new();
        for (key, assertions) in &grouped {
            let Some(winner) = winner(assertions) else {
                continue;
            };
            for other in assertions {
                // The rule is about a *lower* tier contradicting a *higher* one
                // (`docs/CONTEXT_SPEC.md` §7.4), and the ladder numbers the highest tier 1, so a
                // lower tier is the one with the *larger* number. Two documents on the same rung
                // that disagree are not a lower-versus-higher conflict, so neither loses here.
                if other.tier > winner.tier && other.value != winner.value {
                    conflicts.push(Conflict {
                        key: (*key).to_string(),
                        asserted_by: winner.clone(),
                        conflicts_with: other.clone(),
                        resolution: Resolution::HigherTierWins,
                    });
                }
            }
        }
        order_conflicts(&mut conflicts);
        conflicts
    }
}

/// Whether a value names a single assertion the ladder can compare.
///
/// A scalar does; a list, a map, and `null` do not.
fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Str(_) | Value::Int(_) | Value::Bool(_) | Value::Float(_)
    )
}

/// The winning assertion among those for one key: the highest-priority rung, which the ladder numbers
/// as the *smallest* tier, and the smallest id to break a tie between two documents on the same rung.
///
/// The tie-break exists so the output is deterministic even when the rule has no clear winner; it
/// never *reports* a same-rung disagreement, it only decides which assertion the comparison is made
/// against.
fn winner(assertions: &[Assertion]) -> Option<&Assertion> {
    assertions.iter().reduce(|best, candidate| {
        if candidate.tier < best.tier || (candidate.tier == best.tier && candidate.id < best.id) {
            candidate
        } else {
            best
        }
    })
}

/// Orders conflicts by a total order: key, then the winning assertion, then the losing one.
///
/// The two assertions are compared by tier then id, which is unique because an id names its
/// document and a document without one falls back to its path. The asserted *value* is not part of
/// the order, so the sort never depends on how a `Value` compares.
fn order_conflicts(conflicts: &mut [Conflict]) {
    conflicts.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.asserted_by.tier.cmp(&right.asserted_by.tier))
            .then_with(|| left.asserted_by.id.cmp(&right.asserted_by.id))
            .then_with(|| left.conflicts_with.tier.cmp(&right.conflicts_with.tier))
            .then_with(|| left.conflicts_with.id.cmp(&right.conflicts_with.id))
    });
}

#[cfg(test)]
mod tests {
    use super::{ASSERTION_KEYS, PriorityResolver, is_scalar};
    use crate::Value;
    use crate::index::DocumentKind;
    use crate::retrieval::Tier;

    #[test]
    fn every_kind_has_its_rung() {
        let cases = [
            (DocumentKind::AgentEntryPoint, Tier::INSTRUCTION),
            (DocumentKind::AgentProfile, Tier::INSTRUCTION),
            (DocumentKind::Rules, Tier::POLICY),
            (DocumentKind::Conventions, Tier::POLICY),
            (DocumentKind::Architecture, Tier::ARCHITECTURE),
            (DocumentKind::Design, Tier::ARCHITECTURE),
            (DocumentKind::Specification, Tier::SPECIFICATION),
            (DocumentKind::Product, Tier::SPECIFICATION),
            (DocumentKind::TaskRegister, Tier::TASK),
            (DocumentKind::Task, Tier::TASK),
            (DocumentKind::Decision, Tier::DECISION),
            (DocumentKind::MemoryRegister, Tier::MEMORY),
            (DocumentKind::ContextNote, Tier::SOURCE),
            (DocumentKind::Workflow, Tier::SOURCE),
            (DocumentKind::Bug, Tier::HISTORY),
            (DocumentKind::Change, Tier::HISTORY),
        ];
        for (kind, tier) in cases {
            assert_eq!(PriorityResolver::tier_for_kind(kind), tier, "for {kind}");
        }
    }

    #[test]
    fn only_scalars_name_an_assertion() {
        use std::collections::BTreeMap;
        assert!(is_scalar(&Value::Str("x".into())));
        assert!(is_scalar(&Value::Int(1)));
        assert!(is_scalar(&Value::Bool(true)));
        assert!(is_scalar(&Value::Float(1.5)));
        assert!(!is_scalar(&Value::Null));
        assert!(!is_scalar(&Value::List(vec![Value::Int(1)])));
        assert!(!is_scalar(&Value::Map(BTreeMap::default())));
    }

    #[test]
    fn the_assertion_keys_are_the_project_wide_twins() {
        // The list is a policy, and this is the one place it is written down. A change here is a
        // change to what the conflict check compares.
        assert_eq!(ASSERTION_KEYS, &["store", "database"]);
    }
}
