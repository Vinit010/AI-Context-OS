//! `aicontext doctor`: say what is wrong with this `.ai/` tree and what to do about it.
//!
//! The checks live in `aicontext_context::doctor`, because a rule that lived here would be a rule the
//! index, the task commands, and any future reader could not reach. What lives here is everything
//! that is about the command rather than the documents: flags, the identity line, rendering, and the
//! exit code.
//!
//! # Exit codes
//!
//! One code per run (`docs/CLI_SPEC.md` §5). A clean tree is 0, any error-level finding is 3
//! ([`Exit::Validation`]), and a flag or a directory that cannot be used is 2. Warnings are 0 unless
//! `--strict` promotes them, which is the only way a warning can end a run at 3.

use std::path::Path;

use aicontext_context::doctor::{self as checks, UNIMPLEMENTED};
use aicontext_core::Severity;
use serde::Serialize;

use crate::args::{DoctorArgs, GlobalArgs};
use crate::exit::Exit;
use crate::init::detect;
use crate::init::templates;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// The reason v1 has no cache to rebuild, naming the task that will have one.
const NO_INDEX_YET: &str =
    "there is no index cache to rebuild yet; CTX-017, CTX-019, and the rebuild arrive with TASK-031";

/// The codes this binary runs, in catalogue order.
///
/// Also what `--help` and `--explain` name, and what the CLI test asserts against the crate, so a code
/// cannot be claimed in one place and be missing from the other.
pub(crate) const IMPLEMENTED_CODES: &[&str] = &[
    "CTX-001", "CTX-002", "CTX-007", "CTX-012", "CTX-013", "CTX-014", "CTX-018",
];

/// Runs `doctor` and returns the exit code.
pub(crate) fn run(args: &DoctorArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    // `--explain` answers a question rather than examining a project, so it runs before anything
    // touches the filesystem: asking what a code means must work in a directory with no `.ai/` at all.
    if let Some(code) = args.explain.as_deref() {
        return explain(code, args.strict, global, terminal);
    }

    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return report_root_failure(&error, terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return report_root_failure(&error, terminal),
    };

    let languages = observed_languages(&root, terminal);
    let schemas = templates::schema_sources();

    let report = checks::run(&checks::Inputs {
        root: &root,
        observed_languages: &languages,
        schema_source: &schemas,
    });

    let findings = collect(&report, args);
    // Asked after the checks ran, so the exit code is computed from exactly the findings that were
    // printed. A code derived any earlier could disagree with the report it is printed next to.
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

    let counts = counts(&findings);
    if global.json {
        let body = envelope(
            "doctor",
            exit.code(),
            data(&root, &report, args),
            findings,
            counts,
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
        print_human(&name, &root, &report, &findings, exit, &counts, terminal);
    }
    exit
}

/// Prints the human report: the identity line, the findings, what was checked, and the summary.
fn print_human(
    name: &str,
    root: &Path,
    report: &checks::Report,
    findings: &[Finding],
    exit: Exit,
    counts: &str,
    terminal: &mut Terminal,
) {
    terminal.identity("aicontext", name, vcs_label(root));

    for finding in findings {
        let mark = if finding.is_error() {
            terminal.mark_error()
        } else {
            terminal.mark_warn()
        };
        terminal.row(
            &mark,
            &finding.severity,
            &format!(
                "{}  {}  {}\n      fix: {}",
                finding.code, finding.path, finding.message, finding.remediation
            ),
        );
    }

    // The documents that passed, so a run that finds one problem still says what it looked at rather
    // than leaving the developer to guess whether the other files were read.
    for checked in &report.checked {
        let mark = terminal.mark_ok();
        terminal.row(&mark, "ok", &format!("{}  {}", checked.path, checked.note));
    }

    terminal.summary(counts, exit, &next_step(findings));
}

/// Turns the crate's findings into this command's, applying `--only` and `--strict`.
fn collect(report: &checks::Report, args: &DoctorArgs) -> Vec<Finding> {
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

    // Sorted by severity, then code, then path: the order a reader wants, and the same order every
    // time, so two runs of the same tree differ only where the tree differs.
    findings.sort_by(|left, right| {
        rank(&left.severity)
            .cmp(&rank(&right.severity))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.path.cmp(&right.path))
    });
    findings
}

/// What `--explain` prints, and the exit it leaves behind.
///
/// Exit 0 for a code that is in the catalogue and 2 for one that is not: asking about a code that
/// does not exist is a mistake in the question, while asking about a check this version defers is a
/// legitimate question with a legitimate answer.
fn explain(code: &str, strict: bool, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let wanted = code.trim().to_uppercase();
    let explanation = checks::explain(&wanted);

    if explanation.is_unknown() {
        terminal.error_report(
            "DOC-003",
            &format!("{code} is not a doctor check"),
            &format!("try one of {}", known_codes().join(", ")),
        );
        return Exit::Usage;
    }

    let rationale = explanation.rationale().unwrap_or_default();
    let implemented = explanation.is_implemented();

    if global.json {
        let body = envelope(
            "doctor",
            Exit::Ok.code(),
            ExplainData {
                code: wanted.clone(),
                implemented,
                severity: severity_for(&wanted, strict),
                rationale,
            },
            Vec::new(),
            format!("explained {wanted}"),
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
        terminal.identity("aicontext", "explain", "no project read");
        let mark = terminal.mark_info();
        terminal.row(&mark, &wanted, rationale);

        if !implemented {
            terminal.say(
                "\n  This version does not run that check. \
                 `aicontext doctor` runs the checks that exist:",
            );
            for code in IMPLEMENTED_CODES {
                terminal.say(&format!("    {code}"));
            }
        } else if strict {
            terminal.say("\n  With --strict this finding counts as an error and exits 3.");
        }
    }
    Exit::Ok
}

/// Every code `--explain` will accept: what runs, plus what the catalogue defers.
fn known_codes() -> Vec<&'static str> {
    let mut codes: Vec<&'static str> = IMPLEMENTED_CODES.to_vec();
    codes.extend(UNIMPLEMENTED.iter().map(|(code, _)| *code));
    codes
}

/// The JSON payload of `doctor --json`.
#[derive(Debug, Serialize)]
struct DoctorData {
    /// The project root the checks ran against, with `/` separators.
    root: String,
    /// Every check that ran, in catalogue order.
    checks: Vec<&'static str>,
    /// The catalogue codes this version does not run, so a script can see the coverage it lacks.
    unimplemented: Vec<&'static str>,
    /// What `--rebuild-index` did, which in v1 is always "nothing, and here is why".
    rebuild_index: RebuildIndex,
    /// The documents examined with no finding.
    checked: Vec<CheckedRow>,
}

/// One document that was examined and produced no finding.
#[derive(Debug, Serialize)]
struct CheckedRow {
    /// Repository-relative path, with `/` separators.
    path: String,
    /// What was checked, for example `valid front matter`.
    note: String,
}

/// What `--rebuild-index` did.
#[derive(Debug, Serialize)]
struct RebuildIndex {
    /// Whether the flag was passed.
    requested: bool,
    /// Whether a cache was discarded and rebuilt. Always false in v1.
    performed: bool,
    /// Why not, when the flag was passed and nothing happened.
    reason: Option<&'static str>,
}

/// The JSON payload of `doctor --explain`.
#[derive(Debug, Serialize)]
struct ExplainData {
    /// The code, spelled as the catalogue spells it.
    code: String,
    /// Whether this version runs the check.
    implemented: bool,
    /// The severity the check reports, promoted when `--strict` was passed.
    severity: &'static str,
    /// Why the check exists, or what is missing from this version.
    rationale: &'static str,
}

/// The JSON payload for one run.
fn data(root: &Path, report: &checks::Report, args: &DoctorArgs) -> DoctorData {
    DoctorData {
        root: project::display(root),
        checks: IMPLEMENTED_CODES.to_vec(),
        unimplemented: UNIMPLEMENTED.iter().map(|(code, _)| *code).collect(),
        rebuild_index: RebuildIndex {
            requested: args.rebuild_index,
            performed: false,
            reason: args.rebuild_index.then_some(NO_INDEX_YET),
        },
        checked: report
            .checked
            .iter()
            .map(|checked| CheckedRow {
                path: checked.path.clone(),
                note: checked.note.clone(),
            })
            .collect(),
    }
}

/// The languages discovery observed, which `CTX-013` compares against the declared stack.
///
/// Discovery failing is reported and then treated as "nothing observed" rather than being allowed to
/// end the run: an advisory check that cannot see a language stays silent about that language, and
/// says so, rather than inventing findings.
fn observed_languages(root: &Path, terminal: &mut Terminal) -> Vec<String> {
    match detect::detect(root) {
        Ok(signals) => signals
            .into_iter()
            .filter(|signal| signal.kind == "languages")
            .map(|signal| signal.value)
            .collect(),
        Err(error) => {
            terminal.error_report(
                "DOC-004",
                "project discovery failed, so CTX-013 compared against nothing",
                &error.to_string(),
            );
            Vec::new()
        }
    }
}

/// The identity line's VCS label: a fact about the directory, not a branch lookup.
fn vcs_label(root: &Path) -> &'static str {
    if root.join(".git").exists() { "git" } else { "no vcs" }
}

/// The severity a check reports, which `--strict` promotes. A note stays a note.
fn severity_for(code: &str, strict: bool) -> &'static str {
    let severity = base_severity(code);
    if strict {
        severity.under_strict().as_str()
    } else {
        severity.as_str()
    }
}

/// The §8 severity of a code.
///
/// The table is the one in `docs/CONTEXT_SPEC.md` §8, and the test below is what stops it drifting
/// away from the checks that actually run.
fn base_severity(code: &str) -> Severity {
    match code {
        "CTX-001" | "CTX-002" | "CTX-007" | "CTX-016" => Severity::Error,
        "CTX-019" | "CTX-020" => Severity::Info,
        _ => Severity::Warning,
    }
}

/// Sorting rank for a severity word, because [`Finding`] holds the published spelling.
fn rank(severity: &str) -> u8 {
    match severity {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    }
}

/// The counts line, which is the sentence a developer actually reads.
fn counts(findings: &[Finding]) -> String {
    let errors = findings.iter().filter(|f| f.severity == "error").count();
    let warnings = findings.iter().filter(|f| f.severity == "warning").count();
    let notes = findings.iter().filter(|f| f.severity == "info").count();

    let mut parts = Vec::new();
    if errors > 0 {
        parts.push(plural(errors, "error", "errors"));
    }
    if warnings > 0 {
        parts.push(plural(warnings, "warning", "warnings"));
    }
    if notes > 0 {
        parts.push(plural(notes, "note", "notes"));
    }
    if parts.is_empty() {
        return "no findings".to_string();
    }
    parts.join(", ")
}

/// A count with its noun agreeing, spelled out both ways for the same reason `init` does.
fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// What to do next, which depends on what was found.
fn next_step(findings: &[Finding]) -> String {
    match findings.first() {
        Some(first) => format!("`aicontext doctor --explain {}`", first.code),
        None => "`aicontext init` to create a skeleton".to_string(),
    }
}

/// A root that cannot be used, reported through the surface every other command reports through.
fn report_root_failure(error: &ProjectError, terminal: &mut Terminal) -> Exit {
    terminal.error_report(error.code(), &error.to_string(), error.remediation());
    error.exit()
}

#[cfg(test)]
mod tests {
    use super::{
        IMPLEMENTED_CODES, base_severity, collect, counts, data, explain, known_codes, next_step,
        observed_languages, plural, severity_for,
    };
    use crate::args::{ColorChoice, DoctorArgs, GlobalArgs};
    use crate::exit::Exit;
    use crate::output::{Finding, Terminal};
    use aicontext_context::doctor as checks;
    use aicontext_core::Severity;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    /// A writer that keeps what it is given, so a test can read the report back.
    #[derive(Clone)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Sink {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().expect("unlocked").clone()).expect("utf-8")
        }
    }

    impl std::io::Write for Sink {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("unlocked").extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn terminal() -> (Terminal, Sink) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let err = Arc::new(Mutex::new(Vec::new()));
        let terminal = Terminal::sink(
            Box::new(Sink(Arc::clone(&out))),
            Box::new(Sink(Arc::clone(&err))),
            false,
            ColorChoice::Never,
        );
        (terminal, Sink(out))
    }

    fn args() -> DoctorArgs {
        DoctorArgs::default()
    }

    fn global(json: bool) -> GlobalArgs {
        GlobalArgs {
            json,
            ..GlobalArgs::default()
        }
    }

    fn report(findings: Vec<(Severity, &str, &str)>) -> checks::Report {
        checks::Report {
            findings: findings
                .into_iter()
                .map(|(severity, code, path)| checks::Finding {
                    severity,
                    code: code.to_string(),
                    path: path.to_string(),
                    message: format!("{code} needs attention"),
                    remediation: format!("fix {code}"),
                })
                .collect(),
            checked: Vec::new(),
        }
    }

    fn finding(severity: Severity) -> Finding {
        Finding::new(
            "CTX-012",
            severity.as_str(),
            ".ai/schemas/task.schema.json",
            "differs",
            "re-run init",
        )
    }

    #[test]
    fn every_code_the_cli_claims_to_run_is_known_to_the_checks() {
        for code in IMPLEMENTED_CODES {
            assert!(
                checks::explain(code).is_implemented(),
                "{code} is claimed in --help but the crate does not run it"
            );
        }
    }

    #[test]
    fn every_severity_matches_the_catalogue_in_section_8() {
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
    fn strict_promotes_a_warning_to_an_error_and_leaves_an_error_alone() {
        assert_eq!(severity_for("CTX-012", false), "warning");
        assert_eq!(severity_for("CTX-012", true), "error");
        assert_eq!(severity_for("CTX-001", true), "error");
        assert_eq!(
            severity_for("CTX-019", true),
            "warning",
            "a note becomes a warning rather than an error, which still exits 0"
        );
    }

    #[test]
    fn findings_are_ordered_errors_first_then_by_code_then_path() {
        let mut args = args();
        let report = report(vec![
            (Severity::Warning, "CTX-014", ".ai/DECISIONS.md"),
            (Severity::Error, "CTX-007", ".ai/TASKS.md"),
            (Severity::Warning, "CTX-012", ".ai/schemas/task.schema.json"),
            (Severity::Error, "CTX-001", ".ai/AI.md"),
        ]);
        let findings = collect(&report, &mut args);
        let order: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert_eq!(order, ["CTX-001", "CTX-007", "CTX-012", "CTX-014"]);

        args.strict = true;
        let promoted = collect(&report, &args);
        assert!(
            promoted.iter().all(|f| f.severity != "warning"),
            "strict leaves no warnings to ignore"
        );
        assert_eq!(promoted.len(), 4, "strict promotes, it does not hide");
    }

    #[test]
    fn only_narrows_the_findings_that_are_printed() {
        let mut args = args();
        args.only = Some("CTX-01".to_string());
        let report = report(vec![
            (Severity::Error, "CTX-012", ".ai/schemas/task.schema.json"),
            (Severity::Error, "CTX-002", ".ai/PRD.md"),
        ]);
        let findings = collect(&report, &args);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "CTX-012");
    }

    #[test]
    fn the_counts_line_names_only_the_severities_that_are_present() {
        assert_eq!(counts(&[]), "no findings");
        assert_eq!(counts(&[finding(Severity::Error)]), "1 error");
        assert_eq!(
            counts(&[finding(Severity::Warning), finding(Severity::Warning)]),
            "2 warnings"
        );
        assert_eq!(
            counts(&[finding(Severity::Error), finding(Severity::Warning), finding(Severity::Info)]),
            "1 error, 1 warning, 1 note"
        );
    }

    #[test]
    fn a_single_count_is_not_pluralised() {
        assert_eq!(plural(1, "error", "errors"), "1 error");
        assert_eq!(plural(2, "error", "errors"), "2 errors");
    }

    #[test]
    fn the_next_step_explains_the_first_code_found() {
        let findings = vec![Finding::new("CTX-007", "error", ".ai/TASKS.md", "m", "r")];
        assert_eq!(next_step(&findings), "`aicontext doctor --explain CTX-007`");
        assert_eq!(next_step(&[]), "`aicontext init` to create a skeleton");
    }

    #[test]
    fn explaining_an_implemented_code_prints_its_rationale() {
        let (mut terminal, sink) = terminal();
        assert_eq!(
            explain("CTX-007", false, &global(false), &mut terminal),
            Exit::Ok
        );
        let out = sink.text();
        assert!(out.contains("CTX-007"), "{out}");
        assert!(out.contains("does not exist"), "{out}");
    }

    #[test]
    fn explaining_is_case_insensitive_because_the_catalogue_spells_codes_uppercase() {
        let (mut terminal, sink) = terminal();
        assert_eq!(
            explain("ctx-007", false, &global(false), &mut terminal),
            Exit::Ok
        );
        assert!(sink.text().contains("CTX-007"));
    }

    #[test]
    fn explaining_a_deferred_code_says_so_and_lists_what_does_run() {
        let (mut terminal, sink) = terminal();
        assert_eq!(
            explain("CTX-010", false, &global(false), &mut terminal),
            Exit::Ok,
            "a legitimate question with a legitimate answer is not a usage error"
        );
        let out = sink.text();
        assert!(out.contains("does not run"), "{out}");
        for code in IMPLEMENTED_CODES {
            assert!(out.contains(code), "{out} does not offer {code}");
        }
    }

    #[test]
    fn explaining_a_code_that_is_not_a_check_is_a_usage_error() {
        let (mut terminal, _sink) = terminal();
        assert_eq!(
            explain("CTX-999", false, &global(false), &mut terminal),
            Exit::Usage
        );
    }

    #[test]
    fn every_known_code_is_offffered_by_the_unknown_code_message() {
        // The message is the only way a person discovers what they may ask about, so it must contain
        // every code `explain` would accept.
        let codes = known_codes();
        assert!(codes.contains(&"CTX-010"), "a deferred code is still known");
        assert!(codes.contains(&"CTX-001"), "a running code is known");
        assert_eq!(codes.len(), IMPLEMENTED_CODES.len() + checks::UNIMPLEMENTED.len());
    }

    #[test]
    fn strict_is_reported_in_an_explanation_so_the_flag_is_not_surprising() {
        let (mut terminal, sink) = terminal();
        assert_eq!(
            explain("CTX-012", true, &global(false), &mut terminal),
            Exit::Ok
        );
        assert!(sink.text().contains("exits 3"), "{}", sink.text());
    }

    #[test]
    fn json_explanation_carries_the_severity_and_whether_it_runs() {
        let (mut terminal, sink) = terminal();
        assert_eq!(explain("CTX-012", true, &global(true), &mut terminal), Exit::Ok);
        let body: serde_json::Value =
            serde_json::from_str(sink.text().trim()).expect("one JSON object");
        assert_eq!(body["data"]["code"], "CTX-012");
        assert_eq!(body["data"]["implemented"], true);
        assert_eq!(body["data"]["severity"], "error");
        assert_eq!(body["exit_code"], 0);
    }

    #[test]
    fn json_explanation_of_a_deferred_code_is_still_a_success() {
        let (mut terminal, sink) = terminal();
        assert_eq!(explain("CTX-010", false, &global(true), &mut terminal), Exit::Ok);
        let body: serde_json::Value =
            serde_json::from_str(sink.text().trim()).expect("one JSON object");
        assert_eq!(body["data"]["implemented"], false);
    }

    #[test]
    fn discovery_reports_the_languages_it_saw() {
        let root = TempDir::new().expect("temp dir");
        std::fs::write(root.path().join("Cargo.toml"), "[package]\nname = \"x\"\n")
            .expect("write");
        let (mut terminal, _sink) = terminal();
        assert_eq!(observed_languages(root.path(), &mut terminal), ["rust"]);
    }

    #[test]
    fn the_json_payload_says_which_checks_ran_and_which_are_deferred() {
        let root = TempDir::new().expect("temp dir");
        let mut args = args();
        args.rebuild_index = true;
        let body = data(root.path(), &report(Vec::new()), &args);
        assert_eq!(body.checks, IMPLEMENTED_CODES.to_vec());
        assert!(!body.unimplemented.contains(&"CTX-001"), "not deferred");
        assert!(body.unimplemented.contains(&"CTX-003"), "runtime validation");
        assert!(body.rebuild_index.requested);
        assert!(!body.rebuild_index.performed, "v1 rebuilds nothing");
        assert!(body.rebuild_index.reason.is_some());
    }

    #[test]
    fn the_rebuild_flag_is_not_requested_when_it_was_not_passed() {
        let root = TempDir::new().expect("temp dir");
        let body = data(root.path(), &report(Vec::new()), &args());
        assert!(!body.rebuild_index.requested);
        assert!(body.rebuild_index.reason.is_none());
    }
}