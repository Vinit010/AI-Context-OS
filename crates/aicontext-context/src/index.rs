//! The content-hash index: what documents exist in `.ai/`, what is in them, and when to read them
//! again (`docs/CONTEXT_SPEC.md` §6).
//!
//! The index is a **cache**, not a source of truth. It is fully derivable from the repository, it is
//! git-ignored, and a damaged cache is repaired by rebuilding it, never by trusting it (`RULES.md`
//! §4.10: a degraded mode is entered and reported, not hidden).
//!
//! # Reading as little as possible
//!
//! A scan compares each file's size and modification time against the previous cache before reading
//! it. When both match, the file is not opened at all and the cached entry is reused verbatim, so a
//! second scan of an unchanged tree reads nothing (see [`ScanReport::reads`]). Only when the
//! metadata moves is the file read: if its bytes hash to the same value, the parsed entry is kept
//! and only the fingerprint is refreshed; if the hash changes, that one entry is rebuilt. A file's
//! content hash is therefore its identity for invalidation, and one edit invalidates one entry.
//!
//! # What is not read as structure
//!
//! Markdown bodies are **not** parsed for meaning (`decisions/ADR-003-knowledge-storage-contract.md`).
//! Front matter goes through the same parser every document uses ([`crate::Document`]); the body is
//! scanned only for ATX headings and ID-shaped tokens, both of which exist for lexical scoring in
//! `TASK-033` and are recorded, never inferred.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use aicontext_core::{AicontextError, ErrorCode, Severity};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::body::names_entity;
use crate::frontmatter::{DOCUMENT_MAX_BYTES, FrontMatter};
use crate::{Document, Value};

/// The cache's location, relative to a project root (`docs/CONTEXT_SPEC.md` §6).
pub const INDEX_PATH: &str = ".aicontext/index.json";

/// The cache format version. A cache written by another version is rebuilt rather than misread.
///
/// Bumped to 2 when `IndexEntry` gained `body`: a version-1 cache has no body text, and a lexical
/// retriever reading it would score the body as empty. Rebuilding is cheaper than misreading.
const CACHE_VERSION: u32 = 2;

/// The extension, matched case-insensitively, of a document the index reads. `.ai/` is made of
/// Markdown; the two fixed-path YAML files (`ai.yaml`, `permissions/permissions.yaml`) are config,
/// not documents, and are read by the command that needs them rather than indexed.
const DOCUMENT_EXTENSION: &str = "md";

/// What a document is, from its location (`docs/CONTEXT_SPEC.md` §1).
///
/// Inferred from the path, then from the front matter's `type`, and left `None` when neither
/// answers. It is deliberately not derived only from `type`: a document in the wrong place is a
/// `doctor` finding (`CTX-004`), and the index records where a document *is*, not where it claims
/// to be.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum DocumentKind {
    /// `.ai/AI.md`, the agent entry point.
    AgentEntryPoint,
    /// `.ai/PRD.md`.
    Product,
    /// `.ai/ARCHITECTURE.md`.
    Architecture,
    /// `.ai/RULES.md`.
    Rules,
    /// `.ai/CONVENTIONS.md`.
    Conventions,
    /// `.ai/DESIGN.md`.
    Design,
    /// `.ai/TASKS.md`.
    TaskRegister,
    /// `.ai/MEMORY.md`.
    MemoryRegister,
    /// `.ai/specs/<slug>.md`.
    Specification,
    /// `.ai/tasks/TASK-NNN-<slug>.md`, or inline in `TASKS.md`.
    Task,
    /// `.ai/decisions/ADR-NNN-<slug>.md`, or inline.
    Decision,
    /// `.ai/bugs/BUG-NNN-<slug>.md`, or inline.
    Bug,
    /// `.ai/changes/CHG-NNN-<slug>.md`, or inline.
    Change,
    /// `.ai/context/<slug>.md`.
    ContextNote,
    /// `.ai/workflows/<slug>.md`.
    Workflow,
    /// `.ai/agents/<slug>.md`.
    AgentProfile,
}

impl DocumentKind {
    /// The kind a repository-relative path names, before the `type` key is consulted.
    fn from_path(relative: &str) -> Option<Self> {
        let lower = relative.to_ascii_lowercase();
        let exact = match lower.as_str() {
            ".ai/ai.md" => Self::AgentEntryPoint,
            ".ai/prd.md" => Self::Product,
            ".ai/architecture.md" => Self::Architecture,
            ".ai/rules.md" => Self::Rules,
            ".ai/conventions.md" => Self::Conventions,
            ".ai/design.md" => Self::Design,
            ".ai/tasks.md" => Self::TaskRegister,
            ".ai/memory.md" => Self::MemoryRegister,
            _ => {
                let directories = [
                    (".ai/specs/", Self::Specification),
                    (".ai/tasks/", Self::Task),
                    (".ai/decisions/", Self::Decision),
                    (".ai/bugs/", Self::Bug),
                    (".ai/changes/", Self::Change),
                    (".ai/context/", Self::ContextNote),
                    (".ai/workflows/", Self::Workflow),
                    (".ai/agents/", Self::AgentProfile),
                ];
                for (prefix, kind) in directories {
                    if let Some(rest) = lower.strip_prefix(prefix) {
                        // Direct children only: a slug is one path segment (§1).
                        if !rest.contains('/') {
                            return Some(kind);
                        }
                    }
                }
                return None;
            }
        };
        Some(exact)
    }

    /// The kind a front matter `type` names, when the path did not settle it.
    fn from_type(kind: &str) -> Option<Self> {
        match kind {
            "prd" | "product" => Some(Self::Product),
            "architecture" => Some(Self::Architecture),
            "rules" => Some(Self::Rules),
            "conventions" => Some(Self::Conventions),
            "design" => Some(Self::Design),
            "tasks" => Some(Self::TaskRegister),
            "memory" => Some(Self::MemoryRegister),
            "spec" | "specification" => Some(Self::Specification),
            "task" => Some(Self::Task),
            "decision" => Some(Self::Decision),
            "bug" => Some(Self::Bug),
            "change" => Some(Self::Change),
            "context-note" => Some(Self::ContextNote),
            "workflow" => Some(Self::Workflow),
            "agent" | "agent-profile" => Some(Self::AgentProfile),
            _ => None,
        }
    }

    /// The kind of the document at `relative`, using `type` only when the path does not answer.
    #[must_use]
    fn classify(relative: &str, kind: Option<&str>) -> Option<Self> {
        Self::from_path(relative).or_else(|| kind.and_then(Self::from_type))
    }
}

impl fmt::Display for DocumentKind {
    /// The kind's wire name, the same kebab-case string serde derives from the variant name.
    ///
    /// Written out rather than reused from serde so a `--explain` line can name a kind without
    /// serialising it; a test in `tests/index.rs` asserts the two agree.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::AgentEntryPoint => "agent-entry-point",
            Self::Product => "product",
            Self::Architecture => "architecture",
            Self::Rules => "rules",
            Self::Conventions => "conventions",
            Self::Design => "design",
            Self::TaskRegister => "task-register",
            Self::MemoryRegister => "memory-register",
            Self::Specification => "specification",
            Self::Task => "task",
            Self::Decision => "decision",
            Self::Bug => "bug",
            Self::Change => "change",
            Self::ContextNote => "context-note",
            Self::Workflow => "workflow",
            Self::AgentProfile => "agent-profile",
        };
        formatter.write_str(name)
    }
}

/// A Markdown ATX heading, kept for lexical scoring only (`docs/CONTEXT_SPEC.md` §6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Heading {
    /// The heading level, 1 through 6.
    pub level: u8,
    /// The heading text, trimmed of surrounding whitespace.
    pub text: String,
}

/// One indexed document (`docs/CONTEXT_SPEC.md` §6).
///
/// Equality is derived, so two entries are equal only when every field is: the entry is a fact about
/// the file's bytes, and a field that differed would be a different fact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// The `id` from the front matter, when present.
    pub id: Option<String>,
    /// The document's kind, from its location or, failing that, its `type`.
    pub kind: Option<DocumentKind>,
    /// The repository-relative path, always with `/` separators so the cache is portable.
    pub path: PathBuf,
    /// The `title` from the front matter, else the first heading, else nothing.
    pub title: Option<String>,
    /// The front matter, as the crate's own value model.
    pub front_matter: BTreeMap<String, Value>,
    /// Every ATX heading in the body, in document order.
    pub headings: Vec<Heading>,
    /// The body's length in bytes.
    pub body_bytes: u64,
    /// A heuristic token estimate, for budgeting only (`docs/CONTEXT_SPEC.md` §6).
    pub token_estimate: u32,
    /// The SHA-256 of the whole file. This is the entry's identity for invalidation.
    pub content_hash: [u8; 32],
    /// The IDs the document references, from its front matter and its body.
    pub links: Vec<String>,
    /// The full body text (stripped of front matter), stored for lexical retrieval.
    #[serde(default)]
    pub body: String,
}

impl IndexEntry {
    /// The content hash as lower-case hexadecimal, the form the cache writes.
    #[must_use]
    pub fn content_hash_hex(&self) -> String {
        hex(&self.content_hash)
    }
}

/// The index: entries keyed by repository-relative path, each carrying its content hash.
///
/// The hash, not the path, is the invalidation identity (`docs/CONTEXT_SPEC.md` §6): a file whose
/// bytes are unchanged is not re-parsed, whichever path they sit at.
#[derive(Clone, Debug, Default)]
pub struct ContextIndex {
    entries: BTreeMap<String, IndexEntry>,
    fingerprints: BTreeMap<String, Fingerprint>,
}

/// What one scan did. Partial-failure honesty (`RULES.md` §4.8): a file that could not be read is
/// counted, not silently treated as indexed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanReport {
    /// Documents the walk offered.
    pub documents: usize,
    /// Documents whose size and modification time matched the cache, so they were not opened.
    pub reused: usize,
    /// Documents that were read but whose hash was unchanged, so they were not re-parsed.
    pub rehashed: usize,
    /// Documents that were parsed, because they were new or changed.
    pub parsed: usize,
    /// Cached entries dropped because their file no longer exists.
    pub removed: usize,
    /// Documents skipped because they were larger than the parser accepts.
    pub oversize: usize,
    /// Paths the walk or the read could not deliver.
    pub unreadable: usize,
}

impl ScanReport {
    /// How many documents the scan actually read from disk.
    ///
    /// This is the deterministic measure of incrementality: a second scan of an unchanged tree
    /// returns zero, without measuring wall-clock time (`RULES.md` §8 forbids clock assertions).
    #[must_use]
    pub fn reads(&self) -> usize {
        self.rehashed + self.parsed
    }
}

/// Everything that can go wrong reading or writing the cache.
///
/// The index is a cache, so every variant has the same remedy — rebuild it — which is the finding
/// the catalogue already defines as `CTX-019` (`docs/CONTEXT_SPEC.md` §8).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IndexError {
    /// The cache exists but could not be read.
    #[error("cannot read the index cache at {path}: {source}")]
    Read {
        /// The cache path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The cache could not be written.
    #[error("cannot write the index cache at {path}: {source}")]
    Write {
        /// The cache path.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The cache is not JSON, or not this cache's shape.
    #[error("cannot parse the index cache at {path}: {reason}")]
    Format {
        /// The cache path.
        path: PathBuf,
        /// What the parser objected to.
        reason: String,
    },
    /// The cache was written by a different version of the format.
    #[error("the index cache at {path} is version {found}, not {expected}")]
    Version {
        /// The cache path.
        path: PathBuf,
        /// The version found.
        found: u32,
        /// The version this build writes.
        expected: u32,
    },
}

impl IndexError {
    /// The stable code this failure reports.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        ErrorCode::CTX_019
    }

    /// How serious this failure is. A cache is rebuildable, so it never stops the command that
    /// needed it; the caller decides whether to rebuild now or report it.
    #[must_use]
    pub fn severity(&self) -> Severity {
        Severity::Info
    }

    /// What the caller should do about this failure.
    #[must_use]
    pub fn remediation(&self) -> &'static str {
        "rebuild the index; it is a cache and is fully derived from the repository"
    }
}

impl From<IndexError> for AicontextError {
    fn from(error: IndexError) -> Self {
        let code = error.code();
        let message = error.to_string();
        let remediation = error.remediation().to_string();
        Self::new(code, message, remediation).with_source(error)
    }
}

/// The (size, mtime) pair a scan compares before it reads a file.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct Fingerprint {
    size: u64,
    modified_ns: Option<u64>,
}

impl Fingerprint {
    fn of(metadata: &fs::Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified_ns: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                .and_then(|duration| u64::try_from(duration.as_nanos()).ok()),
        }
    }
}

/// The on-disk shape, which carries the version the in-memory value does not need.
#[derive(Serialize, Deserialize)]
struct StoredIndex {
    version: u32,
    entries: BTreeMap<String, IndexEntry>,
    #[serde(default)]
    fingerprints: BTreeMap<String, Fingerprint>,
}

impl ContextIndex {
    /// An index with no entries.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Loads the cache at `path`.
    ///
    /// # Errors
    ///
    /// Returns an [`IndexError`] when the file cannot be read, is not JSON, is not this cache's
    /// shape, or was written by another version. The caller normally rebuilds; nothing here guesses.
    pub fn load(path: &Path) -> Result<Self, IndexError> {
        let text = fs::read_to_string(path).map_err(|source| IndexError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let stored: StoredIndex =
            serde_json::from_str(&text).map_err(|error| IndexError::Format {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;
        if stored.version != CACHE_VERSION {
            return Err(IndexError::Version {
                path: path.to_path_buf(),
                found: stored.version,
                expected: CACHE_VERSION,
            });
        }
        Ok(Self {
            entries: stored.entries,
            fingerprints: stored.fingerprints,
        })
    }

    /// Writes the cache to `path`, creating its parent directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an [`IndexError`] when the directory cannot be created or the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<(), IndexError> {
        let stored = StoredIndex {
            version: CACHE_VERSION,
            entries: self.entries.clone(),
            fingerprints: self.fingerprints.clone(),
        };
        let mut json =
            serde_json::to_string_pretty(&stored).map_err(|error| IndexError::Format {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;
        json.push('\n');
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| IndexError::Write {
                    path: path.to_path_buf(),
                    source,
                })?;
            }
        }
        fs::write(path, json).map_err(|source| IndexError::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Loads the cache (ignoring any failure), scans, and writes the cache back.
    ///
    /// # Errors
    ///
    /// Returns an [`IndexError`] only when the result cannot be written; a missing or damaged cache
    /// is rebuilt, not reported.
    pub fn refresh(root: &Path, cache: &Path) -> Result<(Self, ScanReport), IndexError> {
        let previous = Self::load(cache).ok();
        let (index, report) = Self::build(root, previous.as_ref());
        index.save(cache)?;
        Ok((index, report))
    }

    /// Builds an index by scanning `root/.ai`, reusing `previous` where it can.
    #[must_use]
    pub fn build(root: &Path, previous: Option<&Self>) -> (Self, ScanReport) {
        let mut index = Self::empty();
        let mut report = ScanReport::default();
        let ai = root.join(".ai");
        if !ai.is_dir() {
            return (index, report);
        }

        let mut builder = WalkBuilder::new(&ai);
        builder
            .hidden(false)
            .parents(true)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .require_git(false);

        let mut seen: BTreeSet<String> = BTreeSet::new();
        for result in builder.build() {
            let Ok(entry) = result else {
                report.unreadable += 1;
                continue;
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            if !is_document(path) {
                continue;
            }
            let Some(relative) = relative_key(root, path) else {
                report.unreadable += 1;
                continue;
            };
            report.documents += 1;
            seen.insert(relative.clone());

            let Ok(metadata) = fs::metadata(path) else {
                report.unreadable += 1;
                continue;
            };
            let fingerprint = Fingerprint::of(&metadata);

            if let Some(prev) = previous {
                if let (Some(entry), Some(cached)) = (
                    prev.entries.get(&relative),
                    prev.fingerprints.get(&relative),
                ) {
                    if *cached == fingerprint {
                        index.entries.insert(relative.clone(), entry.clone());
                        index.fingerprints.insert(relative.clone(), *cached);
                        report.reused += 1;
                        continue;
                    }
                }
            }

            let limit = u64::try_from(DOCUMENT_MAX_BYTES).unwrap_or(u64::MAX);
            if metadata.len() > limit {
                // The parser refuses it (CTX-018); the index caches only documents that parse.
                report.oversize += 1;
                continue;
            }

            let Ok(bytes) = fs::read(path) else {
                report.unreadable += 1;
                continue;
            };
            let hash = hash_bytes(&bytes);

            // The content is unchanged even though the metadata moved: keep the parsed entry.
            if let Some(existing) = previous.and_then(|prev| prev.by_hash(&relative, hash)) {
                index.entries.insert(relative.clone(), existing.clone());
                index.fingerprints.insert(relative, fingerprint);
                report.rehashed += 1;
                continue;
            }

            let entry = build_entry(relative.clone(), &bytes, hash);
            index.entries.insert(relative.clone(), entry);
            index.fingerprints.insert(relative, fingerprint);
            report.parsed += 1;
        }

        // Cached entries whose file is no longer offered by the walk. They were never inserted into
        // the fresh index, so this is a count against `previous`, not a removal from `index`.
        if let Some(previous) = previous {
            report.removed = previous
                .entries
                .keys()
                .filter(|path| !seen.contains(*path))
                .count();
        }

        (index, report)
    }

    /// Every entry, in path order.
    pub fn entries(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.values()
    }

    /// The entry at `relative`, if the index has one.
    #[must_use]
    pub fn get(&self, relative: &str) -> Option<&IndexEntry> {
        self.entries.get(relative)
    }

    /// How many entries the index holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all indexed entries by their repository-relative keys, in path order.
    pub fn entries_map(&self) -> std::collections::btree_map::Iter<'_, String, IndexEntry> {
        self.entries.iter()
    }

    /// The previous entry at `relative`, when its content already hashes to `hash`.
    fn by_hash(&self, relative: &str, hash: [u8; 32]) -> Option<&IndexEntry> {
        self.entries
            .get(relative)
            .filter(|entry| entry.content_hash == hash)
    }
}

/// Builds one entry from a document's bytes.
fn build_entry(relative: String, bytes: &[u8], hash: [u8; 32]) -> IndexEntry {
    let source = String::from_utf8_lossy(bytes);
    let document = Document::parse(&source).ok();

    let (front_matter, body, declared) = match &document {
        Some(document) => {
            let front = document.front_matter();
            let map = front.map(front_matter_map).unwrap_or_default();
            let declared = front.and_then(FrontMatter::kind).map(str::to_string);
            (map, document.body().to_string(), declared)
        }
        None => (BTreeMap::new(), source.into_owned(), None),
    };

    let headings = extract_headings(&body);
    let title = front_matter
        .get("title")
        .and_then(value_as_str)
        .map(str::to_string)
        .or_else(|| headings.first().map(|heading| heading.text.clone()));
    let kind = DocumentKind::classify(&relative, declared.as_deref());
    let links = collect_links(&front_matter, &body);
    let body_bytes = u64::try_from(body.len()).unwrap_or(u64::MAX);
    let front_matter_bytes: u64 = front_matter
        .iter()
        .map(|(key, value)| u64::try_from(key.len() + value_len(value)).unwrap_or(u64::MAX))
        .sum();
    let token_estimate = token_estimate(body_bytes, front_matter_bytes, &headings);

    IndexEntry {
        id: front_matter
            .get("id")
            .and_then(value_as_str)
            .map(str::to_string),
        kind,
        path: PathBuf::from(relative),
        title,
        front_matter,
        headings,
        body_bytes,
        token_estimate,
        content_hash: hash,
        links,
        body: body.clone(),
    }
}

/// The full front matter as the crate's value model, in the canonical key order of rule 4.
fn front_matter_map(front: &FrontMatter) -> BTreeMap<String, Value> {
    front.ordered_entries().into_iter().collect()
}

/// A value as a string, when it is one.
fn value_as_str(value: &Value) -> Option<&str> {
    match value {
        Value::Str(text) => Some(text),
        _ => None,
    }
}

/// A rough serialized length of a value, for the token estimate only.
fn value_len(value: &Value) -> usize {
    match value {
        Value::Null => 0,
        Value::Bool(_) | Value::Int(_) | Value::Float(_) => 4,
        Value::Str(text) => text.len(),
        Value::List(items) => items.iter().map(|item| value_len(item) + 1).sum(),
        Value::Map(map) => map
            .iter()
            .map(|(key, item)| key.len() + value_len(item) + 1)
            .sum(),
    }
}

/// The heuristic token estimate: bytes over four, with headings counted as content of their own
/// (`docs/CONTEXT_SPEC.md` §6). Never presented to a user as an exact count.
fn token_estimate(body_bytes: u64, front_matter_bytes: u64, headings: &[Heading]) -> u32 {
    let heading_bytes: u64 = headings
        .iter()
        .map(|heading| u64::try_from(heading.text.len()).unwrap_or(u64::MAX))
        .sum();
    let weighted = (body_bytes + front_matter_bytes + heading_bytes) / 4
        + u64::try_from(headings.len()).unwrap_or(u64::MAX) * 4;
    u32::try_from(weighted).unwrap_or(u32::MAX)
}

/// The referencing IDs a document carries, from its front matter and its body, sorted and unique.
fn collect_links(front_matter: &BTreeMap<String, Value>, body: &str) -> Vec<String> {
    let mut links: BTreeSet<String> = BTreeSet::new();
    for value in front_matter.values() {
        collect_ids(value, &mut links);
    }
    for token in
        body.split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
    {
        if names_entity(token) {
            links.insert(token.to_string());
        }
    }
    links.into_iter().collect()
}

/// Collects the ID-shaped strings in a front-matter value, at any depth.
fn collect_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Str(text) => {
            if names_entity(text) {
                out.insert(text.clone());
            }
        }
        Value::List(items) => items.iter().for_each(|item| collect_ids(item, out)),
        Value::Map(map) => map.values().for_each(|item| collect_ids(item, out)),
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => {}
    }
}

/// Every ATX heading in the body, in document order, ignoring headings inside fenced code.
fn extract_headings(body: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some((level, text)) = heading_level(line) {
            headings.push(Heading {
                level,
                text: text.trim_end().to_string(),
            });
        }
    }
    headings
}

/// The level and text of an ATX heading line, if it is one.
fn heading_level(line: &str) -> Option<(u8, &str)> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((u8::try_from(hashes).ok()?, rest.trim_start()))
}

/// Whether a path is a Markdown document.
fn is_document(path: &Path) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(DOCUMENT_EXTENSION))
}

/// The repository-relative path of `path` under `root`, with `/` separators, or `None` when it is
/// not below `root`.
fn relative_key(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut key = String::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                if !key.is_empty() {
                    key.push('/');
                }
                key.push_str(&part.to_string_lossy());
            }
            _ => return None,
        }
    }
    (!key.is_empty()).then_some(key)
}

/// The SHA-256 of a byte slice.
fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Lower-case hexadecimal, without a `hex` dependency (six lines, per `TASK-020`).
fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
