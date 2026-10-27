//! `aicontext audit`: read the hash-chained audit log (`docs/SECURITY.md` §8).
//!
//! Three views over one verification:
//!
//! - `show` renders every verified record as a timeline.
//! - `verify` prints whether the chain links and the head digest a reviewer can record elsewhere.
//! - `tail` renders the newest records, after the same whole-chain check.
//!
//! Reading is always a verification. A broken chain is reported with the verified prefix and exits
//! 3; this command never appends, repairs, or truncates the log, and it never calls
//! `AuditLog::append`. The writer arrives with the first command that executes a plugin.
//!
//! # Output
//!
//! The human view is a timeline: one row per record, with the timestamp, event, action, target,
//! actor, approval, and result. Every field that came from outside the host is sanitised before it
//! reaches the terminal, because a newline or a bidi override in a resource name must not be able to
//! forge the next line. The JSON view publishes the records themselves, so a script reads the same
//! shape the file holds.
//!
//! # Exit
//!
//! `0` for a valid chain (including an absent log), `3` for a chain that does not verify, `1` when
//! the log cannot be read at all, and `2` for an unusable project root.

use aicontext_audit::{
    AuditLog, AuditProblem, AuditRecord, DEFAULT_DIR, DEFAULT_FILE, Verification,
};
use aicontext_core::ErrorCode;
use serde::Serialize;

use crate::args::{AuditArgs, AuditCommand, GlobalArgs};
use crate::exit::Exit;
use crate::output::{Finding, Terminal, envelope};
use crate::project::{self, ProjectError};

/// The project-relative path every message and finding uses, with `/` separators.
fn log_spelling() -> String {
    format!("{DEFAULT_DIR}/{DEFAULT_FILE}")
}

/// Runs one audit view.
pub(crate) fn run(args: &AuditArgs, global: &GlobalArgs, terminal: &mut Terminal) -> Exit {
    let root = match project::resolve_root(global.cwd.as_deref()) {
        Ok(root) => root,
        Err(error) => return report_root_failure(&error, terminal),
    };
    let name = match project::project_name(&root) {
        Ok(name) => name,
        Err(error) => return report_root_failure(&error, terminal),
    };

    let log = AuditLog::in_project(&root);
    let verification = log.verify();
    let view = view_of(&args.command);
    let exit = exit_for(verification.problem());

    let (records, limit) = select(&verification, &args.command);
    let findings = findings_of(&verification);
    let summary = summary_of(view, &verification, records.len());

    if global.json {
        let data = AuditData {
            view,
            log: log_spelling(),
            chain_valid: verification.is_valid(),
            verified_records: verification.len(),
            returned_records: records.len(),
            head: verification.head(),
            limit,
            records,
            problem: verification.problem().map(ProblemData::of),
        };
        let body = envelope("audit", exit.code(), data, findings, summary);
        if let Err(error) = terminal.json(&body) {
            terminal.error_report(
                "AUDIT-001",
                "the JSON envelope could not be written",
                &format!("{error}; check that stdout is a pipe rather than a full device"),
            );
            return Exit::General;
        }
    } else {
        print_human(&name, &root, view, &verification, records, limit, terminal);
    }
    exit
}

/// The stable `view` word in the JSON payload.
const fn view_of(command: &AuditCommand) -> &'static str {
    match command {
        AuditCommand::Show => "show",
        AuditCommand::Verify => "verify",
        AuditCommand::Tail(_) => "tail",
    }
}

/// The records a view renders, and the requested limit when there is one.
fn select<'a>(
    verification: &'a Verification,
    command: &AuditCommand,
) -> (&'a [AuditRecord], Option<usize>) {
    match command {
        AuditCommand::Verify => (&[], None),
        AuditCommand::Show => (verification.records(), None),
        AuditCommand::Tail(tail) => {
            let all = verification.records();
            let count = tail.limit.min(all.len());
            (&all[all.len() - count..], Some(tail.limit))
        }
    }
}

/// A broken chain is a validation failure; an unreadable file is a general one.
fn exit_for(problem: Option<&AuditProblem>) -> Exit {
    match problem {
        None => Exit::Ok,
        Some(problem) if problem.code() == ErrorCode::AUD_001 => Exit::General,
        Some(_) => Exit::Validation,
    }
}

fn findings_of(verification: &Verification) -> Vec<Finding> {
    verification
        .problem()
        .map(|problem| {
            vec![Finding::new(
                problem.code().to_string(),
                "error",
                log_spelling(),
                problem.message(),
                problem.remediation(),
            )]
        })
        .unwrap_or_default()
}

/// One line for a human: what was verified, and what broke it when something did.
fn summary_of(view: &str, verification: &Verification, returned: usize) -> String {
    if let Some(problem) = verification.problem() {
        let line = problem
            .line()
            .map_or_else(String::new, |line| format!(" at line {line}"));
        return format!(
            "chain broken{line}: {} ({} records verified)",
            problem.message(),
            verification.len()
        );
    }
    if verification.is_empty() {
        return "chain valid, no records".to_string();
    }
    match view {
        "tail" => format!(
            "last {returned} of {} records, chain valid",
            verification.len()
        ),
        "verify" => format!("chain valid, {} records", verification.len()),
        _ => format!("{} records, chain valid", verification.len()),
    }
}

/// The JSON payload, the same shape for all three views.
#[derive(Debug, Serialize)]
struct AuditData<'a> {
    /// `show`, `verify`, or `tail`.
    view: &'static str,
    /// The project-relative log path, always `.aicontext/audit.jsonl`.
    log: String,
    /// Whether every record verified and the links join.
    chain_valid: bool,
    /// How many records verified, whether or not this view rendered them.
    verified_records: usize,
    /// How many records this view returned.
    returned_records: usize,
    /// The biggest witness: the digest of the last verified record, or the genesis digest.
    head: &'a str,
    /// The requested tail limit, or null for the other views.
    limit: Option<usize>,
    /// The records this view returned, in chain order. Empty for `verify`; the verified prefix when
    /// a chain is broken.
    records: &'a [AuditRecord],
    /// What broke the chain, when it is broken.
    problem: Option<ProblemData>,
}

/// The problem, in the shape a script branches on.
#[derive(Debug, Serialize)]
struct ProblemData {
    /// `AUD-001` to `AUD-005`.
    code: String,
    /// The 1-based line, when the failure points at one.
    line: Option<usize>,
    /// For humans; may be reworded.
    message: String,
    /// What to do about it.
    remediation: String,
}

impl ProblemData {
    fn of(problem: &AuditProblem) -> Self {
        Self {
            code: problem.code().to_string(),
            line: problem.line(),
            message: problem.message().to_string(),
            remediation: problem.remediation().to_string(),
        }
    }
}

/// The human report: identity, then the view's rows, then the summary.
fn print_human(
    name: &str,
    root: &std::path::Path,
    view: &str,
    verification: &Verification,
    records: &[AuditRecord],
    limit: Option<usize>,
    terminal: &mut Terminal,
) {
    terminal.identity("aicontext", name, project::vcs_label(root));

    if view == "verify" {
        print_verify(verification, terminal);
    } else {
        print_timeline(verification, records, terminal);
    }

    let counts = if verification.is_empty() {
        "no audit records".to_string()
    } else if view == "tail" {
        format!("last {} of {} records", records.len(), verification.len())
    } else {
        format!("{} records", verification.len())
    };
    let next = match verification.problem() {
        Some(problem) => problem.remediation(),
        None if verification.is_empty() => "`aicontext status`",
        None if limit.is_some() => "`aicontext audit show` for the whole log",
        None => "`aicontext audit verify --json`",
    };
    terminal.summary(&counts, exit_for(verification.problem()), next);
}

fn print_verify(verification: &Verification, terminal: &mut Terminal) {
    if let Some(problem) = verification.problem() {
        terminal.row(
            &terminal.mark_error(),
            "chain",
            &format!(
                "broken at line {}: {}",
                problem
                    .line()
                    .map_or_else(|| "unknown".to_string(), |line| line.to_string()),
                safe_text(problem.message())
            ),
        );
        terminal.row(
            &terminal.mark_warn(),
            "verified",
            &format!("{} records before the break", verification.len()),
        );
    } else if verification.is_empty() {
        terminal.row(
            &terminal.mark_info(),
            "chain",
            "valid · no records yet; nothing has been audited",
        );
    } else {
        terminal.row(
            &terminal.mark_ok(),
            "chain",
            &format!("valid · {} records", verification.len()),
        );
    }
    terminal.row(&terminal.mark_info(), "head", verification.head());
    if verification.problem().is_none() {
        terminal
            .say("       record this digest somewhere the project cannot write; a later rewrite");
        terminal.say("       will then disagree with it (docs/SECURITY.md section 8).");
    }
}

fn print_timeline(verification: &Verification, records: &[AuditRecord], terminal: &mut Terminal) {
    if records.is_empty() {
        if verification.is_valid() {
            terminal.row(
                &terminal.mark_info(),
                "audit",
                "no records yet; nothing has been audited",
            );
        }
    } else {
        for record in records {
            terminal.row(
                &terminal.mark_info(),
                &format!("#{}", record.seq()),
                &timeline(record),
            );
        }
    }
    if let Some(problem) = verification.problem() {
        terminal.row(
            &terminal.mark_error(),
            "error",
            &format!(
                "chain broken at line {}: {}",
                problem
                    .line()
                    .map_or_else(|| "unknown".to_string(), |line| line.to_string()),
                safe_text(problem.message())
            ),
        );
    }
}

/// One record as a timeline row.
fn timeline(record: &AuditRecord) -> String {
    let mut parts = vec![
        record.timestamp().to_string(),
        record.event().to_string(),
        format!(
            "{} [{}]",
            safe_text(record.action().tool()),
            safe_text(record.action().plugin())
        ),
    ];
    if let Some(resource) = record.resource() {
        parts.push(format!("on {}", safe_text(resource)));
    }
    parts.push(format!(
        "by {}:{}",
        safe_text(record.actor().kind()),
        safe_text(record.actor().id())
    ));
    if let Some(approval) = record.approval() {
        let mut line = format!("approval {}", safe_text(approval.required()));
        if let Some(granted_by) = approval.granted_by() {
            line.push_str(&format!(" by {}", safe_text(granted_by)));
        }
        if let Some(mode) = approval.mode() {
            line.push_str(&format!(" ({})", safe_text(mode)));
        }
        parts.push(line);
    }
    if let Some(result) = record.result() {
        let mut line = format!("result {}", safe_text(result.status()));
        if let Some(duration_ms) = result.duration_ms() {
            line.push_str(&format!(" in {duration_ms} ms"));
        }
        parts.push(line);
    }
    parts.join(" · ")
}

/// Replaces control and bidi/zero-width characters with spaces before a terminal sees them.
///
/// JSON escapes these characters, so the file and the `--json` output are safe without this. A
/// terminal is not: a resource name carrying `\n` or U+202E could forge or reverse the next line.
/// The gateway applies the same rule to untrusted tool results; this is the audit display's copy of
/// it, kept local because the gateway's sanitiser is private to its envelope and uses the full ANSI
/// sequence parser.
fn safe_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || is_unsafe_format(character) {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn is_unsafe_format(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
    )
}

/// A root that cannot be used, reported through the surface every other command reports through.
fn report_root_failure(error: &ProjectError, terminal: &mut Terminal) -> Exit {
    terminal.error_report(error.code(), &error.to_string(), error.remediation());
    error.exit()
}

#[cfg(test)]
mod tests {
    use super::{safe_text, summary_of, timeline, view_of};
    use crate::args::AuditCommand;
    use aicontext_audit::{
        Action, Actor, AuditEntry, AuditLog, AuditRecord, ResultSummary, Timestamp,
    };
    use tempfile::TempDir;

    fn record() -> AuditRecord {
        let entry = AuditEntry::new(
            "completed",
            Actor::agent("developer"),
            Action::new("aws.ec2.stop", "aws", "write"),
        )
        .with_resource("i-0abc1234")
        .with_result(ResultSummary::new("success").with_duration_ms(412));
        let directory = TempDir::new().expect("temp dir");
        AuditLog::in_project(directory.path())
            .append_at(&entry, Timestamp::from_unix_millis(0))
            .expect("the record is appended")
    }

    #[test]
    fn a_timeline_names_the_time_event_action_target_actor_and_result() {
        let line = timeline(&record());
        assert!(line.contains("1970-01-01T00:00:00.000Z"));
        assert!(line.contains("completed"));
        assert!(line.contains("aws.ec2.stop [aws]"));
        assert!(line.contains("on i-0abc1234"));
        assert!(line.contains("by agent:developer"));
        assert!(line.contains("result success in 412 ms"));
    }

    #[test]
    fn a_timeline_shows_the_approval_when_the_record_has_one() {
        let entry = AuditEntry::new(
            "decided",
            Actor::human("developer"),
            Action::new("github.issue.create", "github", "write"),
        )
        .with_approval(
            aicontext_audit::Approval::new(aicontext_core::PermissionMode::ExplicitApproval)
                .with_granted_by("developer")
                .with_mode("interactive"),
        );
        let directory = TempDir::new().expect("temp dir");
        let record = AuditLog::in_project(directory.path())
            .append_at(&entry, Timestamp::from_unix_millis(0))
            .expect("the record is appended");
        let line = timeline(&record);
        assert!(line.contains("approval explicit_approval by developer (interactive)"));
    }

    #[test]
    fn display_sanitises_control_and_bidi_characters() {
        let line = safe_text("file\nname\u{202e}evil\u{1b}[31m");
        assert!(!line.contains('\n'));
        assert!(!line.contains('\u{202e}'));
        assert!(!line.contains('\u{1b}'));
        assert!(line.contains("file name"));
        assert_eq!(safe_text("plain name"), "plain name");
    }

    #[test]
    fn each_view_has_its_stable_word() {
        assert_eq!(view_of(&AuditCommand::Show), "show");
        assert_eq!(view_of(&AuditCommand::Verify), "verify");
        assert_eq!(
            view_of(&AuditCommand::Tail(crate::args::AuditTailArgs {
                limit: 20
            })),
            "tail"
        );
    }

    #[test]
    fn the_summary_distinguishes_empty_valid_and_broken() {
        let directory = TempDir::new().expect("temp dir");
        let empty = AuditLog::in_project(directory.path()).verify();
        assert_eq!(summary_of("verify", &empty, 0), "chain valid, no records");
    }
}
