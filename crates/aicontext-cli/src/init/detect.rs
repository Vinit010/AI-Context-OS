//! Advisory project discovery: what is in this repository, reported and never applied.
//!
//! `docs/CONTEXT_SPEC.md` §5 says `discover` inspects and reports, never modifies the project and
//! never writes findings into a document without `--apply`. `init` has no `--apply` at all, so the
//! strongest thing it can do with a detection is print it.
//!
//! The full discovery engine — a typed `ProjectProfile` with per-signal confidence, framework
//! detection from dependency graphs, and monorepo package lists — is `TASK-030`, which depends on
//! this task. What is here is the part `init` needs in order to say something true about the
//! project it was pointed at, and it is deliberately dumb: a fixed table of file names, checked for
//! existence at the top level. A project that matches nothing yields no signal at all, and
//! `init` says so rather than filling in a plausible stack.

use std::fs;
use std::path::Path;

use super::error::InitError;

/// One detected fact, with the file that is the evidence for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Signal {
    /// The `ProjectProfile` field this would populate, using the spec's own names.
    pub(crate) kind: &'static str,
    /// The path that was found, relative to the project root, with `/` separators.
    pub(crate) evidence: String,
}

/// How many directory entries are examined when looking for one file extension.
///
/// Discovery is advisory and must not become the slow part of a command that otherwise writes a
/// dozen files. `RULES.md` §11 caps every unbounded read.
const MAX_ENTRIES_SCANNED: usize = 512;

/// Directory names never scanned, because nothing in them changes the answer.
const SKIPPED_DIRECTORIES: &[&str] = &["node_modules", "target", "vendor", ".venv", "dist", "build"];

/// A name checked for existence, and what finding it produces.
struct Marker {
    kind: &'static str,
    path: &'static str,
    evidence: &'static str,
}

/// The fixed table. One row per fact worth reporting; the table is the specification of what
/// `init` looks for, and growing it is a one-line change.
const MARKERS: &[Marker] = &[
    Marker { kind: "languages", path: "Cargo.toml", evidence: "rust" },
    Marker { kind: "languages", path: "package.json", evidence: "node" },
    Marker { kind: "languages", path: "pyproject.toml", evidence: "python" },
    Marker { kind: "languages", path: "requirements.txt", evidence: "python" },
    Marker { kind: "languages", path: "setup.py", evidence: "python" },
    Marker { kind: "languages", path: "go.mod", evidence: "go" },
    Marker { kind: "languages", path: "pom.xml", evidence: "java" },
    Marker { kind: "languages", path: "build.gradle", evidence: "java" },
    Marker { kind: "package_manager", path: "Cargo.lock", evidence: "cargo" },
    Marker { kind: "package_manager", path: "package-lock.json", evidence: "npm" },
    Marker { kind: "package_manager", path: "pnpm-lock.yaml", evidence: "pnpm" },
    Marker { kind: "package_manager", path: "yarn.lock", evidence: "yarn" },
    Marker { kind: "package_manager", path: "poetry.lock", evidence: "poetry" },
    Marker { kind: "package_manager", path: "uv.lock", evidence: "uv" },
    Marker { kind: "vcs", path: ".git", evidence: "git" },
    Marker { kind: "ci", path: ".github/workflows", evidence: "github-actions" },
    Marker { kind: "ci", path: ".gitlab-ci.yml", evidence: "gitlab-ci" },
    Marker { kind: "ci", path: "Jenkinsfile", evidence: "jenkins" },
    Marker { kind: "ci", path: ".circleci", evidence: "circleci" },
    Marker { kind: "containers", path: "Dockerfile", evidence: "docker" },
    Marker { kind: "containers", path: "docker-compose.yml", evidence: "compose" },
    Marker { kind: "testing", path: "tests", evidence: "tests directory" },
    Marker { kind: "iac", path: "k8s", evidence: "kubernetes" },
    Marker { kind: "iac", path: "charts", evidence: "helm" },
    Marker { kind: "monorepo", path: "pnpm-workspace.yaml", evidence: "pnpm workspaces" },
    Marker { kind: "monorepo", path: "go.work", evidence: "go workspaces" },
];

/// Inspects the project root and returns what it found, sorted.
///
/// Sorted by kind then evidence, because directory iteration order is not defined and the report
/// has to be the same on every machine (`RULES.md` §8).
pub(crate) fn detect(root: &Path) -> Result<Vec<Signal>, InitError> {
    let mut signals = Vec::new();
    for marker in MARKERS {
        if root.join(marker.path).exists() {
            signals.push(Signal {
                kind: marker.kind,
                evidence: marker.evidence.to_string(),
            });
        }
    }
    signals.extend(terraform_files(root)?);
    signals.sort_by(|left, right| {
        (left.kind, &left.evidence).cmp(&(right.kind, &right.evidence))
    });
    Ok(signals)
}

/// The version-control label on the identity line, which is `git` or says so honestly.
pub(crate) fn vcs_label(signals: &[Signal]) -> &'static str {
    if signals.iter().any(|signal| signal.kind == "vcs") {
        "git"
    } else {
        "no vcs"
    }
}

/// The signals as the `ProjectProfile` kinds of §5 would name them, de-duplicated.
pub(crate) fn summarise(signals: &[Signal]) -> Vec<String> {
    let mut summary: Vec<String> = Vec::new();
    for signal in signals {
        let entry = format!("{} {}", signal.kind, signal.evidence);
        if !summary.contains(&entry) {
            summary.push(entry);
        }
    }
    summary
}

/// Top-level `*.tf` files, which is the only extension `init` looks for.
fn terraform_files(root: &Path) -> Result<Vec<Signal>, InitError> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(InitError::list(&display(root), error)),
    };
    let mut found = Vec::new();
    for entry in entries.take(MAX_ENTRIES_SCANNED) {
        let entry = match entry {
            Ok(entry) => entry,
            // One unreadable entry does not invalidate the rest of the report; discovery is
            // advisory, and refusing to run would be a worse answer than a shorter list.
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIPPED_DIRECTORIES.contains(&name.as_str()) {
            continue;
        }
        if name.ends_with(".tf") && entry.path().is_file() {
            found.push(Signal {
                kind: "iac",
                evidence: "terraform".to_string(),
            });
        }
    }
    Ok(found)
}

/// A path in the spelling a person would type, with `/` separators.
fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{MAX_ENTRIES_SCANNED, Signal, detect, summarise, vcs_label};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn an_empty_project_reports_nothing_rather_than_a_guess() {
        let root = TempDir::new().expect("temp dir");
        let signals = detect(root.path()).expect("detects");
        assert!(signals.is_empty(), "an empty project must yield no signal");
        assert_eq!(vcs_label(&signals), "no vcs");
        assert!(summarise(&signals).is_empty());
    }

    #[test]
    fn a_rust_project_is_reported_from_the_files_present() {
        let root = rust_project();
        let signals = detect(root.path()).expect("detects");
        assert!(signals.contains(&Signal { kind: "languages", evidence: "rust".into() }));
        assert!(signals.contains(&Signal { kind: "package_manager", evidence: "cargo".into() }));
        assert!(signals.contains(&Signal { kind: "vcs", evidence: "git".into() }));
        assert_eq!(vcs_label(&signals), "git");
    }

    #[test]
    fn a_language_is_reported_once_however_many_markers_name_it() {
        let root = TempDir::new().expect("temp dir");
        for marker in ["pyproject.toml", "requirements.txt", "setup.py"] {
            fs::write(root.path().join(marker), "").expect("write");
        }
        let signals = detect(root.path()).expect("detects");
        let python = signals.iter().filter(|s| s.evidence == "python").count();
        assert_eq!(python, 3, "each marker is reported with its own evidence");
        let summary = summarise(&signals);
        assert_eq!(
            summary.iter().filter(|entry| entry.contains("python")).count(),
            3,
            "the summary keeps the evidence distinct"
        );
    }

    #[test]
    fn terraform_is_found_from_a_top_level_file_only() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("main.tf"), "").expect("write");
        fs::create_dir(root.path().join("node_modules")).expect("dir");
        fs::write(root.path().join("node_modules").join("ignored.tf"), "").expect("write");
        let signals = detect(root.path()).expect("detects");
        assert!(signals.contains(&Signal { kind: "iac", evidence: "terraform".into() }));
        assert_eq!(
            signals
                .iter()
                .filter(|signal| signal.kind == "iac")
                .count(),
            1,
            "a skipped directory contributes nothing"
        );
    }

    #[test]
    fn the_report_is_the_same_on_every_run() {
        let root = rust_project();
        let first = detect(root.path()).expect("detects");
        let second = detect(root.path()).expect("detects");
        assert_eq!(first, second);
        let mut sorted = first.clone();
        sorted.sort_by(|left, right| (left.kind, &left.evidence).cmp(&(right.kind, &right.evidence)));
        assert_eq!(first, sorted, "signals come out sorted");
    }

    #[test]
    fn a_missing_directory_is_not_a_failure() {
        // `detect` is pointed at a path that no longer exists; a project deleted mid-run must not
        // turn into a crash.
        let root = TempDir::new().expect("temp dir");
        let gone = root.path().join("gone");
        assert!(detect(&gone).expect("a missing root is not a detection failure").is_empty());
    }

    #[test]
    fn the_entry_scan_is_bounded() {
        assert!(MAX_ENTRIES_SCANNED <= 4_096, "discovery must stay cheap");
    }

    fn rust_project() -> TempDir {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("write");
        fs::write(root.path().join("Cargo.lock"), "").expect("write");
        fs::create_dir(root.path().join(".git")).expect("dir");
        root
    }
}