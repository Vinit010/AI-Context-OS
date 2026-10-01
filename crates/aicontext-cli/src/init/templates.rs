//! The skeleton templates, embedded in the binary at compile time.
//!
//! A template is a plain tree under `templates/` in this repository. `init` resolves it as
//! base-then-overlay: the base supplies the whole skeleton, and an overlay replaces a file by
//! matching relative path or adds one the base does not have. `blank` is the exception — it has no
//! base, because a project that asked for no stack guidance should not be handed eight documents it
//! did not ask for.
//!
//! Files are embedded with `include_str!` rather than read at run time so the binary has no
//! installation layout to get wrong, no asset directory to ship, and no way to be confused by a
//! template edited after the fact.
//!
//! Every path is relative to the project root and includes the `.ai/` prefix, so a path in a plan, in
//! an error, and on disk are the same string.

use std::fmt;

use clap::ValueEnum;

/// Which skeleton `init` starts from (`--template`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum TemplateName {
    /// The base skeleton with no stack-specific guidance.
    #[default]
    Default,
    /// The base skeleton plus Rust conventions and architecture.
    Rust,
    /// The base skeleton plus TypeScript-on-Node conventions and architecture.
    Node,
    /// The base skeleton plus Python conventions and architecture.
    Python,
    /// Two documents, no directory tree.
    Blank,
}

impl TemplateName {
    /// The name used on the command line and in output.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Blank => "blank",
        }
    }

    /// Every template, in the order `--help` lists them. Used by the tests that check the enum, the
    /// resolved trees, and the documented names agree.
    #[cfg(test)]
    pub(crate) const ALL: &'static [Self] = &[
        Self::Default,
        Self::Rust,
        Self::Node,
        Self::Python,
        Self::Blank,
    ];

    /// The files this template replaces or adds on top of the base.
    fn overlay(self) -> &'static [TemplateFile] {
        match self {
            Self::Default => &[],
            Self::Rust => RUST,
            Self::Node => NODE,
            Self::Python => PYTHON,
            Self::Blank => BLANK,
        }
    }
}

impl fmt::Display for TemplateName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One file in a resolved template.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TemplateFile {
    /// Relative to the project root, including the `.ai/` prefix, using `/` separators.
    pub(crate) path: &'static str,
    /// The literal template text, before substitution.
    pub(crate) text: &'static str,
    /// Whether this entry only creates a directory and writes nothing.
    pub(crate) dir_only: bool,
}

/// Resolves a template into the exact set of files `init` will consider.
///
/// Returned in path order so the plan a developer approves, the order of writes, and the order of
/// the report never depend on the order the `include_str!` constants happen to be declared in.
pub(crate) fn resolve(template: TemplateName) -> Vec<TemplateFile> {
    let mut files: Vec<TemplateFile> = Vec::new();

    // `blank` has no base. Everything else starts from the base tree, and every tree but `blank`
    // also carries the schema set: a project with no stack guidance still needs to know how to
    // validate its own documents.
    if template != TemplateName::Blank {
        for file in BASE {
            files.push(*file);
        }
        for file in SCHEMAS {
            files.push(*file);
        }
    }

    for overlay in template.overlay() {
        match files.iter_mut().find(|file| file.path == overlay.path) {
            Some(existing) => *existing = *overlay,
            None => files.push(*overlay),
        }
    }

    files.sort_by(|left, right| left.path.cmp(right.path));
    files.dedup_by(|left, right| left.path == right.path);
    files
}

const BASE: &[TemplateFile] = &[
    TemplateFile {
        path: ".ai/AI.md",
        text: include_str!("../../../../templates/base/AI.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/ARCHITECTURE.md",
        text: include_str!("../../../../templates/base/ARCHITECTURE.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/CONVENTIONS.md",
        text: include_str!("../../../../templates/base/CONVENTIONS.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/DESIGN.md",
        text: include_str!("../../../../templates/base/DESIGN.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/MEMORY.md",
        text: include_str!("../../../../templates/base/MEMORY.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/PRD.md",
        text: include_str!("../../../../templates/base/PRD.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/RULES.md",
        text: include_str!("../../../../templates/base/RULES.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/TASKS.md",
        text: include_str!("../../../../templates/base/TASKS.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/context/stack.md",
        text: include_str!("../../../../templates/base/context/stack.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/permissions/permissions.yaml",
        text: include_str!("../../../../templates/base/permissions/permissions.yaml"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/agents",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/bugs",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/changes",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/decisions",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/integrations",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/specs",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/tasks",
        text: "",
        dir_only: true,
    },
    TemplateFile {
        path: ".ai/workflows",
        text: "",
        dir_only: true,
    },
];

/// The schema set, embedded from `schemas/` at the repository root.
///
/// These are the same bytes `crates/aicontext-cli/tests/schemas.rs` validates against the Draft
/// 2020-12 meta-schema, so a project `init` scaffolds receives exactly the schemas CI proved good —
/// not a second copy that could drift from them. `doctor` compares the two trees (CTX-012).
///
/// Embedded rather than read at run time, like every other template: the binary has no asset
/// directory to ship and cannot be confused by a schema edited on disk after it was built.
const SCHEMAS: &[TemplateFile] = &[
    TemplateFile {
        path: ".ai/schemas/action-proposal.schema.json",
        text: include_str!("../../../../schemas/action-proposal.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/agent.schema.json",
        text: include_str!("../../../../schemas/agent.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/ai-config.schema.json",
        text: include_str!("../../../../schemas/ai-config.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/ai-entrypoint.schema.json",
        text: include_str!("../../../../schemas/ai-entrypoint.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/architecture.schema.json",
        text: include_str!("../../../../schemas/architecture.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/bug.schema.json",
        text: include_str!("../../../../schemas/bug.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/change.schema.json",
        text: include_str!("../../../../schemas/change.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/context-note.schema.json",
        text: include_str!("../../../../schemas/context-note.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/conventions.schema.json",
        text: include_str!("../../../../schemas/conventions.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/decision.schema.json",
        text: include_str!("../../../../schemas/decision.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/design.schema.json",
        text: include_str!("../../../../schemas/design.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/memory.schema.json",
        text: include_str!("../../../../schemas/memory.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/permission-policy.schema.json",
        text: include_str!("../../../../schemas/permission-policy.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/prd.schema.json",
        text: include_str!("../../../../schemas/prd.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/project-profile.schema.json",
        text: include_str!("../../../../schemas/project-profile.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/rules.schema.json",
        text: include_str!("../../../../schemas/rules.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/spec.schema.json",
        text: include_str!("../../../../schemas/spec.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/task.schema.json",
        text: include_str!("../../../../schemas/task.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/tasks.schema.json",
        text: include_str!("../../../../schemas/tasks.schema.json"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/schemas/workflow.schema.json",
        text: include_str!("../../../../schemas/workflow.schema.json"),
        dir_only: false,
    },
];

const RUST: &[TemplateFile] = &[
    TemplateFile {
        path: ".ai/ARCHITECTURE.md",
        text: include_str!("../../../../templates/rust/ARCHITECTURE.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/CONVENTIONS.md",
        text: include_str!("../../../../templates/rust/CONVENTIONS.md"),
        dir_only: false,
    },
];

const NODE: &[TemplateFile] = &[
    TemplateFile {
        path: ".ai/ARCHITECTURE.md",
        text: include_str!("../../../../templates/node/ARCHITECTURE.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/CONVENTIONS.md",
        text: include_str!("../../../../templates/node/CONVENTIONS.md"),
        dir_only: false,
    },
];

const PYTHON: &[TemplateFile] = &[
    TemplateFile {
        path: ".ai/ARCHITECTURE.md",
        text: include_str!("../../../../templates/python/ARCHITECTURE.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/CONVENTIONS.md",
        text: include_str!("../../../../templates/python/CONVENTIONS.md"),
        dir_only: false,
    },
];

const BLANK: &[TemplateFile] = &[
    TemplateFile {
        path: ".ai/AI.md",
        text: include_str!("../../../../templates/blank/AI.md"),
        dir_only: false,
    },
    TemplateFile {
        path: ".ai/RULES.md",
        text: include_str!("../../../../templates/blank/RULES.md"),
        dir_only: false,
    },
];

#[cfg(test)]
mod tests {
    use super::{BASE, SCHEMAS, TemplateName, resolve};
    use clap::ValueEnum;
    use std::path::Path;

    #[test]
    fn every_template_resolves_to_a_non_empty_ordered_tree() {
        for name in TemplateName::ALL {
            let files = resolve(*name);
            assert!(!files.is_empty(), "{name} resolved to nothing");
            let mut sorted = files.clone();
            sorted.sort_by(|left, right| left.path.cmp(right.path));
            assert_eq!(files, sorted, "{name} is not in path order");
            assert_eq!(
                files.len(),
                sorted.len(),
                "{name} contains a duplicate path"
            );
        }
    }

    #[test]
    fn a_stack_overlay_replaces_the_base_document_rather_than_adding_one() {
        let base = resolve(TemplateName::Default);
        let rust = resolve(TemplateName::Rust);
        assert_eq!(base.len(), rust.len(), "an overlay must not add files here");
        for file in &rust {
            let original = base
                .iter()
                .find(|other| other.path == file.path)
                .expect("an overlay must only replace a base file");
            if file.path.ends_with("CONVENTIONS.md") || file.path.ends_with("ARCHITECTURE.md") {
                assert_ne!(file.text, original.text, "{} was not replaced", file.path);
            }
        }
    }

    #[test]
    fn the_blank_template_is_two_documents_and_no_directories() {
        let files = resolve(TemplateName::Blank);
        let paths: Vec<&str> = files.iter().map(|file| file.path).collect();
        assert_eq!(paths, vec![".ai/AI.md", ".ai/RULES.md"]);
        assert!(files.iter().all(|file| !file.dir_only));
        assert!(!files.iter().any(|file| file.text.is_empty()));
    }

    #[test]
    fn the_default_template_carries_the_eight_base_documents() {
        let documents: Vec<&str> = resolve(TemplateName::Default)
            .iter()
            .filter(|file| {
                !file.dir_only
                    && Path::new(file.path)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            })
            .map(|file| file.path)
            .collect();
        for expected in [
            ".ai/AI.md",
            ".ai/PRD.md",
            ".ai/ARCHITECTURE.md",
            ".ai/RULES.md",
            ".ai/CONVENTIONS.md",
            ".ai/DESIGN.md",
            ".ai/TASKS.md",
            ".ai/MEMORY.md",
        ] {
            assert!(documents.contains(&expected), "{expected} is missing");
        }
        assert!(!BASE.is_empty());
    }

    #[test]
    fn every_template_file_is_inside_the_ai_directory() {
        for name in TemplateName::ALL {
            for file in resolve(*name) {
                assert!(
                    file.path.starts_with(".ai/"),
                    "{} writes outside .ai: {}",
                    name,
                    file.path
                );
                assert!(!file.path.contains('\\'), "use / separators: {}", file.path);
            }
        }
    }

    #[test]
    fn a_directory_marker_is_never_a_leaf_file() {
        for name in TemplateName::ALL {
            for file in resolve(*name) {
                if !file.dir_only {
                    assert!(!file.text.is_empty(), "{} is empty", file.path);
                }
            }
        }
    }

    #[test]
    fn the_schema_set_is_embedded_byte_for_byte() {
        // `init` copies the schemas from the binary while `tests/schemas.rs` validates the files in
        // `schemas/`. Those are the same contract only while the bytes are the same, so this
        // compares them directly rather than trusting that nobody edited one side.
        for file in SCHEMAS {
            let name = file.path.rsplit('/').next().expect("a file has a name");
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../schemas")
                .join(name);
            let on_disk = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", source.display()));
            assert_eq!(
                file.text, on_disk,
                "{name} differs from schemas/{name}; the copy `init` writes is the one CI validated \
                 only while they match"
            );
        }
    }

    #[test]
    fn every_but_the_blank_template_ships_the_schema_set() {
        for name in TemplateName::ALL {
            let schemas = resolve(*name)
                .iter()
                .filter(|file| file.path.starts_with(".ai/schemas/"))
                .count();
            if *name == TemplateName::Blank {
                assert_eq!(schemas, 0, "blank stays two documents and no tree");
            } else {
                assert_eq!(schemas, SCHEMAS.len(), "{name} is missing schemas");
            }
        }
    }

    #[test]
    fn the_template_names_on_the_command_line_match_the_help_text() {
        let parsed: Vec<String> = TemplateName::value_variants()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            parsed,
            vec!["default", "rust", "node", "python", "blank"],
            "the enum and the documented names disagree"
        );
        for name in TemplateName::ALL {
            assert!(parsed.contains(&name.as_str().to_string()));
        }
    }
}
