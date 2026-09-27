//! The front-matter parser: the machine-readable half of a document.
//!
//! A document is an optional YAML block, fenced by `---`, followed by free text that this module
//! never looks inside. The split is the whole contract from `docs/CONTEXT_SPEC.md` §2, and the
//! reason it lives here rather than a layer up is that every later feature depends on the same
//! split: discovery, the index, `doctor`, and retrieval all read metadata, and none of them should
//! need to understand Markdown to get it.
//!
//! # What "typed" means here, and what it deliberately does not
//!
//! The well-known keys are typed and validated as they are read: `id` becomes a [`DocumentId`], a
//! date is checked against the calendar, `tags` is a list of strings. Keys that belong to a
//! specific document type - `depends_on`, `deciders`, `supersedes` - are **not** interpreted here,
//! because which keys are valid depends on the schema for that location, and those schemas arrive
//! in `TASK-015`. They are kept in [`FrontMatter::extras`] as [`Value`]s, preserved on read and
//! preserved on write (`docs/CONTEXT_SPEC.md` §2 rule 5). Guessing a vocabulary here would put two
//! places in the workspace that know the schema, which `RULES.md` §2 forbids.
//!
//! # One home for each fact
//!
//! - Whether a key is *allowed* for a given document type: the schema, in `TASK-015`.
//! - Whether a required key is *missing*: [`FrontMatter::missing_required_keys`] reports the facts;
//!   the caller decides whether their absence is an error, because rule 1 makes it conditional on
//!   the location (`AI.md` and free-form notes are exempt).
//! - Whether an `id` *matches its location*: `doctor`, in `TASK-014`.
//!
//! # Inline entity blocks are not handled here
//!
//! `docs/CONTEXT_SPEC.md` §2.1 defines a second shape, a level-2 heading followed by a fenced block
//! inside a body, used by the task and memory registers. Reading it means walking Markdown
//! structure, which `TASK-016` is explicitly not allowed to do, so it is left to `doctor` in
//! `TASK-014`, which already has to walk bodies to find inline entities. This module reads the
//! leading block only, and says so rather than half-implementing the other one.
//!
//! # Example
//!
//! ```
//! use aicontext_context::Document;
//!
//! let document = Document::parse(
//!     "---\nid: TASK-014\ntype: task\ntitle: Implement the lexical retriever\n\
//!      status: IN_PROGRESS\ncreated: 2026-09-27\n---\n\nBody text a human wrote.\n",
//! )
//! .expect("a well-formed document");
//!
//! let front_matter = document.front_matter().expect("front matter is present");
//! assert_eq!(front_matter.id().map(ToString::to_string).as_deref(), Some("TASK-014"));
//! assert_eq!(front_matter.kind(), Some("task"));
//! assert_eq!(front_matter.created(), Some("2026-09-27"));
//! assert_eq!(document.body(), "\nBody text a human wrote.\n");
//! ```

use std::collections::BTreeMap;
use std::str::FromStr;

use aicontext_core::DocumentId;

use crate::codec::{SerdeYaml, YamlCodec};
use crate::error::ContextError;
use crate::value::Value;

/// The maximum size of a front-matter block, in bytes (`docs/CONTEXT_SPEC.md` §2 rule 11).
pub const FRONT_MATTER_MAX_BYTES: usize = 64 * 1024;

/// The maximum size of a whole document, in bytes (`docs/CONTEXT_SPEC.md` §2 rule 11).
pub const DOCUMENT_MAX_BYTES: usize = 1024 * 1024;

/// The fence that opens and closes a front-matter block.
const FENCE: &str = "---";

/// The line the block's own first line occupies, given that the fence must be line 1.
///
/// Rule 1 of the format is that front matter is a *leading* block, and [`Block::split`] refuses
/// anything else, so this is a constant rather than a count. It exists as a named value because
/// every error in this module has to add it to a block-relative line, and a bare `2` at each of
/// those sites would be a number nobody could check.
const FIRST_CONTENT_LINE: usize = 2;

/// A document: an optional front-matter block, and a body this crate does not parse.
// `Eq` rather than `PartialEq` alone because no float can reach either type, but `Value` is
// recursive and derives only `PartialEq`, and the distinction is not worth a manual impl.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    front_matter: Option<FrontMatter>,
    body: String,
}

impl Document {
    /// Reads a document from its file contents.
    ///
    /// # Errors
    ///
    /// Returns [`ContextError`] if the block is unterminated, is not a mapping, is not valid YAML,
    /// repeats a key, holds a value of the wrong shape for a well-known key, or exceeds a size
    /// limit. Every one of those names a 1-based line in *this* string, except the size limits,
    /// which are about the file as a whole.
    ///
    /// # Example
    ///
    /// ```
    /// use aicontext_context::{ContextError, Document};
    ///
    /// let error = Document::parse("---\nid: TASK-014\n").expect_err("no closing fence");
    /// assert!(matches!(error, ContextError::FrontMatterNotClosed { line: 1 }));
    /// ```
    pub fn parse(source: &str) -> Result<Self, ContextError> {
        match Block::split(source)? {
            Some(block) => {
                let body = block.body.to_string();
                let front_matter = FrontMatter::from_block(&block)?;
                Ok(Self {
                    front_matter: Some(front_matter),
                    body,
                })
            }
            // No fence on the first line. The whole file is the body, which is a legal document
            // for `AI.md` and free-form notes (rule 1) and a finding for everything else.
            None => Ok(Self {
                front_matter: None,
                body: source.to_string(),
            }),
        }
    }

    /// The parsed front matter, or `None` if the document has no block at all.
    ///
    /// `None` and `Some` with every field absent are different facts: the first means the document
    /// has no machine-readable metadata, the second means it has an empty block. `doctor` reports
    /// them differently, and a parser that collapsed them would make that distinction impossible.
    #[must_use]
    pub fn front_matter(&self) -> Option<&FrontMatter> {
        self.front_matter.as_ref()
    }

    /// The body, exactly as it appeared in the file.
    ///
    /// Free text. Nothing in this crate interprets it, and a human may restructure it freely
    /// (`docs/CONTEXT_SPEC.md` §2 rule 8).
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Renders the document back to its file form: the block, then the body, verbatim.
    ///
    /// The front matter is emitted in the key order of `docs/CONTEXT_SPEC.md` §2 rule 4, and with
    /// `\n` line endings (rule 10). The body is reproduced **exactly**, including its own line
    /// endings, because quietly rewriting a human's prose is not a decision this parser should
    /// make; the write path normalises the whole file at once, and that is where it belongs.
    ///
    /// Round-trips are canonical rather than byte-identical: every key and value is preserved, but
    /// keys the format does not order are emitted sorted rather than as the author left them.
    /// Parsing a rendered document and rendering it again is a fixed point.
    ///
    /// # Errors
    ///
    /// Returns [`ContextError::NotWritable`] if a value in memory has no YAML spelling. Every value
    /// that can be read can be written, so this cannot happen from a parsed document; it is
    /// reported rather than swallowed because the alternative - writing an empty block - would
    /// discard a document's metadata and leave the author with no clue why.
    ///
    /// # Example
    ///
    /// ```
    /// use aicontext_context::Document;
    ///
    /// let document = Document::parse("---\nid: TASK-014\ntitle: Lexical retriever\n---\nBody.\n")
    ///     .expect("a well-formed document");
    ///
    /// assert_eq!(
    ///     document.render().expect("a parsed document can always be written"),
    ///     "---\nid: TASK-014\ntitle: Lexical retriever\n---\nBody.\n"
    /// );
    /// ```
    pub fn render(&self) -> Result<String, ContextError> {
        let Some(front_matter) = &self.front_matter else {
            return Ok(self.body.clone());
        };

        let mut rendered = String::with_capacity(self.body.len() + 256);
        rendered.push_str(FENCE);
        rendered.push('\n');
        rendered.push_str(&front_matter.render_block()?);
        rendered.push_str(FENCE);
        rendered.push('\n');
        rendered.push_str(&self.body);
        Ok(rendered)
    }
}

impl FromStr for Document {
    type Err = ContextError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::parse(source)
    }
}

/// A located front-matter block: the block's text, and the body that followed it.
struct Block<'source> {
    /// The block's lines, with both fences removed.
    yaml: &'source str,
    /// The body after the closing fence, exactly as written.
    body: &'source str,
}

impl<'source> Block<'source> {
    /// Splits `source` into its block and body, or reports that there is no block.
    ///
    /// The opening fence must be the first line of the file, and a leading blank line is not
    /// tolerated. The reason is specific: `---` is also a Markdown horizontal rule, so a document
    /// with no front matter whose body opens with a rule would otherwise be read as a document with
    /// a very strange front matter. Requiring the fence at line 1 removes the ambiguity without a
    /// heuristic.
    fn split(source: &'source str) -> Result<Option<Self>, ContextError> {
        if source.len() > DOCUMENT_MAX_BYTES {
            return Err(ContextError::DocumentTooLarge {
                bytes: source.len(),
                limit: DOCUMENT_MAX_BYTES,
            });
        }

        let opening_end = end_of_line(source, 0);
        if !is_fence(&source[..opening_end]) {
            return Ok(None);
        }

        let mut offset = opening_end;
        while offset < source.len() {
            let end = end_of_line(source, offset);
            if is_fence(&source[offset..end]) {
                let yaml = &source[opening_end..offset];
                if yaml.len() > FRONT_MATTER_MAX_BYTES {
                    return Err(ContextError::FrontMatterTooLarge {
                        bytes: yaml.len(),
                        limit: FRONT_MATTER_MAX_BYTES,
                    });
                }
                if let Some(found) = first_foreign_line_break(yaml) {
                    return Err(ContextError::Yaml {
                        line: FIRST_CONTENT_LINE + found.line,
                        reason: format!(
                            "a bare `{}` ends a line for the YAML layer but not for this file, so \
                             the line numbers would not agree; front matter uses `\\n` line \
                             endings",
                            found.character.escape_debug()
                        ),
                    });
                }
                return Ok(Some(Self {
                    yaml,
                    body: &source[end..],
                }));
            }
            offset = end;
        }

        Err(ContextError::FrontMatterNotClosed { line: 1 })
    }

    /// The block's own top-level keys, with the file line each is written on.
    ///
    /// A line counts as a top-level key when it starts in column 0 with `key:` or with `key` and
    /// nothing else, which is the shape every example in `docs/CONTEXT_SPEC.md` uses. Indented keys
    /// belong to a nested mapping and are skipped; a key spelled in flow style or buried in a block
    /// scalar is not seen here, which is why this informs the error messages rather than replacing
    /// the YAML layer's own checks.
    fn top_level_keys(&self) -> Vec<(&str, usize)> {
        self.yaml
            .lines()
            .enumerate()
            .filter_map(|(offset, line)| {
                if line.starts_with(char::is_whitespace) || line.starts_with('#') {
                    return None;
                }
                let (key, _) = line.split_once(':')?;
                let usable = !key.is_empty() && !key.contains(' ');
                usable.then_some((key, FIRST_CONTENT_LINE + offset))
            })
            .collect()
    }

    /// The file line the named key is written on.
    ///
    /// Falls back to the block's first line when the key cannot be located, which is always a real
    /// line of the block. A wrong answer is therefore imprecise rather than pointing into a part of
    /// the file the key is not in.
    fn line_of(&self, key: &str) -> usize {
        self.top_level_keys()
            .into_iter()
            .find(|(name, _)| *name == key)
            .map_or(FIRST_CONTENT_LINE, |(_, line)| line)
    }
}

/// The end of the line that starts at `offset`, terminator included, or the end of `source`.
fn end_of_line(source: &str, offset: usize) -> usize {
    source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index + 1)
}

/// Where a line ending appeared that this format does not use, and on which block line.
struct ForeignLineBreak {
    /// The block line, 0-based, as [`first_foreign_line_break`] counts them.
    line: usize,
    /// The character that ended the line.
    character: char,
}

/// Finds the first line ending in `yaml` that this format does not use, if there is one.
///
/// YAML treats LF, CR, NEL, LS, and PS all as line breaks, while a file's lines are separated by LF.
/// A block containing a bare CR - or one of the three exotic ones - therefore has *more* lines for
/// the YAML layer than it has for the file, and every line number that layer reports after the
/// offender is one or more too high. The author would be sent past the end of their own document.
///
/// A `CR` immediately before the LF is the ordinary CRLF ending and is left alone, so a
/// Windows-authored file parses exactly as a Unix one does.
fn first_foreign_line_break(yaml: &str) -> Option<ForeignLineBreak> {
    yaml.lines().enumerate().find_map(|(line, text)| {
        // `lines` has already split on the LF, so a trailing CR here is the other half of a CRLF.
        let text = text.strip_suffix('\r').unwrap_or(text);
        let character = text
            .chars()
            .find(|character| matches!(character, '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))?;
        Some(ForeignLineBreak { line, character })
    })
}

/// Whether a line is a `---` fence.
///
/// A trailing carriage return and trailing spaces are tolerated, because both are artefacts an
/// editor can introduce without the author meaning anything by them, and a Windows-authored file
/// must parse identically to a Unix one. Nothing else is: `--- # note` is a YAML comment line, not
/// a fence, and treating it as one would hide whatever it was commenting on.
fn is_fence(line: &str) -> bool {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.trim_end() == FENCE
}

/// The parsed contents of a front-matter block. `PartialEq` only, as for [`Document`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrontMatter {
    id: Option<DocumentId>,
    kind: Option<String>,
    title: Option<String>,
    status: Option<String>,
    created: Option<String>,
    updated: Option<String>,
    tags: Vec<String>,
    /// Whether the document wrote a `tags` key at all.
    ///
    /// Not the same fact as `tags` being empty, and the difference has to survive: `tags: []` is a
    /// claim that the document has no tags, while an absent `tags` is a claim that nobody has said
    /// anything about its tags. Collapsing them would let a rewrite delete a key the author wrote.
    tags_present: bool,
    extras: BTreeMap<String, Value>,
}

impl FrontMatter {
    /// The entity identifier, validated.
    ///
    /// The shape is checked here; whether it *matches the document's location* is checked by
    /// `doctor` (`docs/CONTEXT_SPEC.md` §2 rule 2). A parser that enforced the location would have
    /// to know where the file is, and the same block is read from stdin.
    #[must_use]
    pub fn id(&self) -> Option<&DocumentId> {
        self.id.as_ref()
    }

    /// The `type` key, as written.
    ///
    /// Not an enum, on purpose. The vocabulary is per-location and arrives with the schemas in
    /// `TASK-015`, so a closed type here would be either wrong or would have to be widened later.
    #[must_use]
    pub fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }

    /// The `title` key, as written.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// The `status` key, as written.
    ///
    /// A string, not an enum, for the same reason as [`FrontMatter::kind`]: a task's status
    /// vocabulary and a decision's are disjoint, and the union of every document type's values is
    /// not a type anyone wants to hold.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// The `created` date, guaranteed `YYYY-MM-DD` and a real day on the calendar.
    ///
    /// A string rather than a date type: a date is a domain value shared with the index and with
    /// discovery, so its type belongs in `aicontext-core` and is added by the task that needs it,
    /// rather than defined here and then moved. The validation is real in the meantime, because
    /// `created: 2026-13-45` is a mistake worth reporting now rather than at index time.
    #[must_use]
    pub fn created(&self) -> Option<&str> {
        self.created.as_deref()
    }

    /// The `updated` date, guaranteed as for [`FrontMatter::created`].
    #[must_use]
    pub fn updated(&self) -> Option<&str> {
        self.updated.as_deref()
    }

    /// The `tags` list, which is empty when the document has none.
    ///
    /// Empty here means one of two things - the document wrote `tags: []`, or it wrote no `tags`
    /// key at all - and this module keeps them apart internally so that a rewrite does not
    /// manufacture one from the other. Nothing downstream needs to tell them apart yet: `doctor`
    /// compares a document against its schema, and a schema that requires `tags` is satisfied
    /// equally by an empty list and by an absent key.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// The type-specific and unknown keys, preserved exactly as read.
    ///
    /// Which of these are legitimate depends on the document's schema, so this module makes no
    /// judgement about them. `doctor` compares them against the schema and reports the rest as
    /// `CTX-006` warnings (`docs/CONTEXT_SPEC.md` §2 rule 5).
    #[must_use]
    pub fn extras(&self) -> &BTreeMap<String, Value> {
        &self.extras
    }

    /// The value of a type-specific or unknown key.
    #[must_use]
    pub fn extra(&self, key: &str) -> Option<&Value> {
        self.extras.get(key)
    }

    /// The required keys that are absent, in the order rule 1 lists them.
    ///
    /// This reports the facts; the caller decides whether they matter, because `AI.md` and
    /// free-form notes are exempt from rule 1 and only the caller knows the location. Returning
    /// the missing names rather than a boolean is what lets `doctor` name them in the finding.
    #[must_use]
    pub fn missing_required_keys(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.id.is_none() {
            missing.push("id");
        }
        if self.kind.is_none() {
            missing.push("type");
        }
        if self.title.is_none() {
            missing.push("title");
        }
        missing
    }

    /// Renders the block's contents as YAML, in rule 4 order, without the fences.
    fn render_block(&self) -> Result<String, ContextError> {
        SerdeYaml
            .encode_mapping(&self.ordered_entries())
            .map_err(|error| ContextError::NotWritable {
                reason: error.to_string(),
            })
    }

    /// Every key and value, in the key order of `docs/CONTEXT_SPEC.md` §2 rule 4.
    ///
    /// `id`, `type`, `title`, `status`, then the type-specific keys, then `created`, `updated`,
    /// `tags`. The extras are sorted because the format prescribes no order for them, and a stable
    /// order is what keeps a rewrite's diff to the line that actually changed.
    ///
    /// `tags` is written whenever the document had the key, empty list included: absence and an
    /// empty list are different facts, and rendering must not invent one from the other.
    fn ordered_entries(&self) -> Vec<(String, Value)> {
        let mut entries: Vec<(String, Value)> = Vec::with_capacity(8 + self.extras.len());

        let mut push = |key: &str, value: Value| entries.push((key.to_string(), value));

        if let Some(id) = &self.id {
            push("id", Value::Str(id.as_str().to_string()));
        }
        if let Some(kind) = &self.kind {
            push("type", Value::Str(kind.clone()));
        }
        if let Some(title) = &self.title {
            push("title", Value::Str(title.clone()));
        }
        if let Some(status) = &self.status {
            push("status", Value::Str(status.clone()));
        }
        for (key, value) in &self.extras {
            push(key, value.clone());
        }
        if let Some(created) = &self.created {
            push("created", Value::Str(created.clone()));
        }
        if let Some(updated) = &self.updated {
            push("updated", Value::Str(updated.clone()));
        }
        if self.tags_present {
            push(
                "tags",
                Value::List(self.tags.iter().cloned().map(Value::Str).collect()),
            );
        }

        entries
    }

    /// Reads a block, reporting every failure against the file's line numbers.
    fn from_block(block: &Block<'_>) -> Result<Self, ContextError> {
        // An empty block is a human writing `---` twice. It is not a syntax error, and the missing
        // `id`, `type`, and `title` are the finding the schema should raise, not this parser.
        if block.yaml.trim().is_empty() {
            return Ok(Self::default());
        }

        // A repeated key is found by looking at the text first, because the YAML layer refuses the
        // whole block before this code ever sees a key, and its refusal is a generic syntax message
        // that embeds a block-relative "at line 1 column 8". Reporting it here instead produces an
        // error that names the key and gives a line number in the file, which is the whole point of
        // the exercise. The layer's own check still stands behind this one, so an exotic spelling
        // that the scan below cannot see is refused rather than accepted.
        let mut front_matter = Self::default();
        let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
        for (key, line) in block.top_level_keys() {
            if seen.insert(key, line).is_some() {
                return Err(ContextError::DuplicateKey {
                    line,
                    key: key.to_string(),
                });
            }
        }

        let pairs = SerdeYaml
            .decode_mapping(block.yaml)
            .map_err(|error| ContextError::from_yaml(&error, FIRST_CONTENT_LINE))?;

        for (key, value) in &pairs {
            let line = block.line_of(key);
            match key.as_str() {
                "id" => front_matter.id = Some(read_id(value, line)?),
                "type" => front_matter.kind = Some(read_string(value, "type", line)?),
                "title" => front_matter.title = Some(read_string(value, "title", line)?),
                "status" => front_matter.status = Some(read_string(value, "status", line)?),
                "created" => front_matter.created = Some(read_date(value, "created", line)?),
                "updated" => front_matter.updated = Some(read_date(value, "updated", line)?),
                "tags" => {
                    front_matter.tags = read_tags(value, line)?;
                    front_matter.tags_present = true;
                }
                _ => {
                    front_matter.extras.insert(key.clone(), value.clone());
                }
            }
        }
        Ok(front_matter)
    }
}

/// Reads a string, refusing a value of another shape rather than coercing it.
fn read_string(value: &Value, key: &str, line: usize) -> Result<String, ContextError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ContextError::WrongType {
            line,
            key: key.to_string(),
            expected: "a string".to_string(),
            actual: value.type_name().to_string(),
        })
}

/// Reads an `id`, validating its shape.
fn read_id(value: &Value, line: usize) -> Result<DocumentId, ContextError> {
    let raw = read_string(value, "id", line)?;
    DocumentId::new(&raw).map_err(|source| ContextError::InvalidId {
        line,
        key: "id".to_string(),
        source,
    })
}

/// Reads an ISO-8601 date and checks it against the calendar.
fn read_date(value: &Value, key: &str, line: usize) -> Result<String, ContextError> {
    let raw = read_string(value, key, line)?;
    if is_iso_date(&raw) {
        Ok(raw)
    } else {
        Err(ContextError::InvalidDate {
            line,
            key: key.to_string(),
            value: raw,
        })
    }
}

/// Reads a list of strings.
fn read_tags(value: &Value, line: usize) -> Result<Vec<String>, ContextError> {
    let items = value.as_list().ok_or_else(|| ContextError::WrongType {
        line,
        key: "tags".to_string(),
        expected: "a list of strings".to_string(),
        actual: value.type_name().to_string(),
    })?;

    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| ContextError::WrongType {
                    line,
                    key: "tags".to_string(),
                    expected: "a list of strings".to_string(),
                    actual: format!("a list containing {}", item.type_name()),
                })
        })
        .collect()
}

/// Whether `text` is a real `YYYY-MM-DD` date.
///
/// Checked against the calendar and not the pattern alone, because a pattern check accepts
/// `2026-02-31`. The whole point of storing a date is that it means a day, and the cost of the
/// extra four lines is that a typo'd date fails at the point it was written rather than at index
/// time, in a different tool, with a worse message.
fn is_iso_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    let Some(digits) = (|| {
        Some((
            text.get(0..4)?.parse::<u32>().ok()?,
            text.get(5..7)?.parse::<u32>().ok()?,
            text.get(8..10)?.parse::<u32>().ok()?,
        ))
    })() else {
        return false;
    };
    let (year, month, day) = digits;

    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    day <= days_in_month(year, month)
}

/// The number of days in a month, accounting for leap years.
fn days_in_month(year: u32, month: u32) -> u32 {
    const LENGTHS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month == 2 && is_leap_year(year) {
        return 29;
    }
    LENGTHS[(month - 1) as usize]
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
fn is_leap_year(year: u32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::{
        ContextError, DOCUMENT_MAX_BYTES, Document, FIRST_CONTENT_LINE, FRONT_MATTER_MAX_BYTES,
        FrontMatter, Value, first_foreign_line_break, is_fence, is_iso_date,
    };

    #[test]
    fn a_fence_is_recognised_whatever_the_editor_left_on_the_line() {
        assert!(is_fence("---\n"));
        assert!(is_fence("---"));
        assert!(
            is_fence("---   \n"),
            "trailing spaces are an editor artefact"
        );
        assert!(
            is_fence("--- \r\n"),
            "a Windows line ending is not a different fence"
        );
    }

    #[test]
    fn a_line_that_only_looks_like_a_fence_is_not_one() {
        // Treating `--- # note` as a fence would hide whatever the note was annotating.
        assert!(!is_fence("--- # note\n"));
        assert!(!is_fence("---abc\n"));
        assert!(!is_fence("----\n"));
        assert!(
            !is_fence("  ---\n"),
            "an indented rule is body text, not front matter"
        );
    }

    #[test]
    fn a_real_date_is_accepted_and_a_near_miss_is_not() {
        assert!(is_iso_date("2026-09-27"));
        assert!(is_iso_date("2024-02-29"), "2024 is a leap year");
        assert!(is_iso_date("2000-02-29"), "2000 is a leap year");

        assert!(!is_iso_date("2026-02-30"), "February never has 30 days");
        assert!(!is_iso_date("2023-02-29"), "2023 is not a leap year");
        assert!(!is_iso_date("1900-02-29"), "1900 is not a leap year");
        assert!(!is_iso_date("2026-13-01"), "there is no thirteenth month");
        assert!(!is_iso_date("2026-00-10"));
        assert!(!is_iso_date("2026-09-00"), "there is no zeroth day");
        assert!(!is_iso_date("2026-9-27"), "the month must be two digits");
        assert!(!is_iso_date("20260927"), "the separators are required");
        assert!(
            !is_iso_date("2026-09-27T00:00:00Z"),
            "a timestamp is not a date"
        );
        assert!(!is_iso_date(""), "an empty value is not a date");
        assert!(!is_iso_date("abcd-ef-gh"));
    }

    #[test]
    fn the_content_line_constant_matches_where_the_block_actually_starts() {
        // Every error in this module adds this to a block-relative line, so it has to be the line
        // the block's first key is really on.
        assert_eq!(FIRST_CONTENT_LINE, 2, "line 1 is the opening fence");
    }

    #[test]
    fn the_limits_are_the_ones_the_spec_asks_for() {
        assert_eq!(FRONT_MATTER_MAX_BYTES, 65_536, "64 KiB");
        assert_eq!(DOCUMENT_MAX_BYTES, 1_048_576, "1 MiB");
    }

    #[test]
    fn a_crlf_file_parses_because_a_carriage_return_before_the_newline_is_not_a_line_ending() {
        // This has to keep working, or every Windows-authored document in the workspace stops being a
        // document. The LF is what ends the line; the CR is the other half of the same ending.
        assert!(first_foreign_line_break("id: TASK-014\r\ntitle: T\r\n").is_none());
        assert!(first_foreign_line_break("id: TASK-014\n").is_none());
    }

    #[test]
    fn a_bare_line_ending_the_file_does_not_have_is_found_and_named() {
        // Each of these ends a line for the YAML layer and not for the file, which is exactly the
        // condition that makes a reported line number wrong.
        for (yaml, expected_line, expected_character) in [
            ("id: TASK-014\rtitle: T", 0, '\r'),
            ("id: TASK-014\ntitle: a\rb", 1, '\r'),
            ("id: TASK-014\ntitle: a\u{85}b", 1, '\u{85}'),
            ("id: TASK-014\ntitle: a\u{2028}b", 1, '\u{2028}'),
            ("id: TASK-014\ntitle: a\u{2029}b", 1, '\u{2029}'),
        ] {
            let found =
                first_foreign_line_break(yaml).expect("a line ending the file does not have");
            assert_eq!(found.line, expected_line, "for {yaml:?}");
            assert_eq!(found.character, expected_character, "for {yaml:?}");
        }
    }

    #[test]
    fn a_bare_carriage_return_in_a_block_is_refused_with_a_line_the_author_can_use() {
        // Without this, the block `a: \r\r¡` has four lines for the YAML layer and three for the
        // file, and the failure lands on line 5 of a 3-line document.
        let error = Document::parse("---\nid: TASK-014\rtitle: T\n---\nbody\n")
            .expect_err("a bare carriage return is not a line ending this format uses");

        let ContextError::Yaml { line, reason } = &error else {
            panic!("got {error}");
        };
        assert_eq!(*line, 2, "the offending line, in the file's numbering");
        assert!(
            reason.contains("carriage return") || reason.contains("\\r"),
            "the failure must name the character, got: {reason}"
        );
        assert_eq!(error.line(), Some(2));
    }

    #[test]
    fn a_crlf_document_still_parses_and_still_renders_with_lf_endings() {
        // Rule 10 asks for `\n` on the way out, and a CRLF file must not stop a document from being
        // read on the way in.
        let document = Document::parse("---\r\nid: TASK-014\r\ntitle: T\r\n---\r\nbody\r\n")
            .expect("a Windows-authored document is a document");

        assert_eq!(
            document.body(),
            "body\r\n",
            "the body is not ours to rewrite"
        );
        assert_eq!(
            document.render().expect("writable"),
            "---\nid: TASK-014\ntitle: T\n---\nbody\r\n"
        );
    }

    #[test]
    fn a_value_with_no_yaml_spelling_is_reported_rather_than_dropped() {
        // Only reachable from in here, because the parser refuses to read one in and the fields are
        // private. That is deliberate: the question is what `render` does if it ever holds such a
        // value, and an empty block written in its place would delete the document's metadata while
        // leaving a file that still parses and says nothing at all.
        let mut front_matter = FrontMatter::default();
        front_matter
            .extras
            .insert("ratio".to_string(), Value::Float(f64::NAN));
        let document = Document {
            front_matter: Some(front_matter),
            body: "body\n".to_string(),
        };

        let error = document.render().expect_err("NaN has no YAML spelling");

        let ContextError::NotWritable { reason } = &error else {
            panic!("an unwriteable value is a NotWritable failure, got: {error}");
        };
        assert!(
            reason.contains("non-finite"),
            "the failure must say what could not be written, got: {reason}"
        );
        assert!(
            !document.render().unwrap_or_default().contains("ratio"),
            "and the field must not be quietly dropped"
        );
    }

    #[test]
    fn an_empty_tag_list_is_written_but_an_absent_one_is_not() {
        // The two are different claims about a document, so rendering has to keep them apart.
        let written = Document::parse("---\nid: TASK-014\ntags: []\n---\n")
            .expect("parses")
            .render()
            .expect("writable");
        assert!(written.contains("tags: []"), "got:\n{written}");

        let absent = Document::parse("---\nid: TASK-014\n---\n")
            .expect("parses")
            .render()
            .expect("writable");
        assert!(!absent.contains("tags"), "got:\n{absent}");
    }
}
