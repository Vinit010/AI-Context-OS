//! `aicontext import`: verify an archive and write it into `.ai/` (`docs/CLI_SPEC.md` §4,
//! `TASK-020`).
//!
//! Import is the one command that writes documents a developer did not compose, so it is built
//! around a single rule: **verify everything before writing anything**. The archive is read whole,
//! bounded, parsed, and checked against its own manifest; only an archive that verifies clean is
//! planned; and only a plan with no unresolved conflict is applied. An archive that fails any check
//! writes nothing at all, exactly as `.ai/` was.
//!
//! # Conflicts and `--force`
//!
//! A file that already exists and differs from the archive is a conflict. Conflicts are reported, not
//! resolved: without `--force` the run exits 5 and leaves every file untouched, and `--force`
//! overwrites them only from an interactive terminal, because an absent human is a deny
//! (`RULES.md` §7). Import never deletes a file the archive does not mention — an archive is a merge,
//! not a mirror.
//!
//! # Paths are hostile input
//!
//! Every entry path is validated by [`crate::archive::safe_relative`] before planning, and the
//! manifest digest is recomputed before any path is used, so a tampered archive cannot name a file
//! outside `.ai/` (`MEM-003`, `RULES.md` §7).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::archive::{self, Archive, ArchiveProblem, MAX_ARCHIVE_BYTES};
use crate::args::{GlobalArgs, ImportArgs};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// How many files are listed before the report says how many it held back.
const MAX_LISTED_FILES: usize = 20;

/// A failure that stopped `import` from completing.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ImportError {
    /// The directory the command was pointed at is not usable as a project root.
    #[error("cannot use {path} as the project root: {reason}")]
    UnusableRoot {
        /// The path as the user would recognise it.
        path: String,
        /// Why it cannot be used.
        reason: String,
    },

    /// The archive file could not be read at all, including because it is too large.
    #[error("cannot read the archive {path}: {reason}")]
    ArchiveUnreadable {
        /// The archive path.
        path: String,
        /// Why it cannot be read.
        reason: String,
    },

    /// The bytes are readable but are not an archive this version accepts.
    #[error("{path} cannot be read as an archive: {problem}")]
    NotAnArchive {
        /// The archive path.
        path: String,
        /// What was wrong with the document.
        #[source]
        problem: ArchiveProblem,
    },

    /// `--force` was asked for where there is no human to confirm with.
    #[error("--force would replace {count} file(s), and an absent human is a deny")]
    ForceNeedsTerminal {
        /// How many conflicting files would be replaced.
        count: usize,
    },

    /// A path the archive needs cannot be written: a file is where a directory must go, or a
    /// directory where a file must go.
    #[error("cannot write into {path}: {reason}")]
    Blocked {
        /// The offending path.
        path: String,
        /// Why it blocks the import.
        reason: String,
    },

    /// A file could not be written.
    #[error("cannot write {path}")]
    Write {
        /// The offending path.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// A directory could not be created.
    #[error("cannot create directory {path}")]
    CreateDirectory {
        /// The offending path.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// An existing file that must be compared against the archive could not be read.
    #[error("cannot read {path}")]
    ReadTarget {
        /// The offending path.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },
}

impl From<ProjectError> for ImportError {
    /// The project root could not be resolved, reported under `import`'s own code.
    fn from(error: ProjectError) -> Self {
        let (path, reason) = error.into_parts();
        Self::UnusableRoot { path, reason }
    }
}

impl ImportError {
    /// A stable code for this failure, independent of its wording.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => "IMP-001",
            Self::ArchiveUnreadable { .. } => "IMP-002",
            Self::NotAnArchive { .. } => "IMP-003",
            Self::ForceNeedsTerminal { .. } => "IMP-004",
            Self::Blocked { .. } => "IMP-005",
            Self::Write { .. } => "IMP-006",
            Self::CreateDirectory { .. } => "IMP-007",
            Self::ReadTarget { .. } => "IMP-008",
        }
    }

    /// What the developer can do about it. Every error answers this (`RULES.md` §4.3).
    pub(crate) const fn remediation(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => {
                "pass --cwd with an existing directory, or run import from inside the project"
            }
            Self::ArchiveUnreadable { .. } => {
                "name an archive written by `aicontext export` that you can read"
            }
            Self::NotAnArchive { .. } => {
                "name a file written by `aicontext export`, or re-export it from the source project"
            }
            Self::ForceNeedsTerminal { .. } => {
                "run import in an interactive terminal to confirm, or move the conflicting files aside"
            }
            Self::Blocked { .. } => "move the file or directory aside, then run import again",
            Self::Write { .. } => {
                "check the directory's permissions and free space, then run import again"
            }
            Self::CreateDirectory { .. } => {
                "check that the path is writable and is not a file, then run import again"
            }
            Self::ReadTarget { .. } => {
                "check that the file is readable and that nothing is holding it"
            }
        }
    }

    /// The exit code this failure produces (`docs/CLI_SPEC.md` §5).
    pub(crate) const fn exit(&self) -> Exit {
        match self {
            Self::UnusableRoot { .. }
            | Self::ArchiveUnreadable { .. }
            | Self::NotAnArchive { .. } => Exit::Usage,
            Self::ForceNeedsTerminal { .. } => Exit::ApprovalRequired,
            _ => Exit::General,
        }
    }
}

/// What the plan will do with one path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    /// The path does not exist and will be written.
    Create,
    /// The path exists and already holds exactly the archived content.
    Unchanged,
    /// The path exists and differs from the archive.
    Replace,
}

/// One planned write.
#[derive(Debug)]
struct Planned {
    /// The path relative to `.ai/`, with `/` separators.
    relative: String,
    /// The absolute target path.
    target: PathBuf,
    /// What the plan will do.
    action: Action,
    /// The content the archive carries for this path.
    content: String,
}

/// Runs `import` and returns the exit code.
pub(crate) fn run(args: &ImportArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    match execute(args, global, terminal) {
        Ok(exit) => exit,
        Err(error) => {
            terminal.error_report(error.code(), &error.to_string(), error.remediation());
            error.exit()
        }
    }
}

/// The whole command, returning the exit code it should report.
fn execute(
    args: &ImportArgs,
    global: &GlobalArgs,
    terminal: &mut Terminal,
) -> Result<Exit, ImportError> {
    let root = project::resolve_root(global.cwd.as_deref())?;
    let name = project::project_name(&root)?;
    let context = root.join(".ai");
    let source = resolve_source(&root, &args.path);
    let shown = project::display(&source);
    let vcs = project::vcs_label(&root);

    let bytes = read_archive(&source, &shown)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| ImportError::ArchiveUnreadable {
        path: shown.clone(),
        reason: "it is not UTF-8 text".to_string(),
    })?;
    let archive = archive::parse(text).map_err(|problem| ImportError::NotAnArchive {
        path: shown.clone(),
        problem,
    })?;

    // Verification is pure and runs before any path is used, so a tampered archive is refused whole.
    let findings = archive::verify(&archive, &shown);
    if findings.iter().any(Finding::is_error) {
        let data = ImportData::refused(&name, &root, &shown, args.dry_run, &archive);
        return report(&data, &findings, Exit::Validation, vcs, terminal, global);
    }

    let planned = plan(&context, &archive)?;
    let conflicts = planned
        .iter()
        .filter(|item| item.action == Action::Replace)
        .count();

    if conflicts > 0 && !args.force {
        let findings: Vec<Finding> = planned
            .iter()
            .filter(|item| item.action == Action::Replace)
            .map(|item| {
                Finding::new(
                    "IMP-020",
                    "error",
                    format!(".ai/{}", item.relative),
                    "the file exists and differs from the archive",
                    "re-run with --force in an interactive terminal to replace it",
                )
            })
            .collect();
        let data = ImportData::of(&name, &root, &shown, args.dry_run, &archive, &planned);
        return report(
            &data,
            &findings,
            Exit::ApprovalRequired,
            vcs,
            terminal,
            global,
        );
    }
    if conflicts > 0 && args.force && !terminal.is_terminal() && !args.dry_run {
        return Err(ImportError::ForceNeedsTerminal { count: conflicts });
    }

    if !args.dry_run {
        apply(&planned)?;
    }

    let data = ImportData::of(&name, &root, &shown, args.dry_run, &archive, &planned);
    report(&data, &[], Exit::Ok, vcs, terminal, global)
}

/// The archive path, with a relative one resolved against the project root.
fn resolve_source(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Reads the archive, refusing it before the read if it is too large (`RULES.md` §11).
fn read_archive(path: &Path, shown: &str) -> Result<Vec<u8>, ImportError> {
    let metadata = fs::metadata(path).map_err(|error| ImportError::ArchiveUnreadable {
        path: shown.to_string(),
        reason: if error.kind() == io::ErrorKind::NotFound {
            "it does not exist".to_string()
        } else {
            format!("it cannot be read ({error})")
        },
    })?;
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(ImportError::ArchiveUnreadable {
            path: shown.to_string(),
            reason: format!(
                "it is {} bytes, over the {}-byte archive limit",
                metadata.len(),
                MAX_ARCHIVE_BYTES
            ),
        });
    }
    fs::read(path).map_err(|error| ImportError::ArchiveUnreadable {
        path: shown.to_string(),
        reason: format!("it cannot be read ({error})"),
    })
}

/// Plans every write, reading the existing tree but changing nothing.
///
/// All the reads that can fail happen here, so [`apply`] cannot fail on anything but a write. A
/// target that is a directory, or an ancestor that is a file where a directory must go, is refused
/// before any byte is written.
fn plan(context: &Path, archive: &Archive) -> Result<Vec<Planned>, ImportError> {
    let mut planned = Vec::with_capacity(archive.entries.len());
    for entry in &archive.entries {
        let target = join_relative(context, &entry.path);
        let action = if target.exists() {
            if target.is_dir() {
                return Err(ImportError::Blocked {
                    path: project::display(&target),
                    reason: "a directory is already there".to_string(),
                });
            }
            let existing = fs::read(&target).map_err(|source| ImportError::ReadTarget {
                path: project::display(&target),
                source,
            })?;
            if existing == entry.content.as_bytes() {
                Action::Unchanged
            } else {
                Action::Replace
            }
        } else {
            if let Some(blocker) = blocked_ancestor(&target) {
                return Err(ImportError::Blocked {
                    path: project::display(&blocker),
                    reason: "it is a file, and the archive needs a directory there".to_string(),
                });
            }
            Action::Create
        };
        planned.push(Planned {
            relative: entry.path.clone(),
            target,
            action,
            content: entry.content.clone(),
        });
    }
    Ok(planned)
}

/// Joins a validated `/`-separated relative path onto a base directory.
fn join_relative(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for segment in relative.split('/') {
        path.push(segment);
    }
    path
}

/// The nearest existing ancestor of `target` that is not a directory, if there is one.
///
/// Walking upward stops at the first ancestor that exists; if that one is a directory, every
/// ancestor above it is too, so the missing directories below can be created. The search ends at the
/// filesystem root, which the resolved project root already is inside of.
fn blocked_ancestor(target: &Path) -> Option<PathBuf> {
    let mut cursor = target.parent().map(Path::to_path_buf);
    while let Some(directory) = cursor {
        if directory.exists() {
            return (!directory.is_dir()).then_some(directory);
        }
        cursor = directory.parent().map(Path::to_path_buf);
    }
    None
}

/// Writes every create and replace, creating parent directories as needed.
///
/// Writes are ordered by the manifest's sort, so a run that fails partway leaves the same partial
/// tree every time rather than one that depends on the platform's directory order.
fn apply(planned: &[Planned]) -> Result<(), ImportError> {
    for item in planned {
        if item.action == Action::Unchanged {
            continue;
        }
        if let Some(parent) = item.target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|source| ImportError::CreateDirectory {
                    path: project::display(parent),
                    source,
                })?;
            }
        }
        fs::write(&item.target, item.content.as_bytes()).map_err(|source| ImportError::Write {
            path: project::display(&item.target),
            source,
        })?;
    }
    Ok(())
}

/// The JSON payload of `import`.
#[derive(Clone, Debug, Serialize)]
struct ImportData {
    /// The absolute project root.
    root: String,
    /// The project name taken from the root directory.
    project: String,
    /// The archive path that was read, with `/` separators.
    archive: String,
    /// Whether this was `--dry-run`, so nothing was written.
    dry_run: bool,
    /// The digest the archive's manifest carries.
    digest: String,
    /// How many files the archive holds.
    file_count: usize,
    /// Paths that do not exist yet, or would have been created.
    created: Vec<String>,
    /// Paths that already hold the archived content.
    unchanged: Vec<String>,
    /// Paths that exist and differ from the archive.
    conflicts: Vec<String>,
}

impl ImportData {
    /// A run that refused the archive before planning, so there is nothing to group.
    fn refused(project: &str, root: &Path, archive: &str, dry_run: bool, parsed: &Archive) -> Self {
        Self {
            root: project::display(root),
            project: project.to_string(),
            archive: archive.to_string(),
            dry_run,
            digest: parsed.manifest.digest.clone(),
            file_count: parsed.manifest.file_count,
            created: Vec::new(),
            unchanged: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    /// A run that planned the whole archive.
    fn of(
        project: &str,
        root: &Path,
        archive: &str,
        dry_run: bool,
        parsed: &Archive,
        planned: &[Planned],
    ) -> Self {
        let collect = |action: Action| -> Vec<String> {
            planned
                .iter()
                .filter(|item| item.action == action)
                .map(|item| format!(".ai/{}", item.relative))
                .collect()
        };
        Self {
            root: project::display(root),
            project: project.to_string(),
            archive: archive.to_string(),
            dry_run,
            digest: parsed.manifest.digest.clone(),
            file_count: parsed.manifest.file_count,
            created: collect(Action::Create),
            unchanged: collect(Action::Unchanged),
            conflicts: collect(Action::Replace),
        }
    }
}

/// Reports the run: the envelope under `--json`, the human report otherwise.
fn report(
    data: &ImportData,
    findings: &[Finding],
    exit: Exit,
    vcs: &str,
    terminal: &mut Terminal,
    global: &GlobalArgs,
) -> Result<Exit, ImportError> {
    if global.json {
        let body = envelope(
            "import",
            exit.code(),
            data.clone(),
            findings.to_vec(),
            counts(data, findings, exit),
        );
        terminal.json(&body).map_err(|error| ImportError::Write {
            path: "the JSON envelope on stdout".to_string(),
            source: io::Error::other(error),
        })?;
        return Ok(exit);
    }
    print_human(data, findings, exit, vcs, terminal);
    Ok(exit)
}

/// The human report: identity, the plan, the findings, and the summary.
fn print_human(
    data: &ImportData,
    findings: &[Finding],
    exit: Exit,
    vcs: &str,
    terminal: &mut Terminal,
) {
    terminal.identity("aicontext", &data.project, vcs);

    let mut rows: Vec<(String, &'static str, &str)> = Vec::new();
    for path in &data.created {
        rows.push((terminal.mark_ok(), "create", path));
    }
    for path in &data.unchanged {
        rows.push((terminal.mark_info(), "unchanged", path));
    }
    for path in &data.conflicts {
        rows.push((terminal.mark_warn(), "conflict", path));
    }
    for (mark, state, path) in rows.iter().take(MAX_LISTED_FILES) {
        terminal.row(mark, state, path);
    }
    if rows.len() > MAX_LISTED_FILES {
        terminal.say(&format!(
            "      … and {} more",
            rows.len() - MAX_LISTED_FILES
        ));
    }
    for finding in findings {
        let mark = if finding.is_error() {
            terminal.mark_error()
        } else {
            terminal.mark_warn()
        };
        terminal.row(
            &mark,
            &finding.severity,
            &format!("{}  {}  {}", finding.code, finding.path, finding.message),
        );
    }
    terminal.summary(&counts(data, findings, exit), exit, &next_step(data, exit));
}

/// The counts line, which is the sentence a developer actually reads.
fn counts(data: &ImportData, findings: &[Finding], exit: Exit) -> String {
    if exit == Exit::Validation {
        let errors = findings.iter().filter(|finding| finding.is_error()).count();
        return format!("{errors} errors, nothing written");
    }
    if exit == Exit::ApprovalRequired {
        return format!(
            "{}, nothing written",
            plural(data.conflicts.len(), "conflict", "conflicts")
        );
    }
    if data.created.is_empty() && data.unchanged.is_empty() && data.conflicts.is_empty() {
        return "the archive holds no files".to_string();
    }
    let verb = if data.dry_run {
        "would create"
    } else {
        "created"
    };
    let mut parts = vec![format!(
        "{verb} {}",
        plural(data.created.len(), "file", "files")
    )];
    if !data.unchanged.is_empty() {
        parts.push(format!("{} unchanged", data.unchanged.len()));
    }
    if !data.conflicts.is_empty() {
        let verb = if data.dry_run {
            "would replace"
        } else {
            "replaced"
        };
        parts.push(format!(
            "{verb} {}",
            plural(data.conflicts.len(), "file", "files")
        ));
    }
    parts.join(", ")
}

/// What to do next, which depends on why the run ended.
fn next_step(data: &ImportData, exit: Exit) -> String {
    match exit {
        Exit::Validation => {
            "the archive does not verify; re-export it from the source project".to_string()
        }
        Exit::ApprovalRequired => {
            "re-run with --force in an interactive terminal to replace the conflicting files"
                .to_string()
        }
        _ if data.dry_run => format!("run `aicontext import {}` to write it", data.archive),
        _ => "run `aicontext doctor` to check the imported tree".to_string(),
    }
}

/// A count with its noun agreeing.
fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, ImportError, blocked_ancestor, join_relative, plan, resolve_source};
    use crate::archive::{self, Archive};
    use crate::exit::Exit;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// An archive with one file at `path` holding `content`.
    fn archive_with(path: &str, content: &str) -> Archive {
        archive::build(
            "ledger".to_string(),
            vec![(path.to_string(), content.to_string())],
        )
    }

    #[test]
    fn an_identical_file_is_unchanged_and_a_different_one_is_a_conflict() {
        let root = TempDir::new().expect("temp dir");
        let context = root.path().join(".ai");
        fs::create_dir_all(&context).expect("dir");
        fs::write(context.join("AI.md"), "# ai\n").expect("write");
        fs::write(context.join("RULES.md"), "# rules\n").expect("write");

        let planned = plan(&context, &archive_with("RULES.md", "# rules\n")).expect("plans");
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].action, Action::Unchanged);

        let planned = plan(&context, &archive_with("RULES.md", "# changed\n")).expect("plans");
        assert_eq!(planned[0].action, Action::Replace);

        let planned = plan(&context, &archive_with("NEW.md", "# new\n")).expect("plans");
        assert_eq!(planned[0].action, Action::Create);
    }

    #[test]
    fn a_directory_where_a_file_must_go_blocks_the_whole_import() {
        let root = TempDir::new().expect("temp dir");
        let context = root.path().join(".ai");
        fs::create_dir_all(context.join("AI.md")).expect("dir");

        let error = plan(&context, &archive_with("AI.md", "# ai\n")).expect_err("blocked");
        assert!(matches!(error, ImportError::Blocked { .. }));
        assert_eq!(error.code(), "IMP-005");
    }

    #[test]
    fn a_file_where_a_directory_must_go_blocks_the_whole_import() {
        let root = TempDir::new().expect("temp dir");
        let context = root.path().join(".ai");
        fs::create_dir_all(&context).expect("dir");
        fs::write(context.join("context"), "not a directory").expect("write");

        let error =
            plan(&context, &archive_with("context/stack.md", "stack\n")).expect_err("blocked");
        assert!(matches!(error, ImportError::Blocked { .. }));
    }

    #[test]
    fn a_missing_ancestor_directory_is_not_a_blocker() {
        let root = TempDir::new().expect("temp dir");
        let target = root.path().join(".ai/context/stack.md");
        assert!(
            blocked_ancestor(&target).is_none(),
            "nothing exists yet, so nothing blocks"
        );

        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        assert!(blocked_ancestor(&target).is_none());
    }

    #[test]
    fn join_relative_builds_a_path_inside_the_base() {
        assert_eq!(
            join_relative(Path::new("/srv/project/.ai"), "context/stack.md"),
            Path::new("/srv/project/.ai/context/stack.md")
        );
    }

    #[test]
    fn a_relative_source_resolves_against_the_project_root() {
        let root = Path::new("/srv/project");
        assert_eq!(
            resolve_source(root, Path::new("backup.aix")),
            Path::new("/srv/project/backup.aix")
        );
        assert_eq!(
            resolve_source(root, Path::new("/tmp/backup.aix")),
            Path::new("/tmp/backup.aix")
        );
    }

    #[test]
    fn every_import_error_answers_with_a_code_a_fix_and_an_exit() {
        let errors = [
            ImportError::ArchiveUnreadable {
                path: "a.aix".to_string(),
                reason: "it does not exist".to_string(),
            },
            ImportError::Blocked {
                path: ".ai/x".to_string(),
                reason: "a directory is already there".to_string(),
            },
            ImportError::ForceNeedsTerminal { count: 1 },
        ];
        for error in errors {
            assert!(!error.code().is_empty());
            assert!(!error.remediation().is_empty());
        }
        assert_eq!(
            ImportError::ArchiveUnreadable {
                path: "a.aix".to_string(),
                reason: "x".to_string()
            }
            .exit(),
            Exit::Usage
        );
        assert_eq!(
            ImportError::ForceNeedsTerminal { count: 1 }.exit(),
            Exit::ApprovalRequired
        );
    }
}
