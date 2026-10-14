//! The retrieval plan and the retrievers that satisfy it (`docs/CONTEXT_SPEC.md` §7.1–§7.5,
//! `ARCHITECTURE.md` §4.2 and §6).
//!
//! A plan is a value, not an action. It says which documents are mandatory, which graph edges the
//! current task opens, which front-matter predicates and path globs the caller asked for, and how
//! many tokens there are to spend. It reads no files itself: the task's own keys come from the
//! [`Register`] that already read `TASKS.md`, and every other document is reached later, by a
//! retriever, through the [`ContextIndex`](crate::ContextIndex). Keeping derivation free of I/O is
//! what makes `--explain` trustworthy: it prints what the plan *is*, not what a scan happened to
//! find.
//!
//! The plan is fully serialisable, so a test can assert on the exact bytes and `context --explain`
//! can print it (`docs/CONTEXT_SPEC.md` §7.1).
//!
//! A [`Retriever`] turns a plan and an index into scored candidates. Retrieval is pure and
//! deterministic — no network, no embeddings, no model — so the same plan and index always produce
//! the same candidates (`decisions/ADR-004-deterministic-retrieval.md`). [`LexicalRetriever`] scores
//! with BM25 over a field-weighted document (§7.2); [`LinkedRetriever`], [`FilterRetriever`], and
//! [`GlobRetriever`] follow the task graph, the front-matter predicates, and the path globs — all
//! exact, not ranked (`TASK-034`).

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::budget::ContextBudget;
use crate::index::DocumentKind;
use crate::register::{Register, TaskEntry};
use crate::resolve::PriorityResolver;
use crate::{Document, Value};

/// The agent entry point, always mandatory (`docs/CONTEXT_SPEC.md` §7.1).
pub const MANDATORY_ENTRY_POINT: &str = ".ai/AI.md";

/// Project policy, always mandatory (`docs/CONTEXT_SPEC.md` §7.4, tier 2).
pub const MANDATORY_RULES: &str = ".ai/RULES.md";

/// One rung of the fixed priority ladder (`docs/CONTEXT_SPEC.md` §7.4, `ARCHITECTURE.md` §6).
///
/// A number from 1 (the caller's instruction) to 9 (history). Tier 1 is not a document, but it is
/// part of the ladder and the budget reserves room for it, so the numbering is kept whole rather
/// than starting at the first document.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tier(u8);

impl Tier {
    /// Tier 1 — the current user instruction.
    pub const INSTRUCTION: Self = Self(1);
    /// Tier 2 — project policy: `RULES.md`, `CONVENTIONS.md`.
    pub const POLICY: Self = Self(2);
    /// Tier 3 — architecture: `ARCHITECTURE.md`, `DESIGN.md`.
    pub const ARCHITECTURE: Self = Self(3);
    /// Tier 4 — specification: `specs/`.
    pub const SPECIFICATION: Self = Self(4);
    /// Tier 5 — the current task.
    pub const TASK: Self = Self(5);
    /// Tier 6 — accepted decisions.
    pub const DECISION: Self = Self(6);
    /// Tier 7 — memory.
    pub const MEMORY: Self = Self(7);
    /// Tier 8 — source code.
    pub const SOURCE: Self = Self(8);
    /// Tier 9 — history: changes, superseded decisions, `git log`.
    pub const HISTORY: Self = Self(9);

    /// The tier numbered `number`, or `None` when it is outside 1 through 9.
    #[must_use]
    pub const fn new(number: u8) -> Option<Self> {
        if matches!(number, 1..=9) {
            Some(Self(number))
        } else {
            None
        }
    }

    /// The tier's number, 1 through 9.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// How a [`FrontMatterPredicate`] compares a document's key to its value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PredicateOp {
    /// The key is present and equals the value.
    Equals,
    /// The key is present and does not equal the value.
    NotEquals,
    /// The value is an item of the key's list, or a substring of the key's string.
    Contains,
    /// The negation of [`PredicateOp::Contains`].
    NotContains,
    /// The key is present, whatever its value.
    Exists,
    /// The key is absent.
    NotExists,
}

/// One front-matter test a retriever applies (`docs/CONTEXT_SPEC.md` §7.1).
///
/// The value is text rather than a [`Value`], because a predicate arrives from a command line
/// (`--filter key=value`) and because a [`Value`] is only ever read out of a document
/// (`crate::value`). Comparison projects the document's value to the same text form: a string is
/// itself, a number is its decimal, a boolean is `true` or `false`, and `null` is the empty string.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrontMatterPredicate {
    /// The front-matter key to test.
    pub key: String,
    /// How to compare it.
    pub op: PredicateOp,
    /// The text to compare against. Required by every operator except `exists` and `not-exists`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl FrontMatterPredicate {
    /// A predicate that `key` equals `value`.
    #[must_use]
    pub fn equals(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            op: PredicateOp::Equals,
            value: Some(value.into()),
        }
    }

    /// A predicate that `key`'s list contains, or string contains, `value`.
    #[must_use]
    pub fn contains(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            op: PredicateOp::Contains,
            value: Some(value.into()),
        }
    }

    /// A predicate that `key` is present.
    #[must_use]
    pub fn exists(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            op: PredicateOp::Exists,
            value: None,
        }
    }

    /// A predicate that `key` is absent.
    #[must_use]
    pub fn not_exists(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            op: PredicateOp::NotExists,
            value: None,
        }
    }

    /// Whether a document's front matter satisfies this predicate.
    #[must_use]
    pub fn matches_document(&self, document: &Document) -> bool {
        self.matches(&fields_of(document))
    }

    /// Whether a map of front-matter fields satisfies this predicate.
    #[must_use]
    pub fn matches(&self, fields: &BTreeMap<String, Value>) -> bool {
        let actual = fields.get(&self.key);
        match self.op {
            PredicateOp::Exists => actual.is_some(),
            PredicateOp::NotExists => actual.is_none(),
            PredicateOp::Equals => actual.is_some_and(|actual| self.value_equals(actual)),
            PredicateOp::NotEquals => !actual.is_some_and(|actual| self.value_equals(actual)),
            PredicateOp::Contains => actual.is_some_and(|actual| self.value_contains(actual)),
            PredicateOp::NotContains => !actual.is_some_and(|actual| self.value_contains(actual)),
        }
    }

    /// The text this predicate compares against, or the empty string when it has none.
    fn expected(&self) -> &str {
        self.value.as_deref().unwrap_or_default()
    }

    /// Whether a value projects to the predicate's expected text.
    fn value_equals(&self, actual: &Value) -> bool {
        match actual {
            Value::Str(text) => text == self.expected(),
            Value::Int(number) => number.to_string() == self.expected(),
            Value::Bool(flag) => flag.to_string() == self.expected(),
            Value::Float(number) => number.to_string() == self.expected(),
            Value::Null => self.expected().is_empty() || self.expected() == "null",
            Value::List(_) | Value::Map(_) => false,
        }
    }

    /// Whether a value contains the predicate's expected text, as a list item or a substring.
    fn value_contains(&self, actual: &Value) -> bool {
        match actual {
            Value::Str(text) => text.contains(self.expected()),
            Value::List(items) => items.iter().any(|item| self.value_equals(item)),
            _ => false,
        }
    }
}

/// The front-matter fields of a document, in the canonical key order of rule 4.
fn fields_of(document: &Document) -> BTreeMap<String, Value> {
    document
        .front_matter()
        .map(|front| front.ordered_entries().into_iter().collect())
        .unwrap_or_default()
}

/// A count as a float. Token counts are far below 2^53, so the conversion is exact; a count that
/// somehow exceeded `u32` saturates rather than wrapping.
fn count_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Split text into lower-cased terms of ASCII alphanumerics and `-`, dropping every other
/// character. Used for both the query and every scored field, so a term matches only itself.
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            buf.push(ch.to_ascii_lowercase());
        } else if !buf.is_empty() {
            tokens.push(buf.clone());
            buf.clear();
        }
    }
    if !buf.is_empty() {
        tokens.push(buf);
    }
    tokens
}

/// How a linked id was reached from the task graph (`docs/CONTEXT_SPEC.md` §4).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum LinkedEdge {
    /// The task's `spec`: a `SPEC-*` id or a document path.
    Spec,
    /// A task this one depends on.
    DependsOn,
    /// A bug or change this task relates to.
    RelatesTo,
}

impl fmt::Display for LinkedEdge {
    /// The YAML key the edge came from, so a reason reads `linked from TASK-034 (depends_on)`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Spec => "spec",
            Self::DependsOn => "depends_on",
            Self::RelatesTo => "relates_to",
        })
    }
}

/// One graph edge from the current task, before it is resolved to a document
/// (`docs/CONTEXT_SPEC.md` §7.1).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinkedId {
    /// The referenced id or path, exactly as the task wrote it.
    pub id: String,
    /// How the task reached the reference.
    pub edge: LinkedEdge,
}

impl LinkedId {
    /// An edge that names the task's spec, a `SPEC-*` id or a document path.
    #[must_use]
    pub fn spec(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            edge: LinkedEdge::Spec,
        }
    }

    /// An edge that names a task this one depends on.
    #[must_use]
    pub fn depends_on(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            edge: LinkedEdge::DependsOn,
        }
    }

    /// An edge that names a bug or change this task relates to.
    #[must_use]
    pub fn relates_to(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            edge: LinkedEdge::RelatesTo,
        }
    }
}

/// What a context request asks for, before retrieval runs (`docs/CONTEXT_SPEC.md` §7.1).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrievalPlan {
    /// The task the plan was seeded from, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The free-text seed, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// Documents that are always included: `AI.md`, `RULES.md`, and any `--include`.
    #[serde(default)]
    pub mandatory: Vec<PathBuf>,
    /// Graph edges from the current task, before they are resolved to documents
    /// (`docs/CONTEXT_SPEC.md` §4).
    #[serde(default)]
    pub linked: Vec<LinkedId>,
    /// Front-matter predicates the caller asked for.
    #[serde(default)]
    pub filters: Vec<FrontMatterPredicate>,
    /// Path globs the caller asked for, such as a task's `touches`.
    #[serde(default)]
    pub globs: Vec<String>,
    /// Document kinds the caller wants kept.
    #[serde(default)]
    pub include_kinds: Vec<DocumentKind>,
    /// IDs or paths the caller wants dropped.
    #[serde(default)]
    pub exclude_ids: Vec<String>,
    /// The token budget.
    #[serde(default)]
    pub budget: ContextBudget,
}

/// A trace for a single candidate document considered during retrieval
/// (`docs/CONTEXT_SPEC.md` §7.3).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RetrievalTrace {
    /// Document identifier, or the source path when no id is present.
    pub id: String,
    /// The tier this candidate was associated with when considered.
    pub tier: Tier,
    /// The score assigned to this candidate.
    pub score: f64,
    /// A human-readable reason this candidate was considered.
    pub reason: String,
    /// Path to the document source.
    pub source_path: PathBuf,
    /// Whether the candidate was included in the final packet.
    pub included: bool,
    /// If not included, the reason it was dropped (`budget`, `excluded`, `lower tier`, `denied`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_reason: Option<String>,
}

/// A document that has been scored by a retriever.
#[derive(Clone, Debug)]
pub struct ScoredDocument {
    /// The document's front-matter id, when present.
    pub id: Option<String>,
    /// The repository-relative path to the document.
    pub path: PathBuf,
    /// The tier assigned to this document.
    pub tier: Tier,
    /// The score assigned by the retriever.
    pub score: f64,
    /// A human-readable reason summarising the match.
    pub reason: String,
    /// The full trace for this candidate.
    pub trace: RetrievalTrace,
}

/// A retriever produces candidates for the current plan, given the content index.
///
/// A retriever is pure: it reads the index it is handed and returns candidates. It performs no I/O
/// and calls no model, so the same plan and index always produce the same candidates
/// (`decisions/ADR-004-deterministic-retrieval.md`).
pub trait Retriever {
    /// Retrieve scored documents for `plan` using `index`.
    fn retrieve(
        &self,
        plan: &RetrievalPlan,
        index: &crate::index::ContextIndex,
    ) -> Vec<ScoredDocument>;
}

/// One field's tokens and its weight.
struct WeightedField {
    weight: f64,
    tokens: Vec<String>,
}

/// A document's searchable fields, tokenized once, with its weighted length.
struct DocumentFields {
    fields: Vec<WeightedField>,
    weighted_length: f64,
}

impl DocumentFields {
    /// Tokenize every scored field of an index entry.
    fn from_entry(entry: &crate::index::IndexEntry) -> Self {
        let mut fields = Vec::with_capacity(6);

        let title = entry.title.clone().unwrap_or_default();
        fields.push(WeightedField {
            weight: 3.0,
            tokens: tokenize(&title),
        });

        let id = entry.id.clone().unwrap_or_default();
        fields.push(WeightedField {
            weight: 2.5,
            tokens: tokenize(&id),
        });

        let tags = entry
            .front_matter
            .get("tags")
            .and_then(value_text)
            .unwrap_or_default();
        fields.push(WeightedField {
            weight: 2.0,
            tokens: tokenize(&tags),
        });

        let headings = entry
            .headings
            .iter()
            .map(|heading| heading.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        fields.push(WeightedField {
            weight: 1.5,
            tokens: tokenize(&headings),
        });

        let mut front_matter = String::new();
        for (key, value) in &entry.front_matter {
            // `title`, `id`, and `tags` have their own weighted field, so they are not counted
            // again here.
            if matches!(key.as_str(), "title" | "id" | "tags") {
                continue;
            }
            if let Some(text) = value_text(value) {
                front_matter.push_str(&text);
                front_matter.push(' ');
            }
        }
        fields.push(WeightedField {
            weight: 1.2,
            tokens: tokenize(&front_matter),
        });

        fields.push(WeightedField {
            weight: 1.0,
            tokens: tokenize(&entry.body),
        });

        let weighted_length = fields
            .iter()
            .map(|field| field.weight * count_f64(field.tokens.len()))
            .sum();
        Self {
            fields,
            weighted_length,
        }
    }

    /// The weighted term frequency of `term`, summed across fields.
    fn weighted_tf(&self, term: &str) -> f64 {
        self.fields
            .iter()
            .map(|field| {
                let tf = count_f64(
                    field
                        .tokens
                        .iter()
                        .filter(|token| token.as_str() == term)
                        .count(),
                );
                field.weight * tf
            })
            .sum()
    }

    /// Whether any field contains `term`.
    fn contains(&self, term: &str) -> bool {
        self.fields
            .iter()
            .any(|field| field.tokens.iter().any(|token| token == term))
    }
}

/// BM25F lexical retriever over headings and body with per-field boosts
/// (`docs/CONTEXT_SPEC.md` §7.2).
///
/// Deterministic and pure: no network, no embeddings, no model call, no randomness, and no
/// dependence on hash-map iteration order. Results are ordered by score descending, then by ID
/// ascending.
#[derive(Clone, Debug)]
pub struct LexicalRetriever {
    k1: f64,
    b: f64,
}

impl Default for LexicalRetriever {
    fn default() -> Self {
        Self::new()
    }
}

impl LexicalRetriever {
    /// The specification's parameters: `k1 = 1.2`, `b = 0.75` (`docs/CONTEXT_SPEC.md` §7.2).
    #[must_use]
    pub const fn new() -> Self {
        Self { k1: 1.2, b: 0.75 }
    }

    /// The same retriever with explicit parameters, for a caller that must match a host exactly.
    #[must_use]
    pub const fn with_params(k1: f64, b: f64) -> Self {
        Self { k1, b }
    }
}

impl Retriever for LexicalRetriever {
    fn retrieve(
        &self,
        plan: &RetrievalPlan,
        index: &crate::index::ContextIndex,
    ) -> Vec<ScoredDocument> {
        let query = plan.query.as_deref().unwrap_or("").trim();
        if query.is_empty() {
            return Vec::new();
        }
        let query_terms = dedup(tokenize(query));
        if query_terms.is_empty() {
            return Vec::new();
        }

        // Build the corpus once, in path order, so IDF and the average length are computed over the
        // same documents the results come from.
        let documents: Vec<(&crate::index::IndexEntry, DocumentFields)> = index
            .entries_map()
            .map(|(_key, entry)| (entry, DocumentFields::from_entry(entry)))
            .collect();
        let count = count_f64(documents.len());
        if count == 0.0 {
            return Vec::new();
        }
        let average_length = documents
            .iter()
            .map(|(_, fields)| fields.weighted_length)
            .sum::<f64>()
            / count;

        let mut idf: BTreeMap<String, f64> = BTreeMap::new();
        for term in &query_terms {
            let document_frequency = count_f64(
                documents
                    .iter()
                    .filter(|(_, fields)| fields.contains(term))
                    .count(),
            );
            idf.insert(
                term.clone(),
                (1.0 + (count - document_frequency + 0.5) / (document_frequency + 0.5)).ln(),
            );
        }

        let mut scored: Vec<ScoredDocument> = Vec::new();
        for (entry, fields) in &documents {
            let mut score = 0.0;
            let mut matched: Vec<&str> = Vec::new();
            for term in &query_terms {
                let tf = fields.weighted_tf(term);
                if tf <= 0.0 {
                    continue;
                }
                matched.push(term);
                let term_idf = idf.get(term).copied().unwrap_or(0.0);
                let length_norm = 1.0 - self.b
                    + self.b * fields.weighted_length / average_length.max(f64::EPSILON);
                let denominator = (tf + self.k1 * length_norm).max(f64::EPSILON);
                score += term_idf * (tf * (self.k1 + 1.0)) / denominator;
            }
            if matched.is_empty() {
                continue;
            }

            let path = entry.path.clone();
            let name = entry
                .id
                .clone()
                .unwrap_or_else(|| path.display().to_string());
            let tier = PriorityResolver::tier_of(entry);
            let reason = format!(
                "lexical match on {}",
                matched
                    .iter()
                    .map(|term| format!("'{term}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let trace = RetrievalTrace {
                id: name,
                tier,
                score,
                reason: reason.clone(),
                source_path: path.clone(),
                included: true,
                dropped_reason: None,
            };
            scored.push(ScoredDocument {
                id: entry.id.clone(),
                path,
                tier,
                score,
                reason,
                trace,
            });
        }

        order_candidates(&mut scored);
        scored
    }
}

/// A scored, included candidate for `entry` with a uniform `score` and a human-readable `reason`.
///
/// Shared by the exact retrievers: the graph, filter, and glob retrievers all stamp the document's
/// tier from its kind (like the lexical retriever) and build the same trace shape.
fn candidate_for(entry: &crate::index::IndexEntry, score: f64, reason: String) -> ScoredDocument {
    let tier = PriorityResolver::tier_of(entry);
    let name = entry
        .id
        .clone()
        .unwrap_or_else(|| entry.path.display().to_string());
    let trace = RetrievalTrace {
        id: name,
        tier,
        score,
        reason: reason.clone(),
        source_path: entry.path.clone(),
        included: true,
        dropped_reason: None,
    };
    ScoredDocument {
        id: entry.id.clone(),
        path: entry.path.clone(),
        tier,
        score,
        reason,
        trace,
    }
}

/// Orders candidates by score descending, then by identifier ascending, then by path — the total
/// order every retriever returns, so merged results stay reproducible (`ADR-004`).
///
/// Shared with [`crate::assemble`], which merges what the retrievers returned and must preserve the
/// same order across the merged set.
pub(crate) fn order_candidates(candidates: &mut [ScoredDocument]) {
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// The tier a broken graph edge's target would have sat at had it resolved, by the edge's kind
/// (`docs/CONTEXT_SPEC.md` §7.4): a spec is tier 4, a dependency is tier 5, a bug or change tier 9.
fn edge_tier(edge: LinkedEdge) -> Tier {
    match edge {
        LinkedEdge::Spec => Tier::SPECIFICATION,
        LinkedEdge::DependsOn => Tier::TASK,
        LinkedEdge::RelatesTo => Tier::HISTORY,
    }
}

/// Follows the current task's graph edges (`docs/CONTEXT_SPEC.md` §4) to the documents they name.
///
/// An edge resolves forward, through the index: a front-matter `id`, a repository path, or a
/// heading that defines an inline entity in a register — so a `depends_on` on an inline task in
/// `TASKS.md` resolves to the register that defines it. Resolution is not matching against every
/// document that happens to mention the id. Every resolved edge scores 1.0 and the reason says
/// which edge it was. An edge that resolves to nothing is reported as a broken link — a dropped
/// trace with `dropped_reason: "broken link"` — never skipped (`RULES.md` §4.8).
#[derive(Clone, Copy, Debug, Default)]
pub struct LinkedRetriever;

impl Retriever for LinkedRetriever {
    fn retrieve(
        &self,
        plan: &RetrievalPlan,
        index: &crate::index::ContextIndex,
    ) -> Vec<ScoredDocument> {
        let from = plan.task.as_deref().unwrap_or("the current task");
        let mut candidates = Vec::new();
        for linked in &plan.linked {
            let resolved = index.resolve(&linked.id);
            if resolved.is_empty() {
                let tier = edge_tier(linked.edge);
                let reason = format!(
                    "linked from {from} ({}): '{}' resolves to no indexed document",
                    linked.edge, linked.id
                );
                let trace = RetrievalTrace {
                    id: linked.id.clone(),
                    tier,
                    score: 0.0,
                    reason: reason.clone(),
                    source_path: PathBuf::new(),
                    included: false,
                    dropped_reason: Some("broken link".to_string()),
                };
                candidates.push(ScoredDocument {
                    id: Some(linked.id.clone()),
                    path: PathBuf::new(),
                    tier,
                    score: 0.0,
                    reason,
                    trace,
                });
                continue;
            }
            for entry in resolved {
                let reason = format!("linked from {from} ({})", linked.edge);
                candidates.push(candidate_for(entry, 1.0, reason));
            }
        }
        order_candidates(&mut candidates);
        candidates
    }
}

/// Keeps every document whose front matter satisfies all of the plan's predicates
/// (`docs/CONTEXT_SPEC.md` §7.1).
///
/// The predicates form a conjunction: a document must match every one to be a candidate. Each
/// matched predicate is named in the reason, so an `--explain` reader sees exactly what was asked
/// for. Not ranked: every match scores 1.0.
#[derive(Clone, Copy, Debug, Default)]
pub struct FilterRetriever;

impl Retriever for FilterRetriever {
    fn retrieve(
        &self,
        plan: &RetrievalPlan,
        index: &crate::index::ContextIndex,
    ) -> Vec<ScoredDocument> {
        if plan.filters.is_empty() {
            return Vec::new();
        }
        let reason = format!("matches {}", {
            let parts: Vec<String> = plan.filters.iter().map(describe_filter).collect();
            parts.join(", ")
        });
        let mut candidates = Vec::new();
        for entry in index.entries() {
            if plan
                .filters
                .iter()
                .all(|predicate| predicate.matches(&entry.front_matter))
            {
                candidates.push(candidate_for(entry, 1.0, reason.clone()));
            }
        }
        order_candidates(&mut candidates);
        candidates
    }
}

/// Keeps every document whose repository path matches one of the plan's globs (a task's `touches`,
/// plus any `--globs`).
///
/// A glob is matched against the canonical, `/`-separated repository-relative path of each indexed
/// document. The supported syntax is the subset a `touches` list needs: `*` within a path segment,
/// `**` across whole segments (zero or more), and `?` for one character. A glob that matches
/// nothing is not itself an error — with `touches` covering source directories this is the rule
/// rather than the exception — and the reason names the first glob, in plan order, that matched.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobRetriever;

impl Retriever for GlobRetriever {
    fn retrieve(
        &self,
        plan: &RetrievalPlan,
        index: &crate::index::ContextIndex,
    ) -> Vec<ScoredDocument> {
        if plan.globs.is_empty() {
            return Vec::new();
        }
        let mut candidates = Vec::new();
        for (key, entry) in index.entries_map() {
            let Some(glob) = plan.globs.iter().find(|glob| matches_glob(glob, key)) else {
                continue;
            };
            let reason = format!("matches glob '{glob}'");
            candidates.push(candidate_for(entry, 1.0, reason));
        }
        order_candidates(&mut candidates);
        candidates
    }
}

/// Whether a repository path, in canonical `/`-separated form, matches a glob pattern.
///
/// `*` matches any run of characters inside one path segment, `?` matches exactly one, and `**`
/// matches zero or more whole segments. Every other character matches itself. Paths are matched
/// case-sensitively and the comparison depends on nothing but the two strings, so it runs
/// identically on every host — no filesystem, no glob crate.
fn matches_glob(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    let path: Vec<&str> = path.split('/').collect();
    match_segments(&pattern, &path)
}

/// The segment-by-segment comparison behind [`matches_glob`].
fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.first() {
        None => path.is_empty(),
        Some(segment) if *segment == "**" => {
            match_segments(&pattern[1..], path) || match_segments(pattern, &path[1..])
        }
        Some(segment) => {
            path.first()
                .is_some_and(|part| match_segment(segment, part))
                && match_segments(&pattern[1..], &path[1..])
        }
    }
}

/// Whether a single path segment matches a single pattern segment.
fn match_segment(pattern: &str, segment: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let segment: Vec<char> = segment.chars().collect();
    match_chars(&pattern, &segment)
}

/// The character-level comparison behind [`match_segment`], so `*` and `?` never cross a `/`.
fn match_chars(pattern: &[char], segment: &[char]) -> bool {
    match pattern.first() {
        None => segment.is_empty(),
        Some('*') => {
            match_chars(&pattern[1..], segment)
                || (!segment.is_empty() && match_chars(pattern, &segment[1..]))
        }
        Some('?') => !segment.is_empty() && match_chars(&pattern[1..], &segment[1..]),
        Some(ch) => segment.first() == Some(ch) && match_chars(&pattern[1..], &segment[1..]),
    }
}

/// Flatten a front-matter value to the text BM25 scores; `None` when it carries no terms.
fn value_text(value: &crate::Value) -> Option<String> {
    match value {
        crate::Value::Str(text) => Some(text.clone()),
        crate::Value::Int(number) => Some(number.to_string()),
        crate::Value::Bool(flag) => Some(flag.to_string()),
        crate::Value::Float(number) => Some(number.to_string()),
        crate::Value::Null => None,
        crate::Value::List(items) => {
            let parts: Vec<String> = items.iter().filter_map(value_text).collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
        crate::Value::Map(map) => {
            let parts: Vec<String> = map.values().filter_map(value_text).collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        }
    }
}

/// The caller-supplied parts of a plan, from the `aicontext context` flags
/// (`docs/CLI_SPEC.md` §2, the `context` row).
#[derive(Clone, Debug, Default)]
pub struct PlanOptions {
    /// `--query`, free text.
    pub query: Option<String>,
    /// `--include`, in the order given.
    pub include: Vec<PathBuf>,
    /// `--exclude`, an ID or a path.
    pub exclude: Vec<String>,
    /// Extra globs beyond the task's own `touches`.
    pub globs: Vec<String>,
    /// Extra filters beyond those derived from the task.
    pub filters: Vec<FrontMatterPredicate>,
    /// Kinds to keep.
    pub include_kinds: Vec<DocumentKind>,
    /// `--budget`, overriding the default.
    pub budget: Option<ContextBudget>,
}

impl RetrievalPlan {
    /// A plan for no particular task: the mandatory set plus whatever the caller asked for.
    #[must_use]
    pub fn new(options: &PlanOptions) -> Self {
        let mut plan = Self::default();
        plan.apply_options(options);
        plan
    }

    /// A plan seeded from the task `task_id` names, when the register declares it.
    ///
    /// The task contributes graph edges, not document contents: `spec`, `depends_on`, and
    /// `relates_to` become [`linked`](Self::linked), each remembering which edge it was; `touches`
    /// becomes [`globs`](Self::globs); `epic` and `phase` become equality
    /// [`filters`](Self::filters). An ID the register does not declare still names the task — the
    /// plan is a request, and a missing task is worth reporting rather than hiding — but contributes
    /// no edges.
    #[must_use]
    pub fn for_task(register: &Register, task_id: &str, options: &PlanOptions) -> Self {
        let mut plan = Self::new(options);
        plan.task = Some(task_id.to_string());
        if let Some(task) = register.tasks.iter().find(|task| task.id == task_id) {
            plan.absorb_task(task);
        }
        plan
    }

    /// Fills the caller-supplied fields and the fixed mandatory set.
    fn apply_options(&mut self, options: &PlanOptions) {
        self.query.clone_from(&options.query);
        self.mandatory = mandatory_set(&options.include);
        self.exclude_ids = dedup(options.exclude.iter().cloned());
        self.globs = dedup(options.globs.iter().cloned());
        self.filters.clone_from(&options.filters);
        self.include_kinds = dedup(options.include_kinds.iter().copied());
        self.budget = options.budget.clone().unwrap_or_default();
    }

    /// Adds the graph edges and derived filters a task carries.
    fn absorb_task(&mut self, task: &TaskEntry) {
        let mut linked: Vec<LinkedId> = Vec::new();
        if let Some(spec) = &task.spec {
            linked.push(LinkedId::spec(spec));
        }
        linked.extend(task.depends_on.iter().map(LinkedId::depends_on));
        linked.extend(task.relates_to.iter().map(LinkedId::relates_to));
        self.linked = dedup(linked);

        let globs = self
            .globs
            .iter()
            .cloned()
            .chain(task.touches.iter().cloned());
        self.globs = dedup(globs);

        let mut filters = self.filters.clone();
        if let Some(epic) = &task.epic {
            filters.push(FrontMatterPredicate::equals("epic", epic.clone()));
        }
        if let Some(phase) = task.phase {
            filters.push(FrontMatterPredicate::equals("phase", phase.to_string()));
        }
        self.filters = filters;
    }

    /// A deterministic, human-readable rendering of the plan, for `--explain`.
    ///
    /// Every field is printed, including the empty ones, so two plans can be compared line by line
    /// and an absence is visible rather than implied.
    #[must_use]
    pub fn explain(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "retrieval plan");
        let _ = writeln!(out, "  task:      {}", or_none(self.task.as_deref()));
        let _ = writeln!(out, "  query:     {}", or_none(self.query.as_deref()));
        let _ = writeln!(out, "  mandatory: {}", join_paths(&self.mandatory));
        let _ = writeln!(out, "  linked:    {}", join_linked(&self.linked));
        let _ = writeln!(out, "  globs:     {}", join_strings(&self.globs));
        let _ = writeln!(out, "  filters:   {}", join_filters(&self.filters));
        let _ = writeln!(out, "  kinds:     {}", join_kinds(&self.include_kinds));
        let _ = writeln!(out, "  excluded:  {}", join_strings(&self.exclude_ids));
        let _ = writeln!(
            out,
            "  budget:    {} tokens (reserved {}, {} tier caps)",
            self.budget.max_tokens,
            self.budget.reserved_policy_tokens,
            self.budget.per_tier.len()
        );
        out
    }
}

/// The always-present documents, plus the caller's `--include`, first occurrence wins.
fn mandatory_set(include: &[PathBuf]) -> Vec<PathBuf> {
    let mut set = vec![
        PathBuf::from(MANDATORY_ENTRY_POINT),
        PathBuf::from(MANDATORY_RULES),
    ];
    for path in include {
        if !set.contains(path) {
            set.push(path.clone());
        }
    }
    set
}

/// Removes duplicates while keeping the first occurrence's order.
fn dedup<T: PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// `value`, or a visible marker when there is none.
fn or_none(value: Option<&str>) -> &str {
    value.unwrap_or("(none)")
}

/// A comma-separated list, or a visible marker when it is empty.
fn join_strings(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

/// The graph edges as `id (edge)`, or a visible marker when there are none.
fn join_linked(items: &[LinkedId]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items
            .iter()
            .map(|linked| format!("{} ({})", linked.id, linked.edge))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The mandatory paths, comma-separated, or a visible marker when empty.
fn join_paths(items: &[PathBuf]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The filters, comma-separated, or a visible marker when there are none.
fn join_filters(items: &[FrontMatterPredicate]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items
            .iter()
            .map(describe_filter)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One filter as `key op value`.
fn describe_filter(predicate: &FrontMatterPredicate) -> String {
    let op = match predicate.op {
        PredicateOp::Equals => "=",
        PredicateOp::NotEquals => "!=",
        PredicateOp::Contains => "contains",
        PredicateOp::NotContains => "not contains",
        PredicateOp::Exists => "exists",
        PredicateOp::NotExists => "not exists",
    };
    match &predicate.value {
        Some(value) => format!("{} {} {}", predicate.key, op, value),
        None => format!("{} {}", predicate.key, op),
    }
}

/// The kinds, comma-separated by their wire names, or a visible marker when there are none.
fn join_kinds(items: &[DocumentKind]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}
