//! A sample `.ai/` tree that produces no `doctor` findings.
//!
//! # What this fixture is for
//!
//! Almost every command that reads a project needs one of two things: a `.ai/` tree that is correct,
//! so a test asserts about what the command *did*; or one that is correct except for a single
//! deliberate defect, so a test asserts about what the command *found*. Hand-writing the correct one
//! in each test is how a suite ends up with four slightly different trees and a finding that depends
//! on which copy a test happened to use.
//!
//! [`sample_ai_tree`] is the correct one, in one place, and the tree `doctor` is guaranteed to be
//! clean about is asserted by a test in `aicontext-context` rather than by anything here. That test
//! is the only thing that can catch this fixture drifting: `aicontext-testkit` may not depend on
//! `aicontext-context` (`ARCHITECTURE.md` §3.2), so the proof has to live on the other side.
//!
//! # Why the tree is small rather than a copy of the templates
//!
//! `templates/` is what `init` writes, and it is rendered with the project name and today's date. A
//! fixture that embedded it would need that rendering, would break whenever a template changed, and
//! would carry eight documents and twenty schemas into every test that only needs a file to exist.
//! This tree carries the minimum that satisfies the contract, so a test that needs more writes it.
//!
//! # What makes it clean
//!
//! Four things, each from a rule rather than from what happened to work:
//!
//! - `.ai/AI.md` exists. `CTX-001` is an error without it.
//! - `.ai/TASKS.md` carries front matter, because it is a location `docs/CONTEXT_SPEC.md` §1 gives an
//!   ID convention to, and rule 1 requires front matter at such a location.
//! - The one inline entity carries a fenced `yaml` block, because §2.1 makes a missing block an
//!   error rather than something to guess past.
//! - `.ai/schemas/` exists. Its *contents* are the caller's business — `CTX-012` compares them
//!   against the compiled-in source set, and this crate has no access to it — but a missing directory
//!   is a warning of its own, so the directory is here and the schemas are not.
//!
//! The task's `spec` points at a real file for the same reason §2 rule 10 is satisfied by
//! construction: `CTX-007` checks a path reference resolves, and a fixture pointing at nothing
//! would have an error that has nothing to do with the test.

use std::path::Path;

use crate::error::FixtureError;

/// The task ID the sample tree declares, so a test can refer to it without repeating the literal.
pub const SAMPLE_TASK_ID: &str = "TASK-001";

/// The schema file the sample task points its `spec` at.
pub const SAMPLE_SPEC_PATH: &str = "docs/CONTEXT_SPEC.md";

/// Writes a minimal, finding-free `.ai/` tree into `root`.
///
/// # Errors
///
/// Returns [`FixtureError::Io`] when any file cannot be written.
///
/// # Example
///
/// ```
/// use aicontext_testkit::{TempProject, sample_ai_tree};
///
/// let project = TempProject::new("payments-ledger").expect("a temporary project");
/// sample_ai_tree(project.path()).expect("writes the tree");
///
/// assert!(project.join(".ai/AI.md").is_file());
/// assert!(project.join(".ai/schemas").is_dir());
/// ```
pub fn sample_ai_tree(root: &Path) -> Result<(), FixtureError> {
    write_file(root, ".ai/AI.md", ENTRY_POINT)?;
    write_file(root, ".ai/TASKS.md", TASKS)?;
    write_file(root, SAMPLE_SPEC_PATH, SPEC)?;
    create_dir(root, ".ai/schemas")?;
    Ok(())
}

/// The entry point document.
///
/// No front matter, which is correct here and only here: `AI.md` is the one location rule 1 exempts,
/// because a project whose entry point carries no metadata is still readable by a human and by an
/// agent. A fixture that put front matter on it would test nothing and imply the exemption is wrong.
const ENTRY_POINT: &str = "\
# AI.md

Entry point for agents working in this project.

## Read order

1. `RULES.md` — binding project policy
2. `ARCHITECTURE.md` — crate graph, storage, runtime
3. `TASKS.md` — the task register
";

/// The task register: front matter, then one inline entity.
///
/// The entity block is fenced `yaml` because §2.1 parses it exactly as front matter is parsed. Its
/// `spec` is a real path so `CTX-007` resolves it; `depends_on` is empty because a non-empty one
/// pointing at an undeclared task is precisely the finding a test would otherwise inherit.
const TASKS: &str = "\
---
id: TASKS-001
type: tasks
title: Task Register
---

# TASKS

### TASK-001 — Sample task

```yaml
id: TASK-001
title: Sample task
status: TODO
priority: HIGH
phase: 1
depends_on: []
spec: docs/CONTEXT_SPEC.md
```

The body of a sample task.
";

/// The specification the sample task points at.
const SPEC: &str = "\
# CONTEXT_SPEC

A sample specification, so the sample task's `spec` reference resolves to a file that exists.
";

/// Writes one file, creating its parent directories.
fn write_file(root: &Path, relative: &str, contents: &str) -> Result<(), FixtureError> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| FixtureError::Io {
            action: "create a sample .ai directory",
            source,
        })?;
    }
    std::fs::write(&path, contents).map_err(|source| FixtureError::Io {
        action: "write a sample .ai document",
        source,
    })
}

/// Creates one directory.
fn create_dir(root: &Path, relative: &str) -> Result<(), FixtureError> {
    std::fs::create_dir_all(root.join(relative)).map_err(|source| FixtureError::Io {
        action: "create a sample .ai directory",
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::{SAMPLE_SPEC_PATH, SAMPLE_TASK_ID, sample_ai_tree};
    use crate::temp::TempProject;

    #[test]
    fn the_tree_carries_the_four_files_a_clean_project_needs() {
        let project = TempProject::new("sampled").expect("a project");
        sample_ai_tree(project.path()).expect("writes the tree");

        assert!(project.join(".ai/AI.md").is_file());
        assert!(project.join(".ai/TASKS.md").is_file());
        assert!(project.join(SAMPLE_SPEC_PATH).is_file());
        assert!(
            project.join(".ai/schemas").is_dir(),
            "a missing schema directory is a CTX-012 warning of its own"
        );
    }

    #[test]
    fn the_entry_point_carries_no_front_matter_because_it_is_exempt() {
        // docs/CONTEXT_SPEC.md 2 rule 1 exempts AI.md. A fixture that added a block here would make
        // the exemption look untested, and would mislead the next reader into adding one.
        let project = TempProject::new("exempt").expect("a project");
        sample_ai_tree(project.path()).expect("writes the tree");

        let entry = project.read(".ai/AI.md").expect("reads");
        assert!(
            !entry.starts_with("---"),
            "AI.md is the one location that may omit front matter: {entry}"
        );
    }

    #[test]
    fn the_register_carries_front_matter_and_one_fenced_entity() {
        // Both halves are required: the location requires front matter (rule 1) and the heading
        // requires a block beneath it (rule 2.1).
        let project = TempProject::new("register").expect("a project");
        sample_ai_tree(project.path()).expect("writes the tree");

        let register = project.read(".ai/TASKS.md").expect("reads");
        assert!(register.starts_with("---\nid: TASKS-001\n"), "{register}");
        assert!(
            register.contains(&format!("### {SAMPLE_TASK_ID}")),
            "the entity heading names its ID: {register}"
        );
        assert!(
            register.contains("```yaml"),
            "the entity carries a fenced block: {register}"
        );
    }

    #[test]
    fn the_sample_task_points_at_a_specification_that_exists() {
        // CTX-007 resolves a path reference, so a fixture pointing at nothing carries an error that
        // has nothing to do with the test using it.
        let project = TempProject::new("spec").expect("a project");
        sample_ai_tree(project.path()).expect("writes the tree");

        let register = project.read(".ai/TASKS.md").expect("reads");
        assert!(register.contains(SAMPLE_SPEC_PATH), "{register}");
        assert!(project.join(SAMPLE_SPEC_PATH).is_file());
    }

    #[test]
    fn the_sample_task_depends_on_nothing_that_is_not_declared() {
        let project = TempProject::new("deps").expect("a project");
        sample_ai_tree(project.path()).expect("writes the tree");

        let register = project.read(".ai/TASKS.md").expect("reads");
        assert!(
            register.contains("depends_on: []"),
            "a non-empty list pointing at an undeclared task is a CTX-007 error: {register}"
        );
    }

    #[test]
    fn writing_the_tree_twice_is_the_same_tree() {
        let project = TempProject::new("twice").expect("a project");
        sample_ai_tree(project.path()).expect("first");
        let first = project.files().expect("walks");
        sample_ai_tree(project.path()).expect("second");
        assert_eq!(first, project.files().expect("walks"));
    }

    #[test]
    fn writing_into_a_root_that_is_not_there_is_an_error_not_a_panic() {
        let missing = std::path::Path::new("/definitely/not/a/directory/for/a/fixture");
        let error = sample_ai_tree(missing).expect_err("must not invent a directory");
        assert_eq!(error.code(), "FIX-002");
        assert!(error.to_string().contains("sample .ai"), "{error}");
    }
}