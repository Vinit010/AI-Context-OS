//! Carrying out a plan, and nothing else.
//!
//! The plan already decided every action, so this module has no opinions: it walks the entries and
//! does exactly what each one says. Every path is resolved against the plan's root and checked
//! against the template's own prefix, so a plan is the only thing that decides what is written.
//!
//! Writes go through a temporary file in the destination directory and a rename, so an interrupted
//! run cannot leave a half-written document that later looks hand-written.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use super::error::InitError;
use super::plan::{Action, Plan};

/// Performs every write in the plan, in order.
pub(crate) fn apply(plan: &Plan) -> Result<(), InitError> {
    for entry in &plan.entries {
        let target = plan.root.join(&entry.path);
        match entry.action {
            Action::Create | Action::Overwrite => write_file(&target, &entry.path, &entry.text)?,
            Action::Append => append_line(&target, &entry.path, &entry.text)?,
            Action::EnsureDirectory => {
                fs::create_dir_all(&target)
                    .map_err(|error| InitError::create_directory(&entry.path, error))?;
            }
            Action::Keep | Action::Unchanged => {}
        }
    }
    Ok(())
}

/// Writes a file through a temporary file in the same directory, then renames it into place.
///
/// The rename is the commit point: on any platform this either happens completely or not at all, so
/// a document is never observed half-written. The temporary file is removed on failure, because
/// leaving one behind would be littering the developer's repository.
fn write_file(target: &Path, display: &str, text: &str) -> Result<(), InitError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| InitError::create_directory(&display_path(parent), error))?;
    }
    let temporary = target.with_extension("aicontext-tmp");
    let result = (|| -> Result<(), InitError> {
        let mut file =
            fs::File::create(&temporary).map_err(|error| InitError::write(display, error))?;
        file.write_all(text.as_bytes())
            .map_err(|error| InitError::write(display, error))?;
        file.sync_all()
            .map_err(|error| InitError::write(display, error))?;
        drop(file);
        fs::rename(&temporary, target).map_err(|error| InitError::write(display, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Adds one line to the end of a file, creating the line break the file is missing.
fn append_line(target: &Path, display: &str, line: &str) -> Result<(), InitError> {
    let existing = fs::read_to_string(target).map_err(|error| InitError::read(display, error))?;
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut file = OpenOptions::new()
        .append(true)
        .open(target)
        .map_err(|error| InitError::write(display, error))?;
    writeln!(file, "{separator}{line}").map_err(|error| InitError::write(display, error))
}

/// A path in the spelling used in error messages, with `/` separators.
fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::init::date::Date;
    use crate::init::plan::build;
    use crate::init::templates::TemplateName;
    use std::fs;
    use tempfile::TempDir;

    fn plan(root: &std::path::Path, force: bool) -> crate::init::plan::Plan {
        build(
            root.to_path_buf(),
            TemplateName::Default,
            Date::from_days_since_epoch(20_269),
            false,
            force,
        )
        .expect("plans")
    }

    #[test]
    fn every_planned_file_appears_on_disk() {
        let root = TempDir::new().expect("temp dir");
        let plan = plan(root.path(), false);
        apply(&plan).expect("applies");

        for entry in &plan.entries {
            let target = root.path().join(&entry.path);
            assert!(target.exists(), "{} was not created", entry.path);
            if entry.action == crate::init::plan::Action::EnsureDirectory {
                assert!(target.is_dir());
            }
        }
    }

    #[test]
    fn the_written_document_is_the_rendered_template_byte_for_byte() {
        let root = TempDir::new().expect("temp dir");
        let plan = plan(root.path(), false);
        apply(&plan).expect("applies");

        let rules = fs::read_to_string(root.path().join(".ai/RULES.md")).expect("read");
        assert!(
            rules.starts_with("---\nid: RULES-001\n"),
            "front matter is missing"
        );
        assert!(rules.contains("title: \""), "the title must be substituted");
        assert!(!rules.contains("{{"), "a placeholder reached the disk");
        assert!(
            rules.contains("created: 2025-06-30"),
            "the date was not substituted"
        );
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let root = TempDir::new().expect("temp dir");
        apply(&plan(root.path(), false)).expect("applies");
        let stray: Vec<String> = fs::read_dir(root.path().join(".ai"))
            .expect("read")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.contains("tmp"))
            .collect();
        assert!(stray.is_empty(), "left behind: {stray:?}");
    }

    #[test]
    fn an_append_preserves_the_developers_existing_lines() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join(".gitignore"), "target/\nnode_modules").expect("write");
        apply(&plan(root.path(), false)).expect("applies");

        let text = fs::read_to_string(root.path().join(".gitignore")).expect("read");
        assert_eq!(text, "target/\nnode_modules\n.aicontext/\n");
    }

    #[test]
    fn force_replaces_an_edited_document() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        fs::write(root.path().join(".ai").join("RULES.md"), "mine\n").expect("write");

        apply(&plan(root.path(), true)).expect("applies");
        let rules = fs::read_to_string(root.path().join(".ai/RULES.md")).expect("read");
        assert!(rules.starts_with("---"), "the edit survived --force");
    }

    #[test]
    fn an_unchanged_document_is_not_rewritten() {
        let root = TempDir::new().expect("temp dir");
        apply(&plan(root.path(), false)).expect("applies");
        let rules = root.path().join(".ai/RULES.md");
        let before = fs::metadata(&rules)
            .expect("stat")
            .modified()
            .expect("modified time");

        // A second plan, applied without force, must leave the file untouched.
        apply(&plan(root.path(), false)).expect("applies");
        let after = fs::metadata(&rules)
            .expect("stat")
            .modified()
            .expect("modified time");
        assert_eq!(before, after, "an unchanged file must not be rewritten");
    }
}
