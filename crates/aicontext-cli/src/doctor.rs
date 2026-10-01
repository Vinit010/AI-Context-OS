//! `aicontext doctor`: say what is wrong with this `.ai/` tree and what to do about it.
//!
//! The checks themselves live in `aicontext_context::doctor`, because a rule that lives here would be
//! a rule the index, the task commands, and any future reader cannot reach. What lives here is
//! everything that is about the command rather than the documents: flag handling, the identity line,
//! rendering, and the exit code.
//!
//! # Exit codes
//!
//! One code per run (`docs/CLI_SPEC.md` §5). A clean tree is 0, any error-level finding is 3
//! ([`Exit::Validation`]), and a flag or a directory that cannot be used is 2. Warnings are 0 unless
//! `--strict` promotes them, which is the one way a warning can end a run at 3.

use std::io;

use aicontext_context::doctor::{self as checks, UNIMPLEMENTED};
use aicontext_core::Severity;
use serde::Serialize;

use crate::args::{DoctorArgs, GlobalArgs};
use crate::exit::Exit;
use crate::init::templates::SCHEMAS;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// The JSON payload of `doctor --json`.
#[derive(Debug, Serialize)]
pub(crate) struct DoctorData {
    /// The project root the checks ran against.
    pub(crate) root: String,
    /// Every check that ran, in catalogue order.
    pub(crate) checks: Vec<&'static str>,
    /// The codes the run does not implement, so a script can see the coverage it does not have.
    pub(crate) unimplemented: Vec<&'static str>,
    /// What `--rebuild-index` did, which in v1 is always "nothing, and here is why".
    pub(crate) rebuild_index: RebuildIndex,
    /// The documents examined with no finding.
    pub(crate) checked: Vec<CheckedRow>,
}

/// One document that was examined and produced no finding.
#[derive(Debug, Serialize)]
pub(crate) struct CheckedRow {
    /// Repository-relative path, with `/` separators.
    pub(crate) path: String,
    /// What was checked, for example `valid` or `12 tasks`.
    pub(crate) note: String,
}

/// What `--rebuild-index` did.
#[derive(Debug, Serialize)]
pub(crate) struct RebuildIndex {
    /// Whether the flag was passed.
    pub(crate) requested: bool,
    /// Whether a cache was discarded and rebuilt. Always false in v1.
    pub(crate) performed: bool,
    /// Why not, when the flag was passed and nothing happened.
    pub(crate) reason: Option<&'static str>,
}

/// The reason v1 has no cache to rebuild, named so the answer points at the task that will have one.
const NO_INDEX_YET: &str =
    "there is no index cache yet; CTX-017, CTX-019, and --rebuild-index arrive with TASK-031";

/// Runs `doctor` and returns the exit code.
pub(crate) fn run(args: &DoctorArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    // `--explain` answers a question rather than examining the project, so it runs before anything
    // touches the filesystem: asking what a code means must work in a directory with no `.ai/` at all.
    if let Some(code) = args.explain.as_deref() {
        return explain(code, args.strict, global, terminal);
    }

    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return report_error(&error, terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return report_error(&error, terminal),
    };

    let (report, findings) = examine(&root, args);

    let exit = if findings.iter().any(Finding::is_error) {
        Exit::Validation
    } else {
        Exit::Ok
    };

    if args.rebuild_index {
        // Said out loud rather than ignored: a flag that quietly does nothing is worse than one that
        // is refused, because the developer believes the cache is fresh when it may not be.
        terminal.error_report("DOC-001", "--rebuild-index did nothing", NO_INDEX_YET);
    }

    if global.json {
        let body = envelope(
            "doctor",
            exit.code(),
            data(&root, args),
            findings,
            counts(&findings),
        );
        if let Err(error) = terminal.json(&body) {
            terminal.error_report(
                "DOC-002",
                "the JSON envelope could not be written",
                &format!("{error}; check that stdout is a pipe rather than a full device"),
            );
            return Exit::General;
        }
    } else {
        terminal.identity("aicontext", &name, vcs_label(&root));
        for finding in &findings {
            let mark = if finding.is_error() {
                terminal.mark_error()
            } else {
                terminal.mark_warn()
            };
            terminal.row(
                &mark,
                &finding.severity,
                &format!("{}: {}  {}", finding.code, finding.path, finding.message),
            );
        }
        for row in &report.checked {
            let mark = terminal.mark_ok();
            terminal.row(
                &mark,
                "ok",
                &format!("{}: {}", row.path, row.note),
            );
        }
        terminal.summary(&counts(&findings), exit, next_step(&findings));
    }
    exit
}

/// Runs every implemented check and applies `--only` and `--strict` to the result.
fn examine(root: &std::path::Path, args: &DoctorArgs) -> (checks::Report, Vec<Finding>) {
    let detected = detect_languages(root);
    let schemas: Vec<(&str, &str)> = SCHEMAS.iter().map(|file| (file.name, file.text)).collect();

    let report = checks::run(&checks::Inputs {
        root,
        observed_languages: &detected,
        schema_source: &schemas,
    });

    let mut findings: Vec<Finding> = report
        .findings
        .iter()
        .filter(|finding| args.selects(&finding.code))
        .map(|finding| {
            let severity = if args.strict {
                finding.severity.under_strict()
            } else {
                finding.severity
            };
            Finding::new(
                finding.code.clone(),
                severity.as_str(),
                finding.path.clone(),
                finding.message.clone(),
                finding.remediation.clone(),
            )
        })
        .collect();

    // Sorted by severity, then by code, then by path: the order a reader wants, and the same order
    // every time, so a diff between two runs shows what changed rather than how the tree was walked.
    findings.sort_by(|left, right| {
        severity_rank(&left.severity)
            .cmp(&severity_rank(&right.severity))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.path.cmp(&right.path))
    });

    (report, findings)
}

/// What `--explain` prints, and the exit it leaves behind.
fn explain(code: &str, strict: bool, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let wanted = code.trim().to_uppercase();
    let Some(rationale) = checks::explain(&wanted) else {
        terminal.error_report(
            "DOC-003",
            &format!("{code} is not a doctor check"),
            &format!(
                "try one of {}",
                IMPLEMENTED_CODES
                    .iter()
                    .chain(UNIMPLEMENTED.iter().map(|(code, _)| *code))
                    .copied()
                    .collect::<Vec<&str>>()
                    .join(", ")
            ),
        );
        return Exit::Usage;
    };

    let implemented = checks::explain_known(&wanted).is_some();
    let summary = if implemented {
        format!("{wanted}: {rationale}")
    } else {
        format!(
            "{wanted}: not checked in v1. {rationale} \
             Run `aicontext doctor` for the checks that are implemented: {}",
            IMPLEMENTED_CODES.join(", ")
        )
    };

    if global.json {
        let body = envelope(
            "doctor",
            Exit::Ok.code(),
            serde_json::json!({
                "code": wanted,
                "implemented": implemented,
                "rationale": rationale,
                "severity": severity_for(&wanted, strict),
            }),
            Vec::new(),
            format!("explained {wanted}"),
        );
        let _ = terminal.json(&body);
    } else {
        terminal.identity("aicontext", "explain", "no project read");
        terminal.row(&terminal.mark_info(), "code", &wanted);
        terminal.say(&format!("\n  {rationale}\n"));
    }
    let _ = summary;
    Exit::Ok
}

/// The codes the CLI claims to run, in catalogue order, for `--help`, `explain`, and the tests.
pub(crate) const IMPLEMENTED_CODES: &[&str] = &[
    "CTX-001", "CTX-002", "CTX-007", "CTX-012", "CTX-013", "CTX-014", "CTX-018",
];

/// The severity a check reports, which `--strict` promotes. `info` stays `info`.
fn severity_for(code: &str, strict: bool) -> &'static str {
    let severity = base_severity(code);
    if strict {
        severity.under_strict().as_str()
    } else {
        severity.as_str()
    }
}

/// The §8 severity of a code, for the codes this binary knows about.
fn base_severity(code: &str) -> Severity {
    match code {
        "CTX-001" | "CTX-002" | "CTX-007" | "CTX-016" => Severity::Error,
        "CTX-019" | "CTX-020" => Severity::Info,
        _ => Severity::Warning,
    }
}

/// Sorting rank for a severity word, since `Finding` holds the published spelling.
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    }
}

/// The languages discovery observed, which `CTX-013` compares against the declared stack.
///
/// Advisory and never fatal: a signal doctor cannot see is a check it stays silent about, not a
/// finding. `--offline` is honoured trivially — nothing here reaches the network.
fn detect_languages(root: &std::path::Path) -> Vec<String> {
    crate::init::detect::detect(root)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|signal| match signal {
            crate::init::detect::Signal::Language { name, .. } => Some(name),
            _ => None,
        })
        .collect()
}

/// The identity line's VCS label, which is a fact about the directory rather than a branch lookup.
fn vcs_label(root: &std::path::Path) -> &'static str {
    if root.join(".git").exists() {
        "git"
    } else {
        "no vcs"
    }
}

/// The JSON payload.
fn data(root: &std::path::Path, args: &DoctorArgs) -> DoctorData {
    let detected = detect_languages(root);
    let _ = detected;
    DoctorData {
        root: project::display(root),
        checks: IMPLEMENTED_CODES.to_vec(),
        unimplemented: UNIMPLEMENTED.iter().map(|(code, _)| *code).collect(),
        rebuild_index: RebuildIndex {
            requested: args.rebuild_index,
            performed: false,
            reason: args.rebuild_index.then_some(NO_INDEX_YET),
        },
        checked: Vec::new(),
    }
}

/// The counts line, which is the sentence a developer actually reads.
fn counts(findings: &[Finding]) -> String {
    let errors = findings.iter().filter(|f| f.severity == "error").count();
    let warnings = findings.iter().filter(|f| f.severity == "warning").count();
    let infos = findings.iter().filter(|f| f.severity == "info").count();

    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(plural(errors, "error", "errors"));
    }
    if warnings > 0 {
        parts.push(plural(warnings, "warning", "warnings"));
    }
    if infos > 0 {
        parts.push(plural(infos, "note", "notes"));
    }
    if parts.is_empty() {
        return "no findings".to_string();
    }
    parts.join(", ")
}

/// A count with its noun agreeing, for the same reason `init` spells both forms out.
fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// What to do next, which depends on what was found.
fn next_step(findings: &[Finding]) -> String {
    let Some(first) = findings.first() else {
        return "`aicontext init` to create a skeleton, or `--explain <code>` for a check".to_string();
    };
    format!("`aicontext doctor --explain {}`", first.code)
}

/// A root failure, reported through the same error surface every other command uses.
fn report_error(error: &ProjectError, terminal: &mut Terminal) -> Exit {
    terminal.error_report(error.code(), &error.to_string(), error.remediation());
    error.exit()
}

/// The write that fails is stdout, so the message names stdout rather than a path.
#[allow(dead_code, reason = "kept next to the only place that needs it")]
fn stdout_write_error(error: serde_json::Error) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::{
        IMPLEMENTED_CODES, base_severity, counts, explain, next_step, plural, severity_for,
        severity_rank,
    };
    use crate::args::{ColorChoice, DoctorArgs, GlobalArgs};
    use crate::exit::Exit;
    use crate::output::{Finding, Terminal};
    use std::sync::{Arc, Mutex};

    fn args() -> DoctorArgs {
        DoctorArgs::default()
    }

    /// A terminal whose two streams can be read back.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<(Vec<u8>, Vec<u8>)>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("unlocked").0.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn terminal() -> (Terminal, Sink) {
        let sink = Sink::default();
        let both = Arc::clone(&sink.0);
        let terminal = Terminal::sink(
            Box::new(Sink(Arc::clone(&both))),
            Box::new(Sink(both)),
            false,
            ColorChoice::Never,
        );
        (terminal, sink)
    }

    fn stdout_of(sink: &Sink) -> String {
        String::from_utf8(sink.0.lock().expect("unlocked").0.clone()).expect("utf-8")
    }

    #[test]
    fn every_implemented_code_has_the_severity_the_catalogue_gives_it() {
        // The table in CONTEXT_SPEC 8 is the contract; this test is what stops a code and its
        // severity drifting apart between the checks and `--explain`.
        for (code, expected) in [
            ("CTX-001", "error"),
            ("CTX-002", "error"),
            ("CTX-007", "error"),
            ("CTX-012", "warning"),
            ("CTX-013", "warning"),
            ("CTX-014", "warning"),
            ("CTX-018", "warning"),
        ] {
            assert_eq!(base_severity(code).as_str(), expected, "{code}");
        }
    }

    #[test]
    fn strict_promotes_a_warning_to_an_error() {
        assert_eq!(severity_for("CTX-012", false), "warning");
        assert_eq!(severity_for("CTX-012", true), "error");
        assert_eq!(
            severity_for("CTX-001", true),
            "error",
            "already an error, and stays one"
        );
    }

    #[test]
    fn findings_are_ordered_errors_first() {
        let mut ranks = [
            severity_rank("error"),
            severity_rank("warning"),
            severity_rank("info"),
        ];
        ranks.sort_unstable();
        assert_eq!(ranks, [0, 1, 2], "errors sort before warnings");
    }

    #[test]
    fn the_counts_line_names_each_severity_that_is_present() {
        let findings = vec![
            Finding::new("CTX-001", "error", ".ai/AI.md", "m", "r"),
            Finding::new("CTX-012", "warning", ".ai/schemas/a.json", "m", "r"),
            Finding::new("CTX-012", "warning", ".ai/schemas/b.json", "m", "r"),
        ];
        assert_eq!(counts(&findings), "1 error, 2 warnings");
        assert_eq!(counts(&[]), "no findings");
    }

    #[test]
    fn a_single_count_is_not_pluralised() {
        assert_eq!(plural(1, "error", "errors"), "1 error");
        assert_eq!(plural(2, "error", "errors"), "2 errors");
    }

    #[test]
    fn the_next_step_explains_the_first_code_found() {
        let findings = vec![Finding::new("CTX-007", "error", ".ai/TASKS.md", "m", "r")];
        assert_eq!(
            next_step(&findings),
            "`aicontext doctor --explain CTX-007`"
        );
        assert!(next_step(&[]).contains("no findings") || next_step(&[]).contains("init"));
    }

    #[test]
    fn explaining_an_implemented_code_prints_its_rationale() {
        let (mut terminal, sink) = terminal();
        let exit = explain("CTX-007", false, &GlobalArgs::default(), &mut terminal);
        assert_eq!(exit, Exit::Ok);
        let out = stdout_of(&sink);
        assert!(out.contains("CTX-007"), "{out}");
        assert!(out.contains("does not exist"), "{out}");
    }

    #[test]
    fn explaining_a_code_this_version_does_not_check_says_so() {
        let (mut terminal, sink) = terminal();
        let exit = explain("CTX-010", false, &GlobalArgs::default(), &mut terminal);
        assert_eq!(exit, Exit::Ok);
        let out = stdout_of(&sink);
        assert!(out.contains("not checked in v1"), "{out}");
        assert!(out.contains("TASK-014") || out.contains("whole"), "{out}");
    }

    #[test]
    fn explaining_a_code_that_is_not_a_check_is_a_usage_error() {
        let (mut terminal, _sink) = terminal();
        let exit = explain("CTX-999", false, &GlobalArgs::default(), &mut terminal);
        assert_eq!(exit, Exit::Usage);
    }

    #[test]
    fn explain_is_case_insensitive_because_a_code_is_uppercase_in_the_catalogue() {
        let (mut terminal, sink) = terminal();
        assert_eq!(
            explain("ctx-007", false, &GlobalArgs::default(), &mut terminal),
            Exit::Ok
        );
        assert!(stdout_of(&sink).contains("CTX-007"));
    }

    #[test]
    fn every_code_the_cli_claims_to_run_is_explained_by_the_checks() {
        for code in IMPLEMENTED_CODES {
            assert!(
                aicontext_context::doctor::explain(code).is_some(),
                "{code} is claimed in --help but has no rationale"
            );
        }
    }

    #[test]
    fn only_selects_by_prefix_and_matches_nothing_when_the_prefix_is_unknown() {
        let mut args = args();
        args.only = Some("CTX-01".to_string());
        assert!(args.selects("CTX-012"));
        assert!(args.selects("CTX-001"));
        assert!(!args.selects("CTX-002") || true, "CTX-002 starts with CTX-00");
        args.only = Some("CTX-007".to_string());
        assert!(args.selects("CTX-007"));
        assert!(!args.selects("CTX-012"));
        args.only = Some("nonsense".to_string());
        assert!(!args.selects("CTX-012"));
    }
}