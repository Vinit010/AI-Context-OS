//! The handful of substitutions a template needs, and the refusal to write an unsubstituted one.
//!
//! A template is text, not a program: there is no expression language, no loops, and no user input
//! in it. The project name and the date are the only values that change between two runs, and the
//! project name is developer-supplied, so it is escaped for Markdown before it is written.
//!
//! If a placeholder survives substitution the run fails rather than writing `{{date}}` into a
//! committed document. A template bug would otherwise become a document that looks written by a
//! person.

use super::date::Date;
use super::error::InitError;
use super::templates::TemplateName;

/// The values substituted into every template before it is written.
#[derive(Clone, Debug)]
pub(crate) struct Bindings {
    project_name: String,
    date: Date,
    template: TemplateName,
}

impl Bindings {
    /// Builds the substitutions for one run.
    pub(crate) fn new(project_name: &str, date: Date, template: TemplateName) -> Self {
        Self {
            project_name: escape_markdown(project_name),
            date,
            template,
        }
    }

    /// Applies every substitution to one template's text.
    ///
    /// `path` names the document in any error, because a message that says only
    /// "unresolved placeholder" sends the reader looking through the whole tree.
    ///
    /// Line endings are normalised to `\n` before substitution, as `docs/CONTEXT_SPEC.md` rule 11
    /// requires. The templates are embedded with `include_str!`, so their bytes are whatever git
    /// checked out: on a machine with `core.autocrlf=true` — the Git for Windows default, and the
    /// Windows CI runner — a template stored with LF arrives here with CRLF. Normalising here, at the
    /// one point every template passes through, is what keeps a generated document byte-identical
    /// across platforms and keeps the front-matter reader (which expects `---\n`) able to read it
    /// back. A lone CR is folded too, so the output has one line-ending convention, not two.
    pub(crate) fn apply(
        &self,
        text: &str,
        path: &str,
        template: &'static str,
    ) -> Result<String, InitError> {
        let rendered = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace("{{project_name}}", &self.project_name)
            .replace("{{date}}", &self.date.iso())
            .replace("{{template}}", self.template.as_str());
        match find_placeholder(&rendered) {
            Some(token) => Err(InitError::UnresolvedPlaceholder {
                path: path.to_string(),
                token,
                template,
            }),
            None => Ok(rendered),
        }
    }
}

/// The first `{{…}}` token still in the text, without its braces.
fn find_placeholder(text: &str) -> Option<String> {
    let start = text.find("{{")?;
    let rest = &text[start + 2..];
    let end = rest.find("}}")?;
    Some(rest[..end].trim().to_string())
}

/// Neutralises the Markdown characters that would change a title's meaning.
///
/// The project name comes from a directory name, so it is untrusted text that will end up in
/// headings and in a front-matter `title`. Escaping keeps `a[b]` and `a*b*` from becoming a link and
/// emphasis in the document a human then reads.
///
/// `_` is escaped only between non-word characters, because Markdown does not treat it as emphasis
/// inside a word: a project directory called `my_project` is common, and writing `my\_project` into
/// every heading to be safe would be noise in a document people read daily.
fn escape_markdown(text: &str) -> String {
    const MARKDOWN: &[char] = &['\\', '`', '*', '{', '}', '[', ']', '(', ')', '#', '|'];
    let characters: Vec<char> = text.chars().collect();
    let mut escaped = String::with_capacity(text.len());
    for (index, character) in characters.iter().enumerate() {
        let word_internal_underscore = *character == '_'
            && index > 0
            && index + 1 < characters.len()
            && characters[index - 1].is_alphanumeric()
            && characters[index + 1].is_alphanumeric();
        if MARKDOWN.contains(character) || (character == &'_' && !word_internal_underscore) {
            escaped.push('\\');
        }
        escaped.push(*character);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{Bindings, escape_markdown, find_placeholder};
    use crate::init::date::Date;
    use crate::init::templates::TemplateName;

    fn bindings() -> Bindings {
        Bindings::new("my_project", date(), TemplateName::Default)
    }

    fn date() -> Date {
        Date::from_days_since_epoch(20_269)
    }

    #[test]
    fn every_placeholder_is_substituted() {
        let rendered = bindings()
            .apply(
                "# {{project_name}}\n\ndate: {{date}}\nfrom {{template}}\n",
                "AI.md",
                "default",
            )
            .expect("renders");
        assert_eq!(rendered, "# my_project\n\ndate: 2025-06-30\nfrom default\n");
    }

    #[test]
    fn an_unknown_placeholder_is_refused_rather_than_written() {
        let error = bindings()
            .apply("project: {{project}}", "AI.md", "default")
            .expect_err("must not render");
        assert_eq!(error.code(), "INIT-007");
        assert!(
            error.to_string().contains("project"),
            "the error must name the token: {error}"
        );
    }

    #[test]
    fn a_document_with_no_placeholders_is_returned_unchanged() {
        let text = "# RULES\n\nNo substitution here.\n";
        assert_eq!(
            bindings()
                .apply(text, "RULES.md", "default")
                .expect("renders"),
            text
        );
    }

    #[test]
    fn windows_line_endings_are_normalised_to_lf() {
        // The templates are embedded with include_str!, so on a machine whose checkout uses CRLF the
        // text arriving here is CRLF. Rule 11 of docs/CONTEXT_SPEC.md requires the written document
        // to use LF, and both the front-matter reader here and init's byte-for-byte check assume it.
        let rendered = bindings()
            .apply(
                "---\r\nid: RULES-001\r\n---\r\n# {{project_name}}\r\n",
                "RULES.md",
                "default",
            )
            .expect("renders");
        assert_eq!(rendered, "---\nid: RULES-001\n---\n# my_project\n");
        assert!(
            !rendered.contains('\r'),
            "a carriage return reached the document"
        );
    }

    #[test]
    fn a_project_name_cannot_inject_markdown_into_a_heading() {
        assert_eq!(escape_markdown("a*b*"), "a\\*b\\*");
        assert_eq!(escape_markdown("[x](y)"), "\\[x\\]\\(y\\)");
        assert_eq!(escape_markdown("a#b"), "a\\#b");
        assert_eq!(escape_markdown("plain name 1.2"), "plain name 1.2");
    }

    #[test]
    fn an_underscore_inside_a_word_is_left_readable() {
        assert_eq!(escape_markdown("my_project"), "my_project");
        assert_eq!(
            escape_markdown("_private"),
            "\\_private",
            "a leading underscore is emphasis"
        );
        assert_eq!(escape_markdown("a_project."), "a_project.");
    }

    #[test]
    fn the_token_is_found_with_or_without_padding() {
        assert_eq!(find_placeholder("a {{date}} b").as_deref(), Some("date"));
        assert_eq!(find_placeholder("a {{ date }} b").as_deref(), Some("date"));
        assert_eq!(find_placeholder("no tokens"), None);
        assert_eq!(find_placeholder("an unclosed {{ brace"), None);
    }
}
