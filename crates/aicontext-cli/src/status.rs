//! `aicontext status`: where this project is, in one screen.
//!
//! Four facts, from two sources: the register (`.ai/TASKS.md`) answers *what phase, what task, and
//! what is left*, and the Git wrapper answers *what branch and what has changed*. Nothing is
//! recomputed here, and nothing is validated: `doctor` is the command that says what is wrong, and a
//! status report that also judged would be a second, quieter validator.
//!
//! # Degrading outside a repository
//!
//! A directory with no `.git` is not an error (`docs/CLI_SPEC.md` §4): the report simply has no
//! branch and no changes to show. Git being broken in a way that is *not* "there is no repository" is
//! a warning, not a failure — the register is still worth reporting, and `status` is a read.
//!
//! # Exit
//!
//! Always 0 for a usable root. Only a root that cannot be used at all ([`crate::project::ProjectError`])
//! exits 2, exactly as `doctor` reports it.

use std::path::Path;

use aicontext_context::Register;
use aicontext_git::{Change, FileStatus, Git, GitError, Head, Snapshot};
use serde::Serialize;

use crate::args::{GlobalArgs, StatusArgs};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// How many changed files are listed before the report says how many it held back.
///
/// A hard cap rather than a width calculation: `status` is a glance, and a tree with two hundred
/// changes is a tree whose owner already knows. The count above the list is always exact, so the
/// truncation never hides how much there is (`DESIGN.md` §6).
const MAX_LISTED_CHANGES: usize = 20;

/// Runs `status` and returns the exit code.
pub(crate) fn run(_args: &StatusArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return report_root_failure(&error, terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return report_root_failure(&error, terminal),
    };

    let register = aicontext_context::register::read(&root);
    let mut findings = Vec::new();
    let vcs = read_vcs(&root, &mut findings);

    let data = StatusData {
        root: project::display(&root),
        project: name.clone(),
        vcs,
        current_phase: register.current_phase.clone(),
        current_task: current_task(&register),
        tasks: summarise(&register),
    };

    if global.json {
        let summary = summary_line(&data);
        let body = envelope("status", Exit::Ok.code(), data, findings, summary);
        if let Err(error) = terminal.json(&body) {
            terminal.error_report(
                "STATUS-003",
                "the JSON envelope could not be written",
                &format!("{error}; check that stdout is a pipe rather than a full device"),
            );
            return Exit::General;
        }
    } else {
        print_human(&name, &data, terminal);
    }
    Exit::Ok
}

/// Reads the repository, degrading to "no vcs" rather than failing.
///
/// A directory that is not inside a working tree is the documented, ordinary case and produces no
/// finding. Every other failure is a warning attached to the envelope: the report is still produced,
/// but the reason the branch is missing is stated rather than left to be guessed.
fn read_vcs(root: &Path, findings: &mut Vec<Finding>) -> VcsData {
    let repository = match Git::new(root).open() {
        Ok(repository) => repository,
        Err(GitError::NotARepository { .. }) => return VcsData::none(),
        Err(error) => {
            findings.push(Finding::new(
                "STATUS-001",
                "warning",
                ".git",
                format!("the repository could not be read: {error}"),
                error.remediation(),
            ));
            return VcsData::none();
        }
    };

    match repository.snapshot() {
        Ok(snapshot) => VcsData::git(&snapshot),
        Err(error) => {
            findings.push(Finding::new(
                "STATUS-002",
                "warning",
                ".git",
                format!("the working tree could not be read: {error}"),
                error.remediation(),
            ));
            VcsData::none()
        }
    }
}

/// The current task as the report states it: the named ID, plus whatever the register knows about it.
fn current_task(register: &Register) -> Option<TaskData> {
    let id = register.current_task.as_ref()?;
    let entry = register.current();
    Some(TaskData {
        id: id.clone(),
        title: entry.and_then(|task| task.title.clone()),
        status: entry.and_then(|task| task.status.clone()),
        priority: entry.and_then(|task| task.priority.clone()),
    })
}

/// Counts the register: how many tasks, how many are closed, and which are still open.
fn summarise(register: &Register) -> TaskSummary {
    let total = register.tasks.len();
    let done = register
        .tasks
        .iter()
        .filter(|task| task.is_closed())
        .count();
    let pending_ids: Vec<String> = register
        .tasks
        .iter()
        .filter(|task| !task.is_closed())
        .map(|task| task.id.clone())
        .collect();
    TaskSummary {
        total,
        done,
        pending: pending_ids.len(),
        pending_ids,
    }
}

/// The human report: identity, the four facts, the changed files, and the summary.
fn print_human(name: &str, data: &StatusData, terminal: &mut Terminal) {
    terminal.identity("aicontext", name, &vcs_branch(data));
    let mark = terminal.mark_info();

    match &data.current_phase {
        Some(phase) => terminal.row(&mark, "phase", phase),
        None => terminal.row(&mark, "phase", "not recorded"),
    }
    match &data.current_task {
        Some(task) => terminal.row(&mark, "task", &task_line(task)),
        None => terminal.row(&mark, "task", "none recorded"),
    }
    terminal.row(&mark, "vcs", &vcs_line(data));

    for change in data.vcs.changes.iter().take(MAX_LISTED_CHANGES) {
        terminal.row(" ", &change.state, &change.path);
    }
    if data.vcs.changes.len() > MAX_LISTED_CHANGES {
        terminal.say(&format!(
            "      … and {} more",
            data.vcs.changes.len() - MAX_LISTED_CHANGES
        ));
    }

    terminal.row(&mark, "tasks", &tasks_line(data));

    terminal.summary(&summary_line(data), Exit::Ok, "`aicontext doctor`");
}

/// One task as the JSON payload and the human report state it.
#[derive(Debug, Serialize)]
struct TaskData {
    /// The task ID.
    id: String,
    /// The task's title, when the register has one.
    title: Option<String>,
    /// The task's status, as written.
    status: Option<String>,
    /// The task's priority, as written.
    priority: Option<String>,
}

/// The register, counted.
#[derive(Debug, Serialize)]
struct TaskSummary {
    /// Every task the register declares.
    total: usize,
    /// Tasks that are `DONE` or `CANCELLED`.
    done: usize,
    /// Tasks that are neither.
    pending: usize,
    /// The open task IDs, in register order.
    pending_ids: Vec<String>,
}

/// The JSON payload of `status --json`.
#[derive(Debug, Serialize)]
struct StatusData {
    /// The project root, with `/` separators.
    root: String,
    /// The project name, taken from the root directory.
    project: String,
    /// The repository facts, or `kind: "none"` outside one.
    vcs: VcsData,
    /// The current phase, straight from the register body.
    current_phase: Option<String>,
    /// The current task, straight from the register body.
    current_task: Option<TaskData>,
    /// The register, counted.
    tasks: TaskSummary,
}

/// The repository facts, as the report states them.
#[derive(Debug, Serialize)]
struct VcsData {
    /// `"git"` inside a working tree, `"none"` outside one.
    kind: &'static str,
    /// The branch name, when `HEAD` is on one.
    branch: Option<String>,
    /// The short head commit, when there is one.
    head: Option<String>,
    /// Whether `HEAD` is at a commit on no branch.
    detached: bool,
    /// Every changed file, in the order git reported them.
    changes: Vec<ChangeRow>,
}

impl VcsData {
    /// No repository: the state a directory without `.git` reports.
    fn none() -> Self {
        Self {
            kind: "none",
            branch: None,
            head: None,
            detached: false,
            changes: Vec::new(),
        }
    }

    /// The facts from one `status` invocation.
    fn git(snapshot: &Snapshot) -> Self {
        let (branch, head, detached) = match &snapshot.head {
            Head::Branch { name, commit } => (Some(name.clone()), Some(short(commit)), false),
            Head::Unborn { name } => (Some(name.clone()), None, false),
            Head::Detached { commit } => (None, Some(short(commit)), true),
            // `Head` is `#[non_exhaustive]`: a future state this version does not name reports no
            // branch rather than failing to render the one it does know about.
            _ => (None, None, false),
        };
        Self {
            kind: "git",
            branch,
            head,
            detached,
            changes: snapshot.changes.iter().map(ChangeRow::of).collect(),
        }
    }
}

/// One changed file, in the shape the JSON payload publishes.
#[derive(Debug, Serialize)]
struct ChangeRow {
    /// The path, `/`-separated, relative to the repository root.
    path: String,
    /// What happened, as one word.
    state: String,
    /// The path before the change, for a rename or a copy.
    original_path: Option<String>,
}

impl ChangeRow {
    /// One row from one git change.
    fn of(change: &Change) -> Self {
        Self {
            path: change.path().to_string(),
            state: state_of(change),
            original_path: change.original_path().map(str::to_string),
        }
    }
}

/// What happened to a file, as one word.
fn state_of(change: &Change) -> String {
    match change {
        Change::Untracked { .. } => "untracked".to_string(),
        Change::Unmerged { .. } => "unmerged".to_string(),
        Change::Tracked {
            index, worktree, ..
        } => worktree
            .as_ref()
            .or(index.as_ref())
            .map_or_else(|| "changed".to_string(), |status| spell(*status)),
        // `Change` is `#[non_exhaustive]`: a record this version does not name is still a change.
        _ => "changed".to_string(),
    }
}

/// A tracked file's status, as one word.
fn spell(status: FileStatus) -> String {
    match status {
        FileStatus::Added => "added",
        FileStatus::Modified => "modified",
        FileStatus::Deleted => "deleted",
        FileStatus::Renamed => "renamed",
        FileStatus::Copied => "copied",
        FileStatus::TypeChanged => "type-changed",
        _ => "changed",
    }
    .to_string()
}

/// A full commit hash shortened the way git does.
fn short(full: &str) -> String {
    full.chars().take(7).collect()
}

/// The branch label for the identity line.
fn vcs_branch(data: &StatusData) -> String {
    if data.vcs.kind == "none" {
        return "no vcs".to_string();
    }
    match &data.vcs.branch {
        Some(branch) => format!("branch {branch}"),
        None => "detached".to_string(),
    }
}

/// The human `vcs` row: branch, head, and what changed.
fn vcs_line(data: &StatusData) -> String {
    if data.vcs.kind == "none" {
        return "no repository".to_string();
    }
    let mut parts = Vec::new();
    match &data.vcs.branch {
        Some(branch) => parts.push(format!("on {branch}")),
        None => parts.push("detached".to_string()),
    }
    if let Some(head) = &data.vcs.head {
        parts.push(format!("at {head}"));
    }
    match data.vcs.changes.len() {
        0 => parts.push("clean".to_string()),
        1 => parts.push("1 file changed".to_string()),
        count => parts.push(format!("{count} files changed")),
    }
    parts.join(", ")
}

/// The human `task` row: the ID and title, then the status when it is known.
fn task_line(task: &TaskData) -> String {
    let mut line = task.id.clone();
    if let Some(title) = &task.title {
        line.push_str(" — ");
        line.push_str(title);
    }
    if let Some(status) = &task.status {
        line.push_str("  [");
        line.push_str(status);
        line.push(']');
    }
    line
}

/// The human `tasks` row and the summary counts.
fn tasks_line(data: &StatusData) -> String {
    if data.tasks.total == 0 {
        return "no register".to_string();
    }
    format!(
        "{} total, {} pending, {} done",
        data.tasks.total, data.tasks.pending, data.tasks.done
    )
}

/// The closing counts line.
fn summary_line(data: &StatusData) -> String {
    if data.tasks.total == 0 {
        return "no task register".to_string();
    }
    format!("{} tasks, {} pending", data.tasks.total, data.tasks.pending)
}

/// A root that cannot be used, reported through the surface every other command reports through.
fn report_root_failure(error: &ProjectError, terminal: &mut Terminal) -> Exit {
    terminal.error_report(error.code(), &error.to_string(), error.remediation());
    error.exit()
}
