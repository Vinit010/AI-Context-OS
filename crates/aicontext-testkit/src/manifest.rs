//! Minimal reader for the subset of `Cargo.toml` this workspace uses.
//!
//! # Why not a TOML crate
//!
//! This parser exists so the workspace boundary check can run inside `cargo test` with **zero
//! dependencies**. `RULES.md` §6 requires a written justification for every direct dependency, and
//! the justification for a TOML parser is "we wrote the 120 lines ourselves, and it fails closed
//! instead of guessing". A dependency would be justified only if this needed to handle real-world
//! manifests, which it never will: it only ever reads files in this repository.
//!
//! The rejected alternative is shelling out to `cargo metadata`. It needs dependency resolution
//! (and therefore a warm registry), it starts a process, and it moves a policy check out of the
//! test suite and into a shell script that nobody runs.
//!
//! # Fail closed
//!
//! Anything this parser does not understand is an error, never a silent omission. In particular
//! A target-qualified dependency table is **rejected** rather than skipped, because skipping it
//! would hide exactly the kind of boundary violation the check exists to find.
//!
//! # Deliberate scope
//!
//! Top-level keys in a dependency section are crate names, so a sub-key such as `path` or
//! `features` appearing at the start of a line is malformed input rather than a nested table. Our
//! manifests are ours to keep simple.

use std::collections::BTreeSet;
use std::fmt;

/// A manifest could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The `package` table is absent or has no `name` key.
    MissingPackageName,
    /// A section this reader refuses to guess at, such as a target-specific dependency table.
    UnsupportedSection {
        /// The section header as written in the file.
        section: String,
        /// 1-based line number.
        line: usize,
    },
    /// A dependency or array entry that is not `key = value`.
    MalformedEntry {
        /// The section being read.
        section: String,
        /// 1-based line number.
        line: usize,
        /// The offending text, trimmed.
        text: String,
    },
}

impl ManifestError {
    /// Stable machine code, safe to branch on in tests and CI.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingPackageName => "WS-001",
            Self::UnsupportedSection { .. } => "WS-002",
            Self::MalformedEntry { .. } => "WS-003",
        }
    }

    /// What the reader should do about it.
    #[must_use]
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::MissingPackageName => {
                "add a `name` key to the crate's `package` table; the boundary check needs it"
            }
            Self::UnsupportedSection { .. } => {
                "move platform-specific dependencies into the shared table, or teach \
                 aicontext-testkit to read that section deliberately"
            }
            Self::MalformedEntry { .. } => {
                "keep every dependency on one line: `name = \"version\"` or `name = { path = \"..\" }`"
            }
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPackageName => write!(f, "{}: [package] table has no `name`", self.code()),
            Self::UnsupportedSection { section, line } => {
                write!(
                    f,
                    "{}: line {line}: unsupported section [{section}]",
                    self.code()
                )
            }
            Self::MalformedEntry {
                section,
                line,
                text,
            } => {
                write!(
                    f,
                    "{}: line {line}: malformed entry in [{section}]: `{text}`",
                    self.code()
                )
            }
        }
    }
}

impl std::error::Error for ManifestError {}

/// The dependency tables this reader understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Package,
    Workspace,
    WorkspaceDependencies,
    Dependencies(Kind),
    /// A section with no dependency keys, such as a `profile` table.
    Ignored,
}

/// The parts of a `Cargo.toml` the workspace policy check needs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Manifest {
    name: Option<String>,
    workspace_members: Vec<String>,
    dependencies: BTreeSet<String>,
    dev_dependencies: BTreeSet<String>,
    build_dependencies: BTreeSet<String>,
    workspace_dependencies: BTreeSet<String>,
}

impl Manifest {
    /// Read a manifest.
    ///
    /// A virtual workspace manifest, which has no package table, parses successfully. Use
    /// [`Manifest::require_name`] when a name is required.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError`] for a section this reader refuses to guess at, or a malformed
    /// entry. See [`ManifestError::remediation`].
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let mut manifest = Self::default();
        let mut section = Section::Ignored;
        let mut reading_members = false;
        let mut open_braces: usize = 0;

        for (index, raw) in source.lines().enumerate() {
            let line_number = index + 1;
            let stripped = strip_comment(raw);
            // Leading whitespace is only meaningful for detecting continuation lines, so it is
            // tested before trimming.
            let indented = stripped.starts_with(char::is_whitespace);
            let line = stripped.trim();

            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') {
                if line.starts_with("[[") {
                    // Array of tables, such as a binary target table. Never holds dependencies.
                    section = Section::Ignored;
                    continue;
                }
                let header = line
                    .strip_prefix('[')
                    .and_then(|rest| rest.strip_suffix(']'))
                    .ok_or(ManifestError::MalformedEntry {
                        section: "unknown".to_owned(),
                        line: line_number,
                        text: line.to_owned(),
                    })?
                    .trim();
                section = classify(header, line_number)?;
                continue;
            }

            // Inside a multi-line inline table every line belongs to the value on the opening line.
            if open_braces > 0 {
                open_braces = open_braces.saturating_sub(closing_braces(line));
                continue;
            }

            // Indented lines continue a multi-line array and are not crate names.
            if indented {
                if reading_members {
                    reading_members =
                        collect_members(line, &mut manifest.workspace_members) && reading_members;
                }
                continue;
            }

            if reading_members {
                if !collect_members(line, &mut manifest.workspace_members) {
                    reading_members = false;
                }
                continue;
            }

            let (key, value) = split_entry(line).ok_or_else(|| ManifestError::MalformedEntry {
                section: section_name(section).to_owned(),
                line: line_number,
                text: line.to_owned(),
            })?;

            open_braces = brace_balance(value);

            match section {
                Section::Package if key == "name" => {
                    manifest.name = Some(scalar(value).to_owned());
                }
                Section::Dependencies(Kind::Normal) => {
                    insert(&mut manifest.dependencies, key);
                }
                Section::Dependencies(Kind::Dev) => {
                    insert(&mut manifest.dev_dependencies, key);
                }
                Section::Dependencies(Kind::Build) => {
                    insert(&mut manifest.build_dependencies, key);
                }
                Section::WorkspaceDependencies => {
                    insert(&mut manifest.workspace_dependencies, key);
                }
                Section::Workspace if key == "members" => {
                    reading_members = collect_members(value, &mut manifest.workspace_members);
                }
                _ => {}
            }
        }

        Ok(manifest)
    }

    /// The `package` table's `name` key, if this manifest declares one. A virtual manifest has none.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// The `package` table's `name` key.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::MissingPackageName`] for a virtual manifest, which has no package
    /// to name.
    pub fn require_name(&self) -> Result<&str, ManifestError> {
        self.name().ok_or(ManifestError::MissingPackageName)
    }

    /// Paths listed as workspace.members, in declaration order.
    #[must_use]
    pub fn workspace_members(&self) -> &[String] {
        &self.workspace_members
    }

    /// Crates in the dependencies table.
    #[must_use]
    pub fn dependencies(&self) -> &BTreeSet<String> {
        &self.dependencies
    }

    /// Crates in the dev-dependencies table.
    #[must_use]
    pub fn dev_dependencies(&self) -> &BTreeSet<String> {
        &self.dev_dependencies
    }

    /// Crates in the uild-dependencies table.
    #[must_use]
    pub fn build_dependencies(&self) -> &BTreeSet<String> {
        &self.build_dependencies
    }

    /// Crates in the workspace.dependencies table.
    #[must_use]
    pub fn workspace_dependencies(&self) -> &BTreeSet<String> {
        &self.workspace_dependencies
    }

    /// Declared dependencies, dev-dependencies, and build-dependencies that name a known
    /// workspace crate, sorted for deterministic error messages.
    #[must_use]
    pub fn internal_dependencies(&self, known: &BTreeSet<String>) -> BTreeSet<String> {
        self.dependencies
            .iter()
            .chain(self.dev_dependencies.iter())
            .chain(self.build_dependencies.iter())
            .filter(|name| known.contains(*name))
            .cloned()
            .collect()
    }
}

fn classify(header: &str, line: usize) -> Result<Section, ManifestError> {
    let unquoted = header.trim_matches('"');
    Ok(match unquoted {
        "package" => Section::Package,
        "workspace" => Section::Workspace,
        "workspace.dependencies" => Section::WorkspaceDependencies,
        "dependencies" => Section::Dependencies(Kind::Normal),
        "dev-dependencies" => Section::Dependencies(Kind::Dev),
        "build-dependencies" => Section::Dependencies(Kind::Build),
        // A target-qualified dependency table is rejected rather than ignored: silently skipping
        // it would let a boundary violation hide in a target-qualified dependency table.
        other if other.ends_with("dependencies") => {
            return Err(ManifestError::UnsupportedSection {
                section: other.to_owned(),
                line,
            });
        }
        _ => Section::Ignored,
    })
}

fn insert(set: &mut BTreeSet<String>, key: &str) {
    set.insert(key.trim_matches('"').to_owned());
}

fn split_entry(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim().trim_matches('"');
    (!key.is_empty()).then_some((key, value.trim()))
}

/// Strips surrounding whitespace and one layer of quotes from a scalar value.
fn scalar(value: &str) -> &str {
    value.trim().trim_matches('"')
}

/// Net `{` minus `}` in a value. Positive means an inline table is still open.
fn brace_balance(value: &str) -> usize {
    let opened = value.matches('{').count();
    let closed = value.matches('}').count();
    opened.saturating_sub(closed)
}

/// The number of `}` in a line, used to close a multi-line inline table. Counting only closers
/// means a lone `}` still ends the table; `brace_balance` alone would not.
fn closing_braces(line: &str) -> usize {
    line.matches('}').count()
}

fn section_name(section: Section) -> &'static str {
    match section {
        Section::Package => "package",
        Section::Workspace => "workspace",
        Section::WorkspaceDependencies => "workspace.dependencies",
        Section::Dependencies(Kind::Normal) => "dependencies",
        Section::Dependencies(Kind::Dev) => "dev-dependencies",
        Section::Dependencies(Kind::Build) => "build-dependencies",
        Section::Ignored => "ignored",
    }
}

/// Collects quoted strings from one line of a members array.
/// Returns `true` while the array is still open.
fn collect_members(line: &str, out: &mut Vec<String>) -> bool {
    for token in quoted_strings(line) {
        out.push(token);
    }
    !line.contains(']')
}

fn quoted_strings(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        found.push(after[..end].to_owned());
        rest = &after[end + 1..];
    }
    found
}

/// Removes a trailing `#` comment, ignoring `#` inside quoted strings.
fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (index, character) in line.char_indices() {
        match character {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..index],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::{Manifest, ManifestError, strip_comment};

    const SAMPLE: &str = r#"
[package]
name = "aicontext-core"
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
regex = "1"
"quoted-name" = "1"

[dev-dependencies]
aicontext-testkit = { path = "../aicontext-testkit" }

[build-dependencies]
cc = "1"

[profile.release]
lto = "thin"
"#;

    #[test]
    fn reads_name_and_every_dependency_table() {
        let manifest = Manifest::parse(SAMPLE).expect("valid manifest");
        assert_eq!(manifest.require_name().expect("named"), "aicontext-core");
        assert_eq!(
            manifest.dependencies().iter().collect::<Vec<_>>(),
            vec!["quoted-name", "regex", "serde"]
        );
        assert_eq!(
            manifest.dev_dependencies().iter().collect::<Vec<_>>(),
            vec!["aicontext-testkit"]
        );
        assert_eq!(
            manifest.build_dependencies().iter().collect::<Vec<_>>(),
            vec!["cc"]
        );
    }

    #[test]
    fn ignores_non_dependency_sections() {
        let manifest = Manifest::parse(SAMPLE).expect("valid manifest");
        assert!(!manifest.dependencies().contains("lto"));
        assert!(!manifest.dependencies().contains("version"));
    }

    #[test]
    fn reports_missing_package_name() {
        let manifest = Manifest::parse("[dependencies]\nserde = \"1\"\n").expect("valid manifest");
        assert_eq!(manifest.name(), None);
        let error = manifest.require_name().expect_err("no package name");
        assert_eq!(error, ManifestError::MissingPackageName);
        assert_eq!(error.code(), "WS-001");
    }

    #[test]
    fn rejects_target_specific_dependency_tables() {
        let source = "[package]\nname = \"x\"\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n";
        let error = Manifest::parse(source).expect_err("must not be silently ignored");
        assert!(
            matches!(error, ManifestError::UnsupportedSection { .. }),
            "expected UnsupportedSection, got {error:?}"
        );
        assert_eq!(error.code(), "WS-002");
    }

    #[test]
    fn rejects_malformed_dependency_entry() {
        let source = "[package]\nname = \"x\"\n[dependencies]\nserde\n";
        let error = Manifest::parse(source).expect_err("malformed");
        assert!(
            matches!(error, ManifestError::MalformedEntry { .. }),
            "expected MalformedEntry, got {error:?}"
        );
    }

    #[test]
    fn keeps_comments_out_of_the_result() {
        let source =
            "# leading\n[package]\nname = \"x\" # trailing\n[dependencies]\nserde = \"1\" # v\n";
        let manifest = Manifest::parse(source).expect("valid manifest");
        assert_eq!(manifest.require_name().expect("named"), "x");
        assert!(manifest.dependencies().contains("serde"));
    }

    #[test]
    fn does_not_strip_a_hash_inside_quotes() {
        let line = strip_comment(r#"name = "a#b" # real comment"#);
        assert_eq!(line.trim(), r#"name = "a#b""#);
    }

    #[test]
    fn reads_workspace_members_declared_across_lines() {
        let source = "[workspace]\nmembers = [\n  \"crates/one\",\n  \"crates/two\",\n]\n";
        let manifest = Manifest::parse(source).expect("valid manifest");
        assert_eq!(manifest.workspace_members(), ["crates/one", "crates/two"]);
    }

    #[test]
    fn reads_workspace_members_declared_on_one_line() {
        let source = "[workspace]\nmembers = [\"crates/one\", \"crates/two\"]\n";
        let manifest = Manifest::parse(source).expect("valid manifest");
        assert_eq!(manifest.workspace_members(), ["crates/one", "crates/two"]);
    }

    #[test]
    fn internal_dependencies_are_filtered_and_sorted() {
        let known = ["aicontext-core", "aicontext-testkit"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let manifest = Manifest::parse(SAMPLE).expect("valid manifest");
        assert_eq!(
            manifest
                .internal_dependencies(&known)
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["aicontext-testkit"]
        );
    }

    #[test]
    fn skips_indented_continuation_lines() {
        let source = "[package]\nname = \"x\"\n[dependencies]\nserde = {\n  version = \"1\",\n  path = \"y\"\n}\n";
        let manifest = Manifest::parse(source).expect("valid manifest");
        assert_eq!(
            manifest.dependencies().iter().collect::<Vec<_>>(),
            vec!["serde"]
        );
    }

    #[test]
    fn keeps_reading_after_a_multiline_inline_table_closes() {
        // A closing `}` must end the inline table, not leave the reader inside it. If it does, the
        // dependencies after it are silently dropped and a boundary violation can hide there.
        let source = concat!(
            "[package]\n",
            "name = \"x\"\n",
            "[dependencies]\n",
            "before = \"1\"\n",
            "serde = {\n",
            "  version = \"1\",\n",
            "  path = \"y\"\n",
            "}\n",
            "after = \"1\"\n",
        );
        let manifest = Manifest::parse(source).expect("valid manifest");
        assert_eq!(
            manifest.dependencies().iter().collect::<Vec<_>>(),
            vec!["after", "before", "serde"]
        );
    }

    #[test]
    fn reads_a_second_table_after_a_multiline_inline_table() {
        // Same failure mode, seen through a section boundary rather than a sibling entry.
        let source = concat!(
            "[package]\n",
            "name = \"x\"\n",
            "[dependencies]\n",
            "serde = {\n",
            "  version = \"1\"\n",
            "}\n",
            "[dev-dependencies]\n",
            "aicontext-testkit = { path = \"../aicontext-testkit\" }\n",
        );
        let manifest = Manifest::parse(source).expect("valid manifest");
        assert_eq!(
            manifest.dev_dependencies().iter().collect::<Vec<_>>(),
            vec!["aicontext-testkit"]
        );
    }
}
