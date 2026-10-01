//! What `init` intends to do, decided before it does any of it.
//!
//! A scaffolding command that edits as it discovers is the worst kind: a half-written `.ai/` leaves
//! no record of what was intended. So planning and applying are separate, the plan is complete and
//! printable, and `--dry-run` is the plan printed without the applying.
//!
//! Planning is also where idempotence lives. A document is `unchanged` only when the bytes on disk
//! are equal to the bytes the template would produce, so a document a developer has edited is
//! `kept` rather than overwritten, and re-running `init` in a fresh clone produces no diff at all.

use std::fs;
use std::path::{Path, PathBuf};

use aicontext_core::ErrorCode;

use crate::output::Finding;

use super::date::Date;
use super::detect::{self, Signal};
use super::error::InitError;
use super::render::Bindings;
use super::templates::{TemplateName, resolve};

/// The line `init` adds to `.gitignore`: local index state must never be committed
/// (`docs/CLI_SPEC.md` Â§3.1).
const GITIGNORE_PATH: &str = ".gitignore";
/// The line `init` adds, and the string validation looks for afterwards.
pub(crate) const GITIGNORE_ENTRY: &str = ".aicontext/";

/// The code for a document that no longer matches what the tool generated.
const HAND_EDITED: ErrorCode = ErrorCode::CTX_017;

/// What will happen to one path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    /// The path does not exist and will be written.
    Create,
    /// The path exists and differs from the template; it will be preserved.
    Keep,
    /// The path exists, differs from the template, and `--force` was passed; it will be replaced.
    Overwrite,
    /// The path exists and is already exactly what the template would write.
    Unchanged,
    /// A directory that may or may not exist yet.
    EnsureDirectory,
    /// One line will be added to an existing file.
    Append,
}

impl Action {
    /// The word printed in the report and written to JSON, so the two surfaces agree.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Keep => "keep",
            Self::Overwrite => "overwrite",
            Self::Unchanged => "unchanged",
            Self::EnsureDirectory => "directory",
            Self::Append => "append",
        }
    }

    /// Whether applying this action would destroy or add anything.
    pub(crate) const fn writes(self) -> bool {
        matches!(
            self,
            Self::Create | Self::Overwrite | Self::EnsureDirectory | Self::Append
        )
    }

    /// Whether applying this action would replace a document a developer may have edited.
    pub(crate) const fn destroys(self) -> bool {
        matches!(self, Self::Overwrite)
    }
}

/// One path in the plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    /// Relative to the project root, with `/` separators, exactly as it will appear on disk.
    pub(crate) path: String,
    /// What happens to it.
    pub(crate) action: Action,
    /// The bytes to write, empty for a directory.
    pub(crate) text: String,
}

/// Everything `init` decided, before anything was written.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    /// The absolute project root every path is resolved against.
    pub(crate) root: PathBuf,
    /// The directory name, used for document titles.
    pub(crate) project_name: String,
    /// Which template produced the plan.
    pub(crate) template: TemplateName,
    /// The paths, in the order they will be handled.
    pub(crate) entries: Vec<Entry>,
    /// What discovery found, empty when `--no-detect` was passed.
    pub(crate) signals: Vec<Signal>,
    /// What the plan is telling the developer about the project, by catalogue code.
    pub(crate) findings: Vec<Finding>,
    /// The date stamped into the documents.
    pub(crate) date: Date,
}

impl Plan {
    /// The entries with a given action, in plan order.
    pub(crate) fn with_action(&self, action: Action) -> Vec<&Entry> {
        self.entries
            .iter()
            .filter(|entry| entry.action == action)
            .collect()
    }

    /// How many entries will change something on disk.
    pub(crate) fn writes(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action.writes())
            .count()
    }

    /// How many documents `--force` would replace.
    pub(crate) fn overwrites(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.action.destroys())
            .count()
    }

    /// The version-control label for the identity line.
    pub(crate) fn vcs_label(&self) -> &'static str {
        detect::vcs_label(&self.signals)
    }
}

/// Builds the plan. Reads the project; writes nothing.
///
/// `force` is applied here, to the plan, rather than during writing, so the report and the write
/// cannot disagree about whether an edited document was preserved.
pub(crate) fn build(
    root: PathBuf,
    template: TemplateName,
    date: Date,
    detect_stack: bool,
    force: bool,
) -> Result<Plan, InitError> {
    let project_name = crate::project::project_name(&root)?;
    let bindings = Bindings::new(&project_name, date, template);
    let template_name = template.as_str();

    let mut entries = Vec::new();
    for file in resolve(template) {
        if file.dir_only {
            entries.push(directory_entry(&root, file.path)?);
            continue;
        }
        let text = bindings.apply(file.text, file.path, template_name)?;
        entries.push(file_entry(&root, file.path, &text, force)?);
    }
    entries.push(gitignore_entry(&root)?);

    let signals = if detect_stack {
        detect::detect(&root)?
    } else {
        Vec::new()
    };

    let mut findings = Vec::new();
    for entry in &entries {
        if entry.action == Action::Keep {
            findings.push(Finding::new(
                HAND_EDITED.to_string(),
                "warning",
                &entry.path,
                "differs from the template and was preserved",
                "re-run with --force in a terminal to replace it with the template",
            ));
        }
    }

    Ok(Plan {
        root,
        project_name,
        template,
        entries,
        signals,
        findings,
        date,
    })
}

/// One template file against what is already on disk.
fn file_entry(root: &Path, path: &str, text: &str, force: bool) -> Result<Entry, InitError> {
    let target = root.join(path);
    if !target.exists() {
        return Ok(Entry {
            path: path.to_string(),
            action: Action::Create,
            text: text.to_string(),
        });
    }
    if target.is_dir() {
        return Err(InitError::PathIsDirectory {
            path: path.to_string(),
        });
    }
    let existing = fs::read(&target).map_err(|error| InitError::read(path, error))?;
    if existing == text.as_bytes() {
        return Ok(Entry {
            path: path.to_string(),
            action: Action::Unchanged,
            text: String::new(),
        });
    }
    Ok(Entry {
        path: path.to_string(),
        action: if force {
            Action::Overwrite
        } else {
            Action::Keep
        },
        text: text.to_string(),
    })
}

/// One template directory, which is created whether or not it already exists.
fn directory_entry(root: &Path, path: &str) -> Result<Entry, InitError> {
    let target = root.join(path);
    if target.is_file() {
        return Err(InitError::PathIsFile {
            path: path.to_string(),
        });
    }
    let action = if target.is_dir() {
        Action::Unchanged
    } else {
        Action::EnsureDirectory
    };
    Ok(Entry {
        path: path.to_string(),
        action,
        text: String::new(),
    })
}

/// The `.gitignore` entry, which is appended to rather than rewritten.
///
/// A `.gitignore` belongs to the developer: it may hold a hundred entries and a comment explaining
/// them, and `init` has no business regenerating it.
fn gitignore_entry(root: &Path) -> Result<Entry, InitError> {
    let target = root.join(GITIGNORE_PATH);
    let entry = |action: Action, text: &str| Entry {
        path: GITIGNORE_PATH.to_string(),
        action,
        text: text.to_string(),
    };
    if !target.exists() {
        return Ok(entry(Action::Create, GITIGNORE_ENTRY));
    }
    if target.is_dir() {
        return Err(InitError::PathIsDirectory {
            path: GITIGNORE_PATH.to_string(),
        });
    }
    let existing =
        fs::read_to_string(&target).map_err(|error| InitError::read(GITIGNORE_PATH, error))?;
    if lists_entry(&existing) {
        Ok(entry(Action::Unchanged, ""))
    } else {
        Ok(entry(Action::Append, GITIGNORE_ENTRY))
    }
}

/// Whether a `.gitignore` already ignores the local state directory.
///
/// Both `.aicontext` and `.aicontext/` are accepted, because both are correct and refusing the
/// second would add a duplicate line to someone's file.
fn lists_entry(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim().trim_end_matches('/');
        !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed == ".aicontext"
    })
}

#[cfg(test)]
mod tests {
    use super::{Action, GITIGNORE_ENTRY, build, lists_entry};
    use crate::init::date::Date;
    use crate::init::templates::TemplateName;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn date() -> Date {
        Date::from_days_since_epoch(20_269)
    }

    fn plan(root: &Path) -> super::Plan {
        build(
            root.to_path_buf(),
            TemplateName::Default,
            date(),
            false,
            false,
        )
        .expect("plans")
    }

    fn forced(root: &Path) -> super::Plan {
        build(
            root.to_path_buf(),
            TemplateName::Default,
            date(),
            false,
            true,
        )
        .expect("plans")
    }

    #[test]
    fn a_fresh_project_plans_every_file_and_the_gitignore_entry() {
        let root = TempDir::new().expect("temp dir");
        let plan = plan(root.path());
        assert_eq!(
            plan.with_action(Action::Create).len(),
            plan.entries.len() - plan.with_action(Action::EnsureDirectory).len(),
        );
        assert!(plan.entries.iter().any(|entry| entry.path == ".gitignore"
            && entry.action == Action::Create
            && entry.text == GITIGNORE_ENTRY));
        assert!(
            plan.entries
                .iter()
                .any(|entry| entry.path == ".ai/RULES.md")
        );
    }

    #[test]
    fn an_edited_document_is_kept_and_reported_rather_than_overwritten() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        fs::write(root.path().join(".ai").join("RULES.md"), "mine\n").expect("write");

        let plan = plan(root.path());
        let entry = plan
            .entries
            .iter()
            .find(|e| e.path == ".ai/RULES.md")
            .expect("planned");
        assert_eq!(
            entry.action,
            Action::Keep,
            "an edit must survive a plain run"
        );
        assert_eq!(plan.overwrites(), 0);
        assert_eq!(plan.findings.len(), 1);
        assert_eq!(plan.findings[0].code, "CTX-017");
        assert!(
            !plan.findings[0].is_error(),
            "a preserved edit is a warning"
        );
    }

    #[test]
    fn force_replaces_an_edited_document() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        fs::write(root.path().join(".ai").join("RULES.md"), "mine\n").expect("write");

        let plan = forced(root.path());
        assert_eq!(plan.overwrites(), 1, "--force must be visible in the plan");
        let entry = plan
            .entries
            .iter()
            .find(|e| e.path == ".ai/RULES.md")
            .expect("planned");
        assert_eq!(entry.action, Action::Overwrite);
    }

    #[test]
    fn a_second_run_plans_nothing_because_the_bytes_are_already_equal() {
        let root = TempDir::new().expect("temp dir");
        let first = plan(root.path());
        crate::init::apply::apply(&first).expect("applies");

        let second = plan(root.path());
        assert_eq!(
            second.writes(),
            0,
            "init must be idempotent: {:?}",
            second.entries
        );
        assert!(
            second
                .entries
                .iter()
                .all(|entry| entry.action == Action::Unchanged)
        );
    }

    #[test]
    fn an_existing_gitignore_is_appended_to_and_never_rewritten() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join(".gitignore"), "target/\n").expect("write");
        let entry = plan(root.path())
            .entries
            .into_iter()
            .find(|entry| entry.path == ".gitignore")
            .expect("planned");
        assert_eq!(entry.action, Action::Append);
        assert_eq!(entry.text, GITIGNORE_ENTRY);
    }

    #[test]
    fn a_gitignore_that_already_lists_the_directory_is_left_alone() {
        assert!(lists_entry("target/\n.aicontext/\n"));
        assert!(
            lists_entry(".aicontext\n"),
            "a bare name ignores the same thing"
        );
        assert!(!lists_entry("# .aicontext/\n"), "a comment is not an entry");
        assert!(!lists_entry(".aicontext-old/\n"));
    }

    #[test]
    fn a_directory_where_the_template_needs_a_document_is_refused_by_name() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai").join("RULES.md")).expect("dir");

        let error = build(
            root.path().to_path_buf(),
            TemplateName::Default,
            date(),
            false,
            false,
        )
        .expect_err("a document cannot be a directory");
        assert!(matches!(error, super::InitError::PathIsDirectory { .. }));
        assert_eq!(error.code(), "INIT-002");
        assert!(error.to_string().contains(".ai/RULES.md"), "{error}");
    }

    #[test]
    fn a_file_where_the_template_needs_a_directory_is_refused_by_name() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        fs::write(root.path().join(".ai").join("specs"), "").expect("write");

        let error = build(
            root.path().to_path_buf(),
            TemplateName::Default,
            date(),
            false,
            false,
        )
        .expect_err("a register directory cannot be a file");
        assert!(matches!(error, super::InitError::PathIsFile { .. }));
        assert_eq!(error.code(), "INIT-010");
    }

    #[test]
    fn discovery_can_be_skipped_entirely() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("Cargo.toml"), "").expect("write");
        let plan = build(
            root.path().to_path_buf(),
            TemplateName::Default,
            date(),
            false,
            false,
        )
        .expect("plans");
        assert!(plan.signals.is_empty());
        assert_eq!(plan.vcs_label(), "no vcs");
    }

    #[test]
    fn a_root_that_is_a_file_is_refused() {
        let root = TempDir::new().expect("temp dir");
        let file = root.path().join("not-a-dir");
        fs::write(&file, "").expect("write");
        // The resolution itself is `crate::project`'s; what `init` owns is the code a developer sees,
        // so that is what is asserted here.
        let error = InitError::from(
            crate::project::resolve_root(Some(&file)).expect_err("a file is not a project"),
        );
        assert_eq!(error.code(), "INIT-001");
    }

    #[test]
    fn a_root_that_does_not_exist_is_refused_by_name() {
        let root = TempDir::new().expect("temp dir");
        let missing = root.path().join("nowhere");
        let error = InitError::from(
            crate::project::resolve_root(Some(&missing)).expect_err("must not invent a directory"),
        );
        assert!(error.to_string().contains("nowhere"), "{error}");
    }
}
