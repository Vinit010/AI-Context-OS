//! The token budget and its degradation order (`docs/CONTEXT_SPEC.md` §7.5, `ARCHITECTURE.md` §6).
//!
//! A budget is a ceiling in tokens and a set of optional per-tier ceilings. Applying it to the
//! candidates a retrieval produced decides which documents the packet can afford: tiers 1–5 are
//! **never** dropped, and everything a budget must cut comes from the lowest-priority end upwards —
//! tier 9 first, then 8, 7, and 6, and within a tier the lowest-scoring document first
//! (`docs/CONTEXT_SPEC.md` §7.5). The decision is pure and deterministic: the same candidates and the
//! same budget always cut the same documents (`decisions/ADR-004-deterministic-retrieval.md`).
//!
//! A budget that cannot fit tiers 1–5 is not a smaller packet, it is an impossible request, so it is
//! an error (`CTX-030`) rather than a silent truncation of policy.
//!
//! [`ContextBudget`] lives here, not in [`crate::retrieval`], because §7.5 is its one home; the plan
//! embeds it and `lib.rs` re-exports it, so the public path is unchanged.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use aicontext_core::{ErrorCode, Severity};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::index::ContextIndex;
use crate::retrieval::{ScoredDocument, Tier};

/// The default token budget, the value `docs/CONTEXT_SPEC.md` §7.6 uses in its example packet.
pub const DEFAULT_MAX_TOKENS: u32 = 32_000;

/// The highest tier that may never be dropped (`docs/CONTEXT_SPEC.md` §7.5).
const PROTECTED_MAX_TIER: u8 = 5;

/// The reason a candidate is dropped by the budget (`docs/CONTEXT_SPEC.md` §7.3).
const DROP_REASON_BUDGET: &str = "budget";

/// How many tokens there are to spend, and how they are reserved (`docs/CONTEXT_SPEC.md` §7.5).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextBudget {
    /// The ceiling for the whole packet.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Optional per-tier ceilings. An absent tier uses the whole budget.
    #[serde(default)]
    pub per_tier: BTreeMap<Tier, u32>,
    /// Tokens held back for tiers 1–2, which are never dropped.
    ///
    /// Tiers 1–2 are kept unconditionally, so this reservation is satisfied by construction and does
    /// not change which documents fit; it names the room a caller should leave for the instruction
    /// and policy it injects outside retrieval. It is carried for the per-model cost policy of a
    /// later phase (`ARCHITECTURE.md` §12).
    #[serde(default)]
    pub reserved_policy_tokens: u32,
}

/// The default for a missing `max_tokens`, so a partial budget still names a number.
fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_tokens: DEFAULT_MAX_TOKENS,
            per_tier: BTreeMap::new(),
            reserved_policy_tokens: 0,
        }
    }
}

impl ContextBudget {
    /// This budget with `max_tokens` overridden, as `aicontext context --budget` does.
    #[must_use]
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    /// Applies the budget to `candidates`, using each document's `token_estimate` from `index`.
    ///
    /// Candidates already dropped by retrieval (`included` is false) are left untouched and cost
    /// nothing. Every candidate a retriever produced comes out of the index, so its path resolves to
    /// an entry; a candidate that somehow does not is treated as weightless rather than guessed at.
    ///
    /// # Errors
    ///
    /// Returns [`BudgetError`] when the never-dropped tiers do not fit the ceiling.
    pub fn apply(
        &self,
        candidates: &mut [ScoredDocument],
        index: &ContextIndex,
    ) -> Result<BudgetReport, BudgetError> {
        self.apply_with(candidates, |candidate| {
            index
                .get(candidate.path.to_string_lossy().as_ref())
                .map_or(0, |entry| entry.token_estimate)
        })
    }

    /// Applies the budget with an explicit token cost per candidate.
    ///
    /// This is the cost seam: `apply` prices a candidate from the index's heuristic estimate, while a
    /// caller with a model-specific tokenizer can price it differently
    /// (`ARCHITECTURE.md` §12). The rule is otherwise identical to [`apply`](Self::apply).
    ///
    /// # Errors
    ///
    /// Returns [`BudgetError`] when the never-dropped tiers do not fit the ceiling.
    pub fn apply_with<F>(
        &self,
        candidates: &mut [ScoredDocument],
        cost: F,
    ) -> Result<BudgetReport, BudgetError>
    where
        F: Fn(&ScoredDocument) -> u32,
    {
        let max = u64::from(self.max_tokens);

        // Tiers 1-5 are never dropped. Sum them first and refuse a request that cannot hold them.
        let mut protected: u64 = 0;
        let mut tier_used: BTreeMap<Tier, u64> = BTreeMap::new();
        for candidate in candidates.iter().filter(|c| is_budgeted(c, None)) {
            let tokens = u64::from(cost(candidate));
            protected += tokens;
            *tier_used.entry(candidate.tier).or_default() += tokens;
        }
        if protected > max {
            return Err(BudgetError::TooSmall {
                needed: to_u32_saturating(protected),
                max_tokens: self.max_tokens,
            });
        }
        for (tier, cap) in &self.per_tier {
            if tier.number() <= PROTECTED_MAX_TIER {
                let used = tier_used.get(tier).copied().unwrap_or(0);
                if used > u64::from(*cap) {
                    return Err(BudgetError::TierTooSmall {
                        tier: *tier,
                        needed: to_u32_saturating(used),
                        cap: *cap,
                    });
                }
            }
        }

        // Tiers 6-9 are admitted highest-priority first: the tier number ascending, then the score
        // descending, then the id and path so the order is total. A candidate is cut when it no
        // longer fits the whole budget or its tier's cap.
        let mut order: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| is_budgeted(candidate, Some(PROTECTED_MAX_TIER)))
            .map(|(index, _)| index)
            .collect();
        order.sort_by(|&left, &right| {
            let left = &candidates[left];
            let right = &candidates[right];
            left.tier
                .cmp(&right.tier)
                .then_with(|| {
                    right
                        .score
                        .partial_cmp(&left.score)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| left.trace.id.cmp(&right.trace.id))
                .then_with(|| left.path.cmp(&right.path))
        });

        let mut used: u64 = protected;
        let mut dropped: Vec<usize> = Vec::new();
        for &index in &order {
            let candidate = &candidates[index];
            let tokens = u64::from(cost(candidate));
            let cap = self
                .per_tier
                .get(&candidate.tier)
                .map_or(u64::MAX, |cap| u64::from(*cap));
            let tier_after = tier_used.get(&candidate.tier).copied().unwrap_or(0) + tokens;
            if used + tokens <= max && tier_after <= cap {
                used += tokens;
                *tier_used.entry(candidate.tier).or_default() += tokens;
            } else {
                dropped.push(index);
            }
        }

        // Report the cuts the way §7.5 describes them: tier 9 first, then downwards, and within a
        // tier the lowest-scoring first.
        dropped.sort_by(|&left, &right| {
            let left = &candidates[left];
            let right = &candidates[right];
            right
                .tier
                .cmp(&left.tier)
                .then_with(|| {
                    left.score
                        .partial_cmp(&right.score)
                        .unwrap_or(Ordering::Equal)
                })
                .then_with(|| left.trace.id.cmp(&right.trace.id))
                .then_with(|| left.path.cmp(&right.path))
        });

        let mut report_dropped = Vec::with_capacity(dropped.len());
        for &index in &dropped {
            candidates[index].trace.included = false;
            candidates[index].trace.dropped_reason = Some(DROP_REASON_BUDGET.to_string());
            let candidate = &candidates[index];
            report_dropped.push(DroppedDocument {
                id: candidate.trace.id.clone(),
                tier: candidate.tier,
                reason: DROP_REASON_BUDGET.to_string(),
            });
        }

        Ok(BudgetReport {
            requested: self.max_tokens,
            used: to_u32_saturating(used),
            dropped: report_dropped,
        })
    }
}

/// Whether a candidate takes part in the budget: retrieval kept it, and its tier is on the side of the
/// ladder the caller is asking about.
///
/// A `floor` of `Some(n)` selects tiers above `n` (the droppable half); `None` selects the protected
/// tiers at or below [`PROTECTED_MAX_TIER`].
fn is_budgeted(candidate: &ScoredDocument, floor: Option<u8>) -> bool {
    if !candidate.trace.included {
        return false;
    }
    let number = candidate.tier.number();
    match floor {
        Some(floor) => number > floor && number <= 9,
        None => number <= PROTECTED_MAX_TIER,
    }
}

/// A token count that overflowed `u32` saturates, because it can only be compared against a ceiling
/// that is itself a `u32`.
fn to_u32_saturating(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// One document the budget refused to spend tokens on (`docs/CONTEXT_SPEC.md` §7.6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DroppedDocument {
    /// The document identifier, or its path when it has none.
    pub id: String,
    /// The rung the document sat on.
    pub tier: Tier,
    /// Why it was dropped.
    pub reason: String,
}

/// What a budget did, the `budget:` block of `docs/CONTEXT_SPEC.md` §7.6.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BudgetReport {
    /// The ceiling used to compute the budget.
    pub requested: u32,
    /// The total tokens of the documents kept.
    pub used: u32,
    /// The documents the budget cut.
    #[serde(default)]
    pub dropped: Vec<DroppedDocument>,
}

/// The budget cannot hold the documents that may never be dropped.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum BudgetError {
    /// Tiers 1–5 need more tokens than the whole budget allows.
    #[error(
        "tiers 1-5 need {needed} tokens but the budget is {max_tokens}; \
         raise the budget or shrink a protected document"
    )]
    TooSmall {
        /// Tokens the protected tiers need.
        needed: u32,
        /// The ceiling that could not hold them.
        max_tokens: u32,
    },
    /// A protected tier needs more tokens than its per-tier cap allows.
    #[error("tier {tier} needs {needed} tokens but its cap is {cap}; raise the cap or shrink it")]
    TierTooSmall {
        /// The tier whose cap was exceeded.
        tier: Tier,
        /// Tokens the tier needs.
        needed: u32,
        /// The cap that could not hold them.
        cap: u32,
    },
}

impl BudgetError {
    /// The stable code for this failure, `CTX-030` in `docs/CONTEXT_SPEC.md` §8.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        ErrorCode::CTX_030
    }

    /// How serious this failure is. The request cannot be satisfied, so it is an error.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        Severity::Error
    }

    /// What the caller should do about it.
    #[must_use]
    pub const fn remediation(&self) -> &'static str {
        "raise the token budget, adjust the tier caps, or shrink the protected documents"
    }
}

#[cfg(test)]
mod tests {
    use super::{BudgetError, ContextBudget, DroppedDocument};
    use crate::retrieval::{RetrievalTrace, ScoredDocument, Tier};

    /// A candidate in `tier`, scoring `score`, costing `tokens` by the test's own cost function.
    fn candidate(id: &str, tier: Tier, score: f64) -> ScoredDocument {
        ScoredDocument {
            id: Some(id.to_string()),
            path: format!(".ai/{id}.md").into(),
            tier,
            score,
            reason: "test".to_string(),
            trace: RetrievalTrace {
                id: id.to_string(),
                tier,
                score,
                reason: "test".to_string(),
                source_path: format!(".ai/{id}.md").into(),
                included: true,
                dropped_reason: None,
            },
        }
    }

    /// Every candidate costs 1 token, so the counts read as document counts.
    fn unit(_: &ScoredDocument) -> u32 {
        1
    }

    #[test]
    fn protected_tiers_are_kept_and_lower_tiers_are_cut_from_the_bottom() {
        let mut candidates = vec![
            candidate("ARCH", Tier::ARCHITECTURE, 1.0),
            candidate("DEC", Tier::DECISION, 1.0),
            candidate("MEM", Tier::MEMORY, 1.0),
            candidate("CHG", Tier::HISTORY, 1.0),
        ];
        let budget = ContextBudget::default().with_max_tokens(2);

        let report = budget.apply_with(&mut candidates, unit).expect("fits");

        assert_eq!(report.requested, 2);
        assert_eq!(report.used, 2);
        // Tier 9 goes first, then tier 7; tier 3 and tier 6 stay.
        assert_eq!(
            report
                .dropped
                .iter()
                .map(|d| (d.id.as_str(), d.tier))
                .collect::<Vec<_>>(),
            vec![("CHG", Tier::HISTORY), ("MEM", Tier::MEMORY)]
        );
        assert!(!candidates[3].trace.included);
        assert_eq!(
            candidates[3].trace.dropped_reason.as_deref(),
            Some("budget")
        );
        assert!(candidates[0].trace.included);
        assert!(candidates[1].trace.included);
    }

    #[test]
    fn within_a_tier_the_lowest_score_is_cut_first() {
        let mut candidates = vec![
            candidate("A", Tier::HISTORY, 5.0),
            candidate("B", Tier::HISTORY, 1.0),
            candidate("C", Tier::HISTORY, 3.0),
        ];
        let budget = ContextBudget::default().with_max_tokens(1);

        let report = budget.apply_with(&mut candidates, unit).expect("fits");

        assert_eq!(
            report
                .dropped
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "C"]
        );
    }

    #[test]
    fn a_budget_too_small_for_the_protected_tiers_is_an_error() {
        let mut candidates = vec![candidate("RULE", Tier::POLICY, 1.0)];
        let budget = ContextBudget::default().with_max_tokens(0);

        let error = budget
            .apply_with(&mut candidates, unit)
            .expect_err("tier 2 cannot be dropped");

        assert_eq!(
            error,
            BudgetError::TooSmall {
                needed: 1,
                max_tokens: 0
            }
        );
        assert_eq!(error.code().to_string(), "CTX-030");
        assert!(candidates[0].trace.included, "nothing may be dropped");
        assert_eq!(candidates[0].trace.dropped_reason, None);
    }

    #[test]
    fn a_protected_tier_over_its_cap_is_an_error() {
        let mut candidates = vec![candidate("RULE", Tier::POLICY, 1.0)];
        let budget = ContextBudget {
            max_tokens: 100,
            per_tier: [(Tier::POLICY, 0u32)].into_iter().collect(),
            reserved_policy_tokens: 0,
        };

        let error = budget
            .apply_with(&mut candidates, unit)
            .expect_err("the cap cannot be honoured");

        assert_eq!(
            error,
            BudgetError::TierTooSmall {
                tier: Tier::POLICY,
                needed: 1,
                cap: 0
            }
        );
    }

    #[test]
    fn a_per_tier_cap_bounds_a_droppable_tier() {
        let mut candidates = vec![
            candidate("H1", Tier::HISTORY, 3.0),
            candidate("H2", Tier::HISTORY, 2.0),
            candidate("H3", Tier::HISTORY, 1.0),
        ];
        let budget = ContextBudget {
            max_tokens: 100,
            per_tier: [(Tier::HISTORY, 2u32)].into_iter().collect(),
            reserved_policy_tokens: 0,
        };

        let report = budget.apply_with(&mut candidates, unit).expect("fits");

        assert_eq!(report.used, 2);
        assert_eq!(report.dropped.len(), 1);
        assert_eq!(report.dropped[0].id, "H3");
    }

    #[test]
    fn a_candidate_already_dropped_by_retrieval_is_ignored() {
        let mut candidates = vec![candidate("BROKEN", Tier::HISTORY, 1.0)];
        candidates[0].trace.included = false;
        candidates[0].trace.dropped_reason = Some("broken link".to_string());
        let budget = ContextBudget::default().with_max_tokens(0);

        let report = budget
            .apply_with(&mut candidates, unit)
            .expect("an excluded candidate is not budgeted");

        assert_eq!(report.used, 0);
        assert!(report.dropped.is_empty());
        assert_eq!(
            candidates[0].trace.dropped_reason.as_deref(),
            Some("broken link"),
            "the budget does not overwrite a retrieval reason"
        );
    }

    #[test]
    fn the_report_serialises_to_the_packet_shape() {
        let report = super::BudgetReport {
            requested: 32_000,
            used: 21_400,
            dropped: vec![DroppedDocument {
                id: "CHG-004".to_string(),
                tier: Tier::HISTORY,
                reason: "budget".to_string(),
            }],
        };

        let json = serde_json::to_value(&report).expect("a report serialises");
        assert_eq!(json["requested"].as_u64(), Some(32_000));
        assert_eq!(json["used"].as_u64(), Some(21_400));
        assert_eq!(json["dropped"][0]["id"].as_str(), Some("CHG-004"));
        assert_eq!(json["dropped"][0]["tier"].as_i64(), Some(9));
        assert_eq!(json["dropped"][0]["reason"].as_str(), Some("budget"));
    }
}
