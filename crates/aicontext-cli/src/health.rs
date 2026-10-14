//! `aicontext health`: transparent context metrics (`docs/CONTEXT_SPEC.md` §9).
//!
//! Every number is a count — never a composite score — and every metric carries the command that
//! reproduces it. The metric definitions live in the spec; the counts are derived here from the same
//! checks `doctor` runs, from the content index, from the task register, and, for freshness only,
//! from the git log.
//!
//! # What is deferred
//!
//! §9 also names *stale memory* and *plugin health*. Neither is reported yet, because neither has a
//! source to count: the first needs the memory register, the second the plugin runtime, and neither
//! exists. A metric with nothing to read would be an invented number, so it is left out rather than
//! shown as zero.
//!
//! # Exit codes
//!
//! `0` for a report, `3` when the tree carries an error-level finding (`docs/CLI_SPEC.md` §5), and `2`
//! for a directory that cannot be used. The command writes nothing: it never rebuilds the index cache.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use aicontext_context::doctor::{self as checks};
use aicontext_context::{ContextIndex, DocumentKind, Value, register};
use aicontext_core::Severity;
use aicontext_git::Git;
use serde::Serialize;

use crate::args::GlobalArgs;
use crate::exit::Exit;
use crate::init::templates;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// One metric: a count (or a ratio), what it means, and how to reproduce it.
///
/// `count` is absent only when the metric could not be computed; `total` is present only for a ratio.
/// `unavailable` carries the reason, so a metric that could not run says so instead of reporting a
/// zero that would read as "nothing wrong".
#[derive(Debug, Default, Serialize)]
struct Metric {
    /// Stable machine name, safe to branch on, for example `context-completeness`.
    id: &'static str,
    /// The human name from `docs/CONTEXT_SPEC.md` §9.
    name: &'static str,
    /// What the number means, in one line.
    definition: &'static str,
    /// The count, or the numerator of a ratio.
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
    /// The denominator of a ratio, absent for a plain count.
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    /// Why the metric could not be computed, which is mutually exclusive with `count`.
    #[serde(skip_serializing_if = "Option::is_none")]
    unavailable: Option<String>,
    /// The command a developer can run to reproduce the number.
    reproduce: &'static str,
    /// Extra lines: what is missing, the breakdown, or which documents are stale.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
}

impl Metric {
    /// The value as a human reads it: `n`, `n/m`, or `n/a`.
    fn value(&self) -> String {
        match (self.count, self.total) {
            (Some(count), Some(total)) => format!("{count}/{total}"),
            (Some(count), None) => count.to_string(),
            (None, _) => "n/a".to_string(),
        }
    }

    /// The metric as one indented block: the value, the definition, the command, and any notes.
    fn render(&self) -> String {
        let mut out = format!(
            "  {:<23}  {:>7}  {}",
            self.name,
            self.value(),
            self.definition
        );
        if let Some(reason) = &self.unavailable {
            let _ = write!(out, "\n      unavailable: {reason}");
        }
        let _ = write!(out, "\n      reproduce: {}", self.reproduce);
        for note in &self.notes {
            let _ = write!(out, "\n      {note}");
        }
        out
    }
}

/// The `data` of the `health` envelope: the §9 metrics, in the spec's order.
#[derive(Debug, Serialize)]
struct Health {
    /// One entry per metric, in the order `docs/CONTEXT_SPEC.md` §9 lists them.
    metrics: Vec<Metric>,
}

/// Runs `health` and returns the exit code.
pub(crate) fn run(global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return report_root_failure(&error, terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return report_root_failure(&error, terminal),
    };

    let report = checks_for(&root, terminal);
    let exit = if report.has_errors() {
        Exit::Validation
    } else {
        Exit::Ok
    };

    // The index is built, never saved: `health` writes nothing, and a report is not a reason to
    // create a cache in a checkout the developer may not own.
    let (index, _scan) = ContextIndex::build(&root, None);
    let git = Git::new(&root);
    let metrics = vec![
        context_completeness(&root),
        architecture_consistency(&report),
        task_consistency(&root, &report),
        decision_coverage(&index),
        broken_references(&report),
        documentation_freshness(&index, &git),
    ];
    let summary = summary_line(&metrics, &report);

    if global.json {
        let body = envelope(
            "health",
            exit.code(),
            Health { metrics },
            findings(&report),
            summary,
        );
        if let Err(error) = terminal.json(&body) {
            terminal.error_report(
                "HEALTH-001",
                "the JSON envelope could not be written",
                &format!("{error}; check that stdout is a pipe rather than a full device"),
            );
            return Exit::General;
        }
    } else {
        print_human(&name, &root, &metrics, exit, &summary, terminal);
    }
    exit
}

/// Runs the same checks `doctor` runs, so the counts and the reproducing commands agree.
///
/// The checks are the one place a rule lives (`crates/aicontext-cli/src/doctor.rs`), and running them
/// here rather than re-implementing a count keeps `health` from drifting from `doctor`.
fn checks_for(root: &Path, terminal: &mut Terminal) -> checks::Report {
    let languages = crate::doctor::observed_languages(root, terminal);
    let schemas = templates::schema_sources();
    checks::run(&checks::Inputs {
        root,
        observed_languages: &languages,
        schema_source: &schemas,
    })
}

/// How many findings carry one of `codes`.
fn count_of(report: &checks::Report, codes: &[&str]) -> usize {
    report
        .findings
        .iter()
        .filter(|finding| codes.contains(&finding.code.as_str()))
        .count()
}

/// Context completeness: present of the documents `init` always writes.
fn context_completeness(root: &Path) -> Metric {
    let expected = templates::base_documents();
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|path| !root.join(path).is_file())
        .collect();
    Metric {
        id: "context-completeness",
        name: "context completeness",
        definition: "documents present of those an initialised project expects",
        count: Some(expected.len() - missing.len()),
        total: Some(expected.len()),
        reproduce: "aicontext doctor",
        notes: missing
            .iter()
            .map(|path| format!("missing: {path}"))
            .collect(),
        ..Metric::default()
    }
}

/// Architecture consistency: the `CTX-013` findings, where a lower tier contradicts a higher one.
fn architecture_consistency(report: &checks::Report) -> Metric {
    Metric {
        id: "architecture-consistency",
        name: "architecture consistency",
        definition: "places a lower tier contradicts a higher one (CTX-013)",
        count: Some(count_of(report, &["CTX-013"])),
        reproduce: "aicontext doctor --only CTX-013",
        ..Metric::default()
    }
}

/// Task consistency: the task findings, with the register's status breakdown as the context.
fn task_consistency(root: &Path, report: &checks::Report) -> Metric {
    let register = register::read(root);
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    for task in &register.tasks {
        *statuses
            .entry(task.status.clone().unwrap_or_else(|| "(none)".to_string()))
            .or_default() += 1;
    }

    let mut notes = vec![format!("{} task(s) in the register", register.tasks.len())];
    notes.extend(
        statuses
            .iter()
            .map(|(status, count)| format!("{status}: {count}")),
    );

    Metric {
        id: "task-consistency",
        name: "task consistency",
        definition: "task findings: missing criteria, cycles, illegal transitions (CTX-009/010/011)",
        count: Some(count_of(report, &["CTX-009", "CTX-010", "CTX-011"])),
        reproduce: "aicontext doctor --only CTX-009",
        notes,
        ..Metric::default()
    }
}

/// Decision coverage: accepted decision records of all decision records.
fn decision_coverage(index: &ContextIndex) -> Metric {
    let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
    for entry in index
        .entries()
        .filter(|entry| entry.kind == Some(DocumentKind::Decision))
    {
        let status = entry
            .front_matter
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("(none)")
            .to_string();
        *by_status.entry(status).or_default() += 1;
    }

    let total: usize = by_status.values().sum();
    let accepted = by_status.get("accepted").copied().unwrap_or(0);
    let notes = by_status
        .iter()
        .map(|(status, count)| format!("{status}: {count}"))
        .collect();

    Metric {
        id: "decision-coverage",
        name: "decision coverage",
        definition: "accepted decision records of all decision records",
        count: Some(accepted),
        total: Some(total),
        reproduce: "grep -l '^status: accepted' .ai/decisions/*.md",
        notes,
        ..Metric::default()
    }
}

/// Broken references, counted by check code (`docs/CONTEXT_SPEC.md` §9).
fn broken_references(report: &checks::Report) -> Metric {
    let missing = count_of(report, &["CTX-007"]);
    let deprecated = count_of(report, &["CTX-008"]);
    Metric {
        id: "broken-references",
        name: "broken references",
        definition: "references to a document that does not exist or is deprecated",
        count: Some(missing + deprecated),
        reproduce: "aicontext doctor --only CTX-007",
        notes: vec![
            format!("CTX-007: {missing}"),
            format!("CTX-008: {deprecated}"),
        ],
        ..Metric::default()
    }
}

/// Documentation freshness: documents whose `updated` predates the newest commit touching what they
/// declare in `touches`.
///
/// A document with no `touches` is not counted either way: there is nothing to compare it against, so
/// calling it fresh would be a claim the tree does not support. A tree that is not a git repository,
/// or a git invocation that fails, makes the whole metric unavailable rather than zero.
fn documentation_freshness(index: &ContextIndex, git: &Git) -> Metric {
    const REPRODUCE: &str = "git log -1 --format=%cs -- <touched path>";
    match git.is_repository() {
        Ok(true) => {}
        Ok(false) => return freshness_unavailable("not a git repository", REPRODUCE),
        Err(error) => return freshness_unavailable(error.to_string(), REPRODUCE),
    }

    let mut checked = 0_usize;
    let mut stale: Vec<String> = Vec::new();
    for entry in index.entries() {
        let Some(updated) = entry.front_matter.get("updated").and_then(Value::as_str) else {
            continue;
        };
        let touches: Vec<&str> = entry
            .front_matter
            .get("touches")
            .and_then(Value::as_list)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if touches.is_empty() {
            continue;
        }
        checked += 1;

        let paths: Vec<String> = touches.iter().map(|touch| touch_path(touch)).collect();
        let mut args = vec!["log", "-1", "--format=%cs", "--"];
        args.extend(paths.iter().map(String::as_str));
        match git.run(&args) {
            Ok(bytes) => {
                let newest = String::from_utf8_lossy(&bytes);
                let newest = newest.trim();
                if !newest.is_empty() && updated < newest {
                    stale.push(entry.path.display().to_string());
                }
            }
            Err(error) => return freshness_unavailable(error.to_string(), REPRODUCE),
        }
    }

    Metric {
        id: "documentation-freshness",
        name: "documentation freshness",
        definition: "documents whose `updated` predates the newest commit to what they describe",
        count: Some(stale.len()),
        total: Some(checked),
        reproduce: REPRODUCE,
        notes: stale.iter().map(|path| format!("stale: {path}")).collect(),
        ..Metric::default()
    }
}

/// An unavailable freshness metric, so a missing repository is reported, not counted as zero.
fn freshness_unavailable(reason: impl Into<String>, reproduce: &'static str) -> Metric {
    Metric {
        id: "documentation-freshness",
        name: "documentation freshness",
        definition: "documents whose `updated` predates the newest commit to what they describe",
        unavailable: Some(reason.into()),
        reproduce,
        ..Metric::default()
    }
}

/// The path a `touches` glob names, for `git log`: a trailing `/**` or `/*` becomes its directory.
fn touch_path(glob: &str) -> String {
    let trimmed = glob.trim_end_matches('/');
    let trimmed = trimmed
        .strip_suffix("/**")
        .or_else(|| trimmed.strip_suffix("/*"))
        .unwrap_or(trimmed);
    trimmed.to_string()
}

/// The checks' findings as the envelope publishes them, so the counts can be reproduced in place.
fn findings(report: &checks::Report) -> Vec<Finding> {
    report
        .findings
        .iter()
        .map(|finding| {
            Finding::new(
                finding.code.clone(),
                finding.severity.as_str(),
                finding.path.clone(),
                finding.message.clone(),
                finding.remediation.clone(),
            )
        })
        .collect()
}

/// The closing counts line.
fn summary_line(metrics: &[Metric], report: &checks::Report) -> String {
    let errors = report.at_least(Severity::Error);
    let noun = if metrics.len() == 1 {
        "metric"
    } else {
        "metrics"
    };
    let findings = if errors == 1 {
        "1 error-level finding".to_string()
    } else {
        format!("{errors} error-level findings")
    };
    format!("{} {noun}, {findings}", metrics.len())
}

/// Prints the human report: the identity line, the metrics, and the summary.
fn print_human(
    name: &str,
    root: &Path,
    metrics: &[Metric],
    exit: Exit,
    summary: &str,
    terminal: &mut Terminal,
) {
    terminal.identity("aicontext", name, project::vcs_label(root));
    for metric in metrics {
        terminal.say(&metric.render());
    }
    terminal.summary(summary, exit, "`aicontext doctor`");
}

/// A root that cannot be used, reported through the surface every other command reports through.
fn report_root_failure(error: &ProjectError, terminal: &mut Terminal) -> Exit {
    terminal.error_report(error.code(), &error.to_string(), error.remediation());
    error.exit()
}

#[cfg(test)]
mod tests {
    use super::{Metric, context_completeness, touch_path};

    #[test]
    fn a_trailing_glob_becomes_the_directory_git_walks() {
        assert_eq!(
            touch_path("crates/aicontext-cli/**"),
            "crates/aicontext-cli"
        );
        assert_eq!(touch_path("crates/aicontext-cli/*"), "crates/aicontext-cli");
        assert_eq!(touch_path("crates/aicontext-cli/"), "crates/aicontext-cli");
        assert_eq!(touch_path("docs/CONTEXT_SPEC.md"), "docs/CONTEXT_SPEC.md");
    }

    #[test]
    fn a_ratio_metric_renders_its_count_and_command() {
        let metric = Metric {
            id: "context-completeness",
            name: "context completeness",
            definition: "present of expected",
            count: Some(3),
            total: Some(4),
            reproduce: "aicontext doctor",
            ..Metric::default()
        };
        let rendered = metric.render();
        assert!(rendered.contains("3/4"), "{rendered}");
        assert!(
            rendered.contains("reproduce: aicontext doctor"),
            "{rendered}"
        );
    }

    #[test]
    fn an_unavailable_metric_says_so_rather_than_reporting_a_zero() {
        let metric = Metric {
            name: "documentation freshness",
            unavailable: Some("not a git repository".to_string()),
            reproduce: "git log -1 --format=%cs",
            ..Metric::default()
        };
        let rendered = metric.render();
        assert!(rendered.contains("n/a"), "{rendered}");
        assert!(
            rendered.contains("unavailable: not a git repository"),
            "{rendered}"
        );
    }

    #[test]
    fn completeness_counts_the_skeleton_init_writes() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let expected = crate::init::templates::base_documents();
        assert!(!expected.is_empty());

        // A directory with no `.ai/` is a project missing every expected document.
        let empty = context_completeness(dir.path());
        assert_eq!(empty.count, Some(0));
        assert_eq!(empty.total, Some(expected.len()));
        assert_eq!(empty.notes.len(), expected.len());

        for path in &expected {
            let full = dir.path().join(path);
            std::fs::create_dir_all(full.parent().expect("a parent")).expect("directories");
            std::fs::write(&full, "x").expect("write");
        }
        let complete = context_completeness(dir.path());
        assert_eq!(complete.count, Some(expected.len()));
        assert!(complete.notes.is_empty());
    }
}
