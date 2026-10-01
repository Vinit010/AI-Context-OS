//! Checking what was just written, because a scaffolding command that reports success without
//! looking is only a guess.
//!
//! Validation here is deliberately narrow: it proves that the paths the plan promised exist, that
//! the bytes on disk are the bytes the plan intended, and that each generated document has the shape
//! `docs/CONTEXT_SPEC.md` §2 requires of it. The full check catalogue — schema validation, references,
//! ID uniqueness, credential scanning — belongs to `doctor`, `TASK-014`, and is deliberately not
//! duplicated here.
//!
//! `AI.md` is exempt from the front-matter requirement on purpose: the spec makes it the one plain
//! text document in `.ai/`, because an agent has to be able to read it before it can parse anything.

use std::fs;

use aicontext_core::ErrorCode;

use crate::output::Finding;

use super::error::InitError;
use super::plan::{Action, GITIGNORE_ENTRY, Plan};

/// The documents that must carry front matter.
const FRONT_MATTER_REQUIRED: &[&str] = &[
    ".ai/PRD.md",
    ".ai/ARCHITECTURE.md",
    ".ai/RULES.md",
    ".ai/CONVENTIONS.md",
    ".ai/DESIGN.md",
    ".ai/TASKS.md",
    ".ai/MEMORY.md",
];

/// The most front matter `init` will read back. A document whose front matter is larger than this is
/// already wrong, and reading it all would turn a check into a cost (`RULES.md` §11).
const MAX_FRONTMATTER_BYTES: usize = 8 * 1024;

/// Verifies the result of a plan that has been applied.
///
/// Returns the findings about the documents themselves. A failure to read or match what was written
/// is an [`InitError`], not a finding: the command cannot claim success over it.
pub(crate) fn validate(plan: &Plan) -> Result<Vec<Finding>, InitError> {
    for entry in &plan.entries {
        let target = plan.root.join(&entry.path);
        if !target.exists() {
            return Err(InitError::Write {
                path: entry.path.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the planned path is missing after writing",
                ),
            });
        }
        match entry.action {
            Action::Create | Action::Overwrite => {
                let written =
                    fs::read(&target).map_err(|error| InitError::read(&entry.path, error))?;
                if written != entry.text.as_bytes() {
                    return Err(InitError::Write {
                        path: entry.path.clone(),
                        source: std::io::Error::other(
                            "the file on disk does not match what was written",
                        ),
                    });
                }
            }
            Action::EnsureDirectory if !target.is_dir() => {
                return Err(InitError::Write {
                    path: entry.path.clone(),
                    source: std::io::Error::other("the directory is not a directory"),
                });
            }
            Action::Append => {
                let text = fs::read_to_string(&target)
                    .map_err(|error| InitError::read(&entry.path, error))?;
                if !text.lines().any(|line| line.trim() == GITIGNORE_ENTRY) {
                    return Err(InitError::GitignoreUnchanged {
                        path: GITIGNORE_ENTRY.to_string(),
                    });
                }
            }
            Action::Keep | Action::Unchanged | Action::EnsureDirectory => {}
        }
    }
    Ok(document_findings(plan))
}

/// Structural findings about the documents `init` generated.
///
/// Only files this run wrote are examined. A document a developer owns is `doctor`'s business, and
/// reporting on it here would mean `init` failing over something it did not create.
fn document_findings(plan: &Plan) -> Vec<Finding> {
    let mut findings = Vec::new();
    for entry in &plan.entries {
        if !entry.action.writes() || entry.action == Action::EnsureDirectory {
            continue;
        }
        if !FRONT_MATTER_REQUIRED.contains(&entry.path.as_str()) {
            continue;
        }
        let target = plan.root.join(&entry.path);
        let Ok(text) = fs::read_to_string(&target) else {
            continue;
        };
        let missing = match front_matter(&text) {
            None => vec!["id", "type"],
            Some(keys) => ["id", "type"]
                .into_iter()
                .filter(|key| !keys.iter().any(|present| present == key))
                .collect(),
        };
        for key in missing {
            findings.push(Finding::new(
                ErrorCode::CTX_002.to_string(),
                "error",
                &entry.path,
                format!("front matter has no `{key}`"),
                "a generated document must be valid; this is a template bug, so please report it",
            ));
        }
    }
    findings
}

/// The top-level keys of a document's front matter, or `None` when there is no front matter at all.
///
/// A deliberately small reader: it needs `id` and `type`, and a full YAML parser is `doctor`'s
/// dependency, not `init`'s. Anything this cannot read it refuses to guess about.
fn front_matter(text: &str) -> Option<Vec<String>> {
    let rest = text.strip_prefix("---\n")?;
    if rest.len() > MAX_FRONTMATTER_BYTES {
        return None;
    }
    let end = rest.find("\n---").or_else(|| rest.find("\n..."))?;
    let mut keys = Vec::new();
    for line in rest[..end].lines() {
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || line.starts_with([' ', '\t', '-']) {
            continue;
        }
        keys.push(key.to_string());
    }
    Some(keys)
}

#[cfg(test)]
mod tests {
    use super::{front_matter, validate};
    use crate::init::apply::apply;
    use crate::init::date::Date;
    use crate::init::plan::build;
    use crate::init::templates::TemplateName;
    use std::fs;
    use tempfile::TempDir;

    fn applied(root: &std::path::Path) -> crate::init::plan::Plan {
        let plan = build(
            root.to_path_buf(),
            TemplateName::Default,
            Date::from_days_since_epoch(20_269),
            false,
            false,
        )
        .expect("plans");
        apply(&plan).expect("applies");
        plan
    }

    #[test]
    fn a_fresh_skeleton_validates_without_a_single_finding() {
        let root = TempDir::new().expect("temp dir");
        let findings = validate(&applied(root.path())).expect("validates");
        assert!(
            findings.is_empty(),
            "the templates must validate: {findings:?}"
        );
    }

    #[test]
    fn every_stack_template_validates() {
        for template in TemplateName::ALL {
            let root = TempDir::new().expect("temp dir");
            let plan = build(
                root.path().to_path_buf(),
                *template,
                Date::from_days_since_epoch(20_269),
                false,
                false,
            )
            .expect("plans");
            apply(&plan).expect("applies");
            let findings = validate(&plan).expect("validates");
            assert!(findings.is_empty(), "{template} produced {findings:?}");
        }
    }

    #[test]
    fn the_generated_tree_has_every_register_the_spec_promises() {
        let root = TempDir::new().expect("temp dir");
        applied(root.path());
        for path in [
            ".ai/AI.md",
            ".ai/RULES.md",
            ".ai/permissions/permissions.yaml",
            ".ai/context/stack.md",
            ".ai/specs",
            ".ai/tasks",
            ".ai/decisions",
            ".ai/bugs",
            ".ai/changes",
            ".ai/workflows",
            ".ai/agents",
            ".ai/integrations",
            ".ai/schemas",
        ] {
            assert!(root.path().join(path).exists(), "{path} is missing");
        }
    }

    #[test]
    fn ai_md_is_not_required_to_have_front_matter() {
        let root = TempDir::new().expect("temp dir");
        applied(root.path());
        let ai = fs::read_to_string(root.path().join(".ai/AI.md")).expect("read");
        assert!(
            ai.starts_with("# AI.md"),
            "AI.md must be readable as plain text"
        );
        assert!(front_matter(&ai).is_none());
    }

    #[test]
    fn a_file_that_disappears_after_writing_is_an_error_not_a_finding() {
        let root = TempDir::new().expect("temp dir");
        let plan = applied(root.path());
        fs::remove_file(root.path().join(".ai/RULES.md")).expect("remove");
        assert!(validate(&plan).is_err(), "a missing file must not validate");
    }

    #[test]
    fn a_document_whose_bytes_were_changed_by_something_else_does_not_validate() {
        let root = TempDir::new().expect("temp dir");
        let plan = applied(root.path());
        fs::write(root.path().join(".ai/RULES.md"), "something else").expect("write");
        assert!(validate(&plan).is_err());
    }

    #[test]
    fn the_front_matter_reader_ignores_nested_lines() {
        let keys =
            front_matter("---\nid: RULES-001\ndone:\n  - one\n---\n# RULES\n").expect("parses");
        assert_eq!(keys, vec!["id".to_string(), "done".to_string()]);
    }

    #[test]
    fn a_generated_document_without_front_matter_is_reported_against_both_keys() {
        let root = TempDir::new().expect("temp dir");
        let plan = applied(root.path());
        fs::write(root.path().join(".ai/RULES.md"), "# RULES\n").expect("write");

        let findings = super::document_findings(&plan);
        assert_eq!(findings.len(), 2, "both keys are missing: {findings:?}");
        for finding in &findings {
            assert_eq!(finding.code, "CTX-002");
            assert!(finding.is_error());
            assert_eq!(finding.path, ".ai/RULES.md");
        }
    }
}
