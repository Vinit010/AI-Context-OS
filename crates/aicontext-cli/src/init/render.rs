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
    pub(crate) fn apply(
        &self,
        text: &str,
        path: &str,
        template: &'static str,
    ) -> Result<String, InitError> {
        let rendered = text
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
/// headings and in a front-matter `title`. Escaping keeps `a[b]` and `a_b` from becoming emphasis
/// and italics in the document a human then reads.
fn escape_markdown(text: &str) -> String {
    const MARKDOWN: &[char] = &['\\', '`', '*', '_', '{', '}', '[', ']', '(', ')', '#', '|'];
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if MARKDOWN.contains(&character) {
            escaped.push('\\');
        }
        escaped.push(character);
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
            .apply("# {{project_name}}\n\ndate: {{date}}\nfrom {{template}}\n", "AI.md", "default")
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
            bindings().apply(text, "RULES.md", "default").expect("renders"),
            text
        );
    }

    #[test]
    fn a_project_name_cannot_inject_markdown_into_a_heading() {
        assert_eq!(escape_markdown("a_b"), "a\\_b");
        assert_eq!(escape_markdown("[x](y)"), "\\[x\\]\\(y\\)");
        assert_eq!(escape_markdown("plain name 1.2"), "plain name 1.2");
    }

    #[test]
    fn the_token_is_found_with_or_without_padding() {
        assert_eq!(find_placeholder("a {{date}} b").as_deref(), Some("date"));
        assert_eq!(find_placeholder("a {{ date }} b").as_deref(), Some("date"));
        assert_eq!(find_placeholder("no tokens"), None);
        assert_eq!(find_placeholder("an unclosed {{ brace"), None);
    }
}