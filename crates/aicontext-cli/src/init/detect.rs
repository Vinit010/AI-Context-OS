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
///
/// The value is what `docs/CONTEXT_SPEC.md` §5 would put in a `ProjectProfile`, and the evidence is
/// the path that implied it. Both are reported, because "languages: rust" without "Cargo.toml" is a
/// claim, and "Cargo.toml" without "rust" is trivia.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Signal {
    /// The `ProjectProfile` field this would populate, using the spec's own names.
    pub(crate) kind: &'static str,
    /// The value, for example `rust`.
    pub(crate) value: String,
    /// The path that implies it, relative to the project root, with `/` separators.
    pub(crate) evidence: String,
}

/// How many directory entries are examined when looking for one file extension.
///
/// Discovery is advisory and must not become the slow part of a command that otherwise writes a
/// dozen files. `RULES.md` §11 caps every unbounded read.
const MAX_ENTRIES_SCANNED: usize = 512;

/// Directory names never scanned, because nothing in them changes the answer.
const SKIPPED_DIRECTORIES: &[&str] =
    &["node_modules", "target", "vendor", ".venv", "dist", "build"];

/// A name checked for existence, and what finding it produces.
struct Marker {
    kind: &'static str,
    /// Checked at the top level of the project root.
    path: &'static str,
    /// The value implied by finding it.
    value: &'static str,
}

/// The fixed table. One row per fact worth reporting; the table is the specification of what
/// `init` looks for, and growing it is a one-line change.
const MARKERS: &[Marker] = &[
    Marker {
        kind: "languages",
        path: "Cargo.toml",
        value: "rust",
    },
    Marker {
        kind: "languages",
        path: "package.json",
        value: "node",
    },
    Marker {
        kind: "languages",
        path: "pyproject.toml",
        value: "python",
    },
    Marker {
        kind: "languages",
        path: "requirements.txt",
        value: "python",
    },
    Marker {
        kind: "languages",
        path: "setup.py",
        value: "python",
    },
    Marker {
        kind: "languages",
        path: "go.mod",
        value: "go",
    },
    Marker {
        kind: "languages",
        path: "pom.xml",
        value: "java",
    },
    Marker {
        kind: "languages",
        path: "build.gradle",
        value: "java",
    },
    Marker {
        kind: "package_manager",
        path: "Cargo.lock",
        value: "cargo",
    },
    Marker {
        kind: "package_manager",
        path: "package-lock.json",
        value: "npm",
    },
    Marker {
        kind: "package_manager",
        path: "pnpm-lock.yaml",
        value: "pnpm",
    },
    Marker {
        kind: "package_manager",
        path: "yarn.lock",
        value: "yarn",
    },
    Marker {
        kind: "package_manager",
        path: "poetry.lock",
        value: "poetry",
    },
    Marker {
        kind: "package_manager",
        path: "uv.lock",
        value: "uv",
    },
    Marker {
        kind: "vcs",
        path: ".git",
        value: "git",
    },
    Marker {
        kind: "ci",
        path: ".github/workflows",
        value: "github-actions",
    },
    Marker {
        kind: "ci",
        path: ".gitlab-ci.yml",
        value: "gitlab-ci",
    },
    Marker {
        kind: "ci",
        path: "Jenkinsfile",
        value: "jenkins",
    },
    Marker {
        kind: "ci",
        path: ".circleci",
        value: "circleci",
    },
    Marker {
        kind: "containers",
        path: "Dockerfile",
        value: "docker",
    },
    Marker {
        kind: "containers",
        path: "docker-compose.yml",
        value: "compose",
    },
    Marker {
        kind: "testing",
        path: "tests",
        value: "tests directory",
    },
    Marker {
        kind: "iac",
        path: "k8s",
        value: "kubernetes",
    },
    Marker {
        kind: "iac",
        path: "charts",
        value: "helm",
    },
    Marker {
        kind: "monorepo",
        path: "pnpm-workspace.yaml",
        value: "pnpm workspaces",
    },
    Marker {
        kind: "monorepo",
        path: "go.work",
        value: "go workspaces",
    },
];

/// Inspects the project root and returns what it found, sorted.
///
/// Sorted by kind, then value, then evidence, because directory iteration order is not defined and
/// the report has to be the same on every machine (`RULES.md` §8).
pub(crate) fn detect(root: &Path) -> Result<Vec<Signal>, InitError> {
    let mut signals = Vec::new();
    for marker in MARKERS {
        if root.join(marker.path).exists() {
            signals.push(Signal {
                kind: marker.kind,
                value: marker.value.to_string(),
                evidence: marker.path.to_string(),
            });
        }
    }
    signals.extend(terraform_files(root)?);
    signals.sort_by(|left, right| {
        (&left.kind, &left.value, &left.evidence).cmp(&(&right.kind, &right.value, &right.evidence))
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

/// The distinct facts, in the `ProjectProfile` spelling of §5: one entry per kind and value, with no
/// repetition when several files imply the same fact.
pub(crate) fn summarise(signals: &[Signal]) -> Vec<String> {
    let mut summary: Vec<String> = Vec::new();
    for signal in signals {
        let entry = format!("{} {}", signal.kind, signal.value);
        if !summary.contains(&entry) {
            summary.push(entry);
        }
    }
    summary
}

/// The facts with the file that implied each one, for the human report.
///
/// The JSON payload uses [`summarise`] instead: a script wants the value, a reader wants the reason.
/// One signal per file, so two files implying `python` produce two lines rather than one merged
/// claim that hides where it came from.
pub(crate) fn describe(signals: &[Signal]) -> Vec<String> {
    signals
        .iter()
        .map(|signal| format!("{} {} ({})", signal.kind, signal.value, signal.evidence))
        .collect()
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
        // One unreadable entry does not invalidate the rest of the report; discovery is advisory, and
        // refusing to run would be a worse answer than a shorter list.
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIPPED_DIRECTORIES.contains(&name.as_str()) {
            continue;
        }
        // Case-insensitive because a checkout on Windows can hold `Main.TF`, and a discovery that
        // misses it on one platform and finds it on another is worse than one that misses it.
        let is_terraform = Path::new(&name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tf"));
        if is_terraform && entry.path().is_file() {
            found.push(Signal {
                kind: "iac",
                value: "terraform".to_string(),
                evidence: name,
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
    use super::{MAX_ENTRIES_SCANNED, detect, summarise, vcs_label};
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
        assert!(
            signals
                .iter()
                .any(|s| s.kind == "languages" && s.value == "rust" && s.evidence == "Cargo.toml")
        );
        assert!(
            signals
                .iter()
                .any(|s| s.kind == "vcs" && s.evidence == ".git")
        );
        assert_eq!(vcs_label(&signals), "git");
    }

    #[test]
    fn a_language_is_reported_once_however_many_markers_name_it() {
        let root = TempDir::new().expect("temp dir");
        for marker in ["pyproject.toml", "requirements.txt", "setup.py"] {
            fs::write(root.path().join(marker), "").expect("write");
        }
        let signals = detect(root.path()).expect("detects");

        let python = signals
            .iter()
            .filter(|signal| signal.value == "python")
            .collect::<Vec<_>>();
        assert_eq!(
            python.len(),
            3,
            "each marker is recorded with its own evidence"
        );
        let evidence: Vec<&str> = python
            .iter()
            .map(|signal| signal.evidence.as_str())
            .collect();
        assert_eq!(
            evidence,
            vec!["pyproject.toml", "requirements.txt", "setup.py"]
        );

        let summary = summarise(&signals);
        assert_eq!(
            summary
                .iter()
                .filter(|entry| entry.contains("python"))
                .count(),
            1,
            "the summary states the fact once: {summary:?}"
        );
        assert!(summary.contains(&"languages python".to_string()));
    }

    #[test]
    fn terraform_is_found_from_a_top_level_file_only() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("main.tf"), "").expect("write");
        fs::create_dir(root.path().join("node_modules")).expect("dir");
        fs::write(root.path().join("node_modules").join("ignored.tf"), "").expect("write");
        let signals = detect(root.path()).expect("detects");
        assert!(
            signals
                .iter()
                .any(|s| s.kind == "iac" && s.value == "terraform" && s.evidence == "main.tf")
        );
        assert_eq!(
            signals.iter().filter(|signal| signal.kind == "iac").count(),
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
        sorted.sort_by(|left, right| {
            (&left.kind, &left.value, &left.evidence).cmp(&(
                &right.kind,
                &right.value,
                &right.evidence,
            ))
        });
        assert_eq!(first, sorted, "signals come out sorted");
    }

    #[test]
    fn a_missing_directory_is_not_a_failure() {
        // `detect` is pointed at a path that no longer exists; a project deleted mid-run must not
        // turn into a crash.
        let root = TempDir::new().expect("temp dir");
        let gone = root.path().join("gone");
        assert!(
            detect(&gone)
                .expect("a missing root is not a detection failure")
                .is_empty()
        );
    }

    #[test]
    fn the_entry_scan_is_bounded() {
        const { assert!(MAX_ENTRIES_SCANNED <= 4_096, "discovery must stay cheap") };
    }

    fn rust_project() -> TempDir {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("write");
        fs::write(root.path().join("Cargo.lock"), "").expect("write");
        fs::create_dir(root.path().join(".git")).expect("dir");
        root
    }
}
