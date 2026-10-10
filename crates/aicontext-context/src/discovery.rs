//! Advisory project discovery: what this repository is, reported and never applied.
//!
//! `docs/CONTEXT_SPEC.md` §5 defines discovery: it inspects and reports, and it **never modifies**
//! the project and **never writes** a finding into a document. This module is the typed engine behind
//! that contract, and `TASK-030` is the task that builds it. `init` carried a deliberately dumb
//! subset of this (a fixed table of file names) so it could say something true about the project it
//! was pointed at; the difference here is that discovery produces a typed [`ProjectProfile`], infers
//! frameworks from manifest dependencies as well as from file names, and records the evidence and a
//! confidence behind every field.
//!
//! # Never a guess
//!
//! A project that matches nothing yields a profile whose [`ProjectProfile::unrecognised`] is `true`
//! and whose fields are all empty. That is the honest answer: a confidently wrong stack is worse
//! than an absent one, so nothing is inferred from a directory name alone and `init` reports
//! "unrecognised" rather than filling in something plausible.
//!
//! # Determinism
//!
//! Every list is sorted and nothing depends on directory iteration order (`RULES.md` §8). Two runs
//! over an unchanged tree produce an equal profile.
//!
//! # Reading is bounded
//!
//! The scan is a fixed table of names plus at most [`MAX_ENTRIES_SCANNED`] top-level entries, and a
//! manifest larger than [`MAX_MANIFEST_BYTES`] is not read at all (`RULES.md` §11). Discovery is
//! advisory and must never become the slow part of a command.
//!
//! # Infallible on purpose
//!
//! [`discover`] never fails. A root that cannot be listed is treated as a root with nothing in it,
//! because discovery is a courtesy and a command must not abort over it: `status` and `doctor` both
//! need *some* profile to describe what they saw, and "I could not look" and "I saw nothing" share
//! the same honest output — an empty, unrecognised profile. A caller that must distinguish the two
//! can ask the filesystem directly; this is the same choice `register::read` makes for a missing
//! `TASKS.md` (`MEM-016`).

use std::fs;
use std::path::Path;

/// How many top-level entries are examined when scanning for a file extension.
///
/// Discovery is advisory and must not become the slow part of a command that otherwise writes a
/// dozen files. `RULES.md` §11 caps every unbounded read.
const MAX_ENTRIES_SCANNED: usize = 512;

/// The largest manifest read in full, for framework inference.
///
/// A manifest past this size is skipped rather than truncated, because half a dependency list would
/// produce half a framework list that looks complete. `RULES.md` §11.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// The `ProjectProfile` field name each signal populates, in the spec's own spelling
/// (`docs/CONTEXT_SPEC.md` §5). A `&'static str` rather than an enum because the value is published
/// as a string in `schemas/project-profile.schema.json`.
const KIND_LANGUAGES: &str = "languages";
const KIND_FRAMEWORKS: &str = "frameworks";
const KIND_PACKAGE_MANAGER: &str = "package_manager";
const KIND_VCS: &str = "vcs";
const KIND_TESTING: &str = "testing";
const KIND_CI: &str = "ci";
const KIND_CONTAINERS: &str = "containers";
const KIND_IAC: &str = "iac";
const KIND_CLOUD_HINTS: &str = "cloud_hints";

/// How much a single signal is trusted.
///
/// The profile-level confidence is the *weakest* of its signals, so one doubtful signal cannot be
/// averaged away by several certain ones (`schemas/project-profile.schema.json`). A manifest names a
/// language directly and a lockfile names a package manager directly, so both are `High`; the mere
/// presence of a `tests` directory is a weaker claim and is `Medium`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Confidence {
    /// A guess a plain reader could dispute, for example a tests directory with no test runner.
    Low,
    /// A reasonable inference, for example a directory named `tests`.
    Medium,
    /// A direct statement, for example `Cargo.toml` for Rust or `Cargo.lock` for Cargo.
    High,
}

impl Confidence {
    /// The published spelling, matching the schema's `enum`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// One detected fact, with the file that is the evidence for it.
///
/// The value is what `docs/CONTEXT_SPEC.md` §5 puts in a [`ProjectProfile`], and the evidence is the
/// project-relative path that implied it, with `/` separators so it compares the same on every
/// platform. Both are reported, because "languages: rust" without `Cargo.toml` is a claim, and
/// `Cargo.toml` without "rust" is trivia.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signal {
    /// The profile field this populates, using the spec's own field names.
    pub kind: &'static str,
    /// The value, for example `rust`.
    pub value: String,
    /// The project-relative path that implies it, with `/` separators.
    pub evidence: String,
    /// How much this one fact is trusted.
    pub confidence: Confidence,
}

/// What discovery concluded about a repository (`docs/CONTEXT_SPEC.md` §5).
///
/// The aggregated fields are only ever as trustworthy as [`signals`](Self::signals), which carries
/// the evidence behind each of them. Two properties carry the meaning of the type: every field is a
/// finding with a file behind it, and [`unrecognised`](Self::unrecognised) is the honest answer when
/// nothing matched.
///
/// This type deliberately does not implement `Serialize`. The crate has no serialisation dependency
/// and the CLI already owns the published JSON envelopes; it maps this profile into its own
/// serialisable rows, the same way it does for `doctor` findings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectProfile {
    /// Languages found, from manifests rather than file extensions.
    pub languages: Vec<String>,
    /// Frameworks inferred from manifest dependencies and framework config files.
    pub frameworks: Vec<String>,
    /// The package manager implied by the highest-priority lockfile found, if any.
    pub package_manager: Option<String>,
    /// The version-control system in use. `None` when there is no `.git`.
    pub vcs: Option<String>,
    /// Whether anything test-shaped was found.
    pub testing: bool,
    /// CI systems found, for example `github-actions`.
    pub ci: Vec<String>,
    /// Container tooling found, for example `docker`.
    pub containers: Vec<String>,
    /// Infrastructure-as-code tools found, for example `terraform`.
    pub iac: Vec<String>,
    /// Cloud providers implied by directory or file layout, for example `aws`.
    pub cloud_hints: Vec<String>,
    /// The weakest confidence among the signals, or `None` when nothing was detected.
    pub confidence: Option<Confidence>,
    /// True when nothing was detected at all, so no field was guessed at.
    pub unrecognised: bool,
    /// The evidence behind the fields above, sorted by kind, value, and evidence.
    pub signals: Vec<Signal>,
}

/// A name checked for existence at the project root, and the fact finding it produces.
struct Marker {
    kind: &'static str,
    path: &'static str,
    value: &'static str,
    confidence: Confidence,
}

/// The fixed table of names that imply a fact on their own.
///
/// This table is the specification of what discovery looks for outside manifest contents, and
/// growing it is a one-line change. Every row is a name whose *presence* is the evidence, so the
/// confidence is `High` except where the name is only a hint (a `tests` directory with no runner).
const MARKERS: &[Marker] = &[
    // Languages, from manifests (CONTEXT_SPEC §5: "from manifests rather than file extensions").
    Marker {
        kind: KIND_LANGUAGES,
        path: "Cargo.toml",
        value: "rust",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "package.json",
        value: "javascript",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "tsconfig.json",
        value: "typescript",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "pyproject.toml",
        value: "python",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "requirements.txt",
        value: "python",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "setup.py",
        value: "python",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "go.mod",
        value: "go",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "pom.xml",
        value: "java",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "build.gradle",
        value: "java",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "build.gradle.kts",
        value: "kotlin",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "Gemfile",
        value: "ruby",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "composer.json",
        value: "php",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "Package.swift",
        value: "swift",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "mix.exs",
        value: "elixir",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_LANGUAGES,
        path: "build.sbt",
        value: "scala",
        confidence: Confidence::High,
    },
    // Version control.
    Marker {
        kind: KIND_VCS,
        path: ".git",
        value: "git",
        confidence: Confidence::High,
    },
    // CI systems.
    Marker {
        kind: KIND_CI,
        path: ".github/workflows",
        value: "github-actions",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CI,
        path: ".gitlab-ci.yml",
        value: "gitlab-ci",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CI,
        path: "Jenkinsfile",
        value: "jenkins",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CI,
        path: ".circleci",
        value: "circleci",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CI,
        path: ".travis.yml",
        value: "travis",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CI,
        path: "azure-pipelines.yml",
        value: "azure-pipelines",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CI,
        path: "bitbucket-pipelines.yml",
        value: "bitbucket",
        confidence: Confidence::High,
    },
    // Containers.
    Marker {
        kind: KIND_CONTAINERS,
        path: "Dockerfile",
        value: "docker",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CONTAINERS,
        path: "Containerfile",
        value: "docker",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CONTAINERS,
        path: "docker-compose.yml",
        value: "compose",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CONTAINERS,
        path: "docker-compose.yaml",
        value: "compose",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CONTAINERS,
        path: "compose.yml",
        value: "compose",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CONTAINERS,
        path: "compose.yaml",
        value: "compose",
        confidence: Confidence::High,
    },
    // Tests: a directory is a hint, a runner's own config file is a statement.
    Marker {
        kind: KIND_TESTING,
        path: "tests",
        value: "tests",
        confidence: Confidence::Medium,
    },
    Marker {
        kind: KIND_TESTING,
        path: "test",
        value: "tests",
        confidence: Confidence::Medium,
    },
    Marker {
        kind: KIND_TESTING,
        path: "spec",
        value: "tests",
        confidence: Confidence::Medium,
    },
    Marker {
        kind: KIND_TESTING,
        path: "__tests__",
        value: "tests",
        confidence: Confidence::Medium,
    },
    Marker {
        kind: KIND_TESTING,
        path: "pytest.ini",
        value: "pytest",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_TESTING,
        path: "tox.ini",
        value: "tox",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_TESTING,
        path: "conftest.py",
        value: "pytest",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_TESTING,
        path: "jest.config.js",
        value: "jest",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_TESTING,
        path: "jest.config.ts",
        value: "jest",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_TESTING,
        path: "vitest.config.ts",
        value: "vitest",
        confidence: Confidence::High,
    },
    // Infrastructure as code.
    Marker {
        kind: KIND_IAC,
        path: "k8s",
        value: "kubernetes",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_IAC,
        path: "kubernetes",
        value: "kubernetes",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_IAC,
        path: "charts",
        value: "helm",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_IAC,
        path: "pulumi.yaml",
        value: "pulumi",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_IAC,
        path: "ansible.cfg",
        value: "ansible",
        confidence: Confidence::High,
    },
    // Cloud configuration hints.
    Marker {
        kind: KIND_CLOUD_HINTS,
        path: "aws",
        value: "aws",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CLOUD_HINTS,
        path: ".azure",
        value: "azure",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CLOUD_HINTS,
        path: "gcp",
        value: "gcp",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CLOUD_HINTS,
        path: "serverless.yml",
        value: "serverless",
        confidence: Confidence::High,
    },
    Marker {
        kind: KIND_CLOUD_HINTS,
        path: "serverless.yaml",
        value: "serverless",
        confidence: Confidence::High,
    },
];

/// Lockfiles in priority order, with the package manager each implies.
///
/// The schema makes `package_manager` a single value, so when several lockfiles are present the
/// first row here wins and every lockfile is still recorded as a signal. The order is deliberate:
/// a repository that carries both a `Cargo.lock` and a `package-lock.json` is reported as `cargo`
/// because that is the order a reader would expect the primary toolchain to appear in.
const LOCKFILES: &[(&str, &str)] = &[
    ("Cargo.lock", "cargo"),
    ("package-lock.json", "npm"),
    ("pnpm-lock.yaml", "pnpm"),
    ("yarn.lock", "yarn"),
    ("poetry.lock", "poetry"),
    ("uv.lock", "uv"),
    ("Pipfile.lock", "pipenv"),
    ("go.sum", "go"),
    ("pom.xml", "maven"),
    ("build.gradle", "gradle"),
    ("build.gradle.kts", "gradle"),
    ("composer.lock", "composer"),
    ("Gemfile.lock", "bundler"),
];

/// Manifests whose declared dependencies are scanned for framework names.
const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
];

/// Frameworks a config file declares directly, one row per file name.
const FRAMEWORK_CONFIGS: &[(&str, &str)] = &[
    ("next.config.js", "nextjs"),
    ("next.config.mjs", "nextjs"),
    ("next.config.cjs", "nextjs"),
    ("next.config.ts", "nextjs"),
    ("nuxt.config.js", "nuxt"),
    ("nuxt.config.ts", "nuxt"),
    ("angular.json", "angular"),
    ("svelte.config.js", "svelte"),
    ("vite.config.js", "vite"),
    ("vite.config.ts", "vite"),
    ("astro.config.mjs", "astro"),
    ("astro.config.ts", "astro"),
    ("gatsby-config.js", "gatsby"),
    ("gatsby-config.ts", "gatsby"),
    ("remix.config.js", "remix"),
    ("ember-cli-build.js", "ember"),
    ("tailwind.config.js", "tailwind"),
    ("tailwind.config.ts", "tailwind"),
    ("mkdocs.yml", "mkdocs"),
    ("manage.py", "django"),
];

/// Crates in a `Cargo.toml` that name a framework, most specific first.
const CARGO_FRAMEWORKS: &[(&str, &str)] = &[
    ("actix-web", "actix-web"),
    ("axum", "axum"),
    ("rocket", "rocket"),
    ("warp", "warp"),
    ("poem", "poem"),
    ("tonic", "tonic"),
    ("tauri", "tauri"),
    ("bevy", "bevy"),
    ("leptos", "leptos"),
    ("dioxus", "dioxus"),
    ("yew", "yew"),
    ("sqlx", "sqlx"),
    ("diesel", "diesel"),
];

/// Packages in a `package.json` that name a framework, most specific first.
const NODE_FRAMEWORKS: &[(&str, &str)] = &[
    ("react-native", "react-native"),
    ("react", "react"),
    ("next", "nextjs"),
    ("nuxt", "nuxt"),
    ("@angular/core", "angular"),
    ("svelte", "svelte"),
    ("solid-js", "solid"),
    ("vue", "vue"),
    ("express", "express"),
    ("fastify", "fastify"),
    ("koa", "koa"),
    ("electron", "electron"),
    ("astro", "astro"),
    ("gatsby", "gatsby"),
    ("remix", "remix"),
];

/// Dependencies in a Python manifest that name a framework, most specific first.
const PYTHON_FRAMEWORKS: &[(&str, &str)] = &[
    ("django", "django"),
    ("flask", "flask"),
    ("fastapi", "fastapi"),
    ("starlette", "starlette"),
    ("aiohttp", "aiohttp"),
    ("streamlit", "streamlit"),
    ("gradio", "gradio"),
];

/// Modules in a `go.mod` that name a framework, most specific first.
const GO_FRAMEWORKS: &[(&str, &str)] = &[
    ("gin-gonic/gin", "gin"),
    ("labstack/echo", "echo"),
    ("gofiber/fiber", "fiber"),
    ("go-chi/chi", "chi"),
];

/// Group ids in a JVM build file that name a framework, most specific first.
const JVM_FRAMEWORKS: &[(&str, &str)] = &[
    ("spring-boot", "spring-boot"),
    ("quarkus", "quarkus"),
    ("micronaut", "micronaut"),
];

/// Inspects `root` and returns an advisory profile. Never writes, never fails.
///
/// A root that cannot be listed is read as a root with nothing in it, so the result is the same
/// empty, unrecognised profile that a genuinely empty project produces. See the module docs for why
/// that is deliberate.
#[must_use]
pub fn discover(root: &Path) -> ProjectProfile {
    let mut signals = Vec::new();
    collect_markers(root, &mut signals);
    collect_extension_markers(root, &mut signals);
    collect_frameworks(root, &mut signals);
    collect_package_managers(root, &mut signals);

    signals.sort_by(|left, right| {
        (left.kind, &left.value, &left.evidence).cmp(&(right.kind, &right.value, &right.evidence))
    });

    ProjectProfile {
        languages: distinct(&signals, KIND_LANGUAGES),
        frameworks: distinct(&signals, KIND_FRAMEWORKS),
        package_manager: LOCKFILES
            .iter()
            .copied()
            .find(|&(file, _)| exists(root, file))
            .map(|(_, manager)| manager.to_string()),
        vcs: exists(root, ".git").then(|| "git".to_string()),
        testing: signals.iter().any(|signal| signal.kind == KIND_TESTING),
        ci: distinct(&signals, KIND_CI),
        containers: distinct(&signals, KIND_CONTAINERS),
        iac: distinct(&signals, KIND_IAC),
        cloud_hints: distinct(&signals, KIND_CLOUD_HINTS),
        confidence: signals.iter().map(|signal| signal.confidence).min(),
        unrecognised: signals.is_empty(),
        signals,
    }
}

/// Records one signal for every [`MARKERS`] row whose path exists below `root`.
fn collect_markers(root: &Path, signals: &mut Vec<Signal>) {
    for marker in MARKERS {
        if exists(root, marker.path) {
            signals.push(Signal {
                kind: marker.kind,
                value: marker.value.to_string(),
                evidence: marker.path.to_string(),
                confidence: marker.confidence,
            });
        }
    }
}

/// Records language and `IaC` signals that need a file extension rather than a fixed name.
///
/// The top level is listed once, sorted, and scanned, so the answer does not depend on directory
/// iteration order (`RULES.md` §8). Only the first [`MAX_ENTRIES_SCANNED`] entries are examined.
fn collect_extension_markers(root: &Path, signals: &mut Vec<Signal>) {
    for name in top_level_names(root) {
        let Some(extension) = Path::new(&name).extension() else {
            continue;
        };
        // Extensions are compared case-insensitively because a checkout on Windows can hold
        // `Main.TF` and a discovery that misses it on one platform and finds it on another is worse
        // than one that always looks the same way.
        if extension.eq_ignore_ascii_case("tf") && root.join(&name).is_file() {
            signals.push(Signal {
                kind: KIND_IAC,
                value: "terraform".to_string(),
                evidence: name.clone(),
                confidence: Confidence::High,
            });
        }
        // .NET and Haskell have no single conventional manifest name, so their projects are found by
        // extension.
        if extension.eq_ignore_ascii_case("csproj") || extension.eq_ignore_ascii_case("sln") {
            signals.push(Signal {
                kind: KIND_LANGUAGES,
                value: "csharp".to_string(),
                evidence: name.clone(),
                confidence: Confidence::High,
            });
        }
        if extension.eq_ignore_ascii_case("cabal") {
            signals.push(Signal {
                kind: KIND_LANGUAGES,
                value: "haskell".to_string(),
                evidence: name.clone(),
                confidence: Confidence::High,
            });
        }
    }
}

/// Records a framework for every framework config file present and every known dependency named in
/// a manifest.
fn collect_frameworks(root: &Path, signals: &mut Vec<Signal>) {
    for &(file, framework) in FRAMEWORK_CONFIGS {
        if exists(root, file) {
            signals.push(framework_signal(framework, file));
        }
    }
    for manifest in MANIFESTS {
        let Some(text) = read_manifest(root, manifest) else {
            continue;
        };
        for &(token, framework) in framework_table(manifest) {
            if contains_token(&text, token) {
                signals.push(framework_signal(framework, manifest));
            }
        }
    }
}

/// Records one `package_manager` signal per lockfile present, for the evidence trail.
fn collect_package_managers(root: &Path, signals: &mut Vec<Signal>) {
    for &(file, manager) in LOCKFILES {
        if exists(root, file) {
            signals.push(Signal {
                kind: KIND_PACKAGE_MANAGER,
                value: manager.to_string(),
                evidence: file.to_string(),
                confidence: Confidence::High,
            });
        }
    }
}

/// A framework signal with its manifest or config file as evidence.
fn framework_signal(framework: &str, evidence: &str) -> Signal {
    Signal {
        kind: KIND_FRAMEWORKS,
        value: framework.to_string(),
        evidence: evidence.to_string(),
        confidence: Confidence::High,
    }
}

/// The dependency table to scan for a given manifest, or an empty one.
fn framework_table(manifest: &str) -> &'static [(&'static str, &'static str)] {
    match manifest {
        "Cargo.toml" => CARGO_FRAMEWORKS,
        "package.json" => NODE_FRAMEWORKS,
        "pyproject.toml" | "requirements.txt" => PYTHON_FRAMEWORKS,
        "go.mod" => GO_FRAMEWORKS,
        "pom.xml" | "build.gradle" | "build.gradle.kts" => JVM_FRAMEWORKS,
        _ => &[],
    }
}

/// Whether `needle` appears in `haystack` bounded by characters that are not word characters.
///
/// The bound is what stops `react` matching inside `preact`, and what lets it match inside
/// `react-dom`: a dependency list names packages, and a package is a run bounded by anything that
/// is not a letter, digit, or underscore. The scan is over the manifest's text rather than a parsed
/// dependency graph, so it is deliberately conservative about names and may report a framework the
/// manifest only mentions. That is acceptable because discovery is advisory and every result keeps
/// the manifest as its evidence.
fn contains_token(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(needle).any(|(index, _)| {
        let bytes = haystack.as_bytes();
        let before = index
            .checked_sub(1)
            .map(|i| bytes[i])
            .is_none_or(|byte| !is_word_byte(byte));
        let after = bytes
            .get(index + needle.len())
            .is_none_or(|byte| !is_word_byte(*byte));
        before && after
    })
}

/// Whether `byte` is part of a package name rather than a boundary.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Reads a manifest if it is a file within [`MAX_MANIFEST_BYTES`] and valid UTF-8.
///
/// An unreadable, oversized, or non-UTF-8 manifest yields `None`, because discovery is advisory: a
/// manifest it cannot read contributes no framework rather than ending the scan.
fn read_manifest(root: &Path, name: &str) -> Option<String> {
    let path = root.join(name);
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    fs::read_to_string(&path).ok()
}

/// The top-level entry names below `root`, sorted. Empty when the root cannot be listed.
fn top_level_names(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.take(MAX_ENTRIES_SCANNED) {
        // One unreadable entry does not invalidate the rest of the report; discovery is advisory.
        let Ok(entry) = entry else {
            continue;
        };
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    names
}

/// The distinct values recorded for one signal kind, sorted.
fn distinct(signals: &[Signal], kind: &str) -> Vec<String> {
    let mut values: Vec<String> = signals
        .iter()
        .filter(|signal| signal.kind == kind)
        .map(|signal| signal.value.clone())
        .collect();
    values.sort();
    values.dedup();
    values
}

/// Whether `relative` exists below `root`.
fn exists(root: &Path, relative: &str) -> bool {
    root.join(relative).exists()
}

#[cfg(test)]
mod tests {
    use super::{
        Confidence, KIND_LANGUAGES, contains_token, discover, is_word_byte, read_manifest,
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn an_empty_project_is_unrecognised_rather_than_guessed() {
        let root = TempDir::new().expect("temp dir");
        let profile = discover(root.path());

        assert!(
            profile.unrecognised,
            "an empty project must not be guessed at"
        );
        assert!(profile.languages.is_empty());
        assert!(profile.frameworks.is_empty());
        assert_eq!(profile.package_manager, None);
        assert_eq!(profile.vcs, None);
        assert!(!profile.testing);
        assert!(profile.ci.is_empty());
        assert!(profile.signals.is_empty());
        assert_eq!(profile.confidence, None);
    }

    #[test]
    fn a_missing_root_reads_as_nothing_rather_than_failing() {
        let root = TempDir::new().expect("temp dir");
        let gone = root.path().join("does-not-exist");
        let profile = discover(&gone);
        assert!(profile.unrecognised);
        assert!(profile.signals.is_empty());
    }

    #[test]
    fn a_rust_project_is_reported_from_its_manifest_and_lockfile() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("write");
        fs::write(root.path().join("Cargo.lock"), "").expect("write");
        fs::create_dir(root.path().join(".git")).expect("dir");

        let profile = discover(root.path());

        assert_eq!(profile.languages, ["rust"]);
        assert_eq!(profile.package_manager.as_deref(), Some("cargo"));
        assert_eq!(profile.vcs.as_deref(), Some("git"));
        assert!(!profile.unrecognised);
        assert_eq!(profile.confidence, Some(Confidence::High));
    }

    #[test]
    fn a_dependency_names_a_framework() {
        let root = TempDir::new().expect("temp dir");
        fs::write(
            root.path().join("Cargo.toml"),
            "[dependencies]\naxum = \"0.7\"\nserde = \"1\"\n",
        )
        .expect("write");

        let profile = discover(root.path());

        assert_eq!(profile.frameworks, ["axum"]);
        assert!(
            profile
                .signals
                .iter()
                .any(|s| s.evidence == "Cargo.toml" && s.value == "axum"),
            "the manifest must be the evidence for a framework"
        );
    }

    #[test]
    fn a_word_boundary_stops_a_substring_matching() {
        // `react` must not be found inside `preact`, and must be found inside `react-dom` and a
        // quoted `"react"`.
        assert!(!contains_token("preact = \"1\"", "react"));
        assert!(contains_token("react-dom = \"1\"", "react"));
        assert!(contains_token("\"react\": \"18\"", "react"));
        assert!(contains_token("react", "react"));
        assert!(!contains_token("context", "next"));
        assert!(is_word_byte(b'a'));
        assert!(!is_word_byte(b'-'));
    }

    #[test]
    fn a_framework_config_file_is_evidence_on_its_own() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("next.config.js"), "module.exports = {};\n").expect("write");

        let profile = discover(root.path());

        assert_eq!(profile.frameworks, ["nextjs"]);
    }

    #[test]
    fn several_python_markers_yield_one_language_and_keep_every_evidence() {
        let root = TempDir::new().expect("temp dir");
        for file in ["pyproject.toml", "requirements.txt", "setup.py"] {
            fs::write(root.path().join(file), "").expect("write");
        }

        let profile = discover(root.path());

        assert_eq!(profile.languages, ["python"]);
        let evidence: Vec<&str> = profile
            .signals
            .iter()
            .filter(|s| s.kind == KIND_LANGUAGES)
            .map(|s| s.evidence.as_str())
            .collect();
        assert_eq!(evidence, ["pyproject.toml", "requirements.txt", "setup.py"]);
    }

    #[test]
    fn the_highest_priority_lockfile_wins_and_the_rest_are_still_recorded() {
        let root = TempDir::new().expect("temp dir");
        fs::write(root.path().join("Cargo.lock"), "").expect("write");
        fs::write(root.path().join("package-lock.json"), "{}").expect("write");

        let profile = discover(root.path());

        assert_eq!(profile.package_manager.as_deref(), Some("cargo"));
        let managers: Vec<&str> = profile
            .signals
            .iter()
            .filter(|s| s.kind == "package_manager")
            .map(|s| s.value.as_str())
            .collect();
        assert!(managers.contains(&"cargo"));
        assert!(managers.contains(&"npm"));
    }

    #[test]
    fn terraform_is_found_from_a_top_level_file_only() {
        let root = TempDir::new().expect("temp dir");
        // A capitalization a checkout can carry. The comparison is case-insensitive so this is found
        // on every platform, whether or not the filesystem folds case.
        fs::write(root.path().join("Main.TF"), "").expect("write");
        // A `*.tf` below the top level is not a signal: discovery only reads the top level.
        fs::create_dir(root.path().join("nested")).expect("dir");
        fs::write(root.path().join("nested").join("other.tf"), "").expect("write");

        let profile = discover(root.path());

        assert_eq!(profile.iac, ["terraform"]);
        assert_eq!(
            profile.signals.iter().filter(|s| s.kind == "iac").count(),
            1,
            "only the top level is scanned"
        );
        assert_eq!(profile.signals[0].evidence, "Main.TF");
    }

    #[test]
    fn a_tests_directory_is_only_a_medium_confidence_hint() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir(root.path().join("tests")).expect("dir");

        let profile = discover(root.path());

        assert!(profile.testing);
        assert_eq!(
            profile.confidence,
            Some(Confidence::Medium),
            "the weakest signal sets the profile-level confidence"
        );
    }

    #[test]
    fn the_report_is_sorted_and_repeatable() {
        let root = TempDir::new().expect("temp dir");
        fs::write(
            root.path().join("Cargo.toml"),
            "[dependencies]\naxum = \"0.7\"\n",
        )
        .expect("write");
        fs::write(root.path().join("Dockerfile"), "").expect("write");
        fs::create_dir_all(root.path().join(".github").join("workflows")).expect("dir");

        let first = discover(root.path());
        let second = discover(root.path());
        assert_eq!(first, second, "discovery is deterministic");

        let mut sorted = first.signals.clone();
        sorted.sort_by(|left, right| {
            (left.kind, &left.value, &left.evidence).cmp(&(
                right.kind,
                &right.value,
                &right.evidence,
            ))
        });
        assert_eq!(first.signals, sorted, "signals come out sorted");
    }

    #[test]
    fn an_oversize_manifest_is_skipped_rather_than_truncated() {
        let root = TempDir::new().expect("temp dir");
        // A valid framework token sits past the cap, and a truncated read would still find it.
        let padding = "x".repeat(
            usize::try_from(super::MAX_MANIFEST_BYTES).expect("the cap fits in a usize") + 1,
        );
        fs::write(
            root.path().join("Cargo.toml"),
            format!("{padding}\naxum = \"0.7\"\n"),
        )
        .expect("write");

        assert!(read_manifest(root.path(), "Cargo.toml").is_none());
        let profile = discover(root.path());
        assert!(
            profile.frameworks.is_empty(),
            "a manifest past the cap contributes nothing"
        );
        // The marker table still names Rust, because the file exists; only the contents are skipped.
        assert_eq!(profile.languages, ["rust"]);
    }

    #[test]
    fn a_profile_is_debug_and_comparable() {
        // Two roots that are both empty must produce equal profiles, so `ProjectProfile` has to
        // implement `PartialEq` and `Debug` for a caller to compare and report one.
        let first = TempDir::new().expect("temp dir");
        let second = TempDir::new().expect("temp dir");
        let left = discover(first.path());
        let right = discover(second.path());
        assert_eq!(left, right);
        assert!(format!("{left:?}").contains("ProjectProfile"));
    }
}
