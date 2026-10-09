//! Reading inline entities out of a document body (`docs/CONTEXT_SPEC.md` Â§2.1).
//!
//! A register like `TASKS.md` is a document like any other, but it also carries entities as sections:
//! a heading that begins with an entity ID, followed by a fenced `yaml` block. Both `doctor` and the
//! register reader need that structure, so it lives here once. Keeping a single parser is not tidiness
//! â€” two readers that disagreed about where a section ends would make `doctor` and `status` describe
//! different projects from the same bytes.

use aicontext_core::DocumentId;

use crate::{Document, FrontMatter};

/// The entity ID prefixes an inline entity may declare (`docs/CONTEXT_SPEC.md` Â§2.1).
///
/// A heading that does not start with one of these opens a prose section rather than an entity, which
/// is what lets a register document itself without every heading becoming a task.
pub(crate) const ENTITY_PREFIXES: &[&str] = &["TASK-", "MEM-", "ADR-", "SPEC-", "BUG-", "CHG-"];

/// One inline entity: a heading that named an ID, plus the fenced block beneath it.
pub(crate) struct Entity {
    /// The ID token from the heading, which is the entity's identity when the block omits it.
    pub(crate) heading_id: String,
    /// The fenced block, wrapped and parsed as a document so one YAML dialect serves both.
    pub(crate) block: Option<Document>,
    /// Whether a fenced block was found at all, which Â§2.1 requires rather than guesses past.
    pub(crate) had_block: bool,
}

impl Entity {
    /// The block's front matter.
    pub(crate) fn front(&self) -> Option<&FrontMatter> {
        self.block.as_ref()?.front_matter()
    }

    /// The entity's ID: the block's own, or the heading's when the block omits it.
    pub(crate) fn id(&self) -> &str {
        self.front()
            .and_then(FrontMatter::id)
            .map_or(self.heading_id.as_str(), DocumentId::as_str)
    }
}

/// Whether a value looks like an entity ID rather than a path or free text.
pub(crate) fn is_entity_id(candidate: &str) -> bool {
    ENTITY_PREFIXES
        .iter()
        .any(|prefix| candidate.starts_with(prefix))
}

/// Whether a heading's first token names an entity, as opposed to merely starting like one.
///
/// Both halves are needed. The prefix list keeps a context note or a workflow from being read as an
/// inline entity when it belongs in its own file, and `aicontext-core` then insists the whole token is
/// an ID. Without the second half a heading like `## TASK-014: the plan` would yield the token
/// `TASK-014:`, which resolves against nothing and would report every reference to it as dangling.
pub(crate) fn names_entity(token: &str) -> bool {
    is_entity_id(token) && DocumentId::new(token).is_ok()
}

/// Extracts the inline entities from a document body.
///
/// The heading level is not consulted. `TASKS.md` documents itself and so puts entity headings at a
/// different level from `MEMORY.md`, and a parser that insisted on one level would silently skip
/// every task in the register while reporting nothing. What identifies an entity is that its heading
/// text begins with an entity ID (`docs/CONTEXT_SPEC.md` Â§2.1).
///
/// `own_id` is the document's own identifier. A heading that repeats it is the document's title, not
/// an entity section: a standalone decision is `ADR-001` in its front matter, and its `# ADR-001 â€¦`
/// heading introduces prose, so reporting it as an entity with no block would fail every decision
/// this repository owns.
pub(crate) fn inline_entities(text: &str, own_id: Option<&str>) -> Vec<Entity> {
    let lines: Vec<&str> = text.lines().collect();
    let fenced = fenced_lines(&lines);
    let mut entities = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        // A `#` inside a fenced block is a YAML comment or shell code, not a heading. Treating it as
        // one would end the section early and hide the block that follows it â€” which is exactly what
        // a `notes:` list in a task block looks like.
        if fenced[index] {
            index += 1;
            continue;
        }
        let Some((level, heading_id)) = entity_heading(lines[index]) else {
            index += 1;
            continue;
        };
        let end = section_end(&lines, index, level);
        match own_id {
            Some(own) if own == heading_id => {}
            _ => {
                let block = first_yaml_block(&lines[index + 1..end]);
                entities.push(block_entity(heading_id, block));
            }
        }
        index = end.max(index + 1);
    }

    entities
}

/// Which lines sit inside a fenced code block, as `CommonMark` counts them: a fence opens until the
/// next fence of the same kind, and nothing inside is structure.
///
/// Only backtick fences, because that is what Â§2.1's `yaml` blocks are written with. A document that
/// fences with `~~~` still has its entities found; its fences are simply not tracked, which is the
/// conservative direction to be wrong in.
fn fenced_lines(lines: &[&str]) -> Vec<bool> {
    let mut inside = false;
    lines
        .iter()
        .map(|line| {
            let fenced = line.trim_start().starts_with("```");
            let mask = inside;
            inside = inside != fenced;
            mask
        })
        .collect()
}

/// A heading that names an entity, as `(level, id)`.
pub(crate) fn entity_heading(line: &str) -> Option<(usize, String)> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // `#Title` is not a heading; Markdown requires the space, and treating it as one would let a
    // line of prose that happens to start with a hash open an entity section. The space is checked
    // before anything is trimmed off it, because trimming first would remove the very evidence.
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let token = rest.split_whitespace().next()?;
    names_entity(token).then(|| (hashes, token.to_string()))
}

/// The line at which a heading's section ends: the next heading at the same or a higher level.
///
/// Headings inside a fenced code block do not count. `CommonMark` says a fence wins over a heading,
/// and a ``yaml`` block carrying ``#`` comments would otherwise close its own section one line early.
pub(crate) fn section_end(lines: &[&str], start: usize, level: usize) -> usize {
    let fenced = fenced_lines(lines);
    let mut index = start + 1;
    while index < lines.len() {
        if !fenced[index] {
            if let Some((next_level, _)) = heading_level(lines[index]) {
                if next_level <= level {
                    return index;
                }
            }
        }
        index += 1;
    }
    lines.len()
}

/// The level of an ATX heading line, if it is one.
fn heading_level(line: &str) -> Option<(usize, &str)> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    rest.starts_with([' ', '\t'])
        .then_some((hashes, rest.trim_start()))
}

/// The first fenced `yaml` block within a section, as its inner text.
pub(crate) fn first_yaml_block(lines: &[&str]) -> Option<String> {
    let mut inside = false;
    let mut start = 0;
    for (offset, line) in lines.iter().enumerate() {
        let marker = line.trim_start();
        if inside {
            if marker.starts_with("```") {
                return Some(lines[start..offset].join("\n"));
            }
            continue;
        }
        let Some(after_fence) = marker.strip_prefix("```") else {
            continue;
        };
        if after_fence.trim().eq_ignore_ascii_case("yaml") {
            inside = true;
            start = offset + 1;
        }
    }
    None
}

/// Pairs an entity heading with its block, parsing the block as front matter.
///
/// The block is wrapped in a `---` fence and handed to the same parser every document uses, so inline
/// entities and standalone documents cannot end up with two different YAML dialects.
fn block_entity(heading_id: String, block: Option<String>) -> Entity {
    match block {
        None => Entity {
            heading_id,
            block: None,
            had_block: false,
        },
        Some(text) => {
            let wrapped = format!("---\n{text}\n---\n");
            Entity {
                heading_id,
                block: Document::parse(&wrapped).ok(),
                had_block: true,
            }
        }
    }
}
