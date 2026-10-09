//! The `doctor` checks: what is wrong with this `.ai/` tree, and what to do about it.
//!
//! A validation command is only useful if it is the *only* place a rule lives. Every finding carries
//! a code from `docs/CONTEXT_SPEC.md` Â§8, a severity from that same table, and a remediation, so a
//! developer never has to read the source to know whether something matters or how to fix it.
//!
//! # Scope of v1
//!
//! `TASK-014`'s acceptance names six behaviours, and this module implements exactly those:
//! `CTX-001`, `CTX-002`, `CTX-007`, `CTX-012`, `CTX-013`, and `CTX-014`. `TASK-019` added a seventh,
//! `CTX-016`, which scans `.ai/` for credential-shaped content. The rest of the Â§8
//! catalogue is deliberately absent rather than stubbed, because a check that cannot fail is worse
//! than a missing one â€” it advertises a guarantee the code does not make. Each omission is listed in
//! [`UNIMPLEMENTED`] so `doctor --explain` says so rather than inventing a rationale.
//!
//! One check outside that list is here anyway: `CTX-018`, and only its reporting half. Â§2 rule 12
//! requires the excess past the 1 MiB maximum to be *reported* rather than silently truncated, so a
//! document that size cannot be left out of the report without breaking the format contract. Nothing
//! is configurable about the cap, because nothing configures it yet.
//!
//! Two inputs are passed in rather than discovered here. The observed stack belongs to the discovery
//! engine (`TASK-030` owns the typed profile), and the schema source is embedded in the binary by the
//! CLI, so `CTX-012` compares what is on disk against the bytes that were compiled in rather than
//! against a second copy on disk that could itself have been edited.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use aicontext_core::{DocumentId, ErrorCode, Severity};

use crate::body::{Entity, inline_entities, is_entity_id};
use crate::{DOCUMENT_MAX_BYTES, Document, FrontMatter, Value};

/// The entry point document, the one file whose absence makes the project unusable.
const ENTRY_POINT: &str = ".ai/AI.md";

/// The directory `init` scaffolds and every document check walks.
const CONTEXT_DIR: &str = ".ai";

/// Where `init` copies the schema set.
const SCHEMA_DIR: &str = ".ai/schemas";

/// The most documents examined, so a pathological tree costs a bounded amount of work.
const MAX_DOCUMENTS: usize = 512;

/// The deepest directory nesting walked below `.ai/`.
const MAX_DEPTH: usize = 16;

/// The locations `docs/CONTEXT_SPEC.md` Â§1 gives a kind, a schema, and an ID convention to.
///
/// A document at one of these must carry front matter, because Â§2 rule 1 requires it everywhere except
/// `AI.md` and free-form notes, and rule 2 then requires its `id` to match the convention of exactly
/// these locations. A Markdown file *outside* them has no schema to validate against and no ID
/// convention to match, which is what makes it a free-form note and exempt: requiring an `id` there
/// would produce a document that can never satisfy the rest of the contract.
const CATALOGUE_LOCATIONS: &[(&str, &str)] = &[
    (".ai/PRD.md", "PRD-"),
    (".ai/ARCHITECTURE.md", "ARCH-"),
    (".ai/RULES.md", "RULES-"),
    (".ai/CONVENTIONS.md", "CONV-"),
    (".ai/DESIGN.md", "DESIGN-"),
    (".ai/TASKS.md", "TASKS-"),
    (".ai/MEMORY.md", "MEMORY-"),
    (".ai/specs/", "SPEC-"),
    (".ai/tasks/", "TASK-"),
    (".ai/decisions/", "ADR-"),
    (".ai/bugs/", "BUG-"),
    (".ai/changes/", "CHG-"),
    (".ai/context/", "CTX-"),
    (".ai/workflows/", "WF-"),
    (".ai/agents/", "AGENT-"),
];

/// The keys whose values must resolve to something that exists (`CTX-007`).
///
/// `spec` is included because Â§2 rule 10 requires both its forms resolved: a `SPEC-*` entity and a
/// path under `docs/`. `epic` is deliberately absent â€” a free-text grouping label is not obliged to
/// be an existing ID, and checking it would report every project that has not written
/// `TASK-900-epic.md`.
const REFERENCE_KEYS: &[&str] = &[
    "spec",
    "depends_on",
    "blocks",
    "supersedes",
    "superseded_by",
];

/// The languages `CTX-013` is able to compare, drawn from the discovery table's vocabulary.
const LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "go",
    "java",
    "kotlin",
    "ruby",
    "php",
    "csharp",
    "swift",
    "elixir",
    "haskell",
    "scala",
    "c",
    "cpp",
];

/// The Â§8 checks `TASK-014` does not implement, and why each is absent.
pub const UNIMPLEMENTED: &[(&str, &str)] = &[
    (
        "CTX-003",
        "schema validation needs the runtime Validator, deferred here from TASK-015",
    ),
    (
        "CTX-004",
        "ID-versus-location checking lands with the task register commands",
    ),
    ("CTX-005", "type-versus-schema checking needs CTX-003 first"),
    (
        "CTX-006",
        "unknown-key warnings come with schema comparison in CTX-003",
    ),
    (
        "CTX-008",
        "deprecated-reference warnings follow CTX-007's resolution pass",
    ),
    (
        "CTX-009",
        "waiver checking arrives with the task register commands",
    ),
    (
        "CTX-010",
        "cycle detection needs the whole dependency graph, not one pass",
    ),
    (
        "CTX-011",
        "status-transition history needs git history, owned by TASK-017",
    ),
    (
        "CTX-015",
        "memory-versus-file freshness needs file modification times",
    ),
    (
        "CTX-017",
        "hand-edit detection needs the index, owned by TASK-031",
    ),
    (
        "CTX-019",
        "index staleness needs the index, owned by TASK-031",
    ),
    (
        "CTX-020",
        "the inline-to-split suggestion needs entity counts, owned by TASK-031",
    ),
];

/// One thing wrong with the tree, in the shape `docs/CLI_SPEC.md` Â§6 publishes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    /// Stable code from the Â§8 catalogue, safe to branch on.
    pub code: String,
    /// How serious it is, from the same table.
    pub severity: Severity,
    /// Repository-relative path with `/` separators, stable across platforms.
    pub path: String,
    /// For humans, and may be reworded in a later version.
    pub message: String,
    /// What to do about it.
    pub remediation: String,
}

impl Finding {
    fn new(
        code: ErrorCode,
        severity: Severity,
        path: impl Into<String>,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            code: code.to_string(),
            severity,
            path: path.into(),
            message: message.into(),
            remediation: remediation.into(),
        }
    }
}

/// A document that was examined and produced no finding, so the report can say so.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checked {
    /// Repository-relative path with `/` separators.
    pub path: String,
    /// What was checked, for example `valid` or `12 tasks`.
    pub note: String,
}

/// Everything the checks are run against.
#[derive(Debug)]
pub struct Inputs<'a> {
    /// The project root. Every path in a finding is relative to it.
    pub root: &'a Path,
    /// Languages discovery observed, which `CTX-013` compares against the declared stack.
    ///
    /// Empty means discovery recognised nothing, and a contradiction cannot be asserted against
    /// nothing: the check then stays silent rather than inventing a mismatch.
    pub observed_languages: &'a [String],
    /// The schema source of truth, as `(file name, exact bytes)`.
    ///
    /// Supplied by the binary's embedded copy so `CTX-012` compares against what was compiled in.
    pub schema_source: &'a [(&'a str, &'a str)],
}

/// The result of one run: everything found, and everything examined cleanly.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    /// Every finding, error-level and warning-level alike.
    pub findings: Vec<Finding>,
    /// The documents that produced no finding, in path order.
    pub checked: Vec<Checked>,
}

impl Report {
    /// How many findings are at `severity` or above, which is what decides the exit code.
    #[must_use]
    pub fn at_least(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity >= severity)
            .count()
    }

    /// Whether any error-level finding is present.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.at_least(Severity::Error) > 0
    }
}

/// One document read off disk, parsed, with its inline entities extracted.
struct Loaded {
    /// Repository-relative path with `/` separators.
    path: String,
    /// The parsed document, absent when the file could not be read or its block could not be parsed.
    parsed: Option<Document>,
    /// The parse or read failure, when there is one.
    failure: Option<String>,
    /// Whether the file is past the size cap, which stops every other check reading it.
    oversize: bool,
    /// Inline entities declared in the body, in document order.
    entities: Vec<Entity>,
    /// Whether this document must carry front matter.
    requires_front_matter: bool,
}

impl Loaded {
    /// The document's own front-matter block, when it has a parseable one.
    fn front(&self) -> Option<&FrontMatter> {
        self.parsed.as_ref()?.front_matter()
    }
}

/// Runs every implemented check and returns one report.
///
/// Takes `&Inputs` rather than a dozen arguments because the inputs grow with each check, and a
/// parameter list this long is a struct that has not been named yet.
#[must_use]
pub fn run(inputs: &Inputs<'_>) -> Report {
    let documents = read_tree(inputs.root);
    let mut findings = Vec::new();
    let mut checked = Vec::new();

    check_entry_point(inputs.root, &mut findings);
    check_documents(&documents, &mut findings, &mut checked);
    check_references(inputs.root, &documents, &mut findings);
    check_schemas(inputs, &mut findings);
    check_architecture(inputs, &documents, &mut findings);
    check_decisions(&documents, &mut findings);
    check_secrets(inputs.root, &mut findings);

    // The checks that read a document as part of a whole tree report against that document's path:
    // a dangling reference in `RULES.md`, a declared stack that contradicts `ARCHITECTURE.md`, a
    // deprecated ADR. Those run after `check_documents` has already listed their file as clean, and
    // "ok" beside a finding for the same file is worse than saying nothing. The tree-wide checks
    // cannot suppress the row as they run, because they see other documents, so the row is dropped
    // once every finding is known.
    let blamed: BTreeSet<&str> = findings
        .iter()
        .map(|finding| finding.path.as_str())
        .collect();
    checked.retain(|row| !blamed.contains(row.path.as_str()));

    Report { findings, checked }
}

/// `CTX-001`: the entry point is the file an agent reads before anything else, so its absence means
/// the project has no contract at all.
fn check_entry_point(root: &Path, findings: &mut Vec<Finding>) {
    if root.join(ENTRY_POINT).is_file() {
        return;
    }
    findings.push(Finding::new(
        ErrorCode::CTX_001,
        Severity::Error,
        ENTRY_POINT,
        "AI.md is missing, so there is no entry point for an agent to read",
        "run `aicontext init` to create the skeleton, or restore AI.md from version control",
    ));
}

/// `CTX-002`: front matter is the only machine-read source of metadata, so a document without a
/// parseable block cannot be indexed, referenced, or validated.
///
/// `CTX-018` gets a branch of its own rather than a finding under this code: a file past the size cap
/// is not malformed, and Â§2 rule 12 asks for the excess to be *reported*, which is a different
/// statement from the one `CTX-002` makes.
fn check_documents(documents: &[Loaded], findings: &mut Vec<Finding>, checked: &mut Vec<Checked>) {
    for document in documents {
        if document.oversize {
            findings.push(Finding::new(
                ErrorCode::CTX_018,
                Severity::Warning,
                &document.path,
                format!(
                    "the document is larger than the {DOCUMENT_MAX_BYTES}-byte maximum, and was not read",
                ),
                "split the document, or raise the cap; the rest of its content was not checked",
            ));
            continue;
        }

        if let Some(failure) = &document.failure {
            findings.push(Finding::new(
                ErrorCode::CTX_002,
                Severity::Error,
                &document.path,
                format!("front matter cannot be read: {failure}"),
                "fix the block; `doctor` reports the line the parser stopped on",
            ));
            continue;
        }

        if document.requires_front_matter {
            match document.front() {
                None => {
                    findings.push(Finding::new(
                        ErrorCode::CTX_002,
                        Severity::Error,
                        &document.path,
                        "no front matter block, and this is not a location that may omit one",
                        "add a --- fenced block carrying at least `id`, `type`, and `title`",
                    ));
                    continue;
                }
                Some(front) => {
                    // `FrontMatter` already knows which keys rule 1 requires; the fact belongs to it
                    // and is only interpreted here, because only the location decides whether their
                    // absence matters.
                    for key in front.missing_required_keys() {
                        findings.push(Finding::new(
                            ErrorCode::CTX_002,
                            Severity::Error,
                            &document.path,
                            format!("front matter has no `{key}`"),
                            "every document needs `id`, `type`, and `title`",
                        ));
                    }
                }
            }
        }

        // Remembered so a document that produced a finding is not also listed as checked. "ok" and
        // "broken" in the same report would make the run impossible to read and impossible to trust.
        let before = findings.len();

        check_entity_blocks(document, findings);

        let tasks = document
            .entities
            .iter()
            .filter(|entity| entity.heading_id.starts_with("TASK-"))
            .count();
        let note = match (tasks > 0, document.requires_front_matter) {
            (true, _) => format!("{tasks} tasks"),
            // A free-form note is checked for structure and found to have nothing to declare.
            (false, false) => "no front matter required".to_string(),
            (false, true) => "valid".to_string(),
        };
        if findings.len() == before {
            checked.push(Checked {
                path: document.path.clone(),
                note,
            });
        }
    }
}

/// `CTX-002` for inline entities: Â§2.1 makes a missing or duplicated block an error, not a guess, so
/// a heading that names an entity but carries no readable block is reported against the document.
fn check_entity_blocks(document: &Loaded, findings: &mut Vec<Finding>) {
    for entity in &document.entities {
        if !entity.had_block {
            findings.push(Finding::new(
                ErrorCode::CTX_002,
                Severity::Error,
                &document.path,
                format!(
                    "{} is named by a heading but has no fenced yaml block",
                    entity.heading_id
                ),
                "add the entity's yaml block directly beneath its heading",
            ));
            continue;
        }
        if entity.front().is_none() {
            findings.push(Finding::new(
                ErrorCode::CTX_002,
                Severity::Error,
                &document.path,
                format!("the yaml block under {} cannot be read", entity.heading_id),
                "fix the block's YAML; it is parsed exactly as front matter is",
            ));
        }
    }
}

/// `CTX-007`: a reference to a document that does not exist is an error, because the reader who
/// follows it is sent to a file that is not there.
///
/// Two passes, because resolving a reference needs the whole tree: the first collects every ID the
/// project declares, the second checks what each entity points at.
fn check_references(root: &Path, documents: &[Loaded], findings: &mut Vec<Finding>) {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for document in documents {
        if let Some(id) = document.front().and_then(FrontMatter::id) {
            declared.insert(id.to_string());
        }
        for entity in &document.entities {
            declared.insert(entity.id().to_string());
        }
    }

    for document in documents {
        if let Some(front) = document.front() {
            check_one(root, &document.path, front, &declared, findings);
        }
        for entity in &document.entities {
            if let Some(front) = entity.front() {
                check_one(root, &document.path, front, &declared, findings);
            }
        }
    }
}

/// The references declared by one front-matter block.
fn check_one(
    root: &Path,
    path: &str,
    front: &FrontMatter,
    declared: &BTreeSet<String>,
    findings: &mut Vec<Finding>,
) {
    for key in REFERENCE_KEYS {
        let Some(value) = front.extra(key) else {
            continue;
        };
        for candidate in scalar_values(value) {
            if is_entity_id(&candidate) && !declared.contains(&candidate) {
                findings.push(Finding::new(
                    ErrorCode::CTX_007,
                    Severity::Error,
                    path,
                    format!("`{key}` references {candidate}, which does not exist"),
                    format!("create the document for {candidate}, or remove it from `{key}`"),
                ));
            }
            if *key == "spec" && looks_like_a_path(&candidate) && !root.join(&candidate).is_file() {
                findings.push(Finding::new(
                    ErrorCode::CTX_007,
                    Severity::Error,
                    path,
                    format!("`spec` references {candidate}, which is not a file"),
                    "point `spec` at an existing document under docs/, or at a SPEC-* id",
                ));
            }
        }
    }
}

/// `CTX-012`: the copy in `.ai/schemas` must match the source, or a project validates its documents
/// against schemas this repository never checked.
fn check_schemas(inputs: &Inputs<'_>, findings: &mut Vec<Finding>) {
    let dir = inputs.root.join(SCHEMA_DIR);
    let expected: BTreeSet<&str> = inputs.schema_source.iter().map(|(name, _)| *name).collect();

    // One finding for the whole directory rather than one per schema. A project with no copies at all
    // has one problem and one fix, and a report that lists the same line twenty times buries whatever
    // else is wrong in a list the reader has to scroll past.
    if !dir.is_dir() {
        findings.push(Finding::new(
            ErrorCode::CTX_012,
            Severity::Warning,
            SCHEMA_DIR,
            format!(
                "no schema copies at all, so none of the {} documents can be validated",
                expected.len()
            ),
            "run `aicontext init` to copy them",
        ));
        return;
    }

    for (name, text) in inputs.schema_source {
        let path = format!("{SCHEMA_DIR}/{name}");
        // `init` writes every document with LF line endings (docs/CONTEXT_SPEC.md rule 11), but the
        // source arrives from `include_str!`, so on a checkout whose git config expands newlines it
        // carries CRLF. Compare in the canonical form, or a freshly initialised project on Windows
        // would disagree with its own freshly copied schemas.
        let source = text.replace("\r\n", "\n");
        match fs::read(dir.join(name)) {
            Ok(bytes) if bytes == source.as_bytes() => {}
            Ok(_) => findings.push(Finding::new(
                ErrorCode::CTX_012,
                Severity::Warning,
                &path,
                "the schema copy differs from the source it was copied from",
                "re-run `aicontext init --force` to restore it, or re-apply your own changes",
            )),
            Err(_) => findings.push(Finding::new(
                ErrorCode::CTX_012,
                Severity::Warning,
                &path,
                "the schema is missing from the copy",
                "re-run `aicontext init` to copy it",
            )),
        }
    }

    // A schema the project added is not an error: it may be a deliberate local extension. It is
    // still worth saying, because nothing in this repository's CI has validated it.
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_json(&name) || expected.contains(name.as_str()) {
                continue;
            }
            findings.push(Finding::new(
                ErrorCode::CTX_012,
                Severity::Warning,
                format!("{SCHEMA_DIR}/{name}"),
                "this schema is not part of the source set, so nothing has validated it",
                "remove it, or move it out of .ai/schemas if it is not meant to be validated",
            ));
        }
    }
}

/// `CTX-013`: a declared stack that contradicts what is actually in the repository.
///
/// The comparison is between the declared stack and the observed languages, and only when discovery
/// produced something. A store contradiction of the kind `docs/CLI_SPEC.md` Â§7 illustrates needs store
/// detection, which is `TASK-030`; until then this check compares what it can see and says nothing
/// about what it cannot.
fn check_architecture(inputs: &Inputs<'_>, documents: &[Loaded], findings: &mut Vec<Finding>) {
    if inputs.observed_languages.is_empty() {
        return;
    }
    let Some(document) = documents
        .iter()
        .find(|document| document.path == ".ai/ARCHITECTURE.md")
    else {
        return;
    };
    let Some(front) = document.front() else {
        return;
    };
    let Some(Value::List(declared)) = front.extra("stack") else {
        return;
    };

    for entry in declared {
        let Some(name) = entry.as_str() else {
            continue;
        };
        if !is_language(name) {
            continue;
        }
        if inputs
            .observed_languages
            .iter()
            .any(|observed| fold(observed) == fold(name))
        {
            continue;
        }
        findings.push(Finding::new(
            ErrorCode::CTX_013,
            Severity::Warning,
            &document.path,
            format!(
                "the declared stack names {name}, which discovery did not observe (observed: {})",
                inputs.observed_languages.join(", ")
            ),
            "correct the `stack` list, or add the evidence discovery is missing",
        ));
    }
}

/// `CTX-014`: a deprecated decision with no successor leaves a reader holding a rule that no longer
/// applies and nothing that replaced it.
fn check_decisions(documents: &[Loaded], findings: &mut Vec<Finding>) {
    for document in documents {
        let standalone = document.front().into_iter();
        let inline = document.entities.iter().filter_map(Entity::front);
        for front in standalone.chain(inline) {
            let Some(id) = front.id().map(DocumentId::as_str) else {
                continue;
            };
            if !id.starts_with("ADR-") {
                continue;
            }
            let deprecated = front.status() == Some("deprecated");
            let has_successor = front
                .extra("superseded_by")
                .and_then(Value::as_str)
                .is_some_and(|successor| !successor.is_empty());
            if deprecated && !has_successor {
                findings.push(Finding::new(
                    ErrorCode::CTX_014,
                    Severity::Warning,
                    &document.path,
                    format!("{id} is deprecated with no `superseded_by`"),
                    "set `superseded_by` to the ADR that replaced it, or record why nothing did",
                ));
            }
        }
    }
}

/// `CTX-016`: `.ai/` is copied into every clone, so a credential written here is a credential
/// published. Reports the file, the line, and the rule that matched, and never the match itself.
///
/// Every text file under `.ai/` is examined rather than only the documents, because `init` also
/// copies `permissions.yaml`, and a secret leaked there is exactly as public. A file that cannot be
/// read as UTF-8, or that is past the size cap, is skipped rather than guessed at.
fn check_secrets(root: &Path, findings: &mut Vec<Finding>) {
    for relative in context_files(root) {
        let Ok(text) = fs::read_to_string(root.join(&relative)) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            let Some(rule) = match_secret(line) else {
                continue;
            };
            findings.push(Finding::new(
                ErrorCode::CTX_016,
                Severity::Error,
                &relative,
                format!(
                    "credential-shaped content on line {} matched the `{rule}` rule",
                    index + 1
                ),
                "remove the value and rotate it, and record only where the credential comes from",
            ));
        }
    }
}

/// Every regular file below `.ai/`, as repository-relative paths in sorted order.
///
/// Bounded in depth and in count so a pathological tree costs a bounded amount of work, and skipping
/// anything past the size cap so one enormous file cannot be loaded in order to be scanned.
fn context_files(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    scan_dir(&root.join(CONTEXT_DIR), root, &mut found, 0);
    found.sort();
    found
}

/// Recursive descent for the credential scan, bounded the same way the document walk is.
fn scan_dir(dir: &Path, root: &Path, found: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH || found.len() >= MAX_DOCUMENTS {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, root, found, depth + 1);
            continue;
        }
        let oversize =
            fs::metadata(&path).map_or(true, |metadata| metadata.len() > DOCUMENT_MAX_BYTES as u64);
        if oversize {
            continue;
        }
        found.push(relative_to(root, &path));
    }
}

/// The credential rule a line matches, most specific first, or `None` when it is clean.
fn match_secret(line: &str) -> Option<&'static str> {
    VALUE_RULES
        .iter()
        .find(|(_, detect)| detect(line))
        .map(|(name, _)| *name)
        .or_else(|| credential_key(line).then_some(KEY_RULE))
}

/// A named credential detector: the rule's name and the function that recognises it.
type ValueRule = (&'static str, fn(&str) -> bool);

/// The value-shaped rules, each a named detector, tried in the order a reader would recognise them.
const VALUE_RULES: &[ValueRule] = &[
    ("private-key-block", private_key_block),
    ("aws-access-key-id", aws_access_key_id),
    ("github-token", github_token),
    ("slack-token", slack_token),
    ("google-api-key", google_api_key),
    ("stripe-secret-key", stripe_secret_key),
    ("anthropic-api-key", anthropic_api_key),
    ("openai-api-key", openai_api_key),
    ("json-web-token", json_web_token),
    ("bearer-token", bearer_token),
    ("password-in-url", password_in_url),
];

/// The rule name for a credential-shaped key whose value is a literal secret.
const KEY_RULE: &str = "credential-key";

/// A private key block header, the shape `openssl`, `ssh-keygen`, and GPG all emit.
fn private_key_block(line: &str) -> bool {
    line.contains("-----BEGIN") && line.contains("PRIVATE KEY")
}

/// An AWS access key ID: `AKIA` or `ASIA` and exactly sixteen more upper-case letters or digits.
fn aws_access_key_id(line: &str) -> bool {
    ["AKIA", "ASIA"].iter().any(|prefix| {
        prefixed_run(line, prefix, 16, |c| {
            c.is_ascii_uppercase() || c.is_ascii_digit()
        })
    })
}

/// A GitHub token: the classic `ghp_`-family prefix or the fine-grained `github_pat_` prefix.
fn github_token(line: &str) -> bool {
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| prefixed_run(line, prefix, 36, is_token_char))
        || prefixed_run(line, "github_pat_", 22, is_token_char)
}

/// A Slack token: `xox` and the kind letter Slack uses, then a dash.
fn slack_token(line: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = line[from..].find("xox") {
        let start = from + offset;
        let mut after = line[start + 3..].chars();
        if matches!(after.next(), Some('b' | 'a' | 'p' | 'r' | 's')) && after.next() == Some('-') {
            return true;
        }
        from = start + 3;
    }
    false
}

/// A Google API key: `AIza` and at least thirty-five more URL-safe characters.
fn google_api_key(line: &str) -> bool {
    prefixed_run(line, "AIza", 35, is_token_char)
}

/// A Stripe live secret key: `sk_live_` or `rk_live_` and at least sixteen more characters.
fn stripe_secret_key(line: &str) -> bool {
    ["sk_live_", "rk_live_"]
        .iter()
        .any(|prefix| prefixed_run(line, prefix, 16, |c| c.is_ascii_alphanumeric()))
}

/// An Anthropic API key: `sk-ant-` and at least eight more URL-safe characters.
fn anthropic_api_key(line: &str) -> bool {
    prefixed_run(line, "sk-ant-", 8, is_token_char)
}

/// An `OpenAI` API key: `sk-` on a word boundary and at least twenty more URL-safe characters.
///
/// The boundary matters: without it, the `sk-` inside a word such as `task-name` would look like a
/// key, and every document that mentions a task would be reported.
fn openai_api_key(line: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = line[from..].find("sk-") {
        let start = from + offset;
        let at_boundary = line[..start]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_ascii_alphanumeric());
        let rest = &line[start + 3..];
        if at_boundary && rest.chars().take_while(|c| is_token_char(*c)).count() >= 20 {
            return true;
        }
        from = start + 3;
    }
    false
}

/// A JSON Web Token: `eyJ`, then three dot-separated base64url segments.
fn json_web_token(line: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = line[from..].find("eyJ") {
        let start = from + offset;
        let token: String = line[start..]
            .chars()
            .take_while(|c| is_token_char(*c) || *c == '.')
            .collect();
        let segments: Vec<&str> = token.split('.').collect();
        if segments.len() == 3 && segments.iter().all(|segment| !segment.is_empty()) {
            return true;
        }
        from = start + 3;
    }
    false
}

/// A bearer token in an `Authorization` header: the word, then at least twenty token characters.
fn bearer_token(line: &str) -> bool {
    prefixed_run(line, "Bearer ", 20, is_token_char)
}

/// A URL carrying a user name and a password, which is exposed to every reader of the file.
fn password_in_url(line: &str) -> bool {
    let mut from = 0;
    while let Some(offset) = line[from..].find("://") {
        let start = from + offset + 3;
        let authority = line[start..]
            .split(['/', '?', '#', ' ', '\t'])
            .next()
            .unwrap_or_default();
        if let Some(colon) = authority.find(':') {
            let (user, rest) = authority.split_at(colon);
            let password = &rest[1..];
            if !user.is_empty() && !password.is_empty() && password.contains('@') {
                return true;
            }
        }
        from = start;
    }
    false
}

/// Whether `line` holds `needle` followed by at least `min` characters `allowed` accepts.
fn prefixed_run(line: &str, needle: &str, min: usize, allowed: fn(char) -> bool) -> bool {
    let mut from = 0;
    while let Some(offset) = line[from..].find(needle) {
        let start = from + offset;
        let rest = &line[start + needle.len()..];
        if rest.chars().take_while(|c| allowed(*c)).count() >= min {
            return true;
        }
        from = start + needle.len();
    }
    false
}

/// The characters an opaque token is made of: base64url, plus the separators headers and URLs use.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '=')
}

/// The key names that make an assignment credential-shaped.
const CREDENTIAL_KEYS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "client_secret",
    "api_key",
    "apikey",
    "access_key",
    "access_key_id",
    "secret_key",
    "secret_access_key",
    "aws_access_key_id",
    "aws_secret_access_key",
    "private_key",
    "token",
    "auth_token",
    "access_token",
    "refresh_token",
    "bearer_token",
    "credential",
    "credentials",
];

/// The shortest value a credential-shaped key can hold and still be a literal rather than a stub.
const MIN_KEY_VALUE: usize = 6;

/// Value spellings that name where a secret comes from, or stand in for one, rather than being one.
const NOT_A_SECRET: &[&str] = &[
    "$",
    "<",
    "keychain",
    "env:",
    "op://",
    "vault:",
    "ref:",
    "refs/",
    "null",
    "none",
    "nil",
    "true",
    "false",
    "yes",
    "no",
    "redacted",
    "changeme",
    "example",
    "placeholder",
    "your_",
    "your-",
    "replace",
    "todo",
];

/// Whether a line assigns a literal secret to a credential-shaped key.
///
/// A value that is a reference (`${VAR}`, `keychain:...`) or a stub (`changeme`) is not a secret, and
/// reporting it would train a reader to ignore the check. The value must also be a single unquoted
/// token, which keeps structural lines such as `"token": {` from matching on the key alone.
fn credential_key(line: &str) -> bool {
    let Some(separator) = line.find([':', '=']) else {
        return false;
    };
    let key = line[..separator]
        .trim()
        .trim_matches(['"', '\'', '`', ' '])
        .to_lowercase();
    if !CREDENTIAL_KEYS.contains(&key.as_str()) {
        return false;
    }
    let value = line[separator + 1..]
        .trim()
        .trim_matches(['"', '\'', '`', ' ']);
    is_literal_secret(value)
}

/// Whether a value is a literal secret rather than a reference, a stub, or structure.
fn is_literal_secret(value: &str) -> bool {
    if value.len() < MIN_KEY_VALUE || value.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = value.to_lowercase();
    if NOT_A_SECRET.iter().any(|stub| lower.starts_with(*stub)) {
        return false;
    }
    // A value of one repeated character is a mask (`xxxxxx`, `000000`), not a credential.
    let mut chars = value.chars();
    !matches!(chars.next(), Some(first) if chars.all(|c| c == first))
}

/// Every scalar string a value holds: one for a scalar, each element for a list, nothing for a map.
///
/// An explicit `null` yields nothing, which is the point: Â§2 rule 9 requires `doctor` to treat
/// `spec: null` as "no specification" rather than as a reference that fails to resolve.
fn scalar_values(value: &Value) -> Vec<String> {
    match value {
        Value::Str(text) => vec![text.clone()],
        Value::List(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(ToString::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether a `spec` value is a path rather than a `SPEC-*` id.
fn looks_like_a_path(candidate: &str) -> bool {
    candidate.contains('/') || is_markdown(Path::new(candidate))
}

/// Whether a file name is a JSON schema, compared case-insensitively for Windows.
fn is_json(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

/// Whether a declared stack entry is a language, and so comparable with what discovery observed.
///
/// An entry that is not a language â€” a store, a tool, a framework â€” is left alone, because
/// contradicting an unobserved store would report the absence of a check as a finding.
fn is_language(name: &str) -> bool {
    LANGUAGES.contains(&fold(name).as_str())
}

/// Trims and lower-cases, so `Rust`, `rust`, and ` rust` compare equal.
fn fold(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Reads every `.md` document under `.ai/`, in path order.
fn read_tree(root: &Path) -> Vec<Loaded> {
    let mut found = Vec::new();
    walk(&root.join(CONTEXT_DIR), root, &mut found, 0);
    found.sort_by(|left, right| left.path.cmp(&right.path));
    found
}

/// Recursive descent over one directory, bounded in depth and in documents examined.
fn walk(dir: &Path, root: &Path, found: &mut Vec<Loaded>, depth: usize) {
    if depth > MAX_DEPTH || found.len() >= MAX_DOCUMENTS {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, root, found, depth + 1);
            continue;
        }
        if !is_markdown(&path) {
            continue;
        }
        found.push(read_document(&relative_to(root, &path), &path));
    }
}

/// Whether a path is a Markdown document, compared case-insensitively for Windows.
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// The repository-relative form of a path, always with `/` separators.
fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<String>>()
        .join("/")
}

/// Reads one document: its front matter, whether that is required, and its inline entities.
fn read_document(relative: &str, path: &Path) -> Loaded {
    let requires_front_matter = requires_front_matter(relative);

    let Ok(metadata) = fs::metadata(path) else {
        return unreadable(relative, "the file could not be read");
    };
    if metadata.len() > DOCUMENT_MAX_BYTES as u64 {
        // Bounded before the read, so a file far past the cap cannot be loaded in order to be
        // complained about. Â§2 rule 12 asks for the excess to be reported rather than silently
        // truncated, and a `CTX-018` warning says exactly that.
        return Loaded {
            path: relative.to_string(),
            parsed: None,
            failure: None,
            oversize: true,
            entities: Vec::new(),
            requires_front_matter,
        };
    }

    let Ok(text) = fs::read_to_string(path) else {
        return unreadable(relative, "the file is not valid UTF-8");
    };

    let (parsed, failure) = match Document::parse(&text) {
        Ok(document) => (Some(document), None),
        Err(error) => (None, Some(error.to_string())),
    };

    // The document's own identifier, which is not an inline entity: a standalone decision is its own
    // entity, and its heading repeats the id rather than opening a section that must carry a block.
    let own_id = parsed
        .as_ref()
        .and_then(|document| document.front_matter())
        .and_then(|front| front.id())
        .map(DocumentId::as_str)
        .map(str::to_string);

    Loaded {
        path: relative.to_string(),
        parsed,
        failure,
        oversize: false,
        entities: inline_entities(&text, own_id.as_deref()),
        requires_front_matter,
    }
}

/// Whether a document at this location may omit front matter (`docs/CONTEXT_SPEC.md` Â§2 rule 1).
///
/// Two locations may: `AI.md`, which is the fixed entry point and has no ID, and a Markdown file the
/// Â§1 catalogue does not describe, which is a free-form note. The second is not a convenience â€” Â§2
/// rule 2 then requires the `id` to match the location convention, and a location with no convention
/// could never satisfy that, so demanding an `id` there would make the contract unsatisfiable.
fn requires_front_matter(relative: &str) -> bool {
    relative != ENTRY_POINT && is_catalogue_location(relative)
}

/// Whether Â§1 gives this path a kind, a schema, and an ID convention.
fn is_catalogue_location(relative: &str) -> bool {
    CATALOGUE_LOCATIONS
        .iter()
        .any(|(location, _)| relative.starts_with(location))
}

/// A document that could not be read.
fn unreadable(relative: &str, reason: &str) -> Loaded {
    Loaded {
        path: relative.to_string(),
        parsed: None,
        failure: Some(reason.to_string()),
        oversize: false,
        entities: Vec::new(),
        requires_front_matter: requires_front_matter(relative),
    }
}

/// What `--explain` can say about a code.
///
/// Three answers rather than one, because the difference matters to a caller: a check that runs, a
/// check this version does not run, and a code that is not a doctor check at all. Returning a single
/// `Option<&str>` would force the caller to guess which of the three it had, and a caller that guesses
/// wrong either reports a working check as unimplemented or promises one that does not exist.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Explanation {
    /// The check runs in this version, and this is why it exists.
    Implemented(&'static str),
    /// The code is in the catalogue, this version does not run it, and this is what is missing.
    Deferred(&'static str),
    /// Nothing here knows the code.
    Unknown,
}

impl Explanation {
    /// The rationale, whichever of the three answers it is.
    #[must_use]
    pub const fn rationale(self) -> Option<&'static str> {
        match self {
            Self::Implemented(rationale) | Self::Deferred(rationale) => Some(rationale),
            Self::Unknown => None,
        }
    }

    /// Whether the check runs in this version.
    #[must_use]
    pub const fn is_implemented(self) -> bool {
        matches!(self, Self::Implemented(_))
    }

    /// Whether the code is not a doctor check at all, which is a usage error rather than an answer.
    #[must_use]
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }
}

/// The rationale for one check, or [`Explanation::Unknown`] when the code is not in the catalogue.
///
/// Case-insensitive, because a code is uppercase in `docs/CONTEXT_SPEC.md` and a person typing
/// `--explain ctx-007` has not made a different request than the one that is spelled out.
#[must_use]
pub fn explain(code: &str) -> Explanation {
    let code = code.trim().to_uppercase();
    match code.as_str() {
        "CTX-001" => Explanation::Implemented(
            "AI.md is the file an agent reads first. Without it there is no project contract, so \
             every other check is describing a tree nobody has been told how to use.",
        ),
        "CTX-002" => Explanation::Implemented(
            "Front matter is the only machine-read source of metadata; the Markdown body is never \
             parsed as structure. A document whose block is missing or malformed cannot be indexed, \
             referenced, or validated. Only AI.md and free-form notes are exempt.",
        ),
        "CTX-007" => Explanation::Implemented(
            "The reference names an entity ID or a path under docs/ that does not exist. Either the \
             document is missing or the reference is wrong, and a reader who follows it is sent \
             nowhere. Fix the reference, or create what it points at.",
        ),
        "CTX-012" => Explanation::Implemented(
            "The schema in .ai/schemas differs from the source this repository ships, so the project \
             is validating its documents against something CI never checked. Re-run init, or \
             re-apply your own changes deliberately.",
        ),
        "CTX-013" => Explanation::Implemented(
            "ARCHITECTURE.md declares a language discovery did not observe. One of the two is \
             wrong: either the document describes a stack the repository does not have, or \
             discovery is missing the evidence. Both are worth fixing, so this is a warning.",
        ),
        "CTX-014" => Explanation::Implemented(
            "A deprecated decision has no successor. A reader who finds it learns that a rule no \
             longer applies and not what replaced it, which is the worst state for a governance \
             document to be in.",
        ),
        "CTX-016" => Explanation::Implemented(
            ".ai/ is copied into every clone, so a credential written into it is published with the \
             repository. The scan reports the file, the line, and the rule that matched, and never \
             the value itself, so the report that warns about a leak does not repeat it.",
        ),
        "CTX-018" => Explanation::Implemented(
            "A document past the 1 MiB maximum was not read, because loading it would be the wrong \
             answer to a file that is too large to load. Split it, or raise the cap; the checks below \
             it did not run.",
        ),
        other => match UNIMPLEMENTED
            .iter()
            .find(|(candidate, _)| *candidate == other)
        {
            Some((_, reason)) => Explanation::Deferred(reason),
            None => Explanation::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{FrontMatter, Inputs, Report, Severity, run};
    use crate::body::{
        ENTITY_PREFIXES, entity_heading, first_yaml_block, inline_entities, section_end,
    };
    use aicontext_core::{DocumentId, ErrorCode};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Writes a file, creating its parent directories.
    fn write(root: &Path, relative: &str, contents: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("has a parent")).expect("creates");
        fs::write(path, contents).expect("writes");
    }

    /// Writes the task register: front matter as the real repository writes it, then `body`.
    ///
    /// The front matter matters as much as the body. A register with neither is a free-form note and
    /// is exempt from rule 1, so a test that wants `CTX-002` to fire has to write one the way the
    /// format requires.
    fn register(root: &Path, body: &str) {
        write(
            root,
            ".ai/TASKS.md",
            &format!(
                "---\nid: TASKS-001\ntype: tasks\ntitle: Task Register\n---\n\n# TASKS\n\n{body}"
            ),
        );
    }

    /// A minimal project that produces no findings.
    fn healthy(root: &Path) {
        write(root, ".ai/AI.md", "# AI.md\n\nEntry point.\n");
        register(
            root,
            "### TASK-001 â€” First task\n\n```yaml\nid: TASK-001\nstatus: TODO\n\
             depends_on: []\nspec: docs/CONTEXT_SPEC.md\n```\n\nBody.\n",
        );
        write(root, "docs/CONTEXT_SPEC.md", "# CONTEXT_SPEC\n");
        // A scaffolded project always has this directory, so a healthy fixture does too: otherwise
        // every test that does not care about schemas would inherit a CTX-012 finding about their
        // absence rather than the thing it is testing.
        fs::create_dir_all(root.join(".ai/schemas")).expect("schema dir");
    }

    fn inputs<'a>(
        root: &'a Path,
        languages: &'a [String],
        schemas: &'a [(&'a str, &'a str)],
    ) -> Inputs<'a> {
        Inputs {
            root,
            observed_languages: languages,
            schema_source: schemas,
        }
    }

    /// The finding carrying a code, so a test can assert on it without searching twice.
    fn finding<'a>(report: &'a Report, code: &str) -> &'a super::Finding {
        report
            .findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("expected {code} in {:?}", report.findings))
    }

    #[test]
    fn a_healthy_tree_produces_no_findings() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        let report = run(&inputs(root, &[], &[]));
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert!(report.checked.iter().any(|row| row.path == ".ai/AI.md"));
    }

    #[test]
    fn a_missing_entry_point_is_an_error() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        fs::create_dir_all(root.join(".ai")).expect("creates");
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-001");
        assert_eq!(found.severity, Severity::Error);
        assert_eq!(found.path, ".ai/AI.md");
        assert!(report.has_errors());
    }

    #[test]
    fn malformed_front_matter_is_an_error_naming_the_document() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/RULES.md",
            "---\nid: RULES-001\n  bad: [unclosed\n---\n",
        );
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-002");
        assert_eq!(found.path, ".ai/RULES.md");
        assert!(found.message.contains("cannot be read"), "{found:?}");
    }

    #[test]
    fn a_document_with_no_front_matter_is_an_error_except_the_entry_point() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(root, ".ai/RULES.md", "# RULES\n\nNo block.\n");
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-002");
        assert_eq!(found.path, ".ai/RULES.md");
        assert!(found.message.contains("no front matter"), "{found:?}");
        assert!(
            !report.findings.iter().any(|f| f.path == ".ai/AI.md"),
            "AI.md is exempt: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_document_missing_one_required_key_is_reported_against_that_key() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/RULES.md",
            "---\nid: RULES-001\ntype: rules\n---\n",
        );
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-002");
        assert!(found.message.contains("title"), "{found:?}");
    }

    #[test]
    fn an_inline_entity_with_no_block_is_an_error() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(root, "### TASK-001 â€” One\n\nJust prose.\n");
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-002");
        assert!(found.message.contains("TASK-001"), "{found:?}");
        assert!(found.message.contains("no fenced yaml block"), "{found:?}");
    }

    #[test]
    fn a_dangling_task_reference_is_an_error() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(
            root,
            "### TASK-001 â€” First task\n\n```yaml\nid: TASK-001\ndepends_on: [TASK-099]\n```\n",
        );
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-007");
        assert!(found.message.contains("TASK-099"), "{found:?}");
        assert_eq!(found.severity, Severity::Error);
    }

    #[test]
    fn a_document_named_by_a_tree_wide_finding_is_not_also_listed_as_checked() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // `RULES.md` parses, so the per-document pass is ready to call it clean; the reference
        // resolution pass then fails it. The register is reported, so the contradiction is visible
        // here, which is the point.
        write(
            root,
            ".ai/RULES.md",
            "---\nid: RULES-001\ntype: rules\nstatus: ACTIVE\nspec: docs/NOPE.md\n---\n\n# RULES\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            finding(&report, "CTX-007").message.contains("NOPE"),
            "{:?}",
            report.findings
        );
        assert!(
            !report.checked.iter().any(|row| row.path == ".ai/RULES.md"),
            "and a file cannot be ok and failed in the same report: {:?}",
            report.checked
        );
        // The documents the failing check had nothing to say about are still listed, because
        // suppressing the whole report would hide that the rest of the tree was read.
        assert!(
            report.checked.iter().any(|row| row.path == ".ai/AI.md"),
            "{:?}",
            report.checked
        );
    }

    #[test]
    fn a_task_pointing_at_a_missing_specification_is_an_error() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(
            root,
            "### TASK-001 â€” First\n\n```yaml\nid: TASK-001\nspec: SPEC-auth-missing\n```\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            finding(&report, "CTX-007")
                .message
                .contains("SPEC-auth-missing"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn a_spec_path_that_does_not_exist_is_an_error() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(
            root,
            "### TASK-001 â€” First\n\n```yaml\nid: TASK-001\nspec: docs/NOPE.md\n```\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            finding(&report, "CTX-007").message.contains("docs/NOPE.md"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn an_explicit_null_spec_is_not_a_dangling_reference() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(
            root,
            "### TASK-001 â€” First\n\n```yaml\nid: TASK-001\nspec: null\n```\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-007"),
            "spec: null means no specification: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_reference_to_an_existing_entity_resolves() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(
            root,
            "### TASK-001 â€” First\n\n```yaml\nid: TASK-001\ndepends_on: []\n```\n\n\
             ### TASK-002 â€” Second\n\n```yaml\nid: TASK-002\ndepends_on: [TASK-001]\n```\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-007"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn a_schema_that_differs_from_the_source_is_a_warning() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(root, ".ai/schemas/task.schema.json", "{\"edited\": true}");
        let report = run(&inputs(root, &[], &[("task.schema.json", "{}")]));
        let found = finding(&report, "CTX-012");
        assert_eq!(found.severity, Severity::Warning);
        assert_eq!(found.path, ".ai/schemas/task.schema.json");
    }

    #[test]
    fn a_crlf_schema_source_matches_an_lf_copy() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // `init` writes LF (docs/CONTEXT_SPEC.md rule 11); the compiled-in source can arrive with
        // CRLF from a Windows checkout. The comparison is about content, so a scaffolded project
        // must not be reported as hand-edited just because the two sides spell newlines differently.
        write(root, ".ai/schemas/task.schema.json", "{\n}\n");
        let report = run(&inputs(root, &[], &[("task.schema.json", "{\r\n}\r\n")]));
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.code == "CTX-012"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn a_schema_missing_from_the_copy_is_a_warning() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // The directory exists and holds one of the two schemas, so the other is missing on its own
        // rather than as part of a directory that was never written.
        write(root, ".ai/schemas/memory.schema.json", "{}");
        let report = run(&inputs(
            root,
            &[],
            &[("task.schema.json", "{}"), ("memory.schema.json", "{}")],
        ));
        let found = finding(&report, "CTX-012");
        assert_eq!(found.path, ".ai/schemas/task.schema.json");
        assert!(found.message.contains("missing"), "{:?}", report.findings);
    }

    #[test]
    fn a_directory_with_no_schema_copies_at_all_is_one_finding() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // The fixture is healthy, so the directory is there; this is the project that never ran
        // `init`, so it goes.
        fs::remove_dir_all(root.join(".ai/schemas")).expect("remove schema dir");
        let report = run(&inputs(
            root,
            &[],
            &[("task.schema.json", "{}"), ("memory.schema.json", "{}")],
        ));
        // One problem with one fix, not one finding per schema: the repository that skips `init`
        // would otherwise open its report with twenty identical lines.
        assert_eq!(finding(&report, "CTX-012").path, ".ai/schemas");
        assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    }

    #[test]
    fn a_schema_the_project_added_is_reported_as_unvalidated() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(root, ".ai/schemas/mine.schema.json", "{}");
        let report = run(&inputs(root, &[], &[]));
        assert!(
            finding(&report, "CTX-012")
                .path
                .contains("mine.schema.json"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn a_declared_language_discovery_did_not_see_is_a_warning() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/ARCHITECTURE.md",
            "---\nid: ARCH-001\ntype: architecture\ntitle: A\nstack: [rust, python]\n---\n",
        );
        let report = run(&inputs(root, &["rust".to_string()], &[]));
        let found = finding(&report, "CTX-013");
        assert!(found.message.contains("python"), "{found:?}");
        assert_eq!(found.severity, Severity::Warning);
    }

    #[test]
    fn a_declared_stack_that_matches_discovery_is_silent() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/ARCHITECTURE.md",
            "---\nid: ARCH-001\ntype: architecture\ntitle: A\nstack: [Rust]\n---\n",
        );
        let report = run(&inputs(root, &["rust".to_string()], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-013"),
            "case must not decide a match: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_non_language_stack_entry_is_not_contradicted() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/ARCHITECTURE.md",
            "---\nid: ARCH-001\ntype: architecture\ntitle: A\nstack: [rust, postgresql]\n---\n",
        );
        let report = run(&inputs(root, &["rust".to_string()], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-013"),
            "a store is not comparable with observed languages: {:?}",
            report.findings
        );
    }

    #[test]
    fn nothing_is_contradicted_when_discovery_recognised_nothing() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/ARCHITECTURE.md",
            "---\nid: ARCH-001\ntype: architecture\ntitle: A\nstack: [python]\n---\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-013"),
            "a contradiction needs something observed: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_deprecated_decision_with_no_successor_is_a_warning() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/decisions/ADR-001-x.md",
            "---\nid: ADR-001\ntype: decision\ntitle: X\nstatus: deprecated\n\
             superseded_by: null\n---\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            finding(&report, "CTX-014").message.contains("ADR-001"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn a_deprecated_decision_with_a_successor_is_not_reported() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/decisions/ADR-001-x.md",
            "---\nid: ADR-001\ntype: decision\ntitle: X\nstatus: deprecated\n\
             superseded_by: ADR-002\n---\n",
        );
        write(
            root,
            ".ai/decisions/ADR-002-y.md",
            "---\nid: ADR-002\ntype: decision\ntitle: Y\nstatus: accepted\n---\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-014"),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn a_heading_at_any_level_opens_an_entity_section() {
        // The two shapes this repository actually uses: TASKS.md at level 3, MEMORY.md at level 2.
        for line in [
            "## MEM-001 â€” One",
            "### TASK-001 â€” One",
            "#### ADR-001 â€” One",
        ] {
            let (level, _) = entity_heading(line).unwrap_or_else(|| panic!("{line}"));
            assert!(level >= 2, "{line}");
        }
        assert!(entity_heading("## How to read this file").is_none());
        assert!(entity_heading("# Ordinary heading").is_none());
        assert!(entity_heading("## 2.1 Inline blocks").is_none());
        assert!(entity_heading("##MEM-001 no space").is_none());
    }

    #[test]
    fn an_entity_id_is_read_up_to_the_first_whitespace() {
        for heading in [
            "### TASK-014 â€” Implement doctor",
            "### TASK-014 - Implement doctor",
            "### TASK-014\tImplement doctor",
            "## MEM-001 / A lesson",
        ] {
            let (_, id) = entity_heading(heading).unwrap_or_else(|| panic!("{heading}"));
            assert!(
                DocumentId::new(&id).is_ok(),
                "{id} is not an ID, taken from {heading}"
            );
        }
    }

    #[test]
    fn a_heading_that_only_starts_like_an_id_is_not_an_entity() {
        // `TASK-014:` is not an ID, and treating the prefix as enough would invent an entity that
        // every reference to it then fails to resolve.
        assert!(entity_heading("## TASK-014: the plan").is_none());
        assert!(entity_heading("## TASK-014-extra").is_none());
    }

    #[test]
    fn every_entity_prefix_is_recognised() {
        for prefix in ENTITY_PREFIXES {
            let line = format!("## {prefix}1 â€” Title");
            assert!(
                entity_heading(&line).is_some(),
                "{prefix} is not recognised"
            );
        }
    }

    #[test]
    fn a_section_ends_at_the_next_heading_of_the_same_or_higher_level() {
        let lines = ["### TASK-001", "body", "## MEM-001", "other"];
        assert_eq!(section_end(&lines, 0, 3), 2);
        let lines = ["### TASK-001", "body", "#### deeper", "other"];
        assert_eq!(
            section_end(&lines, 0, 3),
            4,
            "a deeper heading is still inside the section"
        );
    }

    #[test]
    fn only_the_first_yaml_block_in_a_section_is_taken() {
        let lines = [
            "```yaml",
            "id: TASK-001",
            "```",
            "text",
            "```yaml",
            "id: TASK-999",
            "```",
        ];
        let block = first_yaml_block(&lines).expect("a block");
        assert!(block.contains("TASK-001"), "{block}");
        assert!(!block.contains("TASK-999"), "{block}");
    }

    #[test]
    fn a_fenced_block_that_is_not_yaml_is_not_an_entity_block() {
        assert!(first_yaml_block(&["```rust", "let x = 1;", "```"]).is_none());
        assert!(first_yaml_block(&["```", "plain", "```"]).is_none());
    }

    #[test]
    fn an_unclosed_yaml_block_is_not_treated_as_a_block() {
        assert!(first_yaml_block(&["```yaml", "id: TASK-001"]).is_none());
    }

    #[test]
    fn inline_entities_are_read_from_a_register_in_document_order() {
        let text = "### TASK-001 â€” One\n\n```yaml\nid: TASK-001\nstatus: TODO\n```\n\n\
                    ### TASK-002 â€” Two\n\n```yaml\nid: TASK-002\nstatus: DONE\n```\n";
        let entities = inline_entities(text, None);
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].heading_id, "TASK-001");
        assert_eq!(entities[1].heading_id, "TASK-002");
        let status = entities[1].front().and_then(FrontMatter::status);
        assert_eq!(status, Some("DONE"));
    }

    #[test]
    fn a_document_heading_that_repeats_its_own_id_is_not_an_inline_entity() {
        // Found by running `doctor` over this repository: every `.ai/decisions/ADR-NNN-*.md` opens
        // with an `# ADR-NNN — …` title, so all seven decisions were reported as an entity heading
        // followed by no block. The document is the entity; its title is not another one.
        let text = "# ADR-001 — Use Rust\n\nProse, and a second heading for emphasis.\n\n\
                    ## Decision\n\nUse Rust.\n";
        assert!(
            inline_entities(text, Some("ADR-001")).is_empty(),
            "own title is skipped"
        );
        assert_eq!(
            inline_entities(text, None).len(),
            1,
            "but it is one without an own id"
        );
        assert_eq!(
            inline_entities(text, Some("ADR-999"))[0].heading_id,
            "ADR-001",
            "a different document does claim this heading"
        );
    }

    #[test]
    fn a_hash_inside_a_yaml_block_is_a_comment_and_not_a_heading() {
        // Found by running `doctor` over a scaffolded project: the `# On completion, replace...`
        // comment in the task template ended its own section one line early, so the block the heading
        // named was reported as missing. CommonMark says the fence wins, and a task block carrying
        // `# comments` is the ordinary case rather than an edge.
        let text = "### TASK-001 - One\n\n```yaml\nid: TASK-001\nstatus: TODO\n\
                    # a comment that begins with a hash\ndone: []\n```\n";
        let entities = inline_entities(text, None);
        assert_eq!(entities.len(), 1, "one entity, one block");
        assert!(
            entities[0].had_block,
            "the block is under its heading, comments and all"
        );
        assert_eq!(entities[0].heading_id, "TASK-001");
    }

    #[test]
    fn a_heading_that_merely_appears_in_a_fenced_example_is_not_an_entity() {
        // `TASKS.md` documents its own format with a fenced example. Reading the example as structure
        // would report a phantom entity in every document that explains its own format.
        let text = "## How to read this file\n\n```yaml\nid: TASK-NNN\nstatus: TODO\n```\n\n\
                    ## Phase 1\n";
        assert_eq!(
            inline_entities(text, None).len(),
            0,
            "an example block is prose"
        );
    }

    #[test]
    fn the_task_count_appears_in_the_ok_row() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        register(
            root,
            "### TASK-001 â€” One\n\n```yaml\nid: TASK-001\n```\n\n\
             ### TASK-002 â€” Two\n\n```yaml\nid: TASK-002\n```\n",
        );
        let report = run(&inputs(root, &[], &[]));
        let row = report
            .checked
            .iter()
            .find(|row| row.path == ".ai/TASKS.md")
            .expect("a row for the register");
        assert_eq!(row.note, "2 tasks");
    }

    #[test]
    fn a_document_past_the_size_cap_is_reported_rather_than_loaded() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        let big = format!("---\nid: RULES-001\n---\n{}", "x".repeat(1024 * 1024 + 16));
        write(root, ".ai/RULES.md", &big);
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-018");
        assert_eq!(found.severity, Severity::Warning);
        assert!(
            found.message.contains("larger than"),
            "rule 12 asks for the excess to be reported: {found:?}"
        );
        assert!(
            !report.checked.iter().any(|row| row.path == ".ai/RULES.md"),
            "and a document that was not read cannot be in the checked list: {:?}",
            report.checked
        );
    }

    #[test]
    fn a_free_form_note_needs_no_front_matter() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // Not a location Â§1 gives a kind, a schema, and an ID convention to.
        write(
            root,
            ".ai/scratch.md",
            "# A note to myself\n\nNo block, and none owed.\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.path == ".ai/scratch.md"),
            "rule 1 exempts free-form notes: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_catalogue_location_may_not_omit_front_matter_even_in_an_unlisted_directory() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(root, ".ai/tasks/TASK-001-x.md", "# A task\n\nNo block.\n");
        let report = run(&inputs(root, &[], &[]));
        assert!(
            finding(&report, "CTX-002").path == ".ai/tasks/TASK-001-x.md",
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn explain_covers_every_code_a_check_can_emit() {
        for code in [
            "CTX-001", "CTX-002", "CTX-007", "CTX-012", "CTX-013", "CTX-014", "CTX-016", "CTX-018",
        ] {
            assert!(
                super::explain(code).is_implemented(),
                "{code} runs, so it must have a rationale"
            );
        }
        assert!(
            super::explain("CTX-010").rationale().is_some(),
            "an absent check says what is missing"
        );
        assert!(
            super::explain("CTX-999").is_unknown(),
            "a code that is not a check has nothing to say"
        );
    }

    /// Assembles a credential-shaped value without committing a literal one to the repository.
    fn synthetic(prefix: &str, filler: &str) -> String {
        format!("{prefix}{filler}")
    }

    #[test]
    fn a_credential_shaped_value_is_reported_and_never_echoed() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // Assembled at run time so this test does not itself hold a credential-shaped literal.
        let secret = synthetic("ghp_", &"a".repeat(36));
        write(root, ".ai/scratch.env", &format!("GITHUB_TOKEN={secret}\n"));
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-016");
        assert_eq!(found.severity, Severity::Error);
        assert_eq!(found.path, ".ai/scratch.env");
        assert!(found.message.contains("line 1"), "{found:?}");
        assert!(found.message.contains("github-token"), "{found:?}");
        assert!(
            !format!("{:?}", report.findings).contains(&secret),
            "the report must not echo the secret it found"
        );
    }

    #[test]
    fn a_credential_shaped_key_holding_a_literal_is_reported() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        let literal = ["hun", "ter", "2"].concat();
        write(root, ".ai/notes.txt", &format!("password: {literal}\n"));
        let report = run(&inputs(root, &[], &[]));
        let found = finding(&report, "CTX-016");
        assert!(found.message.contains("credential-key"), "{found:?}");
        assert!(found.message.contains("line 1"), "{found:?}");
        assert!(!format!("{:?}", report.findings).contains(&literal));
    }

    #[test]
    fn the_scan_reports_the_line_the_offending_content_is_on() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        let literal = ["sek", "rit"].concat();
        write(
            root,
            ".ai/notes.txt",
            &format!("# a comment\napi_key: {literal}\n"),
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(finding(&report, "CTX-016").message.contains("line 2"));
    }

    #[test]
    fn a_reference_or_a_stub_is_not_a_credential() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        write(
            root,
            ".ai/notes.txt",
            "api_key: ${OPENAI_API_KEY}\ntoken: changeme\nsecret: null\ntoken: xxxxxx\n",
        );
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-016"),
            "a reference, a stub, or a mask is not a secret: {:?}",
            report.findings
        );
    }

    #[test]
    fn the_scan_reads_every_file_not_only_documents() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        // `permissions.yaml` is not Markdown, and a secret leaked into it is still published.
        let secret = synthetic("AKIA", "IOSFODNN7EXAMPLE");
        write(
            root,
            ".ai/permissions/permissions.yaml",
            &format!("note: {secret}\n"),
        );
        let report = run(&inputs(root, &[], &[]));
        assert_eq!(
            finding(&report, "CTX-016").path,
            ".ai/permissions/permissions.yaml"
        );
    }

    #[test]
    fn a_file_past_the_size_cap_is_not_scanned() {
        let root = TempDir::new().expect("temp dir");
        // `root` is the temp directory for its whole life, and the path inside it for the rest of
        // the test, so the call sites read as the project root they are.
        let root = root.path();
        healthy(root);
        let mut body = "x".repeat(1024 * 1024 + 16);
        body.push_str(&synthetic("AKIA", "IOSFODNN7EXAMPLE"));
        body.push('\n');
        write(root, ".ai/big.env", &body);
        let report = run(&inputs(root, &[], &[]));
        assert!(
            !report.findings.iter().any(|f| f.code == "CTX-016"),
            "a file past the cap is skipped rather than loaded: {:?}",
            report.findings
        );
    }

    #[test]
    fn a_finding_carries_every_field_a_script_needs() {
        let found = super::Finding::new(
            ErrorCode::CTX_001,
            Severity::Error,
            ".ai/AI.md",
            "missing",
            "run init",
        );
        assert_eq!(found.code, "CTX-001");
        assert_eq!(found.severity.as_str(), "error");
        assert_eq!(found.path, ".ai/AI.md");
        assert!(!found.remediation.is_empty());
    }
}
