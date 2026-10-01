//! The two output surfaces of the CLI (`docs/CLI_SPEC.md` §1, §6, and `.ai/DESIGN.md` §6).
//!
//! stdout carries data: a single JSON object under `--json`, human text otherwise. stderr carries
//! diagnostics, progress, and the error report. With `--json` the human text still goes to stderr,
//! so a pipe into `jq` is never polluted.
//!
//! Colour is opt-out (`--no-color`, `NO_COLOR`, a non-TTY stdout, `TERM=dumb`) and always
//! redundant with the glyph and the severity word, so a reader who cannot see colour loses nothing.

use std::io::{self, IsTerminal, Write};

use serde::Serialize;

/// Version of the JSON envelope shape. It increments only on a breaking change (`CLI_SPEC` §6).
pub(crate) const ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// One finding about the context tree, in the shape `CLI_SPEC` §6 publishes.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Finding {
    /// Stable machine code, safe to branch on. `CTX-` codes are the documented catalogue.
    pub(crate) code: String,
    /// `error`, `warning`, or `info`. Matches `aicontext_core::Severity`.
    pub(crate) severity: String,
    /// Repository-relative path, always with `/` separators so it is stable across platforms.
    pub(crate) path: String,
    /// For humans, and may be reworded in a later version.
    pub(crate) message: String,
    /// What to do about it.
    pub(crate) remediation: String,
}

impl Finding {
    /// Builds a finding from a `CTX-` code and a severity the catalogue assigns.
    pub(crate) fn new(
        code: impl Into<String>,
        severity: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
        remediation: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: severity.into(),
            path: path.into(),
            message: message.into(),
            remediation: remediation.into(),
        }
    }

    /// Whether this finding counts as an error, which is what `doctor` and `init` exit 3 on.
    pub(crate) fn is_error(&self) -> bool {
        self.severity == "error"
    }
}

/// The single JSON object written to stdout under `--json` (`CLI_SPEC` §6).
#[derive(Debug, Serialize)]
pub(crate) struct Envelope<T: Serialize> {
    /// Shape version; see [`ENVELOPE_SCHEMA_VERSION`].
    schema_version: u32,
    /// The command that produced it, for example `init`.
    command: &'static str,
    /// Whether the run met its contract.
    ok: bool,
    /// The same code the process exits with, so a caller reading the body need not wait for it.
    exit_code: u8,
    /// Command-specific payload.
    data: T,
    /// Every finding, error-level and warning-level alike. Scripts filter on `code`.
    findings: Vec<Finding>,
    /// The warning-level codes alone, so a script can test for warnings without walking `findings`.
    warnings: Vec<String>,
    /// One line for a human: counts and outcome.
    summary: String,
}

/// Builds an [`Envelope`] with the derived fields already filled in.
pub(crate) fn envelope<T: Serialize>(
    command: &'static str,
    exit_code: u8,
    data: T,
    findings: Vec<Finding>,
    summary: String,
) -> Envelope<T> {
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == "warning")
        .map(|finding| finding.code.clone())
        .collect();
    Envelope {
        schema_version: ENVELOPE_SCHEMA_VERSION,
        command,
        ok: exit_code == 0,
        exit_code,
        data,
        findings,
        warnings,
        summary,
    }
}

/// How much of the human report to print.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Verbosity {
    /// Everything, including per-file rows.
    Normal,
    /// Diagnostics only (`--quiet`).
    Quiet,
}

/// Where output goes, and whether colour is allowed.
pub(crate) struct Terminal {
    stdout: Box<dyn Write>,
    stderr: Box<dyn Write>,
    style: Style,
    verbosity: Verbosity,
    glyphs: Glyphs,
    is_terminal: bool,
}

impl Terminal {
    /// Wires a terminal to the process's own streams.
    pub(crate) fn process(verbosity: Verbosity, color: crate::args::ColorChoice) -> Self {
        let is_terminal = io::stdout().is_terminal();
        Self {
            stdout: Box::new(io::stdout()),
            stderr: Box::new(io::stderr()),
            style: Style::decide(color, is_terminal),
            verbosity,
            glyphs: Glyphs::decide(is_terminal),
            is_terminal,
        }
    }

    /// Wires a terminal to in-memory sinks, so a renderer can be asserted on directly.
    pub(crate) fn sink(
        stdout: Box<dyn Write>,
        stderr: Box<dyn Write>,
        is_terminal: bool,
        color: crate::args::ColorChoice,
    ) -> Self {
        Self {
            stdout,
            stderr,
            style: Style::decide(color, is_terminal),
            verbosity: Verbosity::Normal,
            glyphs: Glyphs::decide(is_terminal),
            is_terminal,
        }
    }

    /// Whether anything was written to a real terminal, which `--force` needs in order to prompt.
    pub(crate) fn is_terminal(&self) -> bool {
        self.is_terminal
    }

    /// Whether rows are printed at all under the current verbosity.
    pub(crate) fn shows_rows(&self) -> bool {
        self.verbosity == Verbosity::Normal
    }

    /// One line of human output. stdout when it is the result, stderr when it is a diagnostic.
    pub(crate) fn say(&mut self, line: &str) {
        let _ = writeln!(self.stdout, "{line}");
    }

    /// A diagnostic. Never stdout, so `--json` output stays a single parsable object.
    pub(crate) fn note(&mut self, line: &str) {
        let _ = writeln!(self.stderr, "{line}");
    }

    /// The identity line, first on every run (`.ai/DESIGN.md` §6).
    pub(crate) fn identity(&mut self, product: &str, project: &str, vcs: &str) {
        let line = format!(
            "{}  {}  {}",
            self.style.dim(product),
            self.style.bold(project),
            self.style.dim(vcs)
        );
        self.say(&line);
        self.say("");
    }

    /// A column-aligned row: a glyph, a fixed-width state word, then the message.
    pub(crate) fn row(&mut self, glyph: &str, state: &str, message: &str) {
        if !self.shows_rows() {
            return;
        }
        let padded = format!("{state:<10}");
        self.say(&format!("  {glyph} {padded}{message}"));
    }

    /// The closing line: counts, the exit code, and the next command.
    pub(crate) fn summary(&mut self, counts: &str, exit: crate::exit::Exit, next: &str) {
        let code = self.style.dim(&format!("exit {}", exit.code()));
        let line = if counts.is_empty() {
            format!("{code}")
        } else {
            format!("{counts}  {code}")
        };
        self.say("");
        self.say(&line);
        if !next.is_empty() {
            self.say(&format!("next: {next}"));
        }
    }

    /// The error report: what happened, what to do about it, printed exactly once (`RULES.md` §4.5).
    pub(crate) fn error_report(&mut self, code: &str, message: &str, remediation: &str) {
        let head = self.style.error(code);
        let _ = writeln!(self.stderr, "{head}: {message}");
        if !remediation.is_empty() {
            let _ = writeln!(self.stderr, "  try: {remediation}");
        }
    }

    /// Writes one JSON object to stdout and nothing else.
    pub(crate) fn json(&mut self, value: &impl Serialize) -> Result<(), serde_json::Error> {
        let text = serde_json::to_string_pretty(value)?;
        self.say(&text);
        Ok(())
    }
}

/// The three glyphs the report uses, in UTF-8 and in the ASCII fallback.
///
/// The fallback exists because the severity word beside the glyph already carries the meaning, so
/// a terminal that cannot render `✓` loses no information (`docs/CLI_SPEC.md` §7).
#[derive(Clone, Copy, Debug)]
struct Glyphs {
    ok: &'static str,
    warn: &'static str,
    error: &'static str,
    info: &'static str,
}

impl Glyphs {
    const fn decide(is_terminal: bool) -> Self {
        if is_terminal {
            Self {
                ok: "\u{2713}",
                warn: "\u{26a0}",
                error: "\u{2717}",
                info: "i",
            }
        } else {
            Self {
                ok: "+",
                warn: "!",
                error: "x",
                info: "i",
            }
        }
    }

    const fn ok(self) -> &'static str {
        self.ok
    }
    const fn warn(self) -> &'static str {
        self.warn
    }
    const fn error(self) -> &'static str {
        self.error
    }
    const fn info(self) -> &'static str {
        self.info
    }
}

/// Whether ANSI attributes may be emitted.
#[derive(Clone, Copy, Debug)]
struct Style {
    enabled: bool,
}

impl Style {
    /// Resolves `--color`, `NO_COLOR`, `TERM=dumb`, and a non-TTY stdout into one answer.
    ///
    /// Environment variables are read here and nowhere else, so the decision is testable and a
    /// command never consults the ambient environment for anything else.
    fn decide(choice: crate::args::ColorChoice, is_terminal: bool) -> Self {
        let enabled = match choice {
            crate::args::ColorChoice::Always => true,
            crate::args::ColorChoice::Never => false,
            crate::args::ColorChoice::Auto => {
                is_terminal
                    && std::env::var_os("NO_COLOR").is_none()
                    && std::env::var_os("AICON_TEXT_NO_COLOR").is_none()
                    && std::env::var("TERM").as_deref() != Ok("dumb")
            }
        };
        Self { enabled }
    }

    fn dim(self, text: &str) -> String {
        self.wrap(text, "2")
    }

    fn bold(self, text: &str) -> String {
        self.wrap(text, "1")
    }

    fn error(self, text: &str) -> String {
        self.wrap(text, "1;31")
    }

    fn ok(self, text: &str) -> String {
        self.wrap(text, "32")
    }

    fn warn(self, text: &str) -> String {
        self.wrap(text, "33")
    }

    fn wrap(self, text: &str, code: &str) -> String {
        if self.enabled {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Envelope, Finding, Glyphs, Style, envelope};
    use crate::args::ColorChoice;

    #[test]
    fn an_envelope_carries_the_published_fields() {
        let findings = vec![
            Finding::new("CTX-002", "error", ".ai/RULES.md", "bad block", "fix it"),
            Finding::new("CTX-006", "warning", ".ai/PRD.md", "unknown key", "remove it"),
        ];
        let body = envelope(
            "init",
            3,
            serde_json::json!({ "created": [] }),
            findings,
            "1 error, 1 warning".to_string(),
        );
        let text = serde_json::to_value(&body).expect("serialises");
        assert_eq!(text["schema_version"], 1);
        assert_eq!(text["command"], "init");
        assert_eq!(text["ok"], false);
        assert_eq!(text["exit_code"], 3);
        assert_eq!(text["summary"], "1 error, 1 warning");
        assert_eq!(text["findings"].as_array().map(Vec::len), Some(2));
        assert_eq!(text["warnings"][0], "CTX-006");
    }

    #[test]
    fn findings_carry_a_code_a_script_can_branch_on() {
        let text = serde_json::to_value(Envelope {
            schema_version: 1,
            command: "init",
            ok: true,
            exit_code: 0,
            data: serde_json::json!({}),
            findings: vec![Finding::new(
                "CTX-018",
                "warning",
                ".ai/TASKS.md",
                "document is larger than the maximum",
                "split it",
            )],
            warnings: vec!["CTX-018".to_string()],
            summary: "ok".to_string(),
        })
        .expect("serialises");
        let first = &text["findings"][0];
        for key in ["code", "severity", "path", "message", "remediation"] {
            assert!(first.get(key).is_some(), "{key} is missing from a finding");
        }
    }

    #[test]
    fn colour_is_off_unless_it_was_asked_for() {
        assert!(!Style::decide(ColorChoice::Never, true).enabled);
        assert!(Style::decide(ColorChoice::Always, false).enabled);
    }

    #[test]
    fn glyphs_degrade_when_stdout_is_not_a_terminal() {
        assert_eq!(Glyphs::decide(true).ok(), "\u{2713}");
        assert_eq!(Glyphs::decide(false).ok(), "+");
        assert_eq!(Glyphs::decide(false).error(), "x");
        assert_eq!(Glyphs::decide(false).warn(), "!");
    }
}