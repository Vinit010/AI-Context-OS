//! The terminal side of the approval contract.
//!
//! `aicontext-plugin-runtime` decides; this module is the only place that touches a human: it
//! writes the rendered prompt to stderr, reads one line from stdin, and reports exactly what
//! happened. The decision matrix lives in the broker, so there is one place where `yes` is
//! interpreted and one place where a missing answer becomes a denial.
//!
//! The prompt always goes to stderr (`docs/CLI_SPEC.md` §1: stdout is data), and it is printed even
//! when stdin is not a terminal, because `docs/SECURITY.md` §9 requires the text to be visible in
//! the denial. A non-interactive call never reads stdin: a pipe is not a human.
//!
//! `TASK-074` wires this into the tool gateway. Until then its tests exercise it directly, which is
//! why the module-level dead-code allowance is honest rather than hiding an unused production path.
#![allow(dead_code)]

use std::io::{self, BufRead, IsTerminal, Read, Write};

use aicontext_plugin_runtime::approval::{PromptError, Prompter};

/// The longest answer a prompt will read.
///
/// `yes` is three bytes. The limit leaves room for whitespace and a stray paste but refuses an
/// unbounded line from a misbehaving pipe, which would otherwise be an unbounded read
/// (`RULES.md` §11). A longer answer is a denial, never a truncated `yes`.
const MAX_ANSWER_BYTES: usize = 64;

/// A prompt on the process's terminal.
///
/// The reader and writer are generic so a test can drive the exact bytes a human would type and
/// assert the exact bytes a human would see. `interactive` is decided once, from stdin and stderr
/// both being terminals: stdout may be redirected without losing the ability to ask.
#[derive(Debug)]
pub(crate) struct TerminalPrompter<R, W> {
    input: R,
    output: W,
    interactive: bool,
}

impl TerminalPrompter<io::StdinLock<'static>, io::Stderr> {
    /// Wires a prompter to the process's stdin and stderr.
    #[must_use]
    pub(crate) fn process() -> Self {
        Self {
            input: io::stdin().lock(),
            output: io::stderr(),
            interactive: io::stdin().is_terminal() && io::stderr().is_terminal(),
        }
    }
}

impl<R: BufRead, W: Write> TerminalPrompter<R, W> {
    /// Builds a prompter over an arbitrary reader and writer.
    ///
    /// `interactive` is explicit rather than detected from `R`/`W`, because a `Cursor` in a test is
    /// not a terminal and because the decision must not depend on a platform-specific file type.
    #[must_use]
    pub(crate) fn new(input: R, output: W, interactive: bool) -> Self {
        Self {
            input,
            output,
            interactive,
        }
    }
}

impl<R: BufRead, W: Write> Prompter for TerminalPrompter<R, W> {
    fn ask(&mut self, prompt: &str) -> Result<String, PromptError> {
        // The prompt is shown first in every case. A pipe still gets the text, which is what makes
        // the exit-5 refusal explain itself instead of failing silently.
        self.output
            .write_all(prompt.as_bytes())
            .map_err(|source| PromptError::Io { source })?;
        self.output
            .flush()
            .map_err(|source| PromptError::Io { source })?;

        if !self.interactive {
            self.output
                .write_all(b"\n")
                .map_err(|source| PromptError::Io { source })?;
            self.output
                .flush()
                .map_err(|source| PromptError::Io { source })?;
            return Err(PromptError::Unavailable);
        }

        let mut answer = String::new();
        let read = self
            .input
            .by_ref()
            .take((MAX_ANSWER_BYTES + 1) as u64)
            .read_line(&mut answer);
        match read {
            Ok(0) => Err(PromptError::EndOfInput),
            Ok(_) if answer.len() > MAX_ANSWER_BYTES => Err(PromptError::AnswerTooLong),
            Ok(_) => Ok(answer),
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {
                Err(PromptError::Interrupted)
            }
            Err(source) => Err(PromptError::Io { source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::io::{self, Cursor, Write};

    use aicontext_core::PermissionMode;
    use aicontext_plugin_runtime::approval::{
        ApprovalAudit, ApprovalBroker, ApprovalEvent, ApprovalRequest, PromptError, Prompter,
    };
    use serde_json::json;

    use super::{MAX_ANSWER_BYTES, TerminalPrompter};

    const PROMPT: &str = "Approval required.\n> ";

    fn request() -> ApprovalRequest {
        ApprovalRequest::new(
            "aws.ec2.stop",
            "aws",
            "i-0abc1234",
            json!({ "instance_id": "i-0abc1234" }),
            PermissionMode::Approval,
        )
    }

    #[test]
    fn an_interactive_answer_is_returned_with_its_newline() {
        let mut prompter = TerminalPrompter::new(Cursor::new(b"yes\n".to_vec()), Vec::new(), true);

        let answer = prompter.ask(PROMPT).expect("the cursor has a line");

        assert_eq!(answer, "yes\n");
        assert_eq!(prompter.output, PROMPT.as_bytes().to_vec());
    }

    #[test]
    fn a_pipe_prints_the_prompt_and_denies_without_reading() {
        let input = Cursor::new(b"yes\n".to_vec());
        let mut prompter = TerminalPrompter::new(input, Vec::new(), false);

        let error = prompter.ask(PROMPT).expect_err("a pipe is not a human");

        assert!(matches!(error, PromptError::Unavailable));
        assert_eq!(
            prompter.output,
            format!("{PROMPT}\n").into_bytes(),
            "the prompt must be readable in the refusal"
        );
        assert_eq!(
            prompter.input.position(),
            0,
            "a non-interactive call must never consume stdin"
        );
    }

    #[test]
    fn end_of_input_is_not_an_answer() {
        let mut prompter = TerminalPrompter::new(Cursor::new(Vec::new()), Vec::new(), true);

        assert!(matches!(prompter.ask(PROMPT), Err(PromptError::EndOfInput)));
    }

    #[test]
    fn an_over_long_answer_is_refused_rather_than_truncated() {
        let mut input = vec![b'y'; MAX_ANSWER_BYTES + 1];
        input.push(b'\n');
        let mut prompter = TerminalPrompter::new(Cursor::new(input), Vec::new(), true);

        assert!(matches!(
            prompter.ask(PROMPT),
            Err(PromptError::AnswerTooLong)
        ));
    }

    #[test]
    fn a_write_failure_is_reported() {
        let mut prompter = TerminalPrompter::new(Cursor::new(b"yes\n".to_vec()), Failing, true);

        assert!(matches!(prompter.ask(PROMPT), Err(PromptError::Io { .. })));
    }

    #[test]
    fn only_the_literal_word_yes_approves_through_the_terminal() {
        #[derive(Debug)]
        struct Silent;

        impl ApprovalAudit for Silent {
            type Error = Infallible;

            fn record(&mut self, _event: &ApprovalEvent) -> Result<(), Self::Error> {
                Ok(())
            }
        }

        for (answer, allowed) in [
            ("yes\n", true),
            (" yes \n", true),
            ("no\n", false),
            ("YES\n", false),
            ("y\n", false),
        ] {
            let prompter =
                TerminalPrompter::new(Cursor::new(answer.as_bytes().to_vec()), Vec::new(), true);
            let mut broker = ApprovalBroker::new(prompter, Silent, []);
            let outcome = broker
                .authorize(&request())
                .expect("the silent audit cannot fail");

            assert_eq!(outcome.is_allowed(), allowed, "{answer:?}");
        }
    }

    #[derive(Debug)]
    struct Failing;

    impl Write for Failing {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
    }
}
