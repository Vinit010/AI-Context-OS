//! Reading the task register as a typed value.
//!
//! `TASKS.md` is the one document that answers "where is this project" and "what comes next", so
//! `status` reads it through this module rather than scraping the file itself. Two facts live in the
//! body rather than in front matter â€” the current phase and the current task â€” because
//! `schemas/tasks.schema.json` deliberately keeps them out of the machine block: they change on
//! almost every task boundary, and a register rewritten by hand is not rewritten by a program that
//! has to re-emit the whole file. Everything else (a task's title, status, and priority) is read from
//! the task's own fenced block, using the same inline-entity parser `doctor` uses.

use std::fs;
use std::path::Path;

use aicontext_core::DocumentId;

use crate::body::{Entity, inline_entities};
use crate::{Document, FrontMatter};

/// The register's fixed location.
const REGISTER_PATH: &str = ".ai/TASKS.md";

/// The label whose value names the current phase, as the body writes it.
const CURRENT_PHASE_LABEL: &str = "Current phase:";

/// The label whose value names the current task, as the body writes it.
const CURRENT_TASK_LABEL: &str = "Current task:";

/// The task register, as much of it as a report needs.
///
/// A missing or unreadable register yields an empty value rather than an error: `status` describes a
/// project, and a project with no register is a state to report, not a reason to fail. `doctor` is
/// where an absent register is a finding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Register {
    /// The current phase, as written after `**Current phase:**`.
    pub current_phase: Option<String>,
    /// The current task's ID, as written after `**Current task:**`.
    pub current_task: Option<String>,
    /// Every task the register declares, in document order.
    pub tasks: Vec<TaskEntry>,
}

impl Register {
    /// The task the register names as current, when that ID matches a declared task.
    #[must_use]
    pub fn current(&self) -> Option<&TaskEntry> {
        let id = self.current_task.as_deref()?;
        self.tasks.iter().find(|task| task.id == id)
    }
}

/// One task as the register declares it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskEntry {
    /// The task ID, from the block's `id` or, when that is absent, the heading.
    pub id: String,
    /// The task's title, when its block carries one.
    pub title: Option<String>,
    /// The task's status, as written (`TODO`, `DONE`, â€¦).
    pub status: Option<String>,
    /// The task's priority, as written (`CRITICAL`, `HIGH`, â€¦).
    pub priority: Option<String>,
}

impl TaskEntry {
    /// Whether the task is finished, one way or another.
    ///
    /// `CANCELLED` counts as closed: a task nobody intends to do is not work still to come, and a
    /// report that listed it as outstanding would overstate what is left.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        matches!(self.status.as_deref(), Some("DONE" | "CANCELLED"))
    }
}

/// Reads the register rooted at `root`, never failing.
#[must_use]
pub fn read(root: &Path) -> Register {
    let Ok(text) = fs::read_to_string(root.join(REGISTER_PATH)) else {
        return Register::default();
    };
    let document = Document::parse(&text).ok();
    let body = document
        .as_ref()
        .map_or(text.as_str(), Document::body)
        .to_string();
    let own_id = document
        .as_ref()
        .and_then(|document| document.front_matter())
        .and_then(|front| front.id())
        .map(DocumentId::as_str)
        .map(str::to_string);

    let tasks = inline_entities(&body, own_id.as_deref())
        .iter()
        .filter_map(TaskEntry::from_entity)
        .collect();

    Register {
        current_phase: body_field(&body, CURRENT_PHASE_LABEL),
        current_task: body_field(&body, CURRENT_TASK_LABEL),
        tasks,
    }
}

impl TaskEntry {
    /// Reads a task entry from an inline entity, or `None` if it is not a task.
    fn from_entity(entity: &Entity) -> Option<Self> {
        let id = entity.id();
        if !id.starts_with("TASK-") {
            return None;
        }
        let front = entity.front();
        Some(Self {
            id: id.to_string(),
            title: front.and_then(FrontMatter::title).map(str::to_string),
            status: front.and_then(FrontMatter::status).map(str::to_string),
            priority: front
                .and_then(|front| front.extra("priority"))
                .and_then(|value| value.as_str())
                .map(str::to_string),
        })
    }
}

/// The value of a `**Label:** value` line in the register body, if it has one.
///
/// Matched case-insensitively and without regard to the bold markers, so a register that writes
/// `**Current task:**` and one that writes `Current task:` both resolve. The value is taken to the
/// end of the line, which is what a phase name with spaces in it needs.
fn body_field(body: &str, label: &str) -> Option<String> {
    body.lines().find_map(|raw| {
        let line = raw.trim();
        let line = line.strip_prefix("**").unwrap_or(line);
        if !line.get(..label.len())?.eq_ignore_ascii_case(label) {
            return None;
        }
        let value = line.get(label.len()..)?.trim_start_matches('*').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}
