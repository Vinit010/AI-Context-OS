//! Rendering the approval prompt and the seam that shows it.
//!
//! The prompt is plain text on stderr, not a full-screen TUI (`Q-4`, `MEM-039`). Plain text is
//! what lets a redirected run print the prompt and deny, and it is what lets the tests assert the
//! exact bytes a human sees. The rendering escapes every caller-controlled field with JSON rules,
//! so a newline or an ANSI escape in an argument or reason cannot forge another prompt line.
//!
//! Only the literal answer [`APPROVAL_ANSWER`] approves, after surrounding whitespace is removed.
//! Everything else - including `y`, `YES`, and a blank line - is a denial.

use super::request::{ApprovalRequest, push_hex_byte};

/// The one answer that approves a call.
pub const APPROVAL_ANSWER: &str = "yes";

/// Shows a prompt and returns one line of the human's answer.
///
/// The implementation owns all terminal I/O. A non-interactive implementation returns
/// [`PromptError::Unavailable`] after showing the prompt text, because `docs/SECURITY.md` §9 prints
/// the prompt even when no human can answer it.
pub trait Prompter {
    /// Shows `prompt` and returns one line, without the trailing newline requirement: the broker
    /// trims before comparing.
    ///
    /// # Errors
    ///
    /// [`PromptError`] describes why no answer was obtained. Every variant is treated as a denial.
    fn ask(&mut self, prompt: &str) -> Result<String, PromptError>;
}

/// Why a prompt produced no usable answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PromptError {
    /// There is no interactive terminal to read an answer from.
    #[error("no interactive terminal is available for the approval prompt")]
    Unavailable,
    /// The input reached end of file before a line was read.
    #[error("the approval prompt reached end of input")]
    EndOfInput,
    /// The prompt was interrupted.
    #[error("the approval prompt was interrupted")]
    Interrupted,
    /// The prompter's deadline expired before an answer arrived.
    #[error("the approval prompt timed out")]
    TimedOut,
    /// The answer was longer than the prompt accepts.
    #[error("the approval answer was longer than the prompt accepts")]
    AnswerTooLong,
    /// Writing the prompt or reading the answer failed.
    #[error("the approval prompt could not be read or written: {source}")]
    Io {
        /// The underlying terminal failure.
        #[source]
        source: std::io::Error,
    },
}

/// Renders the exact text a human sees before an approval decision.
///
/// The order is fixed by `docs/PLUGIN_SPEC.md` §6: tool, plugin, target, arguments, reason, mode.
/// The arguments are the compact JSON form, which is also the bytes the audit digest covers.
#[must_use]
pub fn render_prompt(request: &ApprovalRequest) -> String {
    let mut prompt = String::from("Approval required.\n");
    push_field(&mut prompt, "tool", request.tool());
    push_field(&mut prompt, "plugin", request.plugin());
    push_field(&mut prompt, "target", request.target());
    push_field(&mut prompt, "arguments", &request.arguments().to_string());
    push_field(&mut prompt, "reason", request.reason());
    push_field(&mut prompt, "mode", request.mode().as_str());
    prompt.push_str(
        "\nType `yes` to allow this call once; anything else denies.\n\
         > ",
    );
    prompt
}

/// Whether `answer` is the literal approval word.
///
/// Whitespace around the word is the newline the terminal appended and is not meaningful; case,
/// punctuation, and every other word are. A loop that matched a prefix would let `yessir` approve.
pub(crate) fn is_approval_answer(answer: &str) -> bool {
    answer.trim() == APPROVAL_ANSWER
}

/// One `  label:     "escaped value"` line.
fn push_field(prompt: &mut String, label: &str, value: &str) {
    prompt.push_str("  ");
    prompt.push_str(label);
    prompt.push(':');
    // The labels are fixed, so padding only has to line up the one-character difference between
    // `tool` and `arguments`. A fixed width keeps the prompt readable without measuring strings.
    for _ in label.len()..9 {
        prompt.push(' ');
    }
    prompt.push(' ');
    push_quoted(prompt, value);
    prompt.push('\n');
}

/// JSON-style quoting for one field.
fn push_quoted(text: &mut String, value: &str) {
    text.push('"');
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            '\u{08}' => text.push_str("\\b"),
            '\u{0c}' => text.push_str("\\f"),
            control if control.is_control() => push_hex_escape(text, u32::from(control)),
            other => text.push(other),
        }
    }
    text.push('"');
}

/// One `\uNNNN` escape for a control character.
///
/// JSON's four-digit form is enough for every character `char::is_control` returns; the C1 range
/// ends at `U+009F`.
fn push_hex_escape(text: &mut String, code: u32) {
    text.push_str("\\u");
    for shift in [12, 8, 4, 0] {
        let nibble = ((code >> shift) & 0x0f) as u8;
        push_hex_byte(text, nibble);
    }
}

#[cfg(test)]
mod tests {
    use aicontext_core::PermissionMode;
    use serde_json::json;

    use super::{APPROVAL_ANSWER, is_approval_answer, render_prompt};
    use crate::approval::ApprovalRequest;

    fn request() -> ApprovalRequest {
        ApprovalRequest::new(
            "aws.ec2.stop",
            "aws",
            "i-0abc1234",
            json!({ "instance_id": "i-0abc1234" }),
            PermissionMode::Approval,
        )
        .with_reason("the instance is idle")
    }

    #[test]
    fn a_prompt_shows_the_required_fields_in_the_documented_order() {
        assert_eq!(
            render_prompt(&request()),
            concat!(
                "Approval required.\n",
                "  tool:      \"aws.ec2.stop\"\n",
                "  plugin:    \"aws\"\n",
                "  target:    \"i-0abc1234\"\n",
                "  arguments: {\"instance_id\":\"i-0abc1234\"}\n",
                "  reason:    \"the instance is idle\"\n",
                "  mode:      approval\n",
                "\n",
                "Type `yes` to allow this call once; anything else denies.\n",
                "> ",
            )
        );
    }

    #[test]
    fn the_mode_is_rendered_after_the_reason() {
        let prompt = render_prompt(&request());
        let reason = prompt.find("  reason:").expect("the reason line");
        let mode = prompt.find("  mode:").expect("the mode line");
        assert!(reason < mode);
        assert!(prompt.contains("mode:      approval"));
    }

    #[test]
    fn a_control_character_in_a_field_is_escaped_rather_than_printed() {
        let request = ApprovalRequest::new(
            "aws.ec2.stop",
            "aws",
            "i-0abc1234\nplugin: forged",
            json!({ "note": "\u{1b}[2J" }),
            PermissionMode::ExplicitApproval,
        )
        .with_reason("stop?\nmode:      allow\u{1b}[2J");

        let prompt = render_prompt(&request);
        assert!(!prompt.contains('\u{1b}'));
        assert!(prompt.contains("target:    \"i-0abc1234\\nplugin: forged\""));
        assert!(prompt.contains("reason:    \"stop?\\nmode:      allow\\u001b[2J\""));
        assert!(prompt.contains("\\u001b"));
        // The fixed layout has nine newlines. Escaped newlines must not add any.
        assert_eq!(prompt.matches('\n').count(), 9);
    }

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        let request = request().with_reason(r#"a "quoted" reason with \ and a tab\there"#);
        let prompt = render_prompt(&request);
        assert!(prompt.contains(r#"reason:    "a \"quoted\" reason with \\ and a tab\\there""#));
    }

    #[test]
    fn only_the_literal_word_yes_approves_after_trimming() {
        for answer in ["yes", "yes\n", " yes \n", "\tyes\t"] {
            assert!(is_approval_answer(answer), "{answer:?} should approve");
        }
        for answer in ["", "\n", "y", "YES", "Yes", "yes please", "yessir", "no"] {
            assert!(!is_approval_answer(answer), "{answer:?} should deny");
        }
        assert_eq!(APPROVAL_ANSWER, "yes");
    }

    #[test]
    fn an_empty_target_and_reason_are_still_rendered() {
        let request = ApprovalRequest::new(
            "read_file",
            "local",
            "",
            json!({}),
            PermissionMode::Approval,
        );
        let prompt = render_prompt(&request);
        assert!(prompt.contains("  target:    \"\"\n"));
        assert!(prompt.contains("  reason:    \"\"\n"));
        assert!(prompt.contains("  arguments: {}\n"));
    }
}
