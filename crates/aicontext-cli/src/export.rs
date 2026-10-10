//! `aicontext export`: write a deterministic, verifiable archive of `.ai/`
//! (`docs/CLI_SPEC.md` §4, `TASK-020`).
//!
//! The order is the one `init` established — plan, approve, write — because the command reads a
//! developer's documents to produce an artifact, and a command that does that must be trustworthy
//! about what it touched. Everything that can refuse happens before the first byte is written.
//!
//! # What is refused, and why
//!
//! The walk is a reader of external input, so it applies the caps `RULES.md` §11 requires and fails
//! closed. A file that is not UTF-8 text, that is larger than [`MAX_FILE_BYTES`], that is a symlink
//! rather than a regular file, or whose relative path is not one [`crate::archive::safe_relative`]
//! accepts, makes the whole run refuse with exit 3 and **no archive at all** — never a partial one.
//! That is what `PRD.md` F10 means by "no export lossy": a silently truncated archive is worse than
//! a failed export, because it is trusted.
//!
//! # `--dry-run`
//!
//! A dry run performs every check except the terminal requirement for `--force`, writes nothing, and
//! returns the exit code an interactive run of the same command would return. A refusal that a real
//! run would report — the output already existing without `--force` — is still reported, so a script
//! can predict the outcome rather than discover it.

use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::archive::{self, Archive, MAX_ENTRIES, MAX_FILE_BYTES};
use crate::args::{ExportArgs, GlobalArgs};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// How many files are listed before the report says how many it held back, as `status` does.
const MAX_LISTED_FILES: usize = 20;

/// A failure that stopped `export` from writing an archive.
///
/// Every variant carries a stable `EXP-` code, a message naming the concrete path, and a remediation
/// (`RULES.md` §4.3). The code space is `EXP-` rather than `CTX-`, because these are failures of the
/// command, not findings about a document.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExportError {
    /// The directory the command was pointed at is not usable as a project root.
    #[error("cannot use {path} as the project root: {reason}")]
    UnusableRoot {
        /// The path as the user would recognise it.
        path: String,
        /// Why it cannot be used.
        reason: String,
    },

    /// There is no `.ai/` directory to archive.
    #[error("there is no .ai/ directory at {path}")]
    NoContext {
        /// The `.ai` path that was expected.
        path: String,
    },

    /// A file in the tree could not be read.
    #[error("cannot read {path}")]
    Read {
        /// The offending path.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// A directory in the tree could not be listed.
    #[error("cannot list {path}")]
    List {
        /// The offending directory.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// The output path names a directory, and an archive is a file.
    #[error("{path} is a directory, and export writes a file")]
    PathIsDirectory {
        /// The offending path.
        path: String,
    },

    /// The archive could not be written.
    #[error("cannot write {path}")]
    Write {
        /// The offending path.
        path: String,
        /// The operating system's reason.
        #[source]
        source: io::Error,
    },

    /// `--force` was asked for where there is no human to confirm with.
    #[error("--force would replace the archive at {path}, and an absent human is a deny")]
    ForceNeedsTerminal {
        /// The archive that would be replaced.
        path: String,
    },

    /// The archive did not serialise, which would be a bug.
    #[error("the archive could not be rendered as JSON: {source}")]
    Render {
        /// What the serialiser said.
        #[source]
        source: serde_json::Error,
    },
}

impl From<ProjectError> for ExportError {
    /// The project root could not be resolved, reported under `export`'s own code.
    fn from(error: ProjectError) -> Self {
        let (path, reason) = error.into_parts();
        Self::UnusableRoot { path, reason }
    }
}

impl ExportError {
    /// A stable code for this failure, independent of its wording.
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => "EXP-001",
            Self::NoContext { .. } => "EXP-002",
            Self::Read { .. } => "EXP-003",
            Self::List { .. } => "EXP-004",
            Self::Write { .. } => "EXP-005",
            Self::ForceNeedsTerminal { .. } => "EXP-006",
            Self::Render { .. } => "EXP-007",
            Self::PathIsDirectory { .. } => "EXP-008",
        }
    }

    /// What the developer can do about it. Every error answers this (`RULES.md` §4.3).
    pub(crate) const fn remediation(&self) -> &'static str {
        match self {
            Self::UnusableRoot { .. } => {
                "pass --cwd with an existing directory, or run export from inside the project"
            }
            Self::NoContext { .. } => {
                "run `aicontext init` to create the .ai skeleton, then export it"
            }
            Self::Read { .. } => "check that the file is readable and that nothing is holding it",
            Self::List { .. } => "check the directory's permissions, then run export again",
            Self::PathIsDirectory { .. } => "name a file to create, or move the directory aside",
            Self::Write { .. } => {
                "check the directory's permissions and free space, then run export again"
            }
            Self::ForceNeedsTerminal { .. } => {
                "run export in an interactive terminal to confirm, or choose another path"
            }
            Self::Render { .. } => "this is a bug: report it with the archive path",
        }
    }

    /// The exit code this failure produces (`docs/CLI_SPEC.md` §5).
    pub(crate) const fn exit(&self) -> Exit {
        match self {
            Self::UnusableRoot { .. } | Self::NoContext { .. } | Self::PathIsDirectory { .. } => {
                Exit::Usage
            }
            Self::ForceNeedsTerminal { .. } => Exit::ApprovalRequired,
            _ => Exit::General,
        }
    }
}

/// Runs `export` and returns the exit code.
pub(crate) fn run(args: &ExportArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
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
    args: &ExportArgs,
    global: &GlobalArgs,
    terminal: &mut Terminal,
) -> Result<Exit, ExportError> {
    let root = project::resolve_root(global.cwd.as_deref())?;
    let name = project::project_name(&root)?;
    let context = root.join(".ai");
    let output = resolve_output(&root, &args.path);
    let shown = project::display(&output);

    if !context.is_dir() {
        return Err(ExportError::NoContext {
            path: project::display(&context),
        });
    }
    if output.is_dir() {
        return Err(ExportError::PathIsDirectory { path: shown });
    }

    let vcs = project::vcs_label(&root);
    let mut files = Vec::new();
    let mut findings = Vec::new();
    walk(&context, "", &mut files, &mut findings)?;

    // A finding is a reason not to write an archive at all. Refusing whole is the only honest
    // answer when part of the tree cannot be represented (PRD F10).
    if findings.iter().any(Finding::is_error) {
        let data = ExportData::refused(&name, &root, &shown, args.dry_run);
        return report(&data, &findings, Exit::Validation, vcs, terminal, global);
    }

    let built = archive::build(name.clone(), files);
    let text = archive::render(&built).map_err(|source| ExportError::Render { source })?;

    let exists = output.exists();
    let mut findings = Vec::new();
    let exit = if exists && !args.force {
        findings.push(Finding::new(
            "EXP-020",
            "error",
            shown.clone(),
            "an archive already exists at that path",
            "re-run with --force in an interactive terminal to replace it",
        ));
        Exit::ApprovalRequired
    } else if exists && args.force && !terminal.is_terminal() && !args.dry_run {
        return Err(ExportError::ForceNeedsTerminal { path: shown });
    } else {
        if !args.dry_run {
            fs::write(&output, text.as_bytes()).map_err(|source| ExportError::Write {
                path: shown.clone(),
                source,
            })?;
        }
        Exit::Ok
    };

    let data = ExportData::of(&name, &root, &shown, args.dry_run, exists, &built);
    report(&data, &findings, exit, vcs, terminal, global)
}

/// The output path, with a relative one resolved against the project root.
///
/// Resolved against the root rather than the working directory so `--cwd` and a plain run agree on
/// where `backup.aix` lands.
fn resolve_output(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Walks `.ai/` collecting `(relative path, content)`, refusing what an archive may not carry.
///
/// The walk is bounded three ways: by [`MAX_DEPTH`](crate::archive::MAX_DEPTH) through the path
/// check, by [`MAX_ENTRIES`] on the file count, and by [`MAX_FILE_BYTES`] on each file. A directory
/// whose own relative path is refused is not descended into, so a hostile or accidental deep tree
/// costs one finding rather than an unbounded walk (`RULES.md` §11).
fn walk(
    directory: &Path,
    prefix: &str,
    files: &mut Vec<(String, String)>,
    findings: &mut Vec<Finding>,
) -> Result<(), ExportError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|source| ExportError::List {
            path: project::display(directory),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ExportError::List {
            path: project::display(directory),
            source,
        })?;
    // readdir order is platform-defined; children are sorted by name so the walk,
    // and therefore the archive, is the same on every machine.
    entries.sort_by_key(DirEntry::file_name);

    for entry in entries {
        // A previous file already crossed the limit and recorded the finding: stop walking.
        if files.len() > MAX_ENTRIES {
            return Ok(());
        }
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| ExportError::Read {
            path: project::display(&path),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };

        if !archive::safe_relative(&relative) {
            findings.push(Finding::new(
                "EXP-014",
                "error",
                format!(".ai/{relative}"),
                "the path is not one an archive may carry",
                "rename it to a plain relative path of ordinary components",
            ));
            continue;
        }
        if file_type.is_dir() {
            walk(&path, &relative, files, findings)?;
            continue;
        }
        if !file_type.is_file() {
            findings.push(Finding::new(
                "EXP-012",
                "error",
                format!(".ai/{relative}"),
                "it is a symlink, a device, or another kind of file that export will not archive",
                "replace it with a regular file",
            ));
            continue;
        }
        let metadata = entry.metadata().map_err(|source| ExportError::Read {
            path: project::display(&path),
            source,
        })?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            findings.push(Finding::new(
                "EXP-011",
                "error",
                format!(".ai/{relative}"),
                format!(
                    "it is {} bytes, over the {}-byte per-file limit",
                    metadata.len(),
                    MAX_FILE_BYTES
                ),
                "keep large binaries outside .ai/, or split the file",
            ));
            continue;
        }
        let bytes = fs::read(&path).map_err(|source| ExportError::Read {
            path: project::display(&path),
            source,
        })?;
        match String::from_utf8(bytes) {
            Ok(content) => {
                files.push((relative, content));
                if files.len() > MAX_ENTRIES {
                    record_too_many(findings);
                }
            }
            Err(_) => findings.push(Finding::new(
                "EXP-010",
                "error",
                format!(".ai/{relative}"),
                "it is not valid UTF-8 text, so it cannot go in a JSON archive",
                "keep binary files outside .ai/",
            )),
        }
    }
    Ok(())
}

/// Records the file-count finding once, however many walks reach the limit.
fn record_too_many(findings: &mut Vec<Finding>) {
    if findings.iter().any(|finding| finding.code == "EXP-013") {
        return;
    }
    findings.push(Finding::new(
        "EXP-013",
        "error",
        ".ai",
        format!("the tree holds more than {MAX_ENTRIES} files, over the archive limit"),
        "export a smaller tree, or a subdirectory, by hand",
    ));
}

/// The JSON payload of `export`.
#[derive(Clone, Debug, Serialize)]
struct ExportData {
    /// The absolute project root.
    root: String,
    /// The project name taken from the root directory.
    project: String,
    /// The archive path, with `/` separators.
    archive: String,
    /// Whether this was `--dry-run`, so nothing was written.
    dry_run: bool,
    /// Whether an existing archive was, or would be, replaced.
    replacing: bool,
    /// How many files the archive holds. Zero when the tree was refused.
    file_count: usize,
    /// The sum of the files' lengths. Zero when the tree was refused.
    total_bytes: usize,
    /// The manifest digest, or `null` when the tree was refused before an archive was built.
    digest: Option<String>,
    /// One row per file, sorted by path.
    files: Vec<DigestRow>,
}

/// One file's identity in the JSON payload.
#[derive(Clone, Debug, Serialize)]
struct DigestRow {
    /// The path relative to `.ai/`, with `/` separators.
    path: String,
    /// The content's length in bytes.
    bytes: usize,
    /// Lowercase hex SHA-256 of the content.
    sha256: String,
}

impl ExportData {
    /// A run that refused the tree, so no archive exists to describe.
    fn refused(project: &str, root: &Path, archive: &str, dry_run: bool) -> Self {
        Self {
            root: project::display(root),
            project: project.to_string(),
            archive: archive.to_string(),
            dry_run,
            replacing: false,
            file_count: 0,
            total_bytes: 0,
            digest: None,
            files: Vec::new(),
        }
    }

    /// A run that built an archive.
    fn of(
        project: &str,
        root: &Path,
        archive: &str,
        dry_run: bool,
        replacing: bool,
        built: &Archive,
    ) -> Self {
        Self {
            root: project::display(root),
            project: project.to_string(),
            archive: archive.to_string(),
            dry_run,
            replacing,
            file_count: built.manifest.file_count,
            total_bytes: built.manifest.total_bytes,
            digest: Some(built.manifest.digest.clone()),
            files: built
                .manifest
                .files
                .iter()
                .map(|file| DigestRow {
                    path: file.path.clone(),
                    bytes: file.bytes,
                    sha256: file.sha256.clone(),
                })
                .collect(),
        }
    }
}

/// Reports the run: the envelope under `--json`, the human report otherwise.
fn report(
    data: &ExportData,
    findings: &[Finding],
    exit: Exit,
    vcs: &str,
    terminal: &mut Terminal,
    global: &GlobalArgs,
) -> Result<Exit, ExportError> {
    if global.json {
        let body = envelope(
            "export",
            exit.code(),
            data.clone(),
            findings.to_vec(),
            counts(data, findings, exit),
        );
        terminal.json(&body).map_err(|error| ExportError::Write {
            path: "the JSON envelope on stdout".to_string(),
            source: io::Error::other(error),
        })?;
        return Ok(exit);
    }
    print_human(data, findings, exit, vcs, terminal);
    Ok(exit)
}

/// The human report: identity, the archive, the files, the findings, and the summary.
fn print_human(
    data: &ExportData,
    findings: &[Finding],
    exit: Exit,
    vcs: &str,
    terminal: &mut Terminal,
) {
    terminal.identity("aicontext", &data.project, vcs);

    if data.digest.is_some() {
        let mark = if exit == Exit::Ok && !data.dry_run {
            terminal.mark_ok()
        } else {
            terminal.mark_info()
        };
        terminal.row(
            &mark,
            "archive",
            &format!(
                "{}  {} files, {} bytes",
                data.archive, data.file_count, data.total_bytes
            ),
        );
    }
    let mark = terminal.mark_info();
    for file in data.files.iter().take(MAX_LISTED_FILES) {
        terminal.row(
            &mark,
            "file",
            &format!(".ai/{}  {} bytes", file.path, file.bytes),
        );
    }
    if data.files.len() > MAX_LISTED_FILES {
        terminal.say(&format!(
            "      … and {} more",
            data.files.len() - MAX_LISTED_FILES
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
fn counts(data: &ExportData, findings: &[Finding], exit: Exit) -> String {
    if exit == Exit::Validation {
        let errors = findings.iter().filter(|finding| finding.is_error()).count();
        return format!("{errors} errors, nothing written");
    }
    if exit == Exit::ApprovalRequired {
        return "nothing written".to_string();
    }
    let verb = if data.dry_run { "would write" } else { "wrote" };
    format!(
        "{verb} {} ({} bytes)",
        plural(data.file_count, "file", "files"),
        data.total_bytes
    )
}

/// What to do next, which depends on why the run ended.
fn next_step(data: &ExportData, exit: Exit) -> String {
    match exit {
        Exit::Validation => "fix the findings above, then run `aicontext export` again".to_string(),
        Exit::ApprovalRequired => {
            "re-run with --force in an interactive terminal to replace the archive".to_string()
        }
        _ if data.dry_run => format!("run `aicontext export {}` to write it", data.archive),
        _ => format!("verify it with `aicontext import {}`", data.archive),
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
    use super::{ExportError, counts, next_step, resolve_output, walk};
    use crate::archive::MAX_FILE_BYTES;
    use crate::exit::Exit;
    use crate::output::Finding;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// The relative paths a walk collected.
    fn walked(root: &Path) -> (Vec<String>, Vec<Finding>) {
        let mut files = Vec::new();
        let mut findings = Vec::new();
        walk(root, "", &mut files, &mut findings).expect("walks");
        let paths = files.into_iter().map(|(path, _)| path).collect();
        (paths, findings)
    }

    #[test]
    fn a_walk_collects_every_regular_file_relative_to_the_root() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join("context")).expect("dir");
        fs::write(root.path().join("AI.md"), "# ai\n").expect("write");
        fs::write(root.path().join("context/stack.md"), "stack\n").expect("write");

        let (paths, findings) = walked(root.path());
        assert_eq!(paths, ["AI.md", "context/stack.md"]);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_file_that_is_not_utf8_is_a_finding_not_an_archive_entry() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("AI.md"), [0xff, 0xfe, 0x00]).expect("write");

        let (paths, findings) = walked(root.path());
        assert!(paths.is_empty());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "EXP-010");
    }

    #[test]
    fn a_file_over_the_per_file_cap_is_a_finding() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("big.md"), "x".repeat(MAX_FILE_BYTES + 1)).expect("write");

        let (paths, findings) = walked(root.path());
        assert!(paths.is_empty(), "an oversize file must not be collected");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "EXP-011");
    }

    #[test]
    fn a_tree_over_the_entry_limit_stops_and_reports_once() {
        let root = TempDir::new().expect("temp dir");
        for index in 0..super::MAX_ENTRIES + 5 {
            fs::write(root.path().join(format!("f{index}.md")), "x").expect("write");
        }
        let (paths, findings) = walked(root.path());
        assert!(
            paths.len() <= super::MAX_ENTRIES + 1,
            "the walk must stop at the cap"
        );
        let count = findings
            .iter()
            .filter(|finding| finding.code == "EXP-013")
            .count();
        assert_eq!(count, 1, "the finding is recorded once: {findings:?}");
    }

    #[test]
    fn a_path_deeper_than_the_limit_is_refused_rather_than_walked() {
        let root = TempDir::new().expect("temp dir");
        let deep: PathBuf = std::iter::repeat_n("d", 17).collect();
        fs::create_dir_all(root.path().join(&deep)).expect("dirs");
        fs::write(root.path().join(&deep).join("leaf.md"), "x").expect("write");

        let (paths, findings) = walked(root.path());
        assert!(
            paths.is_empty(),
            "a path past the depth cap is not collected"
        );
        assert!(
            findings.iter().any(|finding| finding.code == "EXP-014"),
            "{findings:?}"
        );
    }

    #[test]
    fn a_relative_output_is_resolved_against_the_project_root() {
        let root = Path::new("/srv/project");
        assert_eq!(
            resolve_output(root, Path::new("backup.aix")),
            Path::new("/srv/project/backup.aix")
        );
        assert_eq!(
            resolve_output(root, Path::new("/tmp/backup.aix")),
            Path::new("/tmp/backup.aix"),
            "an absolute path is left alone"
        );
    }

    #[test]
    fn every_export_error_answers_with_a_code_a_fix_and_an_exit() {
        let errors = [
            ExportError::NoContext {
                path: ".ai".to_string(),
            },
            ExportError::PathIsDirectory {
                path: "out".to_string(),
            },
            ExportError::ForceNeedsTerminal {
                path: "out".to_string(),
            },
        ];
        for error in errors {
            assert!(!error.code().is_empty());
            assert!(!error.remediation().is_empty());
        }
        assert_eq!(
            ExportError::NoContext {
                path: ".ai".to_string()
            }
            .exit(),
            Exit::Usage
        );
        assert_eq!(
            ExportError::ForceNeedsTerminal {
                path: "out".to_string()
            }
            .exit(),
            Exit::ApprovalRequired
        );
    }

    #[test]
    fn the_counts_and_next_step_follow_the_outcome() {
        let root = Path::new("/srv/project");
        let built = crate::archive::build(
            "ledger".to_string(),
            vec![("AI.md".to_string(), "# ai\n".to_string())],
        );
        let data = super::ExportData::of("ledger", root, "backup.aix", false, false, &built);
        assert_eq!(counts(&data, &[], Exit::Ok), "wrote 1 file (5 bytes)");
        assert!(next_step(&data, Exit::Ok).contains("import"));

        let dry = super::ExportData::of("ledger", root, "backup.aix", true, false, &built);
        assert!(counts(&dry, &[], Exit::Ok).starts_with("would write"));
        assert!(next_step(&dry, Exit::Ok).contains("to write it"));

        let refused = super::ExportData::refused("ledger", root, "backup.aix", false);
        assert_eq!(
            counts(
                &refused,
                &[Finding::new("EXP-010", "error", ".ai/x", "m", "r")],
                Exit::Validation
            ),
            "1 errors, nothing written"
        );
        assert_eq!(
            counts(&refused, &[], Exit::ApprovalRequired),
            "nothing written"
        );
    }
}
