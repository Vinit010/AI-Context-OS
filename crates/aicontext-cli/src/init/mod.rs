//! `aicontext init`: create the `.ai` skeleton.
//!
//! The command is deliberately split into four steps that can fail separately — plan, approve, write,
//! verify — because a scaffolding command's only job is to be trustworthy about what it touched. The
//! order never changes: everything that can refuse happens before the first byte is written, so a
//! refusal leaves the repository exactly as it was.

pub(crate) mod apply;
pub(crate) mod date;
pub(crate) mod detect;
pub(crate) mod error;
pub(crate) mod plan;
pub(crate) mod render;
pub(crate) mod templates;
pub(crate) mod validate;

use std::io;

use serde::Serialize;

use crate::args::{GlobalArgs, InitArgs};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};

use plan::{Action, Plan};

/// The JSON payload of `init --json`.
///
/// One row per planned path rather than one bucket per action, because the buckets a script would
/// want to build must be derivable without a second tool, and one list cannot disagree with itself.
#[derive(Debug, Serialize)]
pub(crate) struct InitData {
    /// The project name taken from the root directory.
    pub(crate) project: String,
    /// The absolute project root.
    pub(crate) root: String,
    /// The template the plan came from.
    pub(crate) template: String,
    /// Whether this was `--dry-run`, so nothing was written.
    pub(crate) dry_run: bool,
    /// The date stamped into the documents.
    pub(crate) date: String,
    /// What discovery found, in `ProjectProfile` spelling. Advisory, and never written anywhere.
    pub(crate) detected: Vec<String>,
    /// Every planned path with its action, in plan order.
    pub(crate) actions: Vec<ActionRow>,
    /// What `.gitignore` got: `created`, `appended`, or `unchanged`.
    pub(crate) gitignore: &'static str,
}

/// One planned path in the JSON payload.
#[derive(Debug, Serialize)]
pub(crate) struct ActionRow {
    /// The path, relative to the project root, with `/` separators.
    pub(crate) path: String,
    /// `create`, `keep`, `overwrite`, `unchanged`, `directory`, or `append`.
    pub(crate) action: &'static str,
}

/// Runs `init` and returns the exit code.
///
/// `terminal` is already wired to the right streams and colour choice, so this function decides only
/// what to say and where it goes: stdout for the result, stderr for diagnostics.
pub(crate) fn run(args: &InitArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    match execute(args, global, terminal) {
        Ok(exit) => exit,
        Err(error) => {
            terminal.error_report(error.code(), &error.to_string(), error.remediation());
            error.exit()
        }
    }
}

/// The whole command, returning the exit code it should report.
fn execute(
    args: &InitArgs,
    global: &GlobalArgs,
    terminal: &mut Terminal,
) -> Result<Exit, error::InitError> {
    let root = crate::project::resolve_root(global.cwd.as_deref())?;
    let today = date::today_utc()?;
    let plan = plan::build(root, args.template, today, !args.no_detect, args.force)?;

    // The one thing that could destroy a developer's work, refused before anything is written.
    // `--yes` never satisfies it: an absent human is a deny.
    let overwrites = plan.overwrites();
    if overwrites > 0 && !terminal.is_terminal() && !args.dry_run {
        return Err(error::InitError::ForceNeedsTerminal { count: overwrites });
    }

    let mut findings = plan.findings.clone();
    if !args.dry_run {
        apply::apply(&plan)?;
        findings.extend(validate::validate(&plan)?);
    }

    let exit = if findings.iter().any(Finding::is_error) {
        Exit::Validation
    } else {
        Exit::Ok
    };

    if global.json {
        let body = envelope(
            "init",
            exit.code(),
            data(&plan, args.dry_run),
            findings,
            counts(&plan, args.dry_run),
        );
        terminal
            .json(&body)
            .map_err(|error| error::InitError::Write {
                path: "the JSON envelope on stdout".to_string(),
                source: io::Error::other(error),
            })?;
    } else {
        report(&plan, args.dry_run, &findings, terminal);
        terminal.summary(&counts(&plan, args.dry_run), exit, next_step(&plan, exit));
    }
    Ok(exit)
}

/// The JSON payload built from the plan.
fn data(plan: &Plan, dry_run: bool) -> InitData {
    InitData {
        project: plan.project_name.clone(),
        root: crate::project::display(&plan.root),
        template: plan.template.as_str().to_string(),
        dry_run,
        date: plan.date.iso(),
        detected: detect::summarise(&plan.signals),
        actions: plan
            .entries
            .iter()
            .map(|entry| ActionRow {
                path: entry.path.clone(),
                action: entry.action.label(),
            })
            .collect(),
        gitignore: gitignore_state(plan),
    }
}

/// What the `.gitignore` entry did.
fn gitignore_state(plan: &Plan) -> &'static str {
    match plan
        .entries
        .iter()
        .find(|entry| entry.path == ".gitignore")
        .map(|entry| entry.action)
    {
        Some(Action::Create) => "created",
        Some(Action::Append) => "appended",
        _ => "unchanged",
    }
}

/// The human report: identity, one row per path, what was found, then every finding.
fn report(plan: &Plan, dry_run: bool, findings: &[Finding], terminal: &mut Terminal) {
    terminal.identity("aicontext", &plan.project_name, plan.vcs_label());
    let mut rows: Vec<(String, String, String)> = plan
        .entries
        .iter()
        .map(|entry| {
            let message = match entry.action {
                Action::Keep => format!("{} — edited, preserved", entry.path),
                Action::EnsureDirectory if !dry_run => format!("{}/", entry.path),
                _ => entry.path.clone(),
            };
            (
                mark(terminal, entry.action),
                entry.action.label().to_string(),
                message,
            )
        })
        .collect();
    if plan.signals.is_empty() {
        rows.push((
            terminal.mark_info(),
            "detected".to_string(),
            "nothing found, and nothing guessed".into(),
        ));
    } else {
        let summary = detect::describe(&plan.signals).join(", ");
        rows.push((terminal.mark_info(), "detected".to_string(), summary));
    }
    for finding in findings {
        let mark = if finding.is_error() {
            terminal.mark_error()
        } else {
            terminal.mark_warn()
        };
        rows.push((
            mark,
            finding.code.clone(),
            format!("{}: {}", finding.path, finding.message),
        ));
    }
    for (mark, label, message) in rows {
        terminal.row(&mark, &label, &message);
    }
}

/// The mark for an action, redundant with the word beside it.
fn mark(terminal: &Terminal, action: Action) -> String {
    match action {
        Action::Create | Action::Append | Action::EnsureDirectory => terminal.mark_ok(),
        Action::Keep | Action::Overwrite => terminal.mark_warn(),
        Action::Unchanged => terminal.mark_info(),
    }
}

/// The counts line, which is the sentence a developer actually reads.
fn counts(plan: &Plan, dry_run: bool) -> String {
    let created = plan.with_action(Action::Create).len();
    let overwritten = plan.with_action(Action::Overwrite).len();
    let kept = plan.with_action(Action::Keep).len();
    let unchanged = plan.with_action(Action::Unchanged).len();
    let directories = plan.with_action(Action::EnsureDirectory).len();

    let verb = if dry_run { "would write" } else { "wrote" };
    let mut parts = vec![format!(
        "{verb} {}",
        plural(created, "document", "documents")
    )];
    if overwritten > 0 {
        parts.push(format!(
            "{} replaced",
            plural(overwritten, "document", "documents")
        ));
    }
    if kept > 0 {
        parts.push(format!(
            "{} preserved",
            plural(kept, "document", "documents")
        ));
    }
    if directories > 0 {
        parts.push(format!(
            "{} created",
            plural(directories, "directory", "directories")
        ));
    }
    if unchanged > 0 {
        parts.push(format!(
            "{} already current",
            plural(unchanged, "entry", "entries")
        ));
    }
    parts.join(", ")
}

/// A count with its noun agreeing. `directory` does not pluralise by appending `s`, and `1 document(s)`
/// tells a reader nothing about the grammar, so both forms are named rather than guessed.
fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// What to do next, which depends on why the run ended.
fn next_step(plan: &Plan, exit: Exit) -> &'static str {
    if exit == Exit::Validation {
        return "fix the findings above, then run `aicontext init` again";
    }
    if plan.writes() == 0 {
        return "the skeleton is current; run `aicontext doctor` to check it";
    }
    "edit .ai/RULES.md to match how you work, then run `aicontext doctor`"
}

#[cfg(test)]
mod tests {
    use super::{Action, Plan, counts, gitignore_state, next_step, run};
    use crate::args::{ColorChoice, GlobalArgs, InitArgs};
    use crate::exit::Exit;
    use crate::init::apply::apply;
    use crate::init::date::Date;
    use crate::init::plan::build;
    use crate::init::templates::TemplateName;
    use crate::output::Terminal;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn args() -> InitArgs {
        InitArgs {
            dry_run: false,
            force: false,
            no_detect: true,
            template: TemplateName::Default,
        }
    }

    fn terminal() -> Terminal {
        Terminal::sink(
            Box::new(Vec::new()),
            Box::new(Vec::new()),
            false,
            ColorChoice::Never,
        )
    }

    fn global(root: &Path) -> GlobalArgs {
        GlobalArgs {
            cwd: Some(root.to_path_buf()),
            ..GlobalArgs::default()
        }
    }

    #[test]
    fn a_dry_run_writes_nothing_and_says_so() {
        let root = TempDir::new().expect("temp dir");
        let mut terminal = terminal();
        let args = InitArgs {
            dry_run: true,
            ..args()
        };

        assert_eq!(run(&args, &global(root.path()), &mut terminal), Exit::Ok);
        assert!(
            !root.path().join(".ai").exists(),
            "a dry run must not create anything"
        );
    }

    #[test]
    fn a_full_run_creates_the_skeleton_and_the_gitignore_entry() {
        let root = TempDir::new().expect("temp dir");
        let mut terminal = terminal();

        assert_eq!(run(&args(), &global(root.path()), &mut terminal), Exit::Ok);
        assert!(root.path().join(".ai/RULES.md").is_file());
        let gitignore = fs::read_to_string(root.path().join(".gitignore")).expect("read");
        assert!(gitignore.contains(".aicontext/"), "{gitignore}");
        assert!(
            !root.path().join(".aicontext").exists(),
            "the directory must not be created"
        );
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let root = TempDir::new().expect("temp dir");
        let mut terminal = terminal();
        run(&args(), &global(root.path()), &mut terminal);

        let before = fs::read_to_string(root.path().join(".ai/RULES.md")).expect("read");
        assert_eq!(run(&args(), &global(root.path()), &mut terminal), Exit::Ok);
        assert_eq!(
            fs::read_to_string(root.path().join(".ai/RULES.md")).expect("read"),
            before
        );
    }

    #[test]
    fn an_edited_document_survives_and_is_reported() {
        let root = TempDir::new().expect("temp dir");
        let mut terminal = terminal();
        run(&args(), &global(root.path()), &mut terminal);
        fs::write(root.path().join(".ai/RULES.md"), "mine\n").expect("write");

        assert_eq!(run(&args(), &global(root.path()), &mut terminal), Exit::Ok);
        assert_eq!(
            fs::read_to_string(root.path().join(".ai/RULES.md")).expect("read"),
            "mine\n"
        );
    }

    #[test]
    fn force_in_a_pipe_refuses_rather_than_overwriting() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        fs::write(root.path().join(".ai").join("RULES.md"), "mine\n").expect("write");
        let mut terminal = terminal();
        let args = InitArgs {
            force: true,
            ..args()
        };

        assert_eq!(
            run(&args, &global(root.path()), &mut terminal),
            Exit::ApprovalRequired
        );
        assert_eq!(
            fs::read_to_string(root.path().join(".ai/RULES.md")).expect("read"),
            "mine\n"
        );
    }

    #[test]
    fn yes_does_not_satisfy_an_explicit_approval() {
        let root = TempDir::new().expect("temp dir");
        fs::create_dir_all(root.path().join(".ai")).expect("dir");
        fs::write(root.path().join(".ai/RULES.md"), "mine\n").expect("write");
        let mut terminal = terminal();
        let args = InitArgs {
            force: true,
            ..args()
        };
        let global = GlobalArgs {
            yes: true,
            ..global(root.path())
        };

        assert_eq!(run(&args, &global, &mut terminal), Exit::ApprovalRequired);
    }

    #[test]
    fn json_output_is_a_single_object_naming_the_project() {
        let root = TempDir::new().expect("temp dir");
        let mut terminal = Terminal::sink(
            Box::new(Vec::new()),
            Box::new(Vec::new()),
            false,
            ColorChoice::Never,
        );
        let global = GlobalArgs {
            json: true,
            ..global(root.path())
        };

        assert_eq!(run(&args(), &global, &mut terminal), Exit::Ok);
    }

    #[test]
    fn the_counts_line_agrees_in_number() {
        assert_eq!(super::plural(1, "document", "documents"), "1 document");
        assert_eq!(super::plural(0, "document", "documents"), "0 documents");
        assert_eq!(
            super::plural(2, "directory", "directories"),
            "2 directories",
            "`directory` does not pluralise by appending `s`"
        );
    }

    #[test]
    fn the_counts_line_never_claims_more_than_happened() {
        let root = TempDir::new().expect("temp dir");
        let plan = planned(root.path());
        assert!(counts(&plan, false).starts_with("wrote "));
        assert!(!counts(&plan, false).contains("already current"));
        assert!(counts(&plan, true).starts_with("would write "));
    }

    #[test]
    fn the_next_step_reflects_whether_anything_changed() {
        let root = TempDir::new().expect("temp dir");
        assert!(next_step(&planned(root.path()), Exit::Ok).contains("doctor"));

        apply(&planned(root.path())).expect("applies");
        let current = planned(root.path());
        assert_eq!(current.with_action(Action::Create).len(), 0);
        assert!(next_step(&current, Exit::Ok).contains("current"));
        assert!(
            !next_step(&current, Exit::Ok).contains("edit"),
            "nothing changed, so nothing to edit"
        );
        assert_eq!(gitignore_state(&current), "unchanged");
        assert!(next_step(&current, Exit::Validation).contains("fix the findings"));
    }

    fn planned(root: &Path) -> Plan {
        build(
            root.to_path_buf(),
            TemplateName::Default,
            Date::from_days_since_epoch(20_269),
            false,
            false,
        )
        .expect("plans")
    }
}
